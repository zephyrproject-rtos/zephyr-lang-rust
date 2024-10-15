// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! # sync::Mutex/sync::Condvar implementation of ForkSync
//!
//! This implementation of the Fork synchronizer uses a single data object, protected by a
//! `sync::Mutex`, and coordinated by a `sync::Condvar`.

use crate::{
    ForkSync,
    NUM_PHIL,
};
use zephyr::kobj_define;
use zephyr::sync::Mutex;
use zephyr::sync::Condvar;
// use zephyr::time::Forever;

#[derive(Debug)]
pub struct CondSync {
    /// The lock that holds the flag for each philosopher.
    lock: Mutex<[bool; NUM_PHIL]>,
    /// Condition variable to wake other threads.
    cond: Condvar,
}

impl CondSync {
    #[allow(dead_code)]
    pub fn new() -> CondSync  {
        let sys_mutex = MUTEX.init_once(()).unwrap();
        let sys_condvar = CONDVAR.init_once(()).unwrap();

        let lock = Mutex::new_from([false; NUM_PHIL], sys_mutex);
        let cond = Condvar::new_from(sys_condvar);
        CondSync { lock, cond }
    }
}

impl ForkSync for CondSync {
    fn take(&self, index: usize) {
        let mut lock = self.lock.lock().unwrap();
        while lock[index] {
            lock = self.cond.wait(lock).unwrap();
        }
        lock[index] = true;
    }

    fn release(&self, index: usize) {
        let mut lock = self.lock.lock().unwrap();
        lock[index] = false;
        // No predictible waiter, so must wake everyone.
        self.cond.notify_all();
    }
}

kobj_define! {
    static MUTEX: StaticMutex;
    static CONDVAR: StaticCondvar;
}
