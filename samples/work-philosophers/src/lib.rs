// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

// Cargo tries to detect configs that have typos in them.  Unfortunately, the Zephyr Kconfig system
// uses a large number of Kconfigs and there is no easy way to know which ones might conceivably be
// valid.  This prevents a warning about each cfg that is used.
#![allow(unexpected_cfgs)]

extern crate alloc;

use alloc::{boxed::Box, ffi::CString, format};
use zephyr::{kobj_define, printkln, sync::Arc, sys::sync::Semaphore, time::{Duration, Forever}, work::{futures::sleep, Signal, WorkQueue, WorkQueueBuilder}};

/// How many philosophers.  There will be the same number of forks.
const _NUM_PHIL: usize = 6;

/// Size of the stack for the work queue.
const WORK_STACK_SIZE: usize = 2048;

// The dining philosophers problem is a simple example of cooperation between multiple threads.
// This implementation use one of several different underlying mechanism to support this cooperation.

// This example uses dynamic dispatch to allow multiple implementations.  The intent is to be able
// to periodically shut down all of the philosphers and start them up with a differernt sync
// mechanism.  This isn't implemented yet.

/// The philosophers use a fork synchronization mechanism.  Essentially, this is 6 locks, and will be
/// implemented in a few different ways to demonstrate/test different mechanmism in Rust.  All of
/// them implement The ForkSync trait which provides this mechanism.
/*
trait ForkSync: core::fmt::Debug + Sync + Send {
    /// Take the given fork.  The are indexed the same as the philosopher index number.  This will
    /// block until the fork is released.
    fn take(&self, index: usize);

    /// Release the given fork.  Index is the same as take.
    fn release(&self, index: usize);
}
*/

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("Hello world from Rust on {}",
              zephyr::kconfig::CONFIG_BOARD);
    printkln!("Time tick: {}", zephyr::time::SYS_FREQUENCY);

    // Create the work queue to run this.
    let worker = Box::new(WorkQueueBuilder::new()
                          .set_priority(1)
                          .start(WORK_STACK.init_once(()).unwrap()));

    // In addition, create a lower priority worker.
    let lower_worker = Arc::new(WorkQueueBuilder::new()
                                .set_priority(5)
                                .start(LOWER_WORK_STACK.init_once(()).unwrap()));

    // It is important that work queues are not dropped, as they are persistent objects in the
    // Zephyr world.
    let _ = Arc::into_raw(lower_worker.clone());

    // Create a semaphore the phil thread can wait for, part way through its work.
    let sem = Arc::new(Semaphore::new(0, 1).unwrap());

    let th = phil_thread(0, sem.clone(), lower_worker.clone());
    printkln!("Size of phil thread: {}", core::mem::size_of_val(&th));
    // TODO: How to do as much on the stack as we can, for now, don't worry too much.
    // let mut th = pin!(th);

    let th = zephyr::kio::spawn(th, &worker, c"phil-worker");

    // As we no longer use the worker, it is important that it never be dropped.  Leaking the box
    // ensures that it will not be deallocated.
    Box::leak(worker);

    // Sleep a bit to allow the worker to run to where it is waiting.
    zephyr::time::sleep(Duration::millis_at_least(1_000));
    printkln!("Giving the semaphore");
    sem.give();

    let result = th.join();
    printkln!("th result: {:?}", result);

    /*
    // TODO: The allocated Arc doesn't work on all Zephyr platforms, but need to make our own copy
    // of alloc::task
    let waker = Arc::new(PWaker).into();
    let mut cx = Context::from_waker(&waker);

    // Run the future to completion.
    loop {
        match th.as_mut().poll(&mut cx) {
            Poll::Pending => todo!(),
            Poll::Ready(_) => break,
        }
    }
    */
    printkln!("All threads done");

    /*
    let stats = Arc::new(Mutex::new_from(Stats::default(), STAT_MUTEX.init_once(()).unwrap()));

    let syncers = get_syncer();

    printkln!("Pre fork");

    for (i, syncer) in (0..NUM_PHIL).zip(syncers.into_iter()) {
        let child_stat = stats.clone();
        let thread = PHIL_THREADS[i].init_once(PHIL_STACKS[i].init_once(()).unwrap()).unwrap();
        thread.spawn(move || {
            phil_thread(i, syncer, child_stat);
        });
    }

    let delay = Duration::secs_at_least(10);
    loop {
        // Periodically, printout the stats.
        zephyr::time::sleep(delay);
        stats.lock().unwrap().show();
    }
    */
}

async fn phil_thread(n: usize, sem: Arc<Semaphore>, lower_thread: Arc<WorkQueue>) -> usize {
    printkln!("Child {} started", n);
    show_it(&sem).await;
    sleep(Duration::millis_at_least(1000)).await;
    printkln!("Child {} done sleeping", n);

    // Lastly fire off something on the other worker thread.
    let sig = Arc::new(Signal::new().unwrap());
    let sig2 = sig.clone();

    // To help with debugging, we can name these, but it must be a `&'static Cstr`.  We'll build a
    // CString (on the heap), and then leak it to be owned by the work.  The leaking while keeping
    // the `&'static CStr` is a bit messy.
    let name = CString::new(format!("phil-{}", n)).unwrap();
    let name = Box::leak(name.into_boxed_c_str());

    // It _should_ be safe to drop the child, as it uses an internal clone of the Arc to keep it
    // around as long as there is a worker.
    let _child = Box::new(zephyr::kio::spawn(async move {
        sleep(Duration::millis_at_least(800)).await;
        sig2.raise(12345).unwrap();
    }, &lower_thread, name));

    // Wait for the signal.
    let num = sig.wait_async(Forever).await.unwrap();
    printkln!("Signaled value: {}", num);

    42
}

async fn show_it(sem: &Semaphore) {
    for i in 0..10 {
        sleep(Duration::millis_at_least(1)).await;
        printkln!("Tick: {i}");

        // Wait after 5 for the semaphore to be signaled.
        if i == 4 {
            printkln!("Waiting for semaphore");
            sem.take_async(Forever).await.unwrap();
            printkln!("Done waiting");
        }
    }
}

kobj_define! {
    static WORK_STACK: ThreadStack<WORK_STACK_SIZE>;
    static LOWER_WORK_STACK: ThreadStack<WORK_STACK_SIZE>;
}

/*
fn phil_thread(n: usize, syncer: Arc<dyn ForkSync>, stats: Arc<Mutex<Stats>>) {
    printkln!("Child {} started: {:?}", n, syncer);

    // Determine our two forks.
    let forks = if n == NUM_PHIL - 1 {
        // Per Dijkstra, the last phyilosopher needs to reverse forks, or we deadlock.
        (0, n)
    } else {
        (n, n+1)
    };

    loop {
        {
            // printkln!("Child {} hungry", n);
            // printkln!("Child {} take left fork", n);
            syncer.take(forks.0);
            // printkln!("Child {} take right fork", n);
            syncer.take(forks.1);

            let delay = get_random_delay(n, 25);
            // printkln!("Child {} eating ({} ms)", n, delay);
            sleep(delay);
            stats.lock().unwrap().record_eat(n, delay);

            // Release the forks.
            // printkln!("Child {} giving up forks", n);
            syncer.release(forks.1);
            syncer.release(forks.0);

            let delay = get_random_delay(n, 25);
            // printkln!("Child {} thinking ({} ms)", n, delay);
            sleep(delay);
            stats.lock().unwrap().record_think(n, delay);
        }
    }
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
        self.eating[index] += time.to_millis();
    }

    fn record_think(&mut self, index: usize, time: Duration) {
        self.thinking[index] += time.to_millis();
        self.count[index] += 1;
    }

    fn show(&self) {
        printkln!("c:{:?}, e:{:?}, t:{:?}", self.count, self.eating, self.thinking);
    }
}

kobj_define! {
    static PHIL_THREADS: [StaticThread; NUM_PHIL];
    static PHIL_STACKS: [ThreadStack<PHIL_STACK_SIZE>; NUM_PHIL];

    static STAT_MUTEX: StaticMutex;
}
*/
