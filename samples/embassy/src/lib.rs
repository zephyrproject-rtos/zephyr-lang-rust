// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

extern crate alloc;

use core::ffi::c_int;

#[cfg(feature = "executor-thread")]
use embassy_executor::Executor;

#[cfg(feature = "executor-zephyr")]
use zephyr::embassy::Executor;

use alloc::format;
use embassy_executor::{SendSpawner, Spawner};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use log::info;
use static_cell::StaticCell;
use zephyr::raw;
use zephyr::{
    kconfig::CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC, kobj_define, printkln, raw::k_cycle_get_64,
};

/// Maximum number of threads to spawn. As this is async, these do not each need a stack.
const NUM_THREADS: usize = 6;

const THREAD_STACK_SIZE: usize = 2048;

static EXECUTOR_LOW: StaticCell<Executor> = StaticCell::new();
static EXECUTOR_MAIN: StaticCell<Executor> = StaticCell::new();

static LOW_SPAWNER: Channel<CriticalSectionRawMutex, SendSpawner, 1> = Channel::new();

// The main thread priority.
const MAIN_PRIO: c_int = 2;
const LOW_PRIO: c_int = 5;

#[no_mangle]
extern "C" fn rust_main() {
    unsafe {
        zephyr::set_logger().unwrap();
    }

    // Set our own priority.
    unsafe {
        raw::k_thread_priority_set(raw::k_current_get(), MAIN_PRIO);
    }

    // Start up the low priority thread.
    let mut thread = LOW_THREAD
        .init_once(LOW_STACK.init_once(()).unwrap())
        .unwrap();
    thread.set_priority(LOW_PRIO);
    thread.spawn(move || {
        low_executor();
    });

    info!(
        "Starting Embassy executor on {}",
        zephyr::kconfig::CONFIG_BOARD
    );

    let executor = EXECUTOR_MAIN.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(main(spawner)).unwrap();
    })
}

/// The low priority executor.
fn low_executor() -> ! {
    let executor = EXECUTOR_LOW.init(Executor::new());
    executor.run(|spawner| {
        LOW_SPAWNER.try_send(spawner.make_send()).ok().unwrap();
    })
}

#[embassy_executor::task]
async fn main(spawner: Spawner) {
    info!("Benchmark begin");

    let low_spawner = LOW_SPAWNER.receive().await;

    let tester = ThreadTests::new(NUM_THREADS);

    tester.run(spawner, low_spawner, Command::Empty).await;
    tester.run(spawner, low_spawner, Command::Empty).await;
    tester
        .run(spawner, low_spawner, Command::PingPong(10_000))
        .await;
}

/// Async task tests.
///
/// For each test, we have a set of threads that do work, a "high priority" thread higher than those
/// and a low priority thread, lower than any of those.  This is used to test operations in both a
/// fast-path (message or semaphore always available), and slow path (thread must block and be woken
/// by message coming in).  Generally, this is determined by whether high or low priority tasks are
/// providing the data.
struct ThreadTests {
    /// How many threads were actually asked for.
    count: usize,

    /// Forward channels, acts as semaphores forward.
    forward: heapless::Vec<Channel<CriticalSectionRawMutex, (), 1>, NUM_THREADS>,

    back: Channel<CriticalSectionRawMutex, (), 1>,

    /// Each worker sends results back through this.
    answers: Channel<CriticalSectionRawMutex, Answer, 1>,
}

impl ThreadTests {
    /// Construct the tests.
    ///
    /// Note that this uses a single StaticCell, and therefore can only be called once.
    fn new(count: usize) -> &'static Self {
        static THIS: StaticCell<ThreadTests> = StaticCell::new();
        let this = THIS.init(Self {
            count,
            forward: heapless::Vec::new(),
            back: Channel::new(),
            answers: Channel::new(),
        });

        for _ in 0..count {
            this.forward.push(Channel::new()).ok().unwrap();
        }

        this
    }

    async fn run(&'static self, spawner: Spawner, low_spawner: SendSpawner, command: Command) {
        let desc = format!("{:?}", command);
        let timer = BenchTimer::new(&desc, self.count * command.get_count());

        let mut answers: heapless::Vec<Option<usize>, NUM_THREADS> = heapless::Vec::new();
        for _ in 0..self.count {
            answers.push(None).unwrap();
        }
        let mut low = false;
        let mut msg_count = (1 + self.count) as isize;

        // Fire off all of the workers.
        for id in 0..self.count {
            spawner.spawn(worker(self, id, command)).unwrap();
        }

        // And the "low" priority thread (which isn't lower at this time).
        low_spawner.spawn(low_task(self, command)).unwrap();
        //let _ = low_spawner;
        //spawner.spawn(low_task(self, command)).unwrap();

        // Now wait for all of the responses.
        loop {
            match self.answers.receive().await {
                Answer::Worker { id, count } => {
                    if answers[id].replace(count).is_some() {
                        panic!("Multiple results from worker {}", id);
                    }
                    msg_count -= 1;
                    if msg_count <= 0 {
                        break;
                    }
                }

                Answer::Low => {
                    if low {
                        panic!("Multiple result from 'low' worker");
                    }
                    low = true;

                    msg_count -= 1;
                    if msg_count <= 0 {
                        break;
                    }
                }
            }
        }

        if msg_count != 0 {
            panic!("Invalid number of replies\n");
        }

        timer.stop();
    }
}

/// An individual work thread.  This performs the specified operation, returning the result.
#[embassy_executor::task(pool_size = NUM_THREADS)]
async fn worker(this: &'static ThreadTests, id: usize, command: Command) {
    let mut total = 0;

    match command {
        Command::Empty => {
            // Nothing to do.
        }
        Command::PingPong(count) => {
            // The ping pong test, reads messages from in indexed channel (one for each worker), and
            // replies to a shared channel.
            for _ in 0..count {
                this.forward[id].receive().await;
                this.back.send(()).await;
                total += 1;
            }
        }
    }

    this.answers.send(Answer::Worker { id, count: total }).await;
}

/// The low priority worker for the given command.  Exits when finished.
#[embassy_executor::task]
async fn low_task(this: &'static ThreadTests, command: Command) {
    match command {
        Command::Empty => {
            // Nothing to do.
        }
        Command::PingPong(count) => {
            // Each worker expects a message to tell it to work, and will reply with its answer.
            for _ in 0..count {
                for forw in &this.forward {
                    forw.send(()).await;
                    this.back.receive().await;
                }
            }
        }
    }

    this.answers.send(Answer::Low).await;
}

#[derive(Copy, Clone, Debug)]
enum Command {
    /// The empty test.  Does nothing, but invokes everything.  Useful to determine overhead.
    Empty,
    /// Pong test.  Each thread waits for a message on its own channel, and then replies on a shared
    /// channel to a common worker that is performing these operations.
    PingPong(usize),
}

impl Command {
    /// Return how many operations this particular command invokes.
    fn get_count(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::PingPong(count) => count,
        }
    }
}

#[derive(Debug)]
enum Answer {
    /// A worker has finished it's processing.
    Worker {
        /// What is the id of this worker.
        id: usize,
        /// Operation count.
        count: usize,
    },
    /// The low priority task has completed.
    Low,
}

// TODO: Put this benchmarking stuff somewhere useful.
fn now() -> u64 {
    unsafe { k_cycle_get_64() }
}

/// Timing some operations.
///
/// To use:
/// ```
/// /// 500 is the number of iterations happening.
/// let timer = BenchTimer::new("My thing", 500);
/// // operations
/// timer.stop("Thing being timed");
/// ```
pub struct BenchTimer<'a> {
    what: &'a str,
    start: u64,
    count: usize,
}

impl<'a> BenchTimer<'a> {
    pub fn new(what: &'a str, count: usize) -> Self {
        Self {
            what,
            start: now(),
            count,
        }
    }

    pub fn stop(self) {
        let stop = now();
        let time =
            (stop - self.start) as f64 / (CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC as f64) * 1000.0;
        let time = if self.count > 0 {
            time / (self.count as f64) * 1000.0
        } else {
            0.0
        };

        printkln!("    {:8.3} us, {} of {}", time, self.count, self.what);
    }
}

kobj_define! {
    static LOW_THREAD: StaticThread;
    static LOW_STACK: ThreadStack<THREAD_STACK_SIZE>;
}
