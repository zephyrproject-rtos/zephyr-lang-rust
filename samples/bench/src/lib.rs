// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]
// Cargo tries to detect configs that have typos in them.  Unfortunately, the Zephyr Kconfig system
// uses a large number of Kconfigs and there is no easy way to know which ones might conceivably be
// valid.  This prevents a warning about each cfg that is used.
#![allow(unexpected_cfgs)]

extern crate alloc;

use core::mem;

use alloc::collections::vec_deque::VecDeque;
use alloc::vec;
use executor::AsyncTests;
use static_cell::StaticCell;
use zephyr::define_work_queue;
use zephyr::raw::k_yield;
use zephyr::sync::{SpinMutex, Weak};
use zephyr::time::NoWait;
use zephyr::work::{ArcWork, SimpleAction, Work};
use zephyr::{
    kconfig::CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC,
    printkln,
    raw::k_cycle_get_64,
    sync::{
        channel::{bounded, unbounded, Receiver, Sender},
        Arc,
    },
    sys::sync::Semaphore,
    time::Forever,
    work::WorkQueue,
};

mod executor;

/// How many threads to run in the tests.
const NUM_THREADS: usize = 6;

/// Stack size to use for the threads.
#[cfg(target_pointer_width = "32")]
const THREAD_STACK_SIZE: usize = 4 * 1024;

/// Stack size to use for the threads.
#[cfg(target_pointer_width = "64")]
const THREAD_STACK_SIZE: usize = 5120;

/// Stack size to use for the work queue.
const WORK_STACK_SIZE: usize = 4096;

/// This is a global iteration.  Small numbers still test functionality within CI, and large numbers
/// give more meaningful benchmark results.
// const TOTAL_ITERS: usize = 10;
const TOTAL_ITERS: usize = 1_000;
// const TOTAL_ITERS: usize = 10_000;

/// A heapless::Vec, with a maximum size of the number of threads chosen above.
type HeaplessVec<T> = heapless::Vec<T, NUM_THREADS>;

#[no_mangle]
extern "C" fn rust_main() {
    let tester = ThreadTests::new(NUM_THREADS);

    static ATESTER: StaticCell<AsyncTests> = StaticCell::new();
    let atester = ATESTER.init(AsyncTests::new(NUM_THREADS));

    if false {
        atester.run(Command::SemPingPong(TOTAL_ITERS));
        printkln!("Stopping");
        return;
    }

    atester.run(Command::SimpleSem(TOTAL_ITERS));
    atester.run(Command::SimpleSem(TOTAL_ITERS));

    // Some basic benchmarks
    arc_bench();
    spin_bench();
    sem_bench();

    let simple = Simple::new(tester.workq);
    let mut num = 6;
    while num < 250 {
        simple.run(num, TOTAL_ITERS / num);
        num = num * 13 / 10;
    }

    tester.run(Command::Empty);
    atester.run(Command::Empty);
    tester.run(Command::SimpleSem(TOTAL_ITERS));
    atester.run(Command::SimpleSem(TOTAL_ITERS));
    tester.run(Command::SimpleSemYield(TOTAL_ITERS));
    atester.run(Command::SimpleSemYield(TOTAL_ITERS));
    tester.run(Command::SemWait(TOTAL_ITERS));
    atester.run(Command::SemWait(TOTAL_ITERS));
    atester.run(Command::SemWaitSame(TOTAL_ITERS));
    tester.run(Command::SemHigh(TOTAL_ITERS));
    // atester.run(Command::SemHigh(TOTAL_ITERS));
    atester.run(Command::SemOnePingPong(TOTAL_ITERS));
    tester.run(Command::SemPingPong(TOTAL_ITERS));
    atester.run(Command::SemPingPong(TOTAL_ITERS));
    tester.run(Command::SemOnePingPong(TOTAL_ITERS));
    atester.run(Command::SemOnePingPong(TOTAL_ITERS));
    // tester.run(command::semonepingpongasync(num_threads, total_iters / 6));
    // tester.run(command::semonepingpongasync(20, total_iters / 20));
    // tester.run(command::semonepingpongasync(50, total_iters / 50));
    // tester.run(command::semonepingpongasync(100, total_iters / 100));
    // tester.run(command::semonepingpongasync(500, total_iters / 500));
    tester.run(Command::Empty);

    /*
    let mut num = 6;
    while num < 100 {
        tester.run(Command::SemOnePingPongAsync(num, TOTAL_ITERS / num));
        num = num * 13 / 10;
    }
    */

    printkln!("Done with all tests\n");
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
    sems: &'static [Semaphore; NUM_THREADS],

    /// This semaphore is used to ping-ping back to another thread.
    back_sems: &'static [Semaphore; NUM_THREADS],

    /// Each test also has a message queue, for testing, that it has sender and receiver for.
    chans: HeaplessVec<ChanPair<u32>>,

    /// In addition, each test has a channel it owns the receiver for that listens for commands
    /// about what test to run.
    commands: HeaplessVec<Sender<Command>>,

    /// Low and high also take commands.
    low_command: Sender<Command>,
    high_command: Sender<Command>,

    /// A work queue for the main runners.
    workq: &'static WorkQueue,

    /// The test also all return their result to the main.  The threads Send, the main running
    /// receives.
    results: ChanPair<TestResult>,
}

impl ThreadTests {
    /// Construct a new set of tests, firing up the threads.
    ///
    /// Note that the count, although a parameter, must be less than or equal to the size of the
    /// statically defined threads.
    fn new(count: usize) -> Arc<Self> {
        let (low_send, low_recv) = bounded(1);
        let (high_send, high_recv) = bounded(1);

        let workq = WORKQ.start();

        let mut result = Self {
            sems: &SEMS,
            back_sems: &BACK_SEMS,
            chans: HeaplessVec::new(),
            commands: HeaplessVec::new(),
            low_command: low_send,
            high_command: high_send,
            results: ChanPair::new_unbounded(),
            workq,
        };

        let mut thread_commands = HeaplessVec::new();

        for _ in 0..count {
            let chans = ChanPair::new_bounded(1);
            result.chans.push(chans.clone()).unwrap();

            let (csend, crecv) = bounded(1);
            result.commands.push(csend).unwrap();
            thread_commands.push(crecv).unwrap();
        }

        let result = Arc::new(result);

        // Spawn the worker threads.
        for i in 0..count {
            let result2 = result.clone();
            let cmd2 = thread_commands[i].clone();
            let thread = test_worker(result2, cmd2, i);
            thread.set_priority(5);
            thread.start();
        }

        // And fire up the low and high priority workers.
        let result2 = result.clone();
        let thread = test_low_runner(result2, low_recv);
        thread.set_priority(6);
        thread.start();

        let result2 = result.clone();
        let thread = test_high_runner(result2, high_recv);
        thread.set_priority(4);
        thread.start();

        result

        /*
        // Calculate a size to show.
        printkln!(
            "worker size: {} bytes",
            work_size(Self::ping_pong_worker_async(
                result.clone(),
                0,
                result.sems[0],
                result.back_sems[0],
                6
            ))
        );

        result
        */
    }

    fn run(&self, command: Command) {
        // printkln!("Running {:?}", command);

        // In case previous runs left the semaphore non-zero, reset all of them.  This is safe due
        // to nothing using the semaphores right now.
        for sem in self.sems {
            if sem.count_get() > 0 {
                printkln!("Warning: previous test left count: {}", sem.count_get());
                sem.reset();
            }
        }
        for sem in self.back_sems {
            if sem.count_get() > 0 {
                printkln!("Warning: previous test left count: {}", sem.count_get());
                sem.reset();
            }
        }

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
                TestResult::Worker { id, count } => {
                    if results[id].replace(count).is_some() {
                        panic!("Multiple result from worker {}", id);
                    }
                    msg_count -= 1;
                    if msg_count <= 0 {
                        break;
                    }
                }
                TestResult::Low => {
                    if low {
                        panic!("Multiple results from 'low' worker");
                    }
                    low = true;

                    msg_count -= 1;
                    if msg_count <= 0 {
                        break;
                    }
                }
                TestResult::High => {
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

    /// The worker threads themselves.
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

                Command::SemPingPong(count) => {
                    this.ping_pong_worker(
                        id,
                        &this.sems[id],
                        &this.back_sems[id],
                        count,
                        &mut total,
                    );
                }

                Command::SemOnePingPong(count) => {
                    this.ping_pong_worker(id, &this.sems[0], &this.back_sems[0], count, &mut total);
                }

                Command::SemWaitSame(count) => {
                    this.sem_take(&this.sems[id], count, &mut total);
                }
            }

            this.results
                .sender
                .send(TestResult::Worker { id, count: total })
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

    /// Worker side of the ping pong sem, takes the 'sem' and gives to the back_sem.
    fn ping_pong_worker(
        &self,
        id: usize,
        sem: &Semaphore,
        back_sem: &Semaphore,
        count: usize,
        total: &mut usize,
    ) {
        for i in 0..count {
            if false {
                if let Ok(_) = sem.take(NoWait) {
                    panic!("Semaphore was already available: {} loop:{}", id, i);
                }
            }
            sem.take(Forever).unwrap();
            back_sem.give();
            *total += 1;
        }
    }

    fn ping_pong_replier(&self, count: usize) {
        for _ in 0..count {
            for (sem, back) in self.sems.iter().zip(self.back_sems) {
                sem.give();
                back.take(Forever).unwrap();
            }
        }
    }

    fn one_ping_pong_replier(&self, count: usize) {
        for _ in 0..count {
            for _ in 0..self.sems.len() {
                self.sems[0].give();
                self.back_sems[0].take(Forever).unwrap();
            }
        }
    }

    /// And the low priority worker.
    fn low_runner(this: Arc<Self>, command: Receiver<Command>) {
        let _ = this;
        while let Ok(cmd) = command.recv() {
            match cmd {
                Command::Empty => (),
                Command::SimpleSem(_) => (),
                Command::SimpleSemYield(_) => (),
                Command::SemWait(count) | Command::SemWaitSame(count) => {
                    // The low-priority thread does all of the gives, this should cause every single
                    // semaphore operation to wait.
                    for _ in 0..count {
                        for sem in this.sems {
                            sem.give();
                        }
                    }
                }
                Command::SemHigh(_) => (),
                Command::SemPingPong(count) => {
                    this.ping_pong_replier(count);
                }
                Command::SemOnePingPong(count) => {
                    this.one_ping_pong_replier(count);
                }
            }
            // printkln!("low command: {:?}", cmd);

            this.results.sender.send(TestResult::Low).unwrap();
        }
    }

    /// And the high priority worker.
    fn high_runner(this: Arc<Self>, command: Receiver<Command>) {
        while let Ok(cmd) = command.recv() {
            match cmd {
                Command::Empty => (),
                Command::SimpleSem(_) => (),
                Command::SimpleSemYield(_) => (),
                Command::SemWait(_) => (),
                Command::SemWaitSame(_) => (),
                Command::SemPingPong(_) => (),
                Command::SemOnePingPong(_) => (),
                Command::SemHigh(count) => {
                    // The high-priority thread does all of the gives, this should cause every single
                    // semaphore operation to be ready.
                    for _ in 0..count {
                        for sem in this.sems {
                            sem.give();
                        }
                    }
                }
            }
            // printkln!("high command: {:?}", cmd);

            this.results.sender.send(TestResult::High).unwrap();
        }
    }
}

/// Top level Zephyr thread for the workers.
#[zephyr::thread(stack_size = THREAD_STACK_SIZE, pool_size = NUM_THREADS)]
fn test_worker(this: Arc<ThreadTests>, command: Receiver<Command>, id: usize) {
    ThreadTests::worker(this, command, id)
}

/// The low priority worker.
#[zephyr::thread(stack_size = THREAD_STACK_SIZE)]
fn test_low_runner(this: Arc<ThreadTests>, command: Receiver<Command>) {
    ThreadTests::low_runner(this, command)
}

/// The high priority worker.
#[zephyr::thread(stack_size = THREAD_STACK_SIZE)]
fn test_high_runner(this: Arc<ThreadTests>, command: Receiver<Command>) {
    ThreadTests::high_runner(this, command)
}

#[derive(Clone, Debug)]
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

#[derive(Copy, Clone, Debug)]
enum Command {
    /// The empty test.  Does nothing, but invoke everything.  Useful to determine overhead.
    Empty,
    /// A simple semaphore, we give and take the semaphore ourselves.
    SimpleSem(usize),
    /// A simple semaphore, we give and take the semaphore ourselves.  With a yield between each to
    /// force context switches.
    SimpleSemYield(usize),
    /// SimpleSemYield by async with yield (non context switch yield, just work-queue reschedule
    SemWait(usize),
    /// SemWaitAsync but with the 'give' task also on the same work queue.
    SemWaitSame(usize),
    /// Semaphore tests where the high priority thread does the 'give', so every wait should be
    /// read.
    SemHigh(usize),
    /// Semaphores ping-ponging between worker threads and a low priority thread.
    SemPingPong(usize),
    /// SemPingPong, but async
    SemOnePingPong(usize),
}

impl Command {
    /// Determine if this command intents for the "low" worker to be at the same priority as the
    /// main worker.
    pub fn is_same_priority(self) -> bool {
        match self {
            Self::SemWaitSame(_) => true,
            Self::SemOnePingPong(_) => true,
            _ => false,
        }
    }
}

enum TestResult {
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

/// The Simple test just does a ping pong test using manually submitted work.
struct Simple {
    workq: &'static WorkQueue,
}

impl Simple {
    fn new(workq: &'static WorkQueue) -> Self {
        Self { workq }
    }

    fn run(&self, workers: usize, iterations: usize) {
        // printkln!("Running Simple");
        let main = Work::new_arc(SimpleMain::new(workers * iterations, self.workq));

        let children: VecDeque<_> = (0..workers)
            .map(|n| Work::new_arc(SimpleWorker::new(main.0.clone(), self.workq, n)).0)
            .collect();

        let mut locked = main.0.action().locked.lock().unwrap();
        let _ = mem::replace(&mut locked.works, children);
        drop(locked);

        let start = now();
        // Fire off main, which will run everything.
        main.clone().submit_to_queue(self.workq).unwrap();

        // And wait for the completion semaphore.
        main.0.action().done.take(Forever).unwrap();

        let stop = now();
        let time = (stop - start) as f64 / (CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC as f64) * 1000.0;

        let total = workers * iterations;
        let time = if total > 0 {
            time / (total as f64) * 1000.0
        } else {
            0.0
        };

        printkln!(
            "    {:8.3} us, {} of {} workers {} times",
            time,
            total,
            workers,
            iterations
        );

        // Before we go away, make sure that there aren't any leaked workers.
        let mut locked = main.0.action().locked.lock().unwrap();
        while let Some(other) = locked.works.pop_front() {
            assert_eq!(Arc::strong_count(&other), 1);
        }
        drop(locked);

        // And nothing has leaked main, either.
        assert_eq!(Arc::strong_count(&main.0), 1);
    }
}

/// A simple worker.  When run, it submits the main worker to do the next work.
struct SimpleWorker {
    main: Weak<Work<SimpleMain>>,
    workq: &'static WorkQueue,
    _id: usize,
}

impl SimpleWorker {
    fn new(main: Arc<Work<SimpleMain>>, workq: &'static WorkQueue, id: usize) -> Self {
        Self {
            main: Arc::downgrade(&main),
            workq,
            _id: id,
        }
    }
}

impl SimpleAction for SimpleWorker {
    fn act(self: &Self) {
        // Each time we are run, fire the main worker back up.
        let main = ArcWork(self.main.upgrade().unwrap());
        main.clone().submit_to_queue(self.workq).unwrap();
    }
}

/// This is the main worker.
///
/// Each time it is run, it submits the next worker from the queue and exits.
struct SimpleMain {
    /// All of the work items.
    locked: SpinMutex<Locked>,
    workq: &'static WorkQueue,
    done: Semaphore,
}

impl SimpleAction for SimpleMain {
    fn act(self: &Self) {
        // Each time, take a worker from the queue, and submit it.
        let mut lock = self.locked.lock().unwrap();

        if lock.count == 0 {
            // The last time, indicate we are done.
            self.done.give();
            return;
        }

        let worker = lock.works.pop_front().unwrap();
        lock.works.push_back(worker.clone());
        lock.count -= 1;
        drop(lock);

        ArcWork(worker.clone()).submit_to_queue(self.workq).unwrap();
    }
}

impl SimpleMain {
    fn new(count: usize, workq: &'static WorkQueue) -> Self {
        Self {
            locked: SpinMutex::new(Locked::new(count)),
            done: Semaphore::new(0, 1),
            workq,
        }
    }
}

struct Locked {
    works: VecDeque<Arc<Work<SimpleWorker>>>,
    count: usize,
}

impl Locked {
    fn new(count: usize) -> Self {
        Self {
            works: VecDeque::new(),
            count,
        }
    }
}

/// Benchmark the performance of Arc.
fn arc_bench() {
    let thing = Arc::new(123);
    let timer = BenchTimer::new("Arc clone+drop", TOTAL_ITERS);
    for _ in 0..TOTAL_ITERS {
        let _ = thing.clone();
    }
    timer.stop();
}

/// Benchmark SpinMutex.
#[inline(never)]
#[no_mangle]
fn spin_bench() {
    let iters = TOTAL_ITERS;
    let thing = SpinMutex::new(123);
    let timer = BenchTimer::new("SpinMutex lock", iters);
    for _ in 0..iters {
        *thing.lock().unwrap() += 1;
    }
    timer.stop();
}

/// Semaphore benchmark.
///
/// This benchmarks a single thread with a semaphore that is always ready.  This is pretty close to
/// just syscall with spinlock time.
#[inline(never)]
#[no_mangle]
fn sem_bench() {
    let iters = TOTAL_ITERS;
    let sem = Semaphore::new(iters as u32, iters as u32);
    let timer = BenchTimer::new("Semaphore take", iters);
    for _ in 0..iters {
        sem.take(Forever).unwrap();
    }
    timer.stop();
}

// For accurate timing, use the cycle counter.
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

    /// Adjust the count, for cases where we don't know the count until later.
    pub fn adjust_count(&mut self, count: usize) {
        self.count = count;
    }

    pub fn stop(self) {
        let stop = now();
        let raw = (stop - self.start) as f64 / (CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC as f64) * 1.0e6;
        let time =
            (stop - self.start) as f64 / (CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC as f64) * 1000.0;
        let time = if self.count > 0 {
            time / (self.count as f64) * 1000.0
        } else {
            0.0
        };

        printkln!(
            "    {:8.3} us, {} of {} {:8.3}us raw",
            time,
            self.count,
            self.what,
            raw,
        );
    }
}

define_work_queue!(WORKQ, WORK_STACK_SIZE, priority = 5, no_yield = true);

static SEMS: [Semaphore; NUM_THREADS] = [const { Semaphore::new(0, u32::MAX) }; NUM_THREADS];
static BACK_SEMS: [Semaphore; NUM_THREADS] = [const { Semaphore::new(0, u32::MAX) }; NUM_THREADS];
