//! File and filesystem-related syscalls
use crate::fs::{open_file, OpenFlags, Stat};
use crate::mm::{translated_byte_buffer, translated_str, translated_refmut, UserBuffer};
use crate::task::{current_task, current_user_token};





pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// linkat syscall implementation
pub fn sys_linkat(oldpath: *const u8, newpath: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_linkat", current_task().unwrap().pid.0);
    
    let token = current_user_token();
    let old_path = translated_str(token, oldpath);
    let new_path = translated_str(token, newpath);
    
    // 检查是否同名
    if old_path == new_path {
        return -1;
    }
    
    // 获取根inode
    let root_inode = crate::fs::inode::ROOT_INODE.clone();
    
    // 创建硬链接
    match root_inode.link(&old_path, &new_path) {
        Ok(()) => 0,
        Err(()) => -1,
    }
}

/// unlinkat syscall implementation  
pub fn sys_unlinkat(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_unlinkat", current_task().unwrap().pid.0);
    
    let token = current_user_token();
    let path_str = translated_str(token, path);
    
    // 获取根inode
    let root_inode = crate::fs::inode::ROOT_INODE.clone();
    
    // 取消链接
    match root_inode.unlink(&path_str) {
        Ok(()) => 0,
        Err(()) => -1,
    }
}

/// fstat syscall implementation
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    trace!("kernel:pid[{}] sys_fstat", current_task().unwrap().pid.0);
    
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    
    if fd >= inner.fd_table.len() {
        return -1;
    }
    
    if let Some(file) = &inner.fd_table[fd] {
        // 对于 OSInode，获取文件状态
        if let Some(os_inode) = file.as_any().downcast_ref::<crate::fs::inode::OSInode>() {
            let stat = os_inode.get_stat();
            
            // 将状态信息复制到用户空间
            let st_mut = translated_refmut(token, st);
            *st_mut = stat;
            
            0
        } else {
            -1  // 不是普通文件，不支持
        }
    } else {
        -1  // 文件描述符无效
    }
}
