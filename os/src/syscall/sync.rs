use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task, process::KernelMutex};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec;

/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<KernelMutex> = if !blocking {
        Some(KernelMutex::Spin(Arc::new(MutexSpin::new())))
    } else {
        Some(KernelMutex::Blocking(Arc::new(MutexBlocking::new())))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    }
}


// Deadlock detection logic for mutexes
fn check_mutex_deadlock(
    process: &Arc<crate::task::process::ProcessControlBlock>,
    requester_tid: usize,
    requested_mutex_id: usize,
) -> bool {

    let p_inner = process.inner_exclusive_access();
    let num_tasks_max = p_inner.tasks.len();
    let num_mutexes = p_inner.mutex_list.len();

    let mut work = vec![0; num_mutexes];
    let mut allocation = vec![vec![0; num_mutexes]; num_tasks_max];
    let mut need = vec![vec![0; num_mutexes]; num_tasks_max];

    for mid in 0..num_mutexes {
        if let Some(Some(KernelMutex::Blocking(mutex))) = p_inner.mutex_list.get(mid) {
            mutex.with_inner(|m_inner| {
                if let Some(owner_tid) = m_inner.owner {
                    allocation[owner_tid][mid] = 1;
                } else {
                    work[mid] = 1;
                }
                for waiting_task in m_inner.wait_queue.iter() {
                    let waiting_tid = waiting_task.inner_exclusive_access().res.as_ref().unwrap().tid;
                    need[waiting_tid][mid] = 1;
                }
            }); 
        }
    }

    need[requester_tid][requested_mutex_id] = 1;
    drop(p_inner);

    let mut finish = vec![false; num_tasks_max];
    loop {
        let find_result = (0..num_tasks_max).find(|&i| {
            if !finish[i] {
                let can_satisfy = (0..num_mutexes).all(|j| need[i][j] <= work[j]);
                can_satisfy
            } else {
                false
            }
        });

        if let Some(i) = find_result {
            for j in 0..num_mutexes {
                work[j] += allocation[i][j];
            }
            finish[i] = true;
        } else {
            break; 
        }
    }

    !finish[requester_tid]
}

/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    // 提前获取锁，检查是否需要进行死锁检测
    let (deadlock_check_enabled, is_locked, is_blocking) = {
        let process_inner = process.inner_exclusive_access();
        let mutex_item = process_inner.mutex_list[mutex_id].as_ref().unwrap();
        match mutex_item {
            KernelMutex::Blocking(m) => (
                process_inner.deadlock_detection_enabled,
                m.with_inner(|inner_data| inner_data.owner.is_some()),
                true,
            ),
            KernelMutex::Spin(_) => (false, false, false),
        }
    };

    if is_blocking && deadlock_check_enabled && is_locked {
        let requester_tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
        if check_mutex_deadlock(&process, requester_tid, mutex_id) {
            return -0xDEAD;
        }
    }
    
    // 获取 mutex 并加锁
    let mutex = process.inner_exclusive_access().mutex_list[mutex_id].as_ref().unwrap().clone();
    match mutex {
        KernelMutex::Blocking(m) => m.lock(),
        KernelMutex::Spin(m) => m.lock(),
    }
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    // 这里 clone() 是对 KernelMutex 枚举的克隆，是正确的
    let mutex = process_inner.mutex_list[mutex_id].as_ref().unwrap().clone();
    drop(process_inner);

    match mutex {
        KernelMutex::Blocking(m) => m.unlock(),
        KernelMutex::Spin(m) => m.unlock(),
    }
    0
}
// Deadlock detection logic for semaphores
fn check_semaphore_deadlock(
    process: &Arc<crate::task::ProcessControlBlock>,
    requester_tid: usize,
    requested_sem_id: usize,
) -> bool {
    let p_inner = process.inner_exclusive_access();
    let num_tasks_max = p_inner.tasks.len();
    let num_semaphores = p_inner.semaphore_list.len();

    let mut work = vec![0; num_semaphores];
    let mut allocation = vec![vec![0; num_semaphores]; num_tasks_max];
    let mut need = vec![vec![0; num_semaphores]; num_tasks_max];
    
    // Build state matrices
    for sid in 0..num_semaphores {
        if let Some(Some(sem)) = p_inner.semaphore_list.get(sid) {
            let s_inner = sem.inner.exclusive_access();
            work[sid] = s_inner.count.max(0) as usize;
            for (&owner_tid, &count) in &s_inner.owners {
                allocation[owner_tid][sid] = count;
            }
            for waiting_task in s_inner.wait_queue.iter() {
                let waiting_tid = waiting_task.inner_exclusive_access().res.as_ref().unwrap().tid;
                need[waiting_tid][sid] = 1;
            }
        }
    }

    need[requester_tid][requested_sem_id] = 1;
    drop(p_inner);

    // Run safety algorithm
    let mut finish = vec![false; num_tasks_max];
    loop {
        let find_result = (0..num_tasks_max).find(|&i| {
            if !finish[i] {
                (0..num_semaphores).all(|j| need[i][j] <= work[j])
            } else {
                false
            }
        });

        if let Some(i) = find_result {
            for j in 0..num_semaphores {
                work[j] += allocation[i][j];
            }
            finish[i] = true;
        } else {
            break;
        }
    }
    !finish[requester_tid]
}


/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    if process.inner_exclusive_access().deadlock_detection_enabled {
        let sem = process.inner_exclusive_access().semaphore_list[sem_id].as_ref().unwrap().clone();
        if sem.inner.exclusive_access().count < 1 {
            let requester_tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
            if check_semaphore_deadlock(&process, requester_tid, sem_id) {
                return -0xDEAD;
            }
        }
    }

    let sem = process.inner_exclusive_access().semaphore_list[sem_id].as_ref().unwrap().clone();
    sem.down();
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = process_inner.mutex_list[mutex_id].as_ref().unwrap().clone();
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_enable_deadlock_detect",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    if enabled > 1 {
        return -1; // Invalid argument
    }
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.deadlock_detection_enabled = enabled == 1;
    0
}
