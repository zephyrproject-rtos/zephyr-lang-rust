//! Executor-based tests
//!
//! These tests try to be as similar as possible to the thread-based tests, but instead are built
//! around async.

use core::{ffi::c_int, sync::atomic::Ordering};

use alloc::format;
use embassy_executor::SendSpawner;
use embassy_futures::yield_now;
#[allow(unused_imports)]
use embassy_sync::semaphore::{FairSemaphore, GreedySemaphore};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, semaphore::Semaphore,
    signal::Signal,
};
use static_cell::StaticCell;
use zephyr::{embassy::Executor, sync::atomic::AtomicBool};

use crate::{BenchTimer, Command, TestResult, NUM_THREADS, THREAD_STACK_SIZE};

/// As the tests do exercise different executors at different priorities, use the critical section
/// variant.
// type ASemaphore = GreedySemaphore<CriticalSectionRawMutex>;
type ASemaphore = FairSemaphore<CriticalSectionRawMutex, { NUM_THREADS + 2 }>;

/// A Signal for reporting back spawners.
type SpawnerSignal = Signal<CriticalSectionRawMutex, SendSpawner>;

/// Async-based test runner.
pub struct AsyncTests {
    /// Each test thread gets a semaphore, to use as appropriate for that test.
    sems: &'static [ASemaphore],

    /// A back sem for the reverse direction.
    back_sem: &'static ASemaphore,

    /// The spawners, of the three priorities.
    spawners: [SendSpawner; 3],

    /// Tests results are communicated back via this channels.
    results: Channel<CriticalSectionRawMutex, TestResult, 1>,

    /// Main started, allows main to be started lazily, after we've become static.
    main_started: AtomicBool,

    /// Indicates back to the main thread that the given test is finished.
    end_wait: zephyr::sys::sync::Semaphore,
}

/// The executors for the tests.  The values are low, medium, and high priority.
static EXECUTORS: [StaticCell<Executor>; 3] = [const { StaticCell::new() }; 3];

/// Each executor returns a SendSpawner to span tasks on that executor.
static SPAWNERS: [Signal<CriticalSectionRawMutex, SendSpawner>; 3] = [const { Signal::new() }; 3];

/// Static semaphores for the above.
static SEMS: [ASemaphore; NUM_THREADS] = [const { ASemaphore::new(0) }; NUM_THREADS];

static BACK_SEM: ASemaphore = ASemaphore::new(0);

/// Main command
static MAIN_CMD: Channel<CriticalSectionRawMutex, Command, 1> = Channel::new();

impl AsyncTests {
    /// Construct a new set of tests, firing up the threads for the differernt priority executors.
    pub fn new(count: usize) -> Self {
        // printkln!("Starting executors");
        let spawners = [0, 1, 2].map(|id| {
            let thread = executor_thread(&EXECUTORS[id], &SPAWNERS[id]);
            thread.set_priority(id as c_int + 5);
            thread.start();

            zephyr::time::sleep(zephyr::time::Duration::millis_at_least(1));
            if let Some(sp) = SPAWNERS[id].try_take() {
                sp
            } else {
                panic!("Executor thread did not initialize properly");
            }
        });

        Self {
            sems: &SEMS[0..count],
            back_sem: &BACK_SEM,
            spawners,
            results: Channel::new(),
            main_started: AtomicBool::new(false),
            end_wait: zephyr::sys::sync::Semaphore::new(0, 1),
        }
    }

    /// Run one of the given tests.
    ///
    /// Fires off the appropriate workers for the given test.  Tests follow a basic pattern:
    /// There are NUM_THREADS + 2 tasks spawned.  These communicate as appropriate for the given
    /// test, and the results are then collected, waiting for everything to finish, and reported.
    pub fn run(&'static self, command: Command) {
        if !self.main_started.load(Ordering::Relaxed) {
            self.spawners[1].spawn(main_run(self)).unwrap();
            self.main_started.store(true, Ordering::Relaxed);
        }

        if MAIN_CMD.try_send(command).is_err() {
            panic!("Main queue filled up");
        }

        // Wait for the Zephyr semaphore indicating the test is finished.
        self.end_wait.take(zephyr::time::Forever).unwrap();
    }

    /// A simple semaphore test.  Tests the time to use the semaphore with no blocking.
    async fn simple_sem(&self, id: usize, count: usize) -> usize {
        let sem = &self.sems[id];

        for _ in 0..count {
            sem.release(1);
            let rel = sem.acquire(1).await.unwrap();
            rel.disarm();
        }

        count
    }

    /// A simple semaphore test, with yield.  Tests the time to use the semaphore with no blocking.
    async fn simple_sem_yield(&self, id: usize, count: usize) -> usize {
        let sem = &self.sems[id];

        for _ in 0..count {
            sem.release(1);
            let rel = sem.acquire(1).await.unwrap();
            rel.disarm();
            yield_now().await;
        }

        count
    }

    /// The taker side of the SemWait test.
    async fn sem_wait_taker(&self, id: usize, count: usize) -> usize {
        let sem = &self.sems[id];
        for _ in 0..count {
            let rel = sem.acquire(1).await.unwrap();
            rel.disarm();
        }
        count
    }

    /// The giver side of the SemWait test.
    async fn sem_wait_giver(&self, count: usize) {
        for _ in 0..count {
            for sem in self.sems {
                sem.release(1);
            }
        }
    }

    /// The taker side of the SemPingPong test.
    async fn sem_ping_pong_taker(&self, count: usize) -> usize {
        let sem = &self.sems[0];
        let back_sem = self.back_sem;

        // zephyr::printkln!("Taking {count} sems");
        for _ in 0..count {
            //zephyr::printkln!("acquire1");
            let rel = sem.acquire(1).await.unwrap();
            //zephyr::printkln!("acquired1");
            rel.disarm();
            //zephyr::printkln!("release2");
            back_sem.release(1);
        }
        // zephyr::printkln!("Taking sems done");

        count
    }

    /// The giver side of the SemPingPong test.  This uses the first sem and back sem to ping pong
    /// across all of the workers.
    async fn sem_ping_pong_giver(&self, count: usize) {
        let sem = &self.sems[0];
        let back_sem = self.back_sem;

        // zephyr::printkln!("Giving {},{} sems", count, self.sems.len());
        for _ in 0..count {
            for _ in 0..self.sems.len() {
                //zephyr::printkln!("release1");
                sem.release(1);
                //zephyr::printkln!("acquire2");
                let rel = back_sem.acquire(1).await.unwrap();
                //zephyr::printkln!("acquired2");
                rel.disarm();
            }
        }
        // zephyr::printkln!("Giving sems done");
    }
}

/// The low priority worker, depending on the test, performs some operations at a lower priority
/// than the main threads.
#[embassy_executor::task]
async fn low_worker(this: &'static AsyncTests, command: Command) {
    match command {
        Command::Empty => (),
        Command::SimpleSem(_) => (),
        Command::SimpleSemYield(_) => (),
        Command::SemWait(count) => this.sem_wait_giver(count).await,
        Command::SemWaitSame(count) => this.sem_wait_giver(count).await,
        Command::SemPingPong(count) => this.sem_ping_pong_giver(count).await,
        Command::SemOnePingPong(count) => this.sem_ping_pong_giver(count).await,
        command => panic!("Not implemented: {:?}", command),
    }

    this.results.send(TestResult::Low).await;
}

/// The high priority worker, performs some operations at a higher priority than the main threads.
#[embassy_executor::task]
async fn high_worker(this: &'static AsyncTests, command: Command) {
    // printkln!("high_worker started");
    match command {
        Command::Empty => (),
        Command::SimpleSem(_) => (),
        Command::SimpleSemYield(_) => (),
        Command::SemWait(_) => (),
        Command::SemWaitSame(_) => (),
        Command::SemPingPong(_) => (),
        Command::SemOnePingPong(_) => (),
        command => panic!("Not implemented: {:?}", command),
    }

    this.results.send(TestResult::High).await;
}

/// The main worker threads.
///
/// These perform the main work of the test, generally communicating with either the main test task,
/// which is in the same executor, or with the higher or lower priority worker.
#[embassy_executor::task(pool_size = NUM_THREADS)]
async fn worker(this: &'static AsyncTests, command: Command, id: usize) {
    let total;
    match command {
        Command::Empty => total = 1,
        Command::SimpleSem(count) => total = this.simple_sem(id, count).await,
        Command::SimpleSemYield(count) => total = this.simple_sem_yield(id, count).await,
        Command::SemWait(count) => total = this.sem_wait_taker(id, count).await,
        Command::SemWaitSame(count) => total = this.sem_wait_taker(id, count).await,
        Command::SemPingPong(count) => total = this.sem_ping_pong_taker(count).await,
        Command::SemOnePingPong(count) => total = this.sem_ping_pong_taker(count).await,
        command => panic!("Not implemented: {:?}", command),
    }

    this.results
        .send(TestResult::Worker { id, count: total })
        .await;
}

/// Main worker.
///
/// This task performs the main work.
#[embassy_executor::task]
async fn main_run(this: &'static AsyncTests) {
    loop {
        let command = MAIN_CMD.receive().await;
        let msg = format!("async {:?}", command);

        let mut timer = BenchTimer::new(&msg, 1);

        let ntasks = this.sems.len();

        // Before starting, reset the semaphores.
        for sem in this.sems {
            sem.set(0);
        }
        this.back_sem.set(0);

        if true {
            this.spawners[2].spawn(high_worker(this, command)).unwrap();
            if command.is_same_priority() {
                this.spawners[1].spawn(low_worker(this, command)).unwrap();
            } else {
                this.spawners[0].spawn(low_worker(this, command)).unwrap();
            }
        } else {
            this.spawners[1].spawn(high_worker(this, command)).unwrap();
            this.spawners[1].spawn(low_worker(this, command)).unwrap();
        }

        for id in 0..ntasks {
            this.spawners[1].spawn(worker(this, command, id)).unwrap();
        }

        let mut results: heapless::Vec<Option<usize>, NUM_THREADS> = heapless::Vec::new();
        let mut low = false;
        let mut high = false;
        let mut msg_count = (2 + ntasks) as isize;

        for _ in 0..ntasks {
            results.push(None).unwrap();
        }

        loop {
            match this.results.receive().await {
                TestResult::Worker { id, count } => {
                    if results[id].replace(count).is_some() {
                        panic!("Multiple results from worker {}", id);
                    }
                }
                TestResult::Low => {
                    if low {
                        panic!("Multiple results from 'low' worker");
                    }
                    low = true;
                }
                TestResult::High => {
                    if high {
                        panic!("Multiple results from 'high' worker");
                    }
                    high = true;
                }
            }
            msg_count -= 1;
            if msg_count <= 0 {
                break;
            }
        }

        let count: usize = results.iter().map(|x| x.unwrap_or(0)).sum();
        timer.adjust_count(count);

        timer.stop();
        this.end_wait.give();
    }
}

/// Main for each executor.
///
/// This thread starts up an executor, and then places that executor into a 'signal' so the
/// non-async code can get the value.
#[zephyr::thread(stack_size = THREAD_STACK_SIZE, pool_size = 3)]
fn executor_thread(exec: &'static StaticCell<Executor>, spawner_sig: &'static SpawnerSignal) -> ! {
    let exec = exec.init(Executor::new());
    exec.run(|spawner| {
        // TODO: Should we try to have a local spawner as well?
        spawner_sig.signal(spawner.make_send());
    })
}

// For debugging
#[unsafe(no_mangle)]
extern "C" fn invalid_spinlock(_l: *mut zephyr::raw::k_spinlock, _thread: u32, _id: u32) {}
