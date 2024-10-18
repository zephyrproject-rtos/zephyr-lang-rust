//! Device wrappers
//!
//! This module contains implementations of wrappers for various types of devices in zephyr.  In
//! general, these wrap a `*const device` from Zephyr, and provide an API that is appropriate.
//!
//! Most of these instances come from the device tree.

use crate::sync::atomic::{AtomicUsize, Ordering};

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
#[allow(dead_code)]
pub(crate) struct Unique(pub(crate) AtomicUsize);

impl Unique {
    /// Construct a new unique counter.
    pub(crate) const fn new() -> Unique {
        Unique(AtomicUsize::new(0))
    }

    /// Indicates if this particular entity can be used.  This function, on a given `Unique` value
    /// will return true exactly once.
    #[allow(dead_code)]
    pub(crate) fn once(&self) -> bool {
        // `fetch_add` is likely to be faster than compare_exchage.  This does have the limitation
        // that `once` is not called more than `usize::MAX` times.
        self.0.fetch_add(1, Ordering::AcqRel) == 0
    }
}

pub mod gpio {
    //! Most devices in Zephyr operate on a `struct device`.  This provides untyped access to
    //! devices.  We want to have stronger typing in the Zephyr interfaces, so most of these types
    //! will be wrapped in another structure.  This wraps a Gpio device, and provides methods to
    //! most of the operations on gpios.

    use crate::raw;
    use super::Unique;

    /// A single instance of a zephyr device to manage a gpio controller.  A gpio controller
    /// represents a set of gpio pins, that are generally operated on by the same hardware block.
    pub struct Gpio {
        /// The underlying device itself.
        #[allow(dead_code)]
        pub(crate) device: *const raw::device,
    }

    impl Gpio {
        /// Constructor, used by the devicetree generated code.
        ///
        /// TODO: Guarantee single instancing.
        pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device) -> Option<Gpio> {
            if !unique.once() {
                return None;
            }
            Some(Gpio { device })
        }

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
        /// Constructor, used by the devicetree generated code.
        ///
        /// TODO: Guarantee single instancing.
        pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device, pin: u32, dt_flags: u32) -> Option<GpioPin> {
            if !unique.once() {
                return None;
            }
            Some(GpioPin {
                pin: raw::gpio_dt_spec {
                    port: device,
                    pin: pin as raw::gpio_pin_t,
                    dt_flags: dt_flags as raw::gpio_dt_flags_t,
                }
            })
        }

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
    use super::Unique;

    /// A flash controller
    ///
    /// This is a wrapper around the `struct device` in Zephyr that represents a flash controller.
    /// Using the flash controller allows flash operations on the entire device.  See
    /// [`FlashPartition`] for a wrapper that limits the operation to a partition as defined in the
    /// DT.
    #[allow(dead_code)]
    pub struct FlashController {
        pub(crate) device: *const raw::device,
    }

    impl FlashController {
        /// Constructor, intended to be called by devicetree generated code.
        pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device) -> Option<FlashController> {
            if !unique.once() {
                return None;
            }

            Some(FlashController { device })
        }
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

    impl FlashPartition {
        /// Constructor, intended to be called by devicetree generated code.
        pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device, offset: u32, size: u32) -> Option<FlashPartition> {
            if !unique.once() {
                return None;
            }

            // The `get_instance` on the flash controller would try to guarantee a unique instance,
            // but in this case, we need one for each device, so just construct it here.
            // TODO: This is not actually safe.
            let controller = FlashController { device };
            Some(FlashPartition { controller, offset, size })
        }
    }

    // Note that currently, the flash partition shares the controller, so the underlying operations
    // are not actually safe.  Need to rethink how to manage this.
}
