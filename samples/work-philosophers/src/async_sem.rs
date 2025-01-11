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

use core::cell::RefCell;

use alloc::{rc::Rc, vec::Vec};
use zephyr::{
    kio::{sleep, spawn_local},
    printkln,
    sys::sync::Semaphore,
    time::Forever,
};

use crate::{get_random_delay, Stats, NUM_PHIL};

/// Number of iterations of each philospher.
///
/// Should be long enough to exercise the test, but too
/// long and the test will timeout.  The delay calculated will randomly be between 25 and 775, and
/// there are two waits, so typically, each "eat" will take about a second.
const EAT_COUNT: usize = 10;

pub async fn phil() -> Stats {
    // It is a little tricky to be able to use local workers.  We have to have this nested thread
    // that waits.  This is because the Future from `local_phil()` does not implement Send, since it
    // waits for the philosophers, which are not Send.  However, this outer async function does not
    // hold onto any data that is not send, and therefore will be Send.  Fortunately, this extra
    // Future is very lightweight.
    spawn_local(local_phil(), c"phil_wrap").join_async().await
}

async fn local_phil() -> Stats {
    // Our overall stats.
    let stats = Rc::new(RefCell::new(Stats::default()));

    // One fork for each philospher.
    let forks: Vec<_> = (0..NUM_PHIL)
        .map(|_| Rc::new(Semaphore::new(1, 1).unwrap()))
        .collect();

    // Create all of the philosphers
    let phils: Vec<_> = (0..NUM_PHIL)
        .map(|i| {
            // Determine the two forks.  The forks are paired with each philosopher taking the fork of
            // their number, and the next on, module the size of the ring.  However, for the last case,
            // we need to swap the forks used, it is necessary to obey a strict ordering of the locks to
            // avoid deadlocks.
            let forks = if i == NUM_PHIL - 1 {
                [forks[0].clone(), forks[i].clone()]
            } else {
                [forks[i].clone(), forks[i + 1].clone()]
            };

            let phil = one_phil(forks, i, stats.clone());
            printkln!("Size of child {i}: {}", size_of_val(&phil));
            spawn_local(phil, c"phil")
        })
        .collect();

    // Wait for them all to finish.
    for p in phils {
        p.join_async().await;
    }

    // Leak the stats as a test.
    // Uncomment this to test that the expect below does truly detect a missed drop.
    // let _ = Rc::into_raw(stats.clone());

    // At this point, all of the philosphers should have dropped their stats ref, and we should be
    // able to turn stats back into it's value.
    // This tests that completed work does drop the future.
    Rc::into_inner(stats)
        .expect("Failure: a philospher didn't drop it's future")
        .into_inner()
}

/// Simulate a single philospher.
///
/// The forks must be ordered with the first fork having th lowest number, otherwise this will
/// likely deadlock.
///
/// This will run for EAT_COUNT times, and then return.
async fn one_phil(forks: [Rc<Semaphore>; 2], n: usize, stats: Rc<RefCell<Stats>>) {
    for i in 0..EAT_COUNT {
        // Acquire the forks.
        // printkln!("Child {n} take left fork");
        forks[0].take_async(Forever).await.unwrap();
        // printkln!("Child {n} take right fork");
        forks[1].take_async(Forever).await.unwrap();

        // printkln!("Child {n} eating");
        let delay = get_random_delay(n, 25);
        sleep(delay).await;
        stats.borrow_mut().record_eat(n, delay);

        // Release the forks.
        // printkln!("Child {n} giving up forks");
        forks[1].give();
        forks[0].give();

        let delay = get_random_delay(n, 25);
        sleep(delay).await;
        stats.borrow_mut().record_think(n, delay);

        printkln!("Philospher {n} finished eating time {i}");
    }
}
