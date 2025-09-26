//! Types related to task management
use core::usize;
use alloc::collections::BTreeMap;
use super::TaskContext;
use crate::config::TRAP_CONTEXT_BASE;
use crate::mm::{
    kernel_stack_position, MapPermission, MemorySet, PhysPageNum, VirtAddr, KERNEL_SPACE, PTEFlags,
};
use crate::trap::{trap_handler, TrapContext};
/// The task control block (TCB) of a task.
pub struct TaskControlBlock {
    /// Save task context
    pub task_cx: TaskContext,

    /// Maintain the execution status of the current process
    pub task_status: TaskStatus,
    /// Application address space
    pub memory_set: MemorySet,

    /// The phys page number of trap context
    pub trap_cx_ppn: PhysPageNum,

    /// The size(top addr) of program which is loaded from elf file
    pub base_size: usize,

    /// Heap bottom
    pub heap_bottom: usize,

    /// Program break
    pub program_brk: usize,

    /// Count of each syscall
    pub syscall_stats: BTreeMap<usize, usize>,
}

impl TaskControlBlock {
    /// Get the rights of current task with virtual address
    pub fn get_rights_with_virt_addr(&self, virt_addr: VirtAddr) -> Option<PTEFlags>  {
        self.memory_set.get_rights_with_virt_addr(virt_addr)
    }

    /// get the trap context
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }
    /// get the user token
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    /// Based on the elf info in program, build the contents of task in a new address space
    pub fn new(elf_data: &[u8], app_id: usize) -> Self {
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap()
            .ppn();
        let task_status = TaskStatus::Ready;
        // map a kernel-stack in kernel space
        let (kernel_stack_bottom, kernel_stack_top) = kernel_stack_position(app_id);
        KERNEL_SPACE.exclusive_access().insert_framed_area(
            kernel_stack_bottom.into(),
            kernel_stack_top.into(),
            MapPermission::R | MapPermission::W,
        );
        let task_control_block = Self {
            task_status,
            task_cx: TaskContext::goto_trap_return(kernel_stack_top),
            memory_set,
            trap_cx_ppn,
            base_size: user_sp,
            heap_bottom: user_sp,
            program_brk: user_sp,
            syscall_stats: BTreeMap::new(),
        };
        // prepare TrapContext in user space
        let trap_cx = task_control_block.get_trap_cx();
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            kernel_stack_top,
            trap_handler as usize,
        );
        task_control_block
    }
    /// change the location of the program break. return None if failed.
    pub fn change_program_brk(&mut self, size: i32) -> Option<usize> {
        let old_break = self.program_brk;
        let new_brk = self.program_brk as isize + size as isize;
        if new_brk < self.heap_bottom as isize {
            return None;
        }
        let result = if size < 0 {
            self.memory_set
                .shrink_to(VirtAddr(self.heap_bottom), VirtAddr(new_brk as usize))
        } else {
            self.memory_set
                .append_to(VirtAddr(self.heap_bottom), VirtAddr(new_brk as usize))
        };
        if result {
            self.program_brk = new_brk as usize;
            Some(old_break)
        } else {
            None
        }
    }
    /// Record a system call for statistics
    pub fn record_syscall(&mut self, syscall_id: usize) {
        *self.syscall_stats.entry(syscall_id).or_insert(0) += 1;
    }
    
    /// Get system call count
    pub fn get_syscall_count(&self, syscall_id: usize) -> usize {
        self.syscall_stats.get(&syscall_id).copied().unwrap_or(0)
    }

    /// Read a byte from the virtual address of current task. Return None if failed.
    pub fn read_a_byte(&self, virt_addr: VirtAddr) -> Option<u8> {
        let vpn = virt_addr.floor();
        let offset = virt_addr.page_offset();
        if let Some(pte) = self.memory_set.translate(vpn) {
            if pte.is_valid() && pte.readable() {
                let ppn = pte.ppn();
                let bytes_array = ppn.get_bytes_array();
                return Some(bytes_array[offset]);
            }
        }
        None
    }

    /// Write a byte to the virtual address of current task. Return false if failed.
    pub fn write_a_byte(&self, virt_addr: VirtAddr, data: u8) -> bool {
        let vpn = virt_addr.floor();
        let offset = virt_addr.page_offset();
        if let Some(pte) = self.memory_set.translate(vpn) {
            if pte.is_valid() && pte.writable() {
                let ppn = pte.ppn();
                let bytes_array = ppn.get_bytes_array();
                bytes_array[offset] = data;
                return true;
            }
        }
        false
    }

    /// Map a memory region for current task. Return the new program break if success, -1 if failed.
    pub fn task_mmap(&mut self, addr: usize, len: usize, port: usize) -> isize {
        let addr_start = addr;
        let addr_end = addr + len;
        let perm = match port {
            0 => MapPermission::empty(),
            1 => MapPermission::R,
            2 => MapPermission::R | MapPermission::W,
            3 => MapPermission::R | MapPermission::X,
            4 => MapPermission::R | MapPermission::X | MapPermission::W,
            _ => return -1,
        };
        self.memory_set
            .insert_framed_area(VirtAddr(addr_start), VirtAddr(addr_end), perm);
        0
    }

    /// Unmap a memory region for current task.
    pub fn task_unmap(&mut self, addr: usize, len: usize) -> isize {
        let addr_start = addr;
        let addr_end = addr + len;
        self.memory_set.remove_area(VirtAddr(addr_start), VirtAddr(addr_end))
    }
}

#[derive(Copy, Clone, PartialEq)]
/// task status: UnInit, Ready, Running, Exited
pub enum TaskStatus {
    /// uninitialized
    UnInit,
    /// ready to run
    Ready,
    /// running
    Running,
    /// exited
    Exited,
}
