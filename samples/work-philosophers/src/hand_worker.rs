//! Work-queue work, with hand-constructed work.
//!
//! This module tries to demonstrate what a hand-crafted system based on work-queues might be like.
//!
//! As such, this isn't built around any synchronization mechanimsms other than signal.  In some
//! sense, this is structured more like various workers that are coordinating with devices, except
//! that we will use timeouts for the pauses.

use core::{future::Future, pin::Pin, task::{Context, Poll}};

use alloc::vec;
use alloc::vec::Vec;
use zephyr::{kio::{spawn, ContextExt}, printkln, sync::{Arc, SpinMutex}, time::Forever, work::{futures::JoinHandle, Signal, WorkQueue}};

use crate::{get_random_delay, NUM_PHIL};
pub use crate::Stats;

pub fn phil(workq: &WorkQueue) -> Manager {
    let wake_manager = Arc::new(Signal::new().unwrap());

    let actions: Vec<_> = (0..NUM_PHIL).map(|_| Arc::new(Action::new(wake_manager.clone()))).collect();

    let phils: Vec<_> = (0..NUM_PHIL)
        .map(|i| Phil::new(actions[i].clone(), i))
        .map(|act| spawn(act, workq, c"phil"))
        .collect();

    Manager {
        request: wake_manager,
        actions,
        phils,
        forks: vec![ForkState::Idle; NUM_PHIL],
    }
}

#[derive(Copy, Clone, Debug)]
enum ForkState {
    /// Nobody is using the fork.
    Idle,
    /// A single philospher is eating with this fork.
    Eating,
    /// Someone is eating, and the numbered philospher is also waiting to use it.
    Waiting(usize),
}

/// Outer Phil is the main event handler for the work queue system.
pub struct Manager {
    actions: Vec<Arc<Action>>,
    phils: Vec<JoinHandle<Phil>>,
    /// The signal to wake the manager up.
    request: Arc<Signal>,

    // The state of each fork.
    forks: Vec<ForkState>,
}

impl Future for Manager {
    type Output = Stats;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        // Run through the actions, and see what they have to do.
        printkln!("Manager running");

        // Clear out signal before any processing, so it can be set.
        self.request.reset();

        // Loop through all of the actions.
        for i in 0..self.actions.len() {
        // for (i, act) in self.actions.iter().enumerate() {
            let act = &self.actions[i];
            let mut change = None;
            let mut lock = act.fork.lock().unwrap();
            match *lock {
                ForkRequest::Idle => (),
                ForkRequest::Waiting => (),
                ForkRequest::Take(f) => {
                    printkln!("phil {i} wants fork {f}: state {:?}", self.forks[f]);
                    match self.forks[f] {
                        ForkState::Idle => {
                            assert!(change.is_none());

                            // This philospher can have this fork.
                            change = Some((f, ForkState::Eating));

                            // And let them know they got it.
                            *lock = ForkRequest::Idle;
                            act.wake_phil.raise(-1).unwrap();
                        }
                        ForkState::Eating => {
                            // The fork is busy, but remember who is waiting for it.
                            assert!(change.is_none());
                            change = Some((f, ForkState::Waiting(i)));
                            *lock = ForkRequest::Waiting;
                        }
                        ForkState::Waiting(i2) => {
                            // This indicates the forks were not assigned to the philosphers
                            // correctly.
                            panic!("Too many philosphers requesting same fork {i} {i2}");
                        }
                    }
                }
                ForkRequest::Give(f) => {
                    printkln!("phil {i} releases fork {f}: state {:?}", self.forks[f]);
                    match self.forks[f] {
                        ForkState::Idle => {
                            panic!("Philospher returned a fork it did not have");
                        }
                        ForkState::Eating => {
                            // This philospher was the only one using this fork.
                            assert!(change.is_none());
                            change = Some((f, ForkState::Idle));

                            // And let them now that was fine.
                            *lock = ForkRequest::Idle;
                            // TODO: Move this raise to after the lock to shorten the time spent
                            // holding the lock.
                            act.wake_phil.raise(-2).unwrap();
                        }
                        ForkState::Waiting(i2) => {
                            // We (i) are done with the fork, and can now give it to i2.
                            // The state changes to Eating to indicate one waiter.
                            assert!(change.is_none());
                            change = Some((f, ForkState::Eating));

                            // We inform current philospher that we have handled them being done
                            // with the fork.
                            *lock = ForkRequest::Idle;
                            act.wake_phil.raise(-1).unwrap();

                            // And inform the waiter that they can continue.
                            *self.actions[i2].fork.lock().unwrap() = ForkRequest::Idle;
                            self.actions[i2].wake_phil.raise(-2).unwrap();
                        }
                    }
                }
            }
            drop(lock);
            if let Some((f, state)) = change {
                self.forks[f] = state;
            }
        }

        // Unless we're completely done, set to wake on our own signal.
        cx.add_signal(&self.request, Forever);
        printkln!("Manager pending");
        Poll::Pending
    }
}

/// Captures requests from a philospher for exclusive use of some forks.
///
/// This works by having the philosopher set the request, and the Manager sets this to Idle when it
/// has been satisfied.  Each will signal the other after setting this value.
struct Action {
    fork: SpinMutex<ForkRequest>,
    wake_manager: Arc<Signal>,
    wake_phil: Signal,
}

impl Action {
    fn new(wake_manager: Arc<Signal>) -> Self {
        Self {
            fork: SpinMutex::new(ForkRequest::Idle),
            wake_manager,
            wake_phil: Signal::new().unwrap(),
        }
    }
}

/// A single request concerning a fork.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ForkRequest {
    /// There is no request pending.
    Idle,
    /// Take the given numbered fork.
    Take(usize),
    /// Give back the give numbered fork.
    Give(usize),
    /// The philospher has requested a fork, but it is not available.
    Waiting,
}

/// The state of a philospher.
#[derive(Debug, Copy, Clone)]
enum PhilState {
    /// The initial state.
    Init,
    /// Wait upon getting the given fork.
    Take(usize),
    /// Eating is happening, should wake from a sleep.
    Eating,
    /// Waiting on returning the given fork.
    Give(usize),
    /// Resting between bytes.
    Resting,
    /// Done with the whole thing.  poll will always return Ready after setting this.
    Done,
}

/// Phil represents a single dining philospher.
///
/// Each Phil runs on its own, making its requests to 'req' for taking and returning the forks.
struct Phil {
    /// Which philospher are we?
    index: usize,
    /// Our view of our actions.
    action: Arc<Action>,
    /// Current state of this philosopher.
    state: PhilState,
    /// How many times have we finished.
    count: usize,
    /// The forks we should be using.
    forks: [usize; 2],
}

impl Phil {
    fn new(action: Arc<Action>, index: usize) -> Self {
        let forks = if index == NUM_PHIL - 1 {
            [0, index]
        } else {
            [index, index + 1]
        };

        Self {
            index,
            action,
            state: PhilState::Init,
            count: 0,
            forks,
        }
    }
}

impl Future for Phil {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        printkln!("Phil run: {}, {:?}", self.index, self.state);

        match self.state {
            PhilState::Init => {
                // Initially, request the first fork to be taken.
                *self.action.fork.lock().unwrap() = ForkRequest::Take(self.forks[0]);
                self.action.wake_manager.raise(self.index as i32).unwrap();

                // Next wake event is the signal.
                cx.add_signal(&self.action.wake_phil, Forever);

                self.state = PhilState::Take(0);
            }
            PhilState::Take(f) => {
                // Check that we were actually supposed to wake up.  There shouldn't be spurious
                // wakeups in this direction.
                let cur_state = *self.action.fork.lock().unwrap();
                if cur_state != ForkRequest::Idle {
                    panic!("State error, taken fork should be idle: {:?}", cur_state);
                }

                if f == 1 {
                    // Both are taken, We're eating, and we wait by timeout.
                    printkln!("Phil {} eating", self.index);

                    self.state = PhilState::Eating;
                    let delay = get_random_delay(self.index, 25);
                    cx.add_timeout(delay);
                } else {
                    // First fork taken, wait for second.
                    printkln!("Setting state to {:?}", ForkRequest::Take(self.forks[1]));
                    *self.action.fork.lock().unwrap() = ForkRequest::Take(self.forks[1]);
                    self.action.wake_manager.raise(self.index as i32).unwrap();

                    cx.add_signal(&self.action.wake_phil, Forever);

                    self.state = PhilState::Take(1);
                }
            }
            PhilState::Eating => todo!(),
            PhilState::Give(_) => todo!(),
            PhilState::Resting => todo!(),
            PhilState::Done => todo!(),
        }

        Poll::Pending
    }
}
