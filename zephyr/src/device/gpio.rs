//! Most devices in Zephyr operate on a `struct device`.  This provides untyped access to
//! devices.  We want to have stronger typing in the Zephyr interfaces, so most of these types
//! will be wrapped in another structure.  This wraps a Gpio device, and provides methods to
//! most of the operations on gpios.
//!
//! Safey: In general, even just using gpio pins is unsafe in Zephyr.  The gpio drivers are used
//! pervasively throughout Zephyr device drivers.  As such, most of the calls in this module are
//! unsafe.

use crate::raw;
use super::Unique;

/// Global instance to help make gpio in Rust slightly safer.
///
/// To help with safety, the rust types use a global instance of a gpio-token.  Methods will
/// take a mutable reference to this, which will require either a single thread in the
/// application code, or something like a mutex or critical section to manage.  The operation
/// methods are still unsafe, because we have no control over what happens with the gpio
/// operations outside of Rust code, but this will help make the Rust usage at least better.
pub struct GpioToken(());

static GPIO_TOKEN: Unique = Unique::new();

impl GpioToken {
    /// Retrieves the gpio token.  This is unsafe because lots of code in zephyr operates on the
    /// gpio drivers.
    pub unsafe fn get_instance() -> Option<GpioToken> {
        if !GPIO_TOKEN.once() {
            return None;
        }
        Some(GpioToken(()))
    }
}

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

/// A GpioPin represents a single pin on a gpio device.
///
/// This is a lightweight wrapper around the Zephyr `gpio_dt_spec` structure.  Note that
/// multiple pins may share a gpio controller, and as such, all methods on this are both unsafe,
/// and require a mutable reference to the [`GpioToken`].
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
    pub unsafe fn configure(&mut self, _token: &mut GpioToken, extra_flags: raw::gpio_flags_t) {
        // TODO: Error?
        unsafe {
            raw::gpio_pin_configure(self.pin.port,
                self.pin.pin,
                self.pin.dt_flags as raw::gpio_flags_t | extra_flags);
        }
    }

    /// Toggle pin level.
    pub unsafe fn toggle_pin(&mut self, _token: &mut GpioToken) {
        // TODO: Error?
        unsafe {
            raw::gpio_pin_toggle_dt(&self.pin);
        }
    }
}
