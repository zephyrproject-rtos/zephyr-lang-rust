// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! # sys::Mutex implementation of ForkSync
//!
//! This is a simple implementation of the Fork synchronizer that uses underlying Zephyr `k_mutex`
//! wrapped in `sys::Mutex`.  The ForkSync semantics map simply to these.

use crate::{
    ForkSync,
    NUM_PHIL,
};
use zephyr::kobj_define;
use zephyr::sys::sync::Mutex;
use zephyr::time::Forever;

type SysMutexes = [Mutex; NUM_PHIL];

/// A simple implementation of ForkSync based on underlying Zephyr sys::Mutex, which uses explicit
/// lock and release semantics.

#[derive(Debug)]
pub struct SysMutexSync {
    locks: SysMutexes,
}

impl SysMutexSync {
    #[allow(dead_code)]
    pub fn new() -> SysMutexSync  {
        let locks = MUTEXES.each_ref().map(|m| {
            m.init_once(()).unwrap()
        });
        SysMutexSync { locks }
    }
}

impl ForkSync for SysMutexSync {
    fn take(&self, index: usize) {
        self.locks[index].lock(Forever).unwrap();
    }

    fn release(&self, index: usize) {
        unsafe {
            self.locks[index].unlock().unwrap();
        }
    }
}

kobj_define! {
    static MUTEXES: [StaticMutex; NUM_PHIL];
}
