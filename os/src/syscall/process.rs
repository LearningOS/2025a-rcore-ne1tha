//! Process management syscalls
use crate::mm::{VirtAddr, PTEFlags, translated_byte_buffer};
use crate::task::{count_syscall, change_program_brk, exit_current_and_run_next, suspend_current_and_run_next, get_rights_with_virt_addr, read_a_byte, write_a_byte, current_user_token, task_mmap, task_unmap};
use crate::timer::get_time_us;
use crate::config::PAGE_SIZE;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let time_us = get_time_us();
    let sec = time_us / 1_000_000;
    let usec = time_us % 1_000_000;
    let time_val = TimeVal { sec, usec };
        unsafe {
            let time_val_bytes =  core::slice::from_raw_parts(
                &time_val as *const TimeVal as *const u8,
                core::mem::size_of::<TimeVal>()
            );
            let token = current_user_token();
            let ptr = _ts as *mut u8;
            let len = core::mem::size_of::<TimeVal>();
            let buffers = translated_byte_buffer(token, ptr, len);
            let mut bytes_copied = 0;
            for buffer in buffers {
                let bytes_remaining = len - bytes_copied;
                if bytes_remaining == 0 {
                    break;
                }       
                let copy_len = buffer.len().min(bytes_remaining);
                buffer[..copy_len].copy_from_slice(&time_val_bytes[bytes_copied..bytes_copied + copy_len]);
                bytes_copied += copy_len;
            }
        }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");
    let virt_addr: VirtAddr = _id.into();
    let pteflag = get_rights_with_virt_addr(virt_addr).unwrap();
    if pteflag.contains(PTEFlags::U){    
        match _trace_request {
            0 => {
                if let Some(words) = read_a_byte(virt_addr) {
                    words as isize
                } else {
                    -1 as isize
                }
            },
            1 => {
                if write_a_byte(virt_addr, _data as u8) {
                    0 as isize
                } else {
                    -1 as isize
                }
            },
            2 => {
                count_syscall(_id) as isize
            },
        _ => -1 as isize,
        }
    } else {
        -1 as isize
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!("kernel: sys_mmap IMPLEMENTED YET!");
    // Check if start is page-aligned
    if _start % PAGE_SIZE != 0 {
        trace!("sys_mmap: start address not page-aligned");
        return -1;
    }
    // Check if prot has invalid bits set (only bits 0-2 are valid)
    if _port & !0x7 != 0 {
        trace!("sys_mmap: prot has invalid bits set: {:#x}", _port);
        return -1;
    }
    
    // Check if prot is meaningful (at least one permission bit set)
    if _port & 0x7 == 0 {
        trace!("sys_mmap: prot has no meaningful permissions");
        return -1;
    }

    task_mmap(
        _start,
        _len,
        _port,
    )

}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap IMPLEMENTED YET!");
    // Check if start is page-aligned
    if _start % PAGE_SIZE != 0 {
        trace!("sys_munmap: start address not page-aligned");
        return -1;
    }
    
    // If len is 0, it's a valid case but we don't need to unmap anything
    if _len == 0 {
        trace!("sys_munmap: len is 0, nothing to unmap");
        return 0;
    }

    task_unmap(
        _start,
        _len,
    )
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
