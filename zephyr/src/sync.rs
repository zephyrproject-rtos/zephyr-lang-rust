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

#[cfg(CONFIG_RUST_ALLOC)]
pub use portable_atomic_util::Arc;

mod mutex;

pub use mutex::{
    Mutex,
    MutexGuard,
    Condvar,
    LockResult,
    TryLockResult,
};

mod spinmutex;

pub use spinmutex::{
    SpinMutex,
    SpinMutexGuard,
};
