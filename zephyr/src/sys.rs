// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr 'sys' module.
//!
//! The `zephyr-sys` crate contains the direct C bindings to the Zephyr API.  All of these are
//! unsafe.
//!
//! This module `zephyr::sys` contains thin wrappers to these C bindings, that can be used without
//! unsafe, but as unchanged as possible.

use zephyr_sys::k_timeout_t;

pub mod queue;
pub mod sync;
pub mod thread;

// These two constants are not able to be captured by bindgen.  It is unlikely that these values
// would change in the Zephyr headers, but there will be an explicit test to make sure they are
// correct.

/// Represents a timeout with an infinite delay.
///
/// Low-level Zephyr constant.  Calls using this value will wait as long as necessary to perform
/// the requested operation.
pub const K_FOREVER: k_timeout_t = k_timeout_t { ticks: -1 };

/// Represents a null timeout delay.
///
/// Low-level Zephyr Constant.  Calls using this value will not wait if the operation cannot be
/// performed immediately.
pub const K_NO_WAIT: k_timeout_t = k_timeout_t { ticks: 0 };

/// Return the current uptime of the system in ms.
///
/// Direct Zephyr call.  Precision is limited by the system tick timer.
#[inline]
pub fn uptime_get() -> i64 {
    unsafe {
        crate::raw::k_uptime_get()
    }
}

/// Busy wait.
///
/// Busy wait for a give number of microseconds.  This directly calls `zephyr_sys::k_busy_wait`.
///
/// Zephyr has numerous caveats on configurations where this function doesn't work.
pub use zephyr_sys::k_busy_wait as busy_wait;

pub mod critical {
    //! Zephyr implementation of critical sections.
    //!
    //! Critical sections from Rust are handled with a single Zephyr spinlock.  This doesn't allow
    //! any nesting, but neither does the `critical-section` crate.
    //!
    //! This provides the underlying critical section crate, which is useful for external crates
    //! that want this interface.  However, it isn't a particularly hygienic interface to use.  For
    //! something a bit nicer, please see [`sync::SpinMutex`].

    use core::{ffi::c_int, ptr::addr_of_mut};

    use critical_section::RawRestoreState;
    use zephyr_sys::{k_spinlock, k_spin_lock, k_spin_unlock, k_spinlock_key_t};

    struct ZephyrCriticalSection;
    critical_section::set_impl!(ZephyrCriticalSection);

    // The critical section shares a single spinlock.
    static mut LOCK: k_spinlock = unsafe { core::mem::zeroed() };

    unsafe impl critical_section::Impl for ZephyrCriticalSection {
        unsafe fn acquire() -> RawRestoreState {
            let res = k_spin_lock(addr_of_mut!(LOCK));
            res.key as RawRestoreState
        }

        unsafe fn release(token: RawRestoreState) {
            k_spin_unlock(addr_of_mut!(LOCK),
                          k_spinlock_key_t {
                              key: token as c_int,
                          });
        }
    }
}
