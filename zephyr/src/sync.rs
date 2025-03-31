//! Higher level synchronization primitives.
//!
//! These are modeled after the synchronization primitives in
//! [`std::sync`](https://doc.rust-lang.org/stable/std/sync/index.html) and those from
//! [`crossbeam-channel`](https://docs.rs/crossbeam-channel/latest/crossbeam_channel/), in as much
//! as it makes sense.

// Channels are currently only available with allocation.  Bounded channels later might be
// available.
#[cfg(CONFIG_RUST_ALLOC)]
pub mod channel;

pub mod atomic {
    //! Re-export portable atomic.
    //!
    //! Although `core` contains a
    //! [`sync::atomic`](https://doc.rust-lang.org/stable/core/sync/atomic/index.html) module,
    //! these are dependent on the target having atomic instructions, and the types are missing
    //! when the platform cannot support them.  Zephyr, however, does provide atomics on platforms
    //! that don't support atomic instructions, using spinlocks.  In the Rust-embedded world, this
    //! is done through the [`portable-atomic`](https://crates.io/crates/portable-atomic) crate,
    //! which will either just re-export the types from core, or provide an implementation using
    //! spinlocks when those aren't available.

    pub use portable_atomic::*;
}

use core::pin::Pin;

#[cfg(CONFIG_RUST_ALLOC)]
pub use portable_atomic_util::Arc;
#[cfg(CONFIG_RUST_ALLOC)]
pub use portable_atomic_util::Weak;

/// Safe Pinned Weak references.
///
/// Pin<Arc<T>> can't be converted to/from Weak safely, because there is know way to know if a given
/// weak reference came from a pinned Arc.  This wraps the weak reference in a new type so we know
/// that it came from a pinned Arc.
///
/// There is a pin-weak crate that provides this for `std::sync::Arc`, but not for the one in the
/// portable-atomic-utils crate.
pub struct PinWeak<T>(Weak<T>);

impl<T> PinWeak<T> {
    /// Downgrade an `Pin<Arc<T>>` into a `PinWeak`.
    ///
    /// This would be easier to use if it could be added to Arc.
    pub fn downgrade(this: Pin<Arc<T>>) -> Self {
        // SAFETY: we will never return anything other than a Pin<Arc<T>>.
        Self(Arc::downgrade(&unsafe { Pin::into_inner_unchecked(this) }))
    }

    /// Upgrade back to a `Pin<Arc<T>>`.
    pub fn upgrade(&self) -> Option<Pin<Arc<T>>> {
        // SAFETY: The weak was only constructed from a `Pin<Arc<T>>`.
        self.0
            .upgrade()
            .map(|arc| unsafe { Pin::new_unchecked(arc) })
    }
}

mod mutex;

pub use mutex::{Condvar, LockResult, Mutex, MutexGuard, TryLockError, TryLockResult};

mod spinmutex;

pub use spinmutex::{SpinMutex, SpinMutexGuard};
