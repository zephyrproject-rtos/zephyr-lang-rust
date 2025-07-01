// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::Duration;
use static_cell::StaticCell;
use zephyr::{embassy::Executor, printkln, sync::Arc, sys::uptime_get};

mod async_sem;

/// How many philosophers.  There will be the same number of forks.
const NUM_PHIL: usize = 6;

// The dining philosophers problem is a simple example of cooperation between multiple threads.
// This implementation demonstrates a few ways that Zephyr's work-queues can be used to simulate
// this problem.

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("Async dining philosophers{}", zephyr::kconfig::CONFIG_BOARD);
    printkln!("Time tick: {}", zephyr::time::SYS_FREQUENCY);

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(main(spawner)).unwrap();
    })
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

type ResultSignal = Signal<CriticalSectionRawMutex, Arc<Mutex<CriticalSectionRawMutex, Stats>>>;
static RESULT_SIGNAL: ResultSignal = Signal::new();

#[embassy_executor::task]
async fn main(spawner: Spawner) -> () {
    // First run the async semaphore based one.
    printkln!("Running 'async-sem' test");
    spawner
        .spawn(async_sem::phil(spawner, &RESULT_SIGNAL))
        .unwrap();

    let stats = RESULT_SIGNAL.wait().await;
    printkln!("Done with 'async-sem' test");
    stats.lock().await.show();

    printkln!("All threads done");
}

/// Get a random delay, based on the ID of this user, and the current uptime.
fn get_random_delay(id: usize, period: usize) -> Duration {
    let tick = (uptime_get() & (usize::MAX as i64)) as u64;
    let delay = (tick / 100 * (id as u64 + 1)) & 0x1f;

    // Use one greater to be sure to never get a delay of zero.
    Duration::from_millis((delay + 1) * (period as u64))
}

/// Instead of just printint out so much information that the data just scolls by, gather
/// statistics.
#[derive(Default)]
struct Stats {
    /// How many times each philosopher has gone through the loop.
    count: [u64; NUM_PHIL],
    /// How much time each philosopher has spent eating.
    eating: [u64; NUM_PHIL],
    /// How much time each philosopher has spent thinking.
    thinking: [u64; NUM_PHIL],
}

impl Stats {
    fn record_eat(&mut self, index: usize, time: Duration) {
        self.eating[index] += time.as_millis();
    }

    fn record_think(&mut self, index: usize, time: Duration) {
        self.thinking[index] += time.as_millis();
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
