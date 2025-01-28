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
use zephyr::time::NoWait;
use zephyr::{
    kconfig::CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC,
    kio::{spawn, yield_now},
    kobj_define, printkln,
    raw::{k_cycle_get_64, k_yield},
    sync::{
        channel::{bounded, unbounded, Receiver, Sender},
        Arc,
    },
    sys::sync::Semaphore,
    time::Forever,
    work::{WorkQueue, WorkQueueBuilder},
};

/// How many threads to run in the tests.
const NUM_THREADS: usize = 6;

/// Stack size to use for the threads.
const THREAD_STACK_SIZE: usize = 2048;

/// Stack size to use for the work queue.
const WORK_STACK_SIZE: usize = 2048;

#[no_mangle]
extern "C" fn rust_main() {
    let tester = ThreadTests::new(NUM_THREADS);
    tester.run(Command::Empty);
    tester.run(Command::SimpleSem(10_000));
    tester.run(Command::SimpleSemAsync(10_000));
    tester.run(Command::SimpleSemYield(10_000));
    tester.run(Command::SimpleSemYieldAsync(10_000));
    tester.run(Command::SemWait(10_000));
    tester.run(Command::SemWaitAsync(10_000));
    tester.run(Command::SemWaitSameAsync(10_000));
    tester.run(Command::SemHigh(10_000));

    printkln!("Done with all tests\n");
    tester.leak();
}

/// Thread-based tests use this information to manage the test and results.
///
/// For each test, we have a set of threads that do work, a "high priority" thread higher than any
/// of those, and a low-priority thread, lower priority than any of those.  This is used to test
/// operations in both a fast-path (message or sempahore always available), and slow path (thread
/// must block and be woken by message coming in).  Generally, this is determined by whether high or
/// low priority task is providing the data.
struct ThreadTests {
    /// Each test thread gets a semaphore, to use as appropriate for that test.
    sems: Vec<Arc<Semaphore>>,

    /// Each test also has a message queue, for testing, that it has sender and receiver for.
    chans: Vec<ChanPair<u32>>,

    /// In addition, each test has a channel it owns the receiver for that listens for commands
    /// about what test to run.
    commands: Vec<Sender<Command>>,

    /// Low and high also take commands.
    low_command: Sender<Command>,
    high_command: Sender<Command>,

    /// A work queue for the main runners.
    workq: Arc<WorkQueue>,

    /// The test also all return their result to the main.  The threads Send, the main running
    /// receives.
    results: ChanPair<Result>,
}

impl ThreadTests {
    /// Construct a new set of tests, firing up the threads.
    ///
    /// Note that the count, although a parameter, must be less than or equal to the size of the
    /// statically defined threads.
    fn new(count: usize) -> Arc<Self> {
        let (low_send, low_recv) = bounded(1);
        let (high_send, high_recv) = bounded(1);

        let workq = Arc::new(
            WorkQueueBuilder::new()
                .set_priority(5)
                .set_no_yield(true)
                .start(WORK_STACK.init_once(()).unwrap()),
        );

        let mut result = Self {
            sems: Vec::new(),
            chans: Vec::new(),
            commands: Vec::new(),
            results: ChanPair::new_unbounded(),
            low_command: low_send,
            high_command: high_send,
            workq,
        };

        let mut thread_commands = Vec::new();

        for _ in 0..count {
            let sem = Arc::new(Semaphore::new(0, u32::MAX).unwrap());
            result.sems.push(sem.clone());

            let chans = ChanPair::new_bounded(1);
            result.chans.push(chans.clone());

            let (csend, crecv) = bounded(1);
            result.commands.push(csend);
            thread_commands.push(crecv);
        }

        // Result is initialized, move it into the Arc.
        let result = Arc::new(result);

        for i in 0..count {
            let result2 = result.clone();
            let cmd2 = thread_commands[i].clone();
            let mut thread = TEST_THREADS[i]
                .init_once(TEST_STACKS[i].init_once(()).unwrap())
                .unwrap();
            // Main test threads run at priority 0.
            thread.set_priority(5);
            thread.spawn(move || {
                Self::worker(result2, cmd2, i);
            });
        }

        // And fire up the low and high priority workers.
        let result2 = result.clone();
        let mut thread = LOW_THREAD
            .init_once(LOW_STACK.init_once(()).unwrap())
            .unwrap();
        thread.set_priority(6);
        thread.spawn(move || {
            Self::low_runner(result2, low_recv);
        });

        let result2 = result.clone();
        let mut thread = HIGH_THREAD
            .init_once(HIGH_STACK.init_once(()).unwrap())
            .unwrap();
        thread.set_priority(4);
        thread.spawn(move || {
            Self::high_runner(result2, high_recv);
        });

        result
    }

    /// At the end of the tests, leak the work queue.
    fn leak(&self) {
        let _ = Arc::into_raw(self.workq.clone());
    }

    fn run(&self, command: Command) {
        // printkln!("Running {:?}", command);
        let start = now();

        let mut results = vec![None; self.chans.len()];
        let mut low = false;
        let mut high = false;
        let mut msg_count = (2 + results.len()) as isize;

        // Order is important here, start with the highest priority things.
        self.high_command.send(command.clone()).unwrap();
        for cmd in &self.commands {
            cmd.send(command.clone()).unwrap();
        }
        self.low_command.send(command.clone()).unwrap();

        while let Ok(cmd) = self.results.receiver.recv() {
            match cmd {
                Result::Worker { id, count } => {
                    if results[id].replace(count).is_some() {
                        panic!("Multiple result from worker {}", id);
                    }
                    msg_count -= 1;
                    if msg_count <= 0 {
                        break;
                    }
                }
                Result::Low => {
                    if low {
                        panic!("Multiple results from 'low' worker");
                    }
                    low = true;

                    msg_count -= 1;
                    if msg_count <= 0 {
                        break;
                    }
                }
                Result::High => {
                    if high {
                        panic!("Multiple results from 'high' worker");
                    }
                    high = true;

                    msg_count -= 1;
                    if msg_count <= 0 {
                        break;
                    }
                }
            }
        }

        if msg_count != 0 {
            panic!("Receiver channel is closed");
        }
        let stop = now();

        let time = (stop - start) as f64 / (CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC as f64) * 1000.0;

        let total: usize = results.iter().map(|x| x.unwrap_or(0)).sum();

        let time = if total > 0 {
            time / (total as f64) * 1000.0
        } else {
            0.0
        };

        printkln!("    {:8.3} us, {} of {:?}", time, total, command);
    }

    /// Run the thread worker itself.
    fn worker(this: Arc<Self>, command: Receiver<Command>, id: usize) {
        while let Ok(cmd) = command.recv() {
            let mut total = 0;
            // printkln!("Command {}: {:?}", id, cmd);
            match cmd {
                Command::Empty => {
                    // Nothing to do.
                }
                Command::SimpleSem(count) => {
                    this.simple_sem(&this.sems[id], count, &mut total);
                }
                Command::SimpleSemYield(count) => {
                    this.simple_sem_yield(&this.sems[id], count, &mut total);
                }
                Command::SemWait(count) => {
                    this.sem_take(&this.sems[id], count, &mut total);
                }
                Command::SemHigh(count) => {
                    this.sem_take(&this.sems[id], count, &mut total);
                }

                // For the async commands, spawn this on the worker thread and don't reply
                // ourselves.
                Command::SimpleSemAsync(count) => {
                    spawn(
                        Self::simple_sem_async(this.clone(), id, this.sems[id].clone(), count),
                        &this.workq,
                        c"worker",
                    );
                    continue;
                }

                Command::SimpleSemYieldAsync(count) => {
                    spawn(
                        Self::simple_sem_yield_async(
                            this.clone(),
                            id,
                            this.sems[id].clone(),
                            count,
                        ),
                        &this.workq,
                        c"worker",
                    );
                    continue;
                }

                Command::SemWaitAsync(count) => {
                    spawn(
                        Self::sem_take_async(this.clone(), id, this.sems[id].clone(), count),
                        &this.workq,
                        c"worker",
                    );
                    continue;
                }

                Command::SemWaitSameAsync(count) => {
                    spawn(
                        Self::sem_take_async(this.clone(), id, this.sems[id].clone(), count),
                        &this.workq,
                        c"worker",
                    );
                    spawn(
                        Self::sem_giver_async(this.clone(), this.sems.clone(), count),
                        &this.workq,
                        c"giver",
                    );
                    continue;
                }
            }

            this.results
                .sender
                .send(Result::Worker { id, count: total })
                .unwrap();
        }
    }

    /// A simple semaphore test, worker thread version.
    #[inline(never)]
    fn simple_sem(&self, sem: &Semaphore, count: usize, total: &mut usize) {
        for _ in 0..count {
            sem.give();
            sem.take(NoWait).unwrap();
            *total += 1;
        }
    }

    async fn simple_sem_async(this: Arc<Self>, id: usize, sem: Arc<Semaphore>, count: usize) {
        for _ in 0..count {
            sem.give();
            sem.take_async(NoWait).await.unwrap();
        }

        this.results
            .sender
            .send_async(Result::Worker { id, count })
            .await
            .unwrap();
    }

    async fn simple_sem_yield_async(this: Arc<Self>, id: usize, sem: Arc<Semaphore>, count: usize) {
        for _ in 0..count {
            sem.give();
            sem.take_async(NoWait).await.unwrap();
            yield_now().await;
        }

        this.results
            .sender
            .send_async(Result::Worker { id, count })
            .await
            .unwrap();
    }

    /// A simple semaphore test, worker thread version, with a yield
    fn simple_sem_yield(&self, sem: &Semaphore, count: usize, total: &mut usize) {
        for _ in 0..count {
            sem.give();
            sem.take(NoWait).unwrap();
            *total += 1;
            unsafe { k_yield() };
        }
    }

    /// Semaphores where we should be waiting.
    fn sem_take(&self, sem: &Semaphore, count: usize, total: &mut usize) {
        for _ in 0..count {
            sem.take(Forever).unwrap();
            *total += 1;
        }
    }

    async fn sem_take_async(this: Arc<Self>, id: usize, sem: Arc<Semaphore>, count: usize) {
        for _ in 0..count {
            sem.take_async(Forever).await.unwrap();
        }

        this.results
            .sender
            .send_async(Result::Worker { id, count })
            .await
            .unwrap();
    }

    async fn sem_giver_async(this: Arc<Self>, sems: Vec<Arc<Semaphore>>, count: usize) {
        for _ in 0..count {
            for sem in &sems {
                sem.give();
                // Yield after each, forcing us back into the work queue, to allow the workers to
                // run, and block.
                yield_now().await;
            }
        }

        // No report.
        let _ = this;
    }

    /// And the low priority worker.
    fn low_runner(this: Arc<Self>, command: Receiver<Command>) {
        let _ = this;
        while let Ok(cmd) = command.recv() {
            match cmd {
                Command::Empty => (),
                Command::SimpleSem(_) => (),
                Command::SimpleSemAsync(_) => (),
                Command::SimpleSemYield(_) => (),
                Command::SimpleSemYieldAsync(_) => (),
                Command::SemWait(count) | Command::SemWaitAsync(count) => {
                    // The low-priority thread does all of the gives, this should cause every single
                    // semaphore operation to wait.
                    for _ in 0..count {
                        for sem in &this.sems {
                            sem.give();
                        }
                    }
                }
                Command::SemWaitSameAsync(_) => (),
                Command::SemHigh(_) => (),
            }
            // printkln!("low command: {:?}", cmd);

            this.results.sender.send(Result::Low).unwrap();
        }
    }

    /// And the high priority worker.
    fn high_runner(this: Arc<Self>, command: Receiver<Command>) {
        while let Ok(cmd) = command.recv() {
            match cmd {
                Command::Empty => (),
                Command::SimpleSem(_) => (),
                Command::SimpleSemAsync(_) => (),
                Command::SimpleSemYield(_) => (),
                Command::SimpleSemYieldAsync(_) => (),
                Command::SemWait(_) => (),
                Command::SemWaitAsync(_) => (),
                Command::SemWaitSameAsync(_) => (),
                Command::SemHigh(count) => {
                    // The high-priority thread does all of the gives, this should cause every single
                    // semaphore operation to be ready.
                    for _ in 0..count {
                        for sem in &this.sems {
                            sem.give();
                        }
                    }
                }
            }
            // printkln!("high command: {:?}", cmd);

            this.results.sender.send(Result::High).unwrap();
        }
    }
}

#[derive(Clone)]
struct ChanPair<T> {
    sender: Sender<T>,
    receiver: Receiver<T>,
}

impl<T> ChanPair<T> {
    fn new_bounded(cap: usize) -> Self {
        let (sender, receiver) = bounded(cap);
        Self { sender, receiver }
    }

    fn new_unbounded() -> Self {
        let (sender, receiver) = unbounded();
        Self { sender, receiver }
    }
}

#[derive(Clone, Debug)]
enum Command {
    /// The empty test.  Does nothing, but invoke everything.  Useful to determine overhead.
    Empty,
    /// A simple semaphore, we give and take the semaphore ourselves.
    SimpleSem(usize),
    /// Simple semaphore, with async
    SimpleSemAsync(usize),
    /// A simple semaphore, we give and take the semaphore ourselves.  With a yield between each to
    /// force context switches.
    SimpleSemYield(usize),
    /// SimpleSemYield by async with yield (non context switch yield, just work-queue reschedule
    SimpleSemYieldAsync(usize),
    /// Semaphore test where the low priority thread does the 'give', so every wait should
    /// block.
    SemWait(usize),
    /// Same as SemWait, but async.
    SemWaitAsync(usize),
    /// SemWaitAsync but with the 'give' task also on the same work queue.
    SemWaitSameAsync(usize),
    /// Semaphore tests where the high priority thread does the 'give', so every wait should be
    /// read.
    SemHigh(usize),
}

enum Result {
    Worker {
        /// What is the id of this worker.
        id: usize,
        /// Operation count.
        count: usize,
    },
    /// The Low priority worker is done.
    Low,
    /// The high priority worker is done.
    High,
}

// For accurate timing, use the cycle counter.
fn now() -> u64 {
    unsafe { k_cycle_get_64() }
}

kobj_define! {
    static TEST_THREADS: [StaticThread; NUM_THREADS];
    static TEST_STACKS: [ThreadStack<THREAD_STACK_SIZE>; NUM_THREADS];

    static LOW_THREAD: StaticThread;
    static LOW_STACK: ThreadStack<THREAD_STACK_SIZE>;

    static HIGH_THREAD: StaticThread;
    static HIGH_STACK: ThreadStack<THREAD_STACK_SIZE>;

    static WORK_STACK: ThreadStack<WORK_STACK_SIZE>;
}
