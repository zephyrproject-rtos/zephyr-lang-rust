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

pub mod critical {
    //! Zephyr implementation of critical sections.
    //!
    //! Critical sections from Rust are handled with a single Zephyr spinlock.  This doesn't allow
    //! any nesting, but neither does the `critical-section` crate.

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

pub mod gpio {
    //! Most devices in Zephyr operate on a `struct device`.  This provides untyped access to
    //! devices.  We want to have stronger typing in the Zephyr interfaces, so most of these types
    //! will be wrapped in another structure.  This wraps a Gpio device, and provides methods to
    //! most of the operations on gpios.

    use crate::raw;

    /// A single instance of a zephyr device to manage a gpio controller.  A gpio controller
    /// represents a set of gpio pins, that are generally operated on by the same hardware block.
    pub struct Gpio {
        /// The underlying device itself.
        #[allow(dead_code)]
        pub(crate) device: *const raw::device,
    }

    impl Gpio {
        /// Verify that the device is ready for use.  At a minimum, this means the device has been
        /// successfully initialized.
        pub fn is_ready(&self) -> bool {
            unsafe {
                raw::device_is_ready(self.device)
            }
        }
    }

    /// A GpioPin represents a single pin on a gpio device.  This is a lightweight wrapper around
    /// the Zephyr `gpio_dt_spec` structure.
    #[allow(dead_code)]
    pub struct GpioPin {
        pub(crate) pin: raw::gpio_dt_spec,
    }

    impl GpioPin {
        /// Verify that the device is ready for use.  At a minimum, this means the device has been
        /// successfully initialized.
        pub fn is_ready(&self) -> bool {
            self.get_gpio().is_ready()
        }

        /// Get the underlying Gpio device.
        pub fn get_gpio(&self) -> Gpio {
            Gpio {
                device: self.pin.port,
            }
        }

        /// Configure a single pin.
        pub fn configure(&mut self, extra_flags: raw::gpio_flags_t) {
            // TODO: Error?
            unsafe {
                raw::gpio_pin_configure(self.pin.port,
                    self.pin.pin,
                    self.pin.dt_flags as raw::gpio_flags_t | extra_flags);
            }
        }

        /// Toggle pin level.
        pub fn toggle_pin(&mut self) {
            // TODO: Error?
            unsafe {
                raw::gpio_pin_toggle_dt(&self.pin);
            }
        }
    }
}

pub mod flash {
    //! Device wrappers for flash controllers, and flash partitions.

    use crate::raw;

    #[allow(dead_code)]
    pub struct FlashController {
        pub(crate) device: *const raw::device,
    }

    /// A wrapper for flash partitions.  There is no Zephyr struct that corresponds with this
    /// information, which is typically used in a more direct underlying manner.
    #[allow(dead_code)]
    pub struct FlashPartition {
        /// The underlying controller.
        #[allow(dead_code)]
        pub(crate) controller: FlashController,
        #[allow(dead_code)]
        pub(crate) offset: u32,
        #[allow(dead_code)]
        pub(crate) size: u32,
    }
}
