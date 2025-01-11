// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]
// Cargo tries to detect configs that have typos in them.  Unfortunately, the Zephyr Kconfig system
// uses a large number of Kconfigs and there is no easy way to know which ones might conceivably be
// valid.  This prevents a warning about each cfg that is used.
#![allow(unexpected_cfgs)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use zephyr::{
    kio::spawn,
    kobj_define, printkln,
    sync::Arc,
    sys::uptime_get,
    time::{Duration, Tick},
    work::WorkQueueBuilder,
};

mod async_sem;
mod hand_worker;

/// How many philosophers.  There will be the same number of forks.
///
/// For async, this can typically be quite a bit larger than the number of threads possible.
const NUM_PHIL: usize = 6;
//const NUM_PHIL: usize = 16;

/// Size of the stack for the work queue.
const WORK_STACK_SIZE: usize = 2048;

// The dining philosophers problem is a simple example of cooperation between multiple threads.
// This implementation demonstrates a few ways that Zephyr's work-queues can be used to simulate
// this problem.

#[no_mangle]
extern "C" fn rust_main() {
    printkln!(
        "Async/work-queue dining philosophers{}",
        zephyr::kconfig::CONFIG_BOARD
    );
    printkln!("Time tick: {}", zephyr::time::SYS_FREQUENCY);

    // Create the work queue to run this.
    let worker = Arc::new(
        WorkQueueBuilder::new()
            .set_priority(1)
            .start(WORK_STACK.init_once(()).unwrap()),
    );

    // In addition, create a lower priority worker.
    let lower_worker = Arc::new(
        WorkQueueBuilder::new()
            .set_priority(5)
            .start(LOWER_WORK_STACK.init_once(()).unwrap()),
    );

    // It is important that work queues are not dropped, as they are persistent objects in the
    // Zephyr world.
    let _ = Arc::into_raw(lower_worker.clone());
    let _ = Arc::into_raw(worker.clone());

    // Run the by-hand worker.
    printkln!("Running hand-worker test");
    let work = hand_worker::phil(&lower_worker);
    let handle = spawn(work, &worker, c"hand-work");
    let stats = handle.join();
    printkln!("Done with hand-worker");
    stats.show();

    // Run the async semaphore based worker.
    printkln!("Running 'async-sem' test");
    // let handle = spawn(async_sem::phil(), &worker, c"async-sem");
    let work = async_sem::phil();
    printkln!("size of async-sem worker: {}", size_of_val(&work));
    let handle = spawn(work, &worker, c"async-sem");
    let stats = handle.join();
    printkln!("Done with 'async-sem' test");
    stats.show();

    printkln!("All threads done");
}

kobj_define! {
    static WORK_STACK: ThreadStack<WORK_STACK_SIZE>;
    static LOWER_WORK_STACK: ThreadStack<WORK_STACK_SIZE>;
}

/// Get a random delay, based on the ID of this user, and the current uptime.
fn get_random_delay(id: usize, period: usize) -> Duration {
    let tick = (uptime_get() & (usize::MAX as i64)) as usize;
    let delay = (tick / 100 * (id + 1)) & 0x1f;

    // Use one greater to be sure to never get a delay of zero.
    Duration::millis_at_least(((delay + 1) * period) as Tick)
}

/// Instead of just printint out so much information that the data just scolls by, gather
/// statistics.
// #[derive(Default)]
pub struct Stats {
    /// How many times each philosopher has gone through the loop.
    count: Vec<u16>,
    /// How much time each philosopher has spent eating.
    eating: Vec<u16>,
    /// How much time each philosopher has spent thinking.
    thinking: Vec<u16>,
}

// Implement default manually, as the fixed arrays only implement initialization up to 64 elements.
impl Default for Stats {
    fn default() -> Self {
        Self {
            count: vec![0; NUM_PHIL],
            eating: vec![0; NUM_PHIL],
            thinking: vec![0; NUM_PHIL],
        }
    }
}

impl Stats {
    fn record_eat(&mut self, index: usize, time: Duration) {
        self.eating[index] += time.to_millis() as u16;
    }

    fn record_think(&mut self, index: usize, time: Duration) {
        self.thinking[index] += time.to_millis() as u16;
        self.count[index] += 1;
    }

    fn show(&self) {
        printkln!(
            "c:{:?}, e:{:?}, t:{:?}",
            self.count,
            self.eating,
            self.thinking
        );
    }
}
