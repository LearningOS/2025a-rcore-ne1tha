//! Process management syscalls
//!
use alloc::sync::Arc;

use crate::{
    fs::{open_file, OpenFlags},
    mm::{translated_refmut, translated_str, VirtAddr, PAGE_SIZE},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next,
    },
};
use crate::task::BIG_STRIDE;
use crate::task::TaskControlBlock;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel:pid[{}] sys_get_time", current_task().unwrap().pid.0);
    
    let token = current_user_token();

    
    // 处理 TimeVal 可能跨页的情况
    let mut time_val = TimeVal {
        sec: 0,
        usec: 0,
    };
    
    // 获取当前时间（微秒）
    let time_us = crate::timer::get_time_us();
    time_val.sec = time_us / 1_000_000;
    time_val.usec = time_us % 1_000_000;
    
    // 安全地将数据复制到用户空间
    let page_table = crate::mm::PageTable::from_token(token);
    let ts_va = VirtAddr::from(ts as usize);
    
    // 检查 TimeVal 是否跨页
    let start_page = ts_va.floor();
    let end_va = ts_va + core::mem::size_of::<TimeVal>();
    let end_page = VirtAddr::from(end_va).floor();
    
    if start_page == end_page {
        // 单页情况
        if let Some(pa) = page_table.translate_va(ts_va) {

            let ts_mut = pa.get_mut::<TimeVal>();
            *ts_mut = time_val;
            return 0;
        }
    } else {
        // 跨页情况 - 分别处理两个页
        let first_page_size = PAGE_SIZE - ts_va.page_offset();
        let second_page_size = core::mem::size_of::<TimeVal>() - first_page_size;
        
        // 复制第一部分到第一页
        if let Some(pa) = page_table.translate_va(ts_va) {
            unsafe {
                let src_ptr = &time_val as *const TimeVal as *const u8;
                let dst_ptr = pa.0 as *mut u8;
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, first_page_size);
            }
        } else {
            return -1;
        }
        
        // 复制第二部分到第二页
        let second_va = VirtAddr::from(ts_va.0 + first_page_size);
        if let Some(pa) = page_table.translate_va(second_va) {
            unsafe {
                let src_ptr = (&time_val as *const TimeVal as *const u8).add(first_page_size);
                let dst_ptr = pa.0 as *mut u8;
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, second_page_size);
            }
        } else {
            return -1;
        }
        
        return 0;
    }
    
    -1
}

/// mmap syscall implementation
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel:pid[{}] sys_mmap", current_task().unwrap().pid.0);
    
    // 检查长度是否为0
    if len == 0 {
        return 0;
    }
    
    // 检查start是否按页对齐
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    
    // 检查prot参数有效性
    if (prot & !0x7) != 0 {
        return -1;
    }
    
    if (prot & 0x7) == 0 {
        return -1;
    }
    
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);
    
    // 使用公共方法检查地址范围是否重叠
    if inner.memory_set.check_overlap(start_va, end_va) {
        return -1;
    }
    
    match inner.memory_set.mmap(start_va, len, prot) {
        Ok(()) => 0,
        Err(_) => -1, // 物理内存不足或其他错误
    }
}

/// munmap syscall implementation
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_munmap", current_task().unwrap().pid.0);
    
    // 检查长度是否为0
    if len == 0 {
        return 0;
    }
    
    // 检查start是否按页对齐
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    
    let start_va = VirtAddr::from(start);
    
    match inner.memory_set.munmap(start_va, len) {
        Ok(()) => 0,
        Err(_) => -1, // 存在未被映射的虚存
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
/// spawn syscall implementation
pub fn sys_spawn(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_spawn", current_task().unwrap().pid.0);
    
    let token = current_user_token();
    let path_str = translated_str(token, path);
    
    // 打开可执行文件
    let current_task = current_task().unwrap();
    let app_inode = if let Some(inode) = open_file(&path_str, OpenFlags::RDONLY) {
        inode
    } else {
        return -1;
    };
    
    let elf_data = app_inode.read_all();
    
    // 创建新任务（类似 fork 但不复制内存）
    let new_task = Arc::new(TaskControlBlock::new(elf_data.as_slice()));
    let new_pid = new_task.pid.0;
    
    // 设置父子关系
    {
        let mut current_inner = current_task.inner_exclusive_access();
        let mut new_inner = new_task.inner_exclusive_access();
        
        new_inner.parent = Some(Arc::downgrade(&current_task));
        current_inner.children.push(new_task.clone());
    }
    
    // 添加到调度器
    add_task(new_task);
    
    new_pid as isize
}
/// Set task priority syscall implementation with stride scheduling
pub fn sys_set_priority(prio: isize) -> isize {
    trace!("kernel:pid[{}] sys_set_priority", current_task().unwrap().pid.0);
    
    // 验证优先级范围：必须 >= 2
    if prio < 2 {
        return -1;
    }
    
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    
    let old_priority = inner.priority;
    
    // 更新优先级和对应的 pass 值
    inner.priority = prio;
    inner.pass = BIG_STRIDE / (prio as usize);
    
    old_priority
}
