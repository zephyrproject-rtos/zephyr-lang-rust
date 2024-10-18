//! Device wrappers
//!
//! This module contains implementations of wrappers for various types of devices in zephyr.  In
//! general, these wrap a `*const device` from Zephyr, and provide an API that is appropriate.
//!
//! Most of these instances come from the device tree.

// Allow for a Zephyr build that has no devices at all.
#![allow(dead_code)]

use crate::sync::atomic::{AtomicBool, Ordering};

pub mod gpio;
pub mod flash;
pub mod uart;

// Allow dead code, because it isn't required for a given build to have any devices.
/// Device uniqueness.
///
/// As the zephyr devices are statically defined structures, this `Unique` value ensures that the
/// user is only able to get a single instance of any given device.
///
/// Note that some devices in zephyr will require more than one instance of the actual device.  For
/// example, a [`GpioPin`] will reference a single pin, but the underlying device for the gpio
/// driver will be shared among then.  Generally, the constructor for the individual device will
/// call `get_instance_raw()` on the underlying device.
pub(crate) struct Unique(pub(crate) AtomicBool);

impl Unique {
    // Note that there are circumstances where these are in zero-initialized memory, so false must
    // be used here, and the result of `once` inverted.
    /// Construct a new unique counter.
    pub(crate) const fn new() -> Unique {
        Unique(AtomicBool::new(false))
    }

    /// Indicates if this particular entity can be used.  This function, on a given `Unique` value
    /// will return true exactly once.
    pub(crate) fn once(&self) -> bool {
        // `fetch_add` is likely to be faster than compare_exchage.  This does have the limitation
        // that `once` is not called more than `usize::MAX` times.
        !self.0.fetch_or(true, Ordering::AcqRel)
    }
}
