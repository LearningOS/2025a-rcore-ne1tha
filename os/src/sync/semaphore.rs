//! Semaphore

use crate::sync::UPSafeCell;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::collections::btree_map::BTreeMap;
use alloc::{collections::VecDeque, sync::Arc};

/// semaphore structure
pub struct Semaphore {
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

pub struct SemaphoreInner {
    pub count: isize,
    pub owners: BTreeMap<usize, usize>,
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    owners: BTreeMap::new(),
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let mut inner = self.inner.exclusive_access();
        let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;

        // Assuming the thread calling up must own a resource
        if let Some(count) = inner.owners.get_mut(&tid) {
            if *count > 0 {
                *count -= 1;
            }
        }

        inner.count += 1;
        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                // The woken task now acquires a resource.
                let woken_tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
                *inner.owners.entry(woken_tid).or_insert(0) += 1;
                wakeup_task(task);
            }
        }
    }

    /// down
    pub fn down(&self) {
        trace!("kernel: Semaphore::down");
        let mut inner = self.inner.exclusive_access();
        let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
        inner.count -= 1;
        if inner.count < 0 {
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
            // Upon waking, ownership has been transferred by `up`.
        } else {
            // Acquired resource immediately
            *inner.owners.entry(tid).or_insert(0) += 1;
        }
    }
}
