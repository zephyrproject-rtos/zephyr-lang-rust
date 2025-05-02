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
    unsafe { crate::raw::k_uptime_get() }
}

// The below implementation, based on interrupt locking has only been tested on single CPU.  The
// implementation suggests it should work on SMP, and can be tested.  The docs for irq_lock()
// explicitly state that it cannot be used from userspace. Unfortunately, spinlocks have
// incompatible semantics with critical sections, so to work with userspace we'd need probably a
// syscall.
#[cfg(CONFIG_USERSPACE)]
compile_error!("Critical-section implementation does not work with CONFIG_USERSPACE");

pub mod critical {
    //! Zephyr implementation of critical sections.
    //!
    //! The critical-section crate explicitly states that critical sections can be nested.
    //! Unfortunately, Zephyr spinlocks cannot be nested.  It is possible to nest different ones,
    //! but the critical-section implementation API doesn't give access to the stack.

    use core::{
        ffi::c_int,
        sync::atomic::{fence, Ordering},
    };

    use critical_section::RawRestoreState;
    use zephyr_sys::{zr_irq_lock, zr_irq_unlock};

    struct ZephyrCriticalSection;
    critical_section::set_impl!(ZephyrCriticalSection);

    unsafe impl critical_section::Impl for ZephyrCriticalSection {
        unsafe fn acquire() -> RawRestoreState {
            let res = zr_irq_lock();
            fence(Ordering::Acquire);
            res as RawRestoreState
        }

        unsafe fn release(token: RawRestoreState) {
            fence(Ordering::Release);
            zr_irq_unlock(token as c_int);
        }
    }
}
