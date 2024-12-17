// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

extern crate alloc;

use core::{pin::Pin, sync::atomic::Ordering};

use alloc::{boxed::Box, vec::Vec};
use rand::Rng;
use rand_pcg::Pcg32;
use zephyr::{
    printkln, sync::{atomic::AtomicUsize, Arc}, time::{Duration, NoWait, Tick}, timer::{Callback, CallbackTimer, SimpleTimer, StoppedTimer}
};

// Test the timers interface.  There are a couple of things this tries to test:
// 1. Do timers dynamically allocated and dropped work.
// 2. Can simple timers count properly.
// 3. Can we wait on a Simple timer.
// 4. Do callbacks work with messages and semaphores.

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("Tick frequency: {}", zephyr::time::SYS_FREQUENCY);
    timer_test();
    printkln!("All tests passed");
}

fn timer_test() {
    let mut rng = Pcg32::new(1, 1);

    // Track a global "stop" time when the entire test should be shut down.
    // let mut total_test = StoppedTimer::new().start_simple(Duration::secs_at_least(5), NoWait);
    let mut total_test = StoppedTimer::new().start_simple(Duration::secs_at_least(5), NoWait);

    // This simple timer lets us pause periodically to allow other timers to build up.
    let mut period = StoppedTimer::new().start_simple(
        Duration::millis_at_least(100),
        Duration::millis_at_least(100),
    );

    let mut simples: Vec<_> = (0..10).map(|_| TestSimple::new(&mut rng)).collect();
    let atomics: Vec<_> = (0..10).map(|_| TestAtomic::new(&mut rng)).collect();

    let mut count = 0;
    loop {
        // Wait for the period timer.
        let num = period.read_count_wait();

        if num > 1 {
            // Getting this is actually a good indicator that we've overwhelmed ourselves with
            // timers, and are stress testing things.
            printkln!("Note: Missed period ticks");
        }

        count += 1;

        if count % 10 == 0 {
            printkln!("Ticks {}", count);
        }

        if total_test.read_count() > 0 {
            break;
        }

        simples.iter_mut().for_each(|m| m.update());
    }

    // Collect all of the times they fired.
    let simple_count: usize = simples.iter().map(|s| s.count).sum();
    printkln!("Simple fired {} times", simple_count);
    let atomic_count: usize = atomics.iter().map(|s| s.count()).sum();
    printkln!("Atomics fired {} times", atomic_count);

    printkln!("Period ticks: {}", count);

    // Now that everything is done and cleaned up, allow a little time to pass to make sure there
    // are no stray timers.  We can re-use the total test timer.
    let mut total_test = total_test.stop().start_simple(Duration::millis_at_least(1), NoWait);
    total_test.read_count_wait();
}

/// Test a SimpleTimer.
///
/// This allocates a simple timer, and starts it with a small somewhat random period.  It will track
/// the total number of times that it fires when checked.
struct TestSimple {
    timer: SimpleTimer,
    _delay: Tick,
    count: usize,
}

impl TestSimple {
    fn new(rng: &mut impl Rng) -> TestSimple {
        let delay = rng.gen_range(2..16);
        TestSimple {
            timer: StoppedTimer::new()
                .start_simple(Duration::from_ticks(delay), Duration::from_ticks(delay)),
            _delay: delay,
            count: 0,
        }
    }

    /// Update from the total count from the timer itself.
    fn update(&mut self) {
        self.count += self.timer.read_count() as usize;
    }
}

/// Test a callback using an atomic counter.
///
/// This allocates a Callback timer, and uses the callback to increment an atomic value.
struct TestAtomic {
    _timer: Pin<Box<CallbackTimer<Arc<AtomicUsize>>>>,
    counter: Arc<AtomicUsize>,
}

impl TestAtomic {
    fn new(rng: &mut impl Rng) -> TestAtomic {
        let delay = rng.gen_range(2..16);
        let counter = Arc::new(AtomicUsize::new(0));
        TestAtomic {
            _timer: StoppedTimer::new().start_callback(
                Callback {
                    call: Self::expiry,
                    data: counter.clone(),
                },
                Duration::from_ticks(delay),
                Duration::from_ticks(delay),
            ),
            counter: counter.clone(),
        }
    }

    // Read the atomic count.
    fn count(&self) -> usize {
        self.counter.load(Ordering::Acquire)
    }

    /// Expire the function
    fn expiry(data: &Arc<AtomicUsize>) {
        data.fetch_add(1, Ordering::Relaxed);
    }
}
