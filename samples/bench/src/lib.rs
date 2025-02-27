// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]
// Cargo tries to detect configs that have typos in them.  Unfortunately, the Zephyr Kconfig system
// uses a large number of Kconfigs and there is no easy way to know which ones might conceivably be
// valid.  This prevents a warning about each cfg that is used.
#![allow(unexpected_cfgs)]

extern crate alloc;

use core::mem;
use core::pin::Pin;

use alloc::collections::vec_deque::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use static_cell::StaticCell;
use zephyr::sync::SpinMutex;
use zephyr::time::NoWait;
use zephyr::work::futures::work_size;
use zephyr::work::{SimpleAction, Work};
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

/// This is a global iteration.  Small numbers still test functionality within CI, and large numbers
/// give more meaningful benchmark results.
const TOTAL_ITERS: usize = 1_000;
// const TOTAL_ITERS: usize = 10_000;

/// A heapless::Vec, with a maximum size of the number of threads chosen above.
type HeaplessVec<T> = heapless::Vec<T, NUM_THREADS>;

#[no_mangle]
extern "C" fn rust_main() {
    let tester = ThreadTests::new(NUM_THREADS);

    // Some basic benchmarks
    arc_bench();
    spin_bench();
    sem_bench();

    let simple = Simple::new(tester.workq.clone());
    let mut num = 6;
    while num < 500 {
        simple.run(num, TOTAL_ITERS / num);
        num = num * 13 / 10;
    }

    tester.run(Command::Empty);
    tester.run(Command::SimpleSem(TOTAL_ITERS));
    tester.run(Command::SimpleSemAsync(TOTAL_ITERS));
    tester.run(Command::SimpleSemYield(TOTAL_ITERS));
    tester.run(Command::SimpleSemYieldAsync(TOTAL_ITERS));
    tester.run(Command::SemWait(TOTAL_ITERS));
    tester.run(Command::SemWaitAsync(TOTAL_ITERS));
    tester.run(Command::SemWaitSameAsync(TOTAL_ITERS));
    tester.run(Command::SemHigh(TOTAL_ITERS));
    tester.run(Command::SemPingPong(TOTAL_ITERS));
    tester.run(Command::SemPingPongAsync(TOTAL_ITERS));
    tester.run(Command::SemOnePingPong(TOTAL_ITERS));
    /*
    tester.run(Command::SemOnePingPongAsync(NUM_THREADS, TOTAL_ITERS / 6));
    tester.run(Command::SemOnePingPongAsync(20, TOTAL_ITERS / 20));
    tester.run(Command::SemOnePingPongAsync(50, TOTAL_ITERS / 50));
    tester.run(Command::SemOnePingPongAsync(100, TOTAL_ITERS / 100));
    tester.run(Command::SemOnePingPongAsync(500, TOTAL_ITERS / 500));
    tester.run(Command::Empty);
    */
    let mut num = 6;
    while num < 100 {
        tester.run(Command::SemOnePingPongAsync(num, TOTAL_ITERS / num));
        num = num * 13 / 10;
    }

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
    sems: HeaplessVec<&'static Semaphore>,

    /// This semaphore is used to ping-ping back to another thread.
    back_sems: HeaplessVec<&'static Semaphore>,

    /// Each test also has a message queue, for testing, that it has sender and receiver for.
    chans: HeaplessVec<ChanPair<u32>>,

    /// In addition, each test has a channel it owns the receiver for that listens for commands
    /// about what test to run.
    commands: HeaplessVec<Sender<Command>>,

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

        // Leak the workqueue so it doesn't get dropped.
        let _ = Arc::into_raw(workq.clone());

        let mut result = Self {
            sems: HeaplessVec::new(),
            back_sems: HeaplessVec::new(),
            chans: HeaplessVec::new(),
            commands: HeaplessVec::new(),
            results: ChanPair::new_unbounded(),
            low_command: low_send,
            high_command: high_send,
            workq,
        };

        let mut thread_commands = Vec::new();

        static SEMS: [StaticCell<Semaphore>; NUM_THREADS] =
            [const { StaticCell::new() }; NUM_THREADS];
        static BACK_SEMS: [StaticCell<Semaphore>; NUM_THREADS] =
            [const { StaticCell::new() }; NUM_THREADS];

        for i in 0..count {
            let sem = SEMS[i].init(Semaphore::new(0, u32::MAX));
            result.sems.push(sem).unwrap();

            let sem = BACK_SEMS[i].init(Semaphore::new(0, u32::MAX));
            result.back_sems.push(sem).unwrap();

            let chans = ChanPair::new_bounded(1);
            result.chans.push(chans.clone()).unwrap();

            let (csend, crecv) = bounded(1);
            result.commands.push(csend).unwrap();
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
    }

    fn run(&self, command: Command) {
        // printkln!("Running {:?}", command);

        // In case previous runs left the semaphore non-zero, reset all of them.  This is safe due
        // to nothing using the semaphores right now.
        for sem in &self.sems {
            if sem.count_get() > 0 {
                printkln!("Warning: previous test left count: {}", sem.count_get());
                sem.reset();
            }
        }
        for sem in &self.back_sems {
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

                // For the async commands, spawn this on the worker thread and don't reply
                // ourselves.
                Command::SimpleSemAsync(count) => {
                    spawn(
                        Self::simple_sem_async(this.clone(), id, this.sems[id], count),
                        &this.workq,
                        c"worker",
                    );
                    continue;
                }

                Command::SimpleSemYieldAsync(count) => {
                    spawn(
                        Self::simple_sem_yield_async(this.clone(), id, this.sems[id], count),
                        &this.workq,
                        c"worker",
                    );
                    continue;
                }

                Command::SemWaitAsync(count) => {
                    spawn(
                        Self::sem_take_async(this.clone(), id, this.sems[id], count),
                        &this.workq,
                        c"worker",
                    );
                    continue;
                }

                Command::SemWaitSameAsync(count) => {
                    spawn(
                        Self::sem_take_async(this.clone(), id, this.sems[id], count),
                        &this.workq,
                        c"worker",
                    );
                    if id == 0 {
                        spawn(
                            Self::sem_giver_async(this.clone(), this.sems.clone(), count),
                            &this.workq,
                            c"giver",
                        );
                    }
                    continue;
                }

                Command::SemPingPongAsync(count) => {
                    spawn(
                        Self::ping_pong_worker_async(
                            this.clone(),
                            id,
                            this.sems[id],
                            this.back_sems[id],
                            count,
                        ),
                        &this.workq,
                        c"worker",
                    );
                    if id == 0 {
                        spawn(
                            Self::ping_pong_replier_async(this.clone(), count),
                            &this.workq,
                            c"giver",
                        );
                    }

                    continue;
                }

                Command::SemOnePingPongAsync(nthread, count) => {
                    if id == 0 {
                        for th in 0..nthread {
                            spawn(
                                Self::ping_pong_worker_async(
                                    this.clone(),
                                    th,
                                    this.sems[0],
                                    this.back_sems[0],
                                    count,
                                ),
                                &this.workq,
                                c"worker",
                            );
                        }
                        spawn(
                            Self::one_ping_pong_replier_async(this.clone(), nthread, count),
                            &this.workq,
                            c"giver",
                        );
                    }

                    // Avoid the reply for the number of workers that are within the range.  This
                    // does assume that nthread will always be >= the number configured.
                    if id < this.sems.len() {
                        continue;
                    }
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

    async fn simple_sem_async(this: Arc<Self>, id: usize, sem: &'static Semaphore, count: usize) {
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

    async fn simple_sem_yield_async(
        this: Arc<Self>,
        id: usize,
        sem: &'static Semaphore,
        count: usize,
    ) {
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

    async fn sem_take_async(this: Arc<Self>, id: usize, sem: &'static Semaphore, count: usize) {
        for _ in 0..count {
            // Enable this to verify that we are actually blocking.
            if false {
                if let Ok(_) = sem.take(NoWait) {
                    panic!("Semaphore was already available");
                }
            }
            sem.take_async(Forever).await.unwrap();
        }

        this.results
            .sender
            .send_async(Result::Worker { id, count })
            .await
            .unwrap();
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

    async fn ping_pong_worker_async(
        this: Arc<Self>,
        id: usize,
        sem: &'static Semaphore,
        back_sem: &'static Semaphore,
        count: usize,
    ) {
        for i in 0..count {
            if false {
                if let Ok(_) = sem.take(NoWait) {
                    panic!("Semaphore was already available: {} loop:{}", id, i);
                }
            }
            sem.take_async(Forever).await.unwrap();
            back_sem.give();
        }

        // Only send for an ID in range.
        if id < this.sems.len() {
            this.results
                .sender
                .send_async(Result::Worker { id, count })
                .await
                .unwrap();
        }
    }

    fn ping_pong_replier(&self, count: usize) {
        for _ in 0..count {
            for (sem, back) in self.sems.iter().zip(&self.back_sems) {
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

    async fn ping_pong_replier_async(this: Arc<Self>, count: usize) {
        for _ in 0..count {
            for (sem, back) in this.sems.iter().zip(&this.back_sems) {
                sem.give();
                back.take_async(Forever).await.unwrap();
            }
        }

        // No reply.
    }

    async fn one_ping_pong_replier_async(this: Arc<Self>, nthread: usize, count: usize) {
        for _ in 0..count {
            for _ in 0..nthread {
                this.sems[0].give();
                this.back_sems[0].take_async(Forever).await.unwrap();
            }
        }

        // No reply.
    }

    async fn sem_giver_async(this: Arc<Self>, sems: HeaplessVec<&'static Semaphore>, count: usize) {
        for _ in 0..count {
            for sem in &sems {
                sem.give();

                // Yield after each loop.  This should only force a reschedule each task's operation,
                // just enough to make sure everything still blocks.
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
                Command::SemPingPong(count) => {
                    this.ping_pong_replier(count);
                }
                Command::SemOnePingPong(count) => {
                    this.one_ping_pong_replier(count);
                }
                Command::SemPingPongAsync(_) => (),
                Command::SemOnePingPongAsync(_, _) => (),
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
                Command::SemPingPong(_) => (),
                Command::SemOnePingPong(_) => (),
                Command::SemHigh(count) => {
                    // The high-priority thread does all of the gives, this should cause every single
                    // semaphore operation to be ready.
                    for _ in 0..count {
                        for sem in &this.sems {
                            sem.give();
                        }
                    }
                }
                Command::SemPingPongAsync(_) => (),
                Command::SemOnePingPongAsync(_, _) => (),
            }
            // printkln!("high command: {:?}", cmd);

            this.results.sender.send(Result::High).unwrap();
        }
    }
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
    /// Semaphores ping-ponging between worker threads and a low priority thread.
    SemPingPong(usize),
    /// SemPingPong, but async
    SemPingPongAsync(usize),
    /// PingPong but with a single shared semaphore.  Demonstrates multiple threads queued on the
    /// same object.
    SemOnePingPong(usize),
    /// Same as SemOnePingPong, but async.  The first parameter is the number of async tasks.
    SemOnePingPongAsync(usize, usize),
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

/// The Simple test just does a ping pong test using manually submitted work.
struct Simple {
    workq: Arc<WorkQueue>,
}

impl Simple {
    fn new(workq: Arc<WorkQueue>) -> Self {
        Self { workq }
    }

    fn run(&self, workers: usize, iterations: usize) {
        // printkln!("Running Simple");
        let main = Work::new(SimpleMain::new(workers * iterations, self.workq.clone()));

        let children: VecDeque<_> = (0..workers)
            .map(|n| Work::new(SimpleWorker::new(main.clone(), self.workq.clone(), n)))
            .collect();

        let mut locked = main.action().locked.lock().unwrap();
        let _ = mem::replace(&mut locked.works, children);
        drop(locked);

        let start = now();
        // Fire off main, which will run everything.
        Work::submit_to_queue(main.clone(), &self.workq).unwrap();

        // And wait for the completion semaphore.
        main.action().done.take(Forever).unwrap();

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
    }
}

/// A simple worker.  When run, it submits the main worker to do the next work.
struct SimpleWorker {
    main: Pin<Arc<Work<SimpleMain>>>,
    workq: Arc<WorkQueue>,
    _id: usize,
}

impl SimpleWorker {
    fn new(main: Pin<Arc<Work<SimpleMain>>>, workq: Arc<WorkQueue>, id: usize) -> Self {
        Self {
            main,
            workq,
            _id: id,
        }
    }
}

impl SimpleAction for SimpleWorker {
    fn act(self: Pin<&Self>) {
        // Each time we are run, fire the main worker back up.
        Work::submit_to_queue(self.main.clone(), &self.workq).unwrap();
    }
}

/// This is the main worker.
///
/// Each time it is run, it submits the next worker from the queue and exits.
struct SimpleMain {
    /// All of the work items.
    locked: SpinMutex<Locked>,
    workq: Arc<WorkQueue>,
    done: Semaphore,
}

impl SimpleAction for SimpleMain {
    fn act(self: Pin<&Self>) {
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

        Work::submit_to_queue(worker.clone(), &self.workq).unwrap();
    }
}

impl SimpleMain {
    fn new(count: usize, workq: Arc<WorkQueue>) -> Self {
        Self {
            locked: SpinMutex::new(Locked::new(count)),
            done: Semaphore::new(0, 1),
            workq,
        }
    }
}

struct Locked {
    works: VecDeque<Pin<Arc<Work<SimpleWorker>>>>,
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
    static TEST_THREADS: [StaticThread; NUM_THREADS];
    static TEST_STACKS: [ThreadStack<THREAD_STACK_SIZE>; NUM_THREADS];

    static LOW_THREAD: StaticThread;
    static LOW_STACK: ThreadStack<THREAD_STACK_SIZE>;

    static HIGH_THREAD: StaticThread;
    static HIGH_STACK: ThreadStack<THREAD_STACK_SIZE>;

    static WORK_STACK: ThreadStack<WORK_STACK_SIZE>;
}
