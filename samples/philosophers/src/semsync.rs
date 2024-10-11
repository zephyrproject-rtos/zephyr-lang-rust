// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Semaphore based sync.
//!
//! This is the simplest type of sync, which uses a single semaphore per fork.

extern crate alloc;

use alloc::vec::Vec;
use alloc::boxed::Box;

use zephyr::{
    kobj_define,
    sync::Arc,
    time::Forever,
};

use crate::{ForkSync, NUM_PHIL};

#[derive(Debug)]
pub struct SemSync {
    /// The forks for this philosopher.  This is a big excessive, as we really don't need all of
    /// them, but the ForSync code uses the index here.
    forks: [zephyr::sys::sync::Semaphore; NUM_PHIL],
}

impl ForkSync for SemSync {
    fn take(&self, index: usize) {
        self.forks[index].take(Forever).unwrap();
    }

    fn release(&self, index: usize) {
        self.forks[index].give();
    }
}

#[allow(dead_code)]
pub fn semaphore_sync() -> Vec<Arc<dyn ForkSync>> {
    let forks = SEMS.each_ref().map(|m| {
        // Each fork starts as taken.
        m.init_once((1, 1)).unwrap()
    });

    let syncers = (0..NUM_PHIL).map(|_| {
        let syncer = SemSync {
            forks: forks.clone(),
        };
        let item = Box::new(syncer) as Box<dyn ForkSync>;
        Arc::from(item)
    }).collect();

    syncers
}

kobj_define! {
    static SEMS: [StaticSemaphore; NUM_PHIL];
}
