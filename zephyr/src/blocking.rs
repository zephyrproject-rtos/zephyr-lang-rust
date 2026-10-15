// Copyright (c) 2026 Open Device Partnership and Contributors
// SPDX-License-Identifier: Apache-2.0

//! A pool of worker threads for offloading blocking Zephyr calls, with an async front-end.
//!
//! Some Zephyr subsystem APIs are synchronous and block the calling thread (for example, a fuel
//! gauge read that waits for an I2C transfer) and do not provide an asynchronous alternative.
//!
//! This module provides a way to asynchronously make such blocking calls without stalling
//! the executor thread, by offloading the blocking call onto a worker thread pool.
//!
//! The specifics of the thread pool can be configured via Kconfig.
//!
//! **Note**: This implementation is **NOT** priority-aware. Operations are submitted to the pool
//! in a FIFO manner regardless of the priority of the calling thread. Likewise, worker threads
//! remain at a fixed priority (configured via Kconfig) and do **NOT** inherit priority from the
//! calling thread.
//!
//! In the near future, this should be expanded to implement a priority-aware mechanism similar
//! to the existing P4WQ.
//!
//! # Examples
//!
//! ```ignore
//! zephyr::blocking::run(|| zephyr::time::sleep(zephyr::time::Duration::millis(500))).await;
//! ```

use core::{
    cell::UnsafeCell,
    ffi::{c_int, c_void},
    future::Future,
    marker::PhantomPinned,
    pin::Pin,
    ptr,
    task::{Context, Poll, Waker},
};

use embassy_sync::waitqueue::{AtomicWaker, MultiWakerRegistration};

use crate::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use crate::sync::SpinMutex;
use crate::sys::queue::Queue;

/// Number of worker threads, from `CONFIG_RUST_BLOCKING_POOL_THREADS`.
const THREADS: usize = crate::kconfig::CONFIG_RUST_BLOCKING_POOL_THREADS as usize;
/// Worker stack size, from `CONFIG_RUST_BLOCKING_POOL_STACK_SIZE`.
const STACK_SIZE: usize = crate::kconfig::CONFIG_RUST_BLOCKING_POOL_STACK_SIZE as usize;
/// Fixed worker priority, from `CONFIG_RUST_BLOCKING_POOL_PRIORITY`.
const PRIORITY: c_int = crate::kconfig::CONFIG_RUST_BLOCKING_POOL_PRIORITY as c_int;
/// Number of concurrent in-flight operations, from `CONFIG_RUST_BLOCKING_POOL_SLOTS`.
const SLOTS: usize = crate::kconfig::CONFIG_RUST_BLOCKING_POOL_SLOTS as usize;
/// Number of tasks that may wait for a slot at once, from `CONFIG_RUST_BLOCKING_POOL_WAITERS`.
const WAITERS: usize = crate::kconfig::CONFIG_RUST_BLOCKING_POOL_WAITERS as usize;

/// The shared FIFO the workers drain.
static QUEUE: Queue = Queue::new();

/// Static pool of slots.
///
/// TODO: This is needed to avoid allocation, though we could potentially add a simpler
/// implementation that relies on `Box` and thus doesn't need several Kconfig entries
/// if alloc is enabled.
static SLOT_POOL: SlotPool = SlotPool::new();

/// Run a blocking closure on the worker pool and await its result.
///
/// The closure must be `Send` because it is moved between threads,
/// and must be `'static` because it cannot borrow from the caller's stack since it may outlive
/// the caller on the worker thread.
///
/// **Note:** Because the closure is moved to a worker thread, you must ensure that it will not
/// overflow the worker thread's stack. The worker thread stack size is configured via Kconfig.
///
/// # Deadlock safety
///
/// There is potential for deadlock if all worker threads depend on the completion of another
/// operation in the queue that is not yet being worked on, so in general operations
/// should not depend on any other operations.
///
/// # Cancel safety
///
/// If the future is canceled before a worker thread picks up the operation,
/// then the operation is simply removed from the queue and never executes.
///
/// However, if a worker thread has already picked up the operation and begun executing it,
/// canceling will **NOT** stop the blocking operation. It will still run to completion,
/// but its result is simply discarded. Any side effects of the operation will still be seen.
pub fn run<F, R>(op: F) -> impl Future<Output = R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    BlockingFuture {
        shared: Shared {
            op: UnsafeCell::new(Some(op)),
            result: UnsafeCell::new(None),
        },
        slot: None,
        _pin: PhantomPinned,
    }
}

/// [`Slot`] state, stored internally in an `AtomicU8`.
#[repr(u8)]
enum SlotState {
    /// The slot is currently not in use.
    Free,
    /// The slot is taken by a future and is being initialized.
    Claimed,
    /// The slot is in the queue and a worker may pick it up (or already has).
    Queued,
    /// The work on the slot is complete and the slot contains a valid result.
    Done,
    /// The future holding the slot has been dropped,
    /// but a worker has already started running the operation and will discard the result.
    Orphaned,
}

/// Fixed-size storage for internal bookkeeping data.
///
/// Because this is stored within a Zephyr queue,
/// it must be `#[repr(C)]` and the first field is reserved for the kernel's use.
#[repr(C)]
struct Slot {
    /// Reserved for `k_queue`'s linkage while enqueued.
    _link: UnsafeCell<usize>,
    /// Keeps the future alive while a worker is using its memory.
    lock: SpinMutex<()>,
    /// The current [`SlotState`].
    state: AtomicU8,
    /// Points at the `Shared<F, R>` inside the future (null when not in use).
    shared: UnsafeCell<*mut ()>,
    /// Recovers the erased `F` and `R` when a worker runs the slot (`None` when not in use).
    runner: UnsafeCell<Option<unsafe fn(&'static Slot)>>,
    /// Woken once the result has been written by the worker.
    waker: AtomicWaker,
}

// SAFETY: The `lock` and `state` fields ensure `Slot` is mutated consistently and safely
// even when shared between threads.
unsafe impl Sync for Slot {}

impl Slot {
    const fn new() -> Self {
        Self {
            _link: UnsafeCell::new(0),
            lock: SpinMutex::new(()),
            state: AtomicU8::new(SlotState::Free as u8),
            shared: UnsafeCell::new(ptr::null_mut()),
            runner: UnsafeCell::new(None),
            waker: AtomicWaker::new(),
        }
    }
}

/// The fixed set of [`Slot`]s, and the tasks waiting for one to free up.
struct SlotPool {
    slots: [Slot; SLOTS],
    waiters: SpinMutex<MultiWakerRegistration<WAITERS>>,
}

impl SlotPool {
    const fn new() -> Self {
        Self {
            slots: [const { Slot::new() }; SLOTS],
            waiters: SpinMutex::new(MultiWakerRegistration::new()),
        }
    }

    /// Return a free [`Slot`] if available, otherwise register a waker and return `None`.
    fn claim(&'static self, waker: &Waker) -> Option<&'static Slot> {
        let mut waiters = self.waiters.lock().expect("Locking is infallible");
        let slot = self.slots.iter().find(|slot| {
            slot.state
                .compare_exchange(
                    SlotState::Free as u8,
                    SlotState::Claimed as u8,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
        });
        if slot.is_none() {
            waiters.register(waker);
        }
        slot
    }

    /// Release a slot back to the pool and wake every task waiting for one.
    fn release(&'static self, slot: &'static Slot) {
        slot.state.store(SlotState::Free as u8, Ordering::Release);
        self.waiters.lock().expect("Locking is infallible").wake();
    }
}

/// Data shared between a future and a worker.
struct Shared<F, R> {
    /// The blocking operation/closure to perform on a worker thread.
    ///
    /// Written by the future, taken by the worker before it blocks.
    op: UnsafeCell<Option<F>>,
    /// The result from a blocking operation/closure.
    ///
    /// Written by the worker on completion, taken by the future.
    result: UnsafeCell<Option<R>>,
}

/// The future returned by [`run`].
struct BlockingFuture<F, R> {
    shared: Shared<F, R>,
    slot: Option<&'static Slot>,
    _pin: PhantomPinned,
}

impl<F, R> Future for BlockingFuture<F, R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<R> {
        // SAFETY: Nothing is moved out of the future, only the `slot` field is reassigned
        let this = unsafe { self.get_unchecked_mut() };

        // If polled for the first time, need to claim a slot and enqueue it
        let Some(slot) = this.slot else {
            // If no free slots, can't do much just yet so return Pending
            let Some(slot) = SLOT_POOL.claim(cx.waker()) else {
                return Poll::Pending;
            };

            // We have a slot, so get it ready then send it off to the work queue
            this.slot = Some(slot);
            slot.waker.register(cx.waker());

            // SAFETY: The slot is `Claimed`, so this future owns it exclusively and no worker can
            // observe it until it is enqueued below
            unsafe {
                *slot.shared.get() = &this.shared as *const Shared<F, R> as *mut ();
                *slot.runner.get() = Some(run_op::<F, R>);
            }

            start_workers_once();
            slot.state.store(SlotState::Queued as u8, Ordering::Release);

            // SAFETY: `slot` is `'static`, so it outlives the queued item and isn't moved
            // The first field of a `Slot` is a `usize` and is not modified by our code
            unsafe { QUEUE.send(slot as *const Slot as *mut c_void) };

            return Poll::Pending;
        };

        slot.waker.register(cx.waker());
        if slot.state.load(Ordering::Acquire) == SlotState::Done as u8 {
            // SAFETY: The worker published `Done` after writing the result and does not
            // touch the shared data afterwards
            let result = unsafe { (*this.shared.result.get()).take() }
                .expect("Blocking slot reported Done without a result");

            this.slot = None;
            SLOT_POOL.release(slot);
            Poll::Ready(result)
        } else {
            Poll::Pending
        }
    }
}

impl<F, R> Drop for BlockingFuture<F, R> {
    fn drop(&mut self) {
        // This function exists to reduce binary size bloat, especially if we have many
        // monomorphized `BlockingFuture`s.
        //
        // See: https://www.possiblerust.com/pattern/non-generic-inner-functions
        fn cancel(slot: &'static Slot) {
            // SAFETY: `slot` was passed to `QUEUE.send` by this same pointer
            if unsafe { QUEUE.remove(slot as *const Slot as *mut c_void) } {
                // We beat every worker to it, so nothing ran and the closure drops with us
                SLOT_POOL.release(slot);
                return;
            }

            let orphaned = {
                let _guard = slot.lock.lock().expect("Locking is infallible");
                if slot.state.load(Ordering::Acquire) == SlotState::Done as u8 {
                    false
                } else {
                    // SAFETY: Held under `lock`, so no worker is reading the pointer right now
                    // After this store the worker discards its result instead of writing it back
                    unsafe { *slot.shared.get() = ptr::null_mut() };
                    slot.state
                        .store(SlotState::Orphaned as u8, Ordering::Release);
                    true
                }
            };

            if !orphaned {
                SLOT_POOL.release(slot);
            }
        }

        if let Some(slot) = self.slot {
            cancel(slot);
        }
    }
}

/// Runner for one closure type, executed on a worker thread.
///
/// The closure is moved onto this thread's stack before it runs, so the blocking call itself never
/// touches the future's memory and never runs with the slot lock held.
///
/// # Safety
///
/// `slot` must have been wired up for `F` and `R` by [`BlockingFuture::poll`], and must have been
/// dequeued from [`QUEUE`] (so that this thread owns it).
unsafe fn run_op<F, R>(slot: &'static Slot)
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let op = {
        let _guard = slot.lock.lock().expect("Locking is infallible");
        if slot.state.load(Ordering::Acquire) == SlotState::Orphaned as u8 {
            None
        } else {
            // SAFETY: We hold `lock` and the state is not `Orphaned`,
            // so the future is still alive and cannot be dropped until we release the lock
            let shared = unsafe { &*(*slot.shared.get() as *const Shared<F, R>) };

            // SAFETY: The same lock excludes the future, so this cell is ours to take from
            unsafe { (*shared.op.get()).take() }
        }
    };

    // Note: This isn't factored into the above to avoid nested spinlocks
    let Some(op) = op else {
        SLOT_POOL.release(slot);
        return;
    };

    // The blocking call, on our own stack, with no lock held
    let mut result = Some(op());

    let orphaned = {
        let _guard = slot.lock.lock().expect("Locking is infallible");
        if slot.state.load(Ordering::Acquire) == SlotState::Orphaned as u8 {
            true
        } else {
            // SAFETY: Same as above, we hold the lock so the future can't drop yet
            let shared = unsafe { &*(*slot.shared.get() as *const Shared<F, R>) };

            // SAFETY: The same lock excludes the future, so this cell is ours to write
            unsafe { *shared.result.get() = result.take() };

            slot.state.store(SlotState::Done as u8, Ordering::Release);
            false
        }
    };

    if orphaned {
        SLOT_POOL.release(slot);
    } else {
        slot.waker.wake();
    }
}

/// Lazily spawn the worker pool once, on first call.
fn start_workers_once() {
    static STARTED: AtomicBool = AtomicBool::new(false);

    if STARTED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        for _ in 0..THREADS {
            let worker = worker();
            worker.set_priority(PRIORITY);
            worker.start();
        }
    }
}

/// Worker thread body: drain the shared queue forever, running each slot's operation.
#[zephyr::thread(stack_size = STACK_SIZE, pool_size = THREADS)]
fn worker() {
    loop {
        // SAFETY: `recv` returns a pointer previously passed to `send`,
        // which is always a slot in `SLOT_POOL`
        let item = unsafe { QUEUE.recv(crate::time::Forever) };

        // SAFETY: Same as above, `SLOT_POOL` is `'static`, and this is never null because waiting
        // `Forever` only returns null via `k_queue_cancel_wait`, which we never call
        let slot: &'static Slot = unsafe { &*(item as *const Slot) };

        // SAFETY: `runner` was set before the slot was enqueued, and only this worker can release
        // the slot from here on, so it cannot be changed underneath us
        let runner = unsafe { (*slot.runner.get()).expect("Queued blocking slot with no runner") };

        // SAFETY: We own the dequeued slot, and `runner` is paired with its shared data
        unsafe { runner(slot) };
    }
}
