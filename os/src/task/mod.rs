//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::__switch;
use crate::mm::{PTEFlags, VirtPageNum, VirtAddr};
pub use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                })
            },
        }
    };
}

impl TaskManager {
    /// Get the rights of current task with virtual address
    fn get_rights_with_virt_addr(&self, virt_addr: VirtAddr) -> Option<PTEFlags> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].get_rights_with_virt_addr(virt_addr)
    }

    ///add count of syscall for task id
    fn add_count_syscall(&self, _id: usize) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].record_syscall(_id);
    }

    ///get count of syscall for task id
    fn count_syscall(&self, _id: usize) -> usize {
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].get_syscall_count(_id)
    }

    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        (*next_task).task_status = TaskStatus::Running;
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }

    /// Read a byte from the virtual address of current task
    fn read_a_byte(&self, virt_addr: VirtAddr) -> Option<u8> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].read_a_byte(virt_addr)
    }

    /// Write a byte to the virtual address of current task
    fn write_a_byte(&self, virt_addr: VirtAddr, data: u8) -> bool {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].write_a_byte(virt_addr, data)
    }

    /// Map a memory region for current task
    fn task_mmap(&self, addr: usize, len: usize, port: usize) -> isize {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_mmap(addr, len, port)
    }

    fn task_unmap(&self, addr: usize, len: usize) -> isize {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_unmap(addr, len)
    }  
}

/// Check if the address range [addr, addr + len) has an address existing in current task
pub fn check_if_address_exist(addr: usize, len: usize) -> bool {
    let first_page = VirtAddr::from(addr).floor();
    let last_page = VirtAddr::from(addr + len - 1).floor();
    for vpn in first_page.0..=last_page.0 {
        if TASK_MANAGER
            .get_rights_with_virt_addr(VirtAddr::from(VirtPageNum(vpn)))
            .is_some()
        {
            return true;
        }
    }
    false
}


/// Check if the address range [addr, addr + len) have an address inexisting in current task
pub fn check_if_address_inexist(addr: usize, len: usize) -> bool {
    let first_page = VirtAddr::from(addr).floor();
    let last_page = VirtAddr::from(addr + len - 1).floor();
    for vpn in first_page.0..=last_page.0 {
        if TASK_MANAGER
            .get_rights_with_virt_addr(VirtAddr::from(VirtPageNum(vpn)))
            .is_none()
        {
            return true;
        }
    }
    false
}


/// Unmap a memory region for current task
pub fn task_unmap(addr: usize, len: usize) -> isize {
    if check_if_address_inexist(addr, len) {
        return -1;
    }
    TASK_MANAGER.task_unmap(addr, len)
}

/// Map a memory region for current task
pub fn task_mmap(addr: usize, len: usize, port: usize) -> isize {
    if check_if_address_exist(addr, len) {
        return -1;
    }
    TASK_MANAGER.task_mmap(addr, len, port)
}

/// Write a byte to the virtual address of current task
pub fn write_a_byte(virt_addr: VirtAddr, data: u8) -> bool {
    TASK_MANAGER.write_a_byte(virt_addr, data)
}

/// Read a byte from the virtual address of current task
pub fn read_a_byte(virt_addr: VirtAddr) -> Option<u8> {
    TASK_MANAGER.read_a_byte(virt_addr)
}

/// Get the rights of current task with virtual address
pub fn get_rights_with_virt_addr(virt_addr: VirtAddr) -> Option<PTEFlags> {
    TASK_MANAGER.get_rights_with_virt_addr(virt_addr)
}


/// Add count of syscall for current task
pub fn add_count_syscall(id: usize) {
    TASK_MANAGER.add_count_syscall(id);
}

/// Get count of syscall for current task
pub fn count_syscall(id: usize) -> usize {
    TASK_MANAGER.count_syscall(id)
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}
