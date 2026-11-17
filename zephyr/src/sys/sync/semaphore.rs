// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr Semaphore support
//!
//! This is a thin wrapper around Zephyr's `k_sem`.  This is one of the few of the `sys` primitives
//! in Zephyr that is actually perfectly usable on its own, without needing additional wrappers.
//!
//! Zephyr implements counting semaphores, with both an upper and lower bound on the count.  Note
//! that calling 'give' on a semaphore that is at the maximum count will discard the 'give'
//! operation, which in situation where counting is actually desired, will result in the count being
//! incorrect.

use core::ffi::c_uint;
use core::fmt;

use crate::object::{ObjectInit, ZephyrObject};
use crate::{
    error::{to_result_void, Result},
    raw::{k_sem, k_sem_count_get, k_sem_give, k_sem_init, k_sem_reset, k_sem_take},
    time::Timeout,
};

pub use crate::raw::K_SEM_MAX_LIMIT;

/// General Zephyr Semaphores
pub struct Semaphore(pub(crate) ZephyrObject<k_sem>);

/// By nature, Semaphores are both Sync and Send.  Safety is handled by the underlying Zephyr
/// implementation (which is why Clone is also implemented).
unsafe impl Sync for Semaphore {}
unsafe impl Send for Semaphore {}

impl Semaphore {
    /// Return the initialized native semaphore for C interoperability.
    ///
    /// The static borrow prevents moving or dropping the kernel object while
    /// native code holds its pointer. Callers must preserve Zephyr's semaphore
    /// invariants: in particular, do not reinitialize it or mutate its fields.
    pub fn as_raw(&'static self) -> *mut k_sem {
        // SAFETY: The static borrow keeps the initialized object stationary
        // and alive for every subsequent use of the returned pointer.
        unsafe { self.0.get() }
    }

    /// Take a semaphore without blocking the async executor thread.
    ///
    /// Enable the `async-semaphores` Cargo feature and set `CONFIG_POLL=y`
    /// and `CONFIG_RUST_ALLOC=y`. Poll and drop this future in thread context
    /// on the [Zephyr Embassy executor](crate::embassy::Executor) running in a
    /// supervisor thread. Each pending wait allocates bookkeeping, and the
    /// executor allocates a poll snapshot when there are registered waits.
    ///
    /// ```
    /// use zephyr::sys::sync::Semaphore;
    ///
    /// static READY: Semaphore = Semaphore::new(0, 1);
    ///
    /// async fn consumer() {
    ///     READY.take_async().await;
    /// }
    /// ```
    ///
    /// Rust producers can call [`Semaphore::give`]. C code and interrupt
    /// handlers can give the native semaphore using `k_sem_give()` and the
    /// pointer from [`Semaphore::as_raw`]. For a C-owned static semaphore, use
    /// [`SemaphoreRef::from_raw`] to create a handle.
    ///
    /// The executor uses Zephyr's `k_poll()` to detect availability. A
    /// notification does not reserve a token. Async waiters retry with a
    /// nonblocking take, and competing synchronous or async takers can acquire
    /// the token first. Async waiters have no FIFO ordering guarantee.
    ///
    /// Dropping a pending future unregisters the wait without consuming a
    /// token. To set a timeout, race the wait against an async timer using
    /// `embassy_futures::select::select`. Resetting the semaphore leaves async
    /// waits pending until a subsequent give. The static borrow keeps the
    /// native object alive while the executor holds a poll snapshot, even if a
    /// future is dropped or forgotten.
    ///
    /// # Cancellation across executors
    ///
    /// When multiple executors wait on the same semaphore, Zephyr can notify
    /// just one executor per give. If that executor cancels its wait before
    /// taking the token, another executor can remain asleep even though the
    /// token is available. A subsequent give can wake another executor. Keep
    /// cancellable waits on a given semaphore on the same executor when those
    /// waits must make progress without another give.
    #[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
    pub async fn take_async(&'static self) {
        SemaphoreRef(self.as_raw()).take_async().await
    }

    /// Create a new semaphore.
    ///
    /// Create a new dynamically allocated Semaphore.  This semaphore can only be used from system
    /// threads.  The arguments are as described in [the
    /// docs](https://docs.zephyrproject.org/latest/kernel/services/synchronization/semaphores.html).
    ///
    /// Note that this API has changed, and it now doesn't return a Result, since the Result time
    /// generally doesn't work (in stable rust) with const.
    pub const fn new(initial_count: c_uint, limit: c_uint) -> Semaphore {
        // Due to delayed init, we need to replicate the object checks in the C `k_sem_init`.

        if limit == 0 || initial_count > limit {
            panic!("Invalid semaphore initialization");
        }

        let this = <ZephyrObject<k_sem>>::new_raw();

        unsafe {
            let addr = this.get_uninit();
            (*addr).count = initial_count;
            (*addr).limit = limit;
        }

        // to_result_void(k_sem_init(item.get(), initial_count, limit))?;
        Semaphore(this)
    }

    /// Take a semaphore.
    ///
    /// Can be called from ISR if called with [`NoWait`].
    ///
    /// [`NoWait`]: crate::time::NoWait
    pub fn take<T>(&self, timeout: T) -> Result<()>
    where
        T: Into<Timeout>,
    {
        let timeout: Timeout = timeout.into();
        let ret = unsafe { k_sem_take(self.0.get(), timeout.0) };
        to_result_void(ret)
    }

    /// Give a semaphore.
    ///
    /// This routine gives to the semaphore, unless the semaphore is already at its maximum
    /// permitted count.
    pub fn give(&self) {
        unsafe { k_sem_give(self.0.get()) }
    }

    /// Resets a semaphor's count to zero.
    ///
    /// This resets the count to zero.  Any outstanding [`take`] calls will be aborted with
    /// `Error(EAGAIN)`.
    ///
    /// [`take`]: Self::take
    pub fn reset(&self) {
        unsafe { k_sem_reset(self.0.get()) }
    }

    /// Get a semaphore's count.
    ///
    /// Returns the current count.
    pub fn count_get(&self) -> usize {
        unsafe { k_sem_count_get(self.0.get()) as usize }
    }
}

impl ObjectInit<k_sem> for ZephyrObject<k_sem> {
    fn init(item: *mut k_sem) {
        // SAFEFY: Get the initial values used in new.  The address may have changed, but only due
        // to a move.
        unsafe {
            let count = (*item).count;
            let limit = (*item).limit;

            if k_sem_init(item, count, limit) != 0 {
                // Note that with delayed init, we cannot do anything with invalid values.  We're
                // replicated the check in `new` above, so would only catch semantic changes in the
                // implementation of `k_sem_init`.
                unreachable!();
            }
        }
    }
}

impl fmt::Debug for Semaphore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Semaphore")
    }
}

/// A shared handle to a native semaphore with a static lifetime.
///
/// Use this to await semaphores owned by C drivers without taking ownership or
/// reinitializing them. Constructing a handle requires validating the native
/// object's initialization and lifetime once; subsequent operations are safe.
#[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
#[derive(Clone, Copy)]
pub struct SemaphoreRef(*mut k_sem);

#[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
// SAFETY: Construction requires an initialized, static native semaphore.
// Zephyr serializes access to it, including from other threads and interrupts.
unsafe impl Send for SemaphoreRef {}
#[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
// SAFETY: Same native semaphore synchronization as Send above.
unsafe impl Sync for SemaphoreRef {}

#[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
impl SemaphoreRef {
    /// Borrow an initialized native semaphore for async use.
    ///
    /// # Safety
    ///
    /// `semaphore` must point to an initialized, properly aligned `k_sem` that
    /// remains alive and at the same address for the rest of the program.
    /// It must never be reinitialized or mutated outside Zephyr's semaphore
    /// API. The calling thread and executor must have permission to access it.
    pub unsafe fn from_raw(semaphore: *mut k_sem) -> Self {
        assert!(!semaphore.is_null());
        Self(semaphore)
    }

    /// Take a native semaphore asynchronously.
    ///
    /// Has the same executor, cancellation, and reset semantics as
    /// [`Semaphore::take_async`]. The handle never owns or destroys the object.
    pub async fn take_async(self) {
        crate::embassy::semaphore::take(self).await
    }

    pub(crate) fn as_raw(self) -> *mut k_sem {
        self.0
    }

    pub(crate) fn try_take(self) -> bool {
        // SAFETY: from_raw guarantees the object remains initialized and live.
        unsafe { k_sem_take(self.0, crate::sys::K_NO_WAIT) == 0 }
    }
}
