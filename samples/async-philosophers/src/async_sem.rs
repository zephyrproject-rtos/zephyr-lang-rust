//! Async Semaphore based demo
//!
//! This implementation on the dining philosopher problem uses Zephyr semaphores to represent the
//! forks.  Each philosopher dines as per the algorithm a number of times, and when the are all
//! finished, the test is considered successful.  Deadlock will result in the primary thread not
//! completing.
//!
//! Notably, this uses Rc and RefCell along with spawn_local to demonstrate that multiple async
//! tasks run on the same worker do not need Send.  It is just important that write operations on
//! the RefCell do not `.await` or a panic is likely.

use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    mutex::Mutex,
    semaphore::{FairSemaphore, Semaphore},
};
use embassy_time::Timer;
use zephyr::{printkln, sync::Arc};

use crate::{get_random_delay, ResultSignal, Stats, NUM_PHIL};

type ESemaphore = FairSemaphore<CriticalSectionRawMutex, NUM_PHIL>;

/// The semaphores for the forks.
static FORKS: [ESemaphore; NUM_PHIL] = [const { ESemaphore::new(1) }; NUM_PHIL];

/// The semaphore to wait for them all to finish.
static DONE_SEM: ESemaphore = ESemaphore::new(0);

/// Number of iterations of each philospher.
///
/// Should be long enough to exercise the test, but too
/// long and the test will timeout.  The delay calculated will randomly be between 25 and 775, and
/// there are two waits, so typically, each "eat" will take about a second.
const EAT_COUNT: usize = 10;

#[embassy_executor::task]
pub async fn phil(spawner: Spawner, stats_sig: &'static ResultSignal) {
    // Our overall stats.
    let stats = Arc::new(Mutex::new(Stats::default()));

    // Spawn off each philosopher.
    for i in 0..NUM_PHIL {
        let forks = if i == NUM_PHIL - 1 {
            [&FORKS[0], &FORKS[i]]
        } else {
            [&FORKS[i], &FORKS[i + 1]]
        };

        spawner.spawn(one_phil(forks, i, stats.clone())).unwrap();
    }

    // Wait for them all to finish.
    DONE_SEM.acquire(NUM_PHIL).await.unwrap();

    // Send the stats back.
    stats_sig.signal(stats);
}

/// Simulate a single philospher.
///
/// The forks must be ordered with the first fork having th lowest number, otherwise this will
/// likely deadlock.
///
/// This will run for EAT_COUNT times, and then return.
#[embassy_executor::task(pool_size = NUM_PHIL)]
async fn one_phil(
    forks: [&'static ESemaphore; 2],
    n: usize,
    stats: Arc<Mutex<CriticalSectionRawMutex, Stats>>,
) {
    for i in 0..EAT_COUNT {
        // Acquire the forks.
        // printkln!("Child {n} take left fork");
        forks[0].acquire(1).await.unwrap().disarm();
        // printkln!("Child {n} take right fork");
        forks[1].acquire(1).await.unwrap().disarm();

        // printkln!("Child {n} eating");
        let delay = get_random_delay(n, 25);
        Timer::after(delay).await;
        stats.lock().await.record_eat(n, delay);

        // Release the forks.
        // printkln!("Child {n} giving up forks");
        forks[1].release(1);
        forks[0].release(1);

        let delay = get_random_delay(n, 25);
        Timer::after(delay).await;
        stats.lock().await.record_think(n, delay);

        printkln!("Philospher {n} finished eating time {i}");
    }

    DONE_SEM.release(1);
}
