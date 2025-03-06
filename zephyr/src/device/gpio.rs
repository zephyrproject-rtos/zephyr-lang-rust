//! Most devices in Zephyr operate on a `struct device`.  This provides untyped access to
//! devices.  We want to have stronger typing in the Zephyr interfaces, so most of these types
//! will be wrapped in another structure.  This wraps a Gpio device, and provides methods to
//! most of the operations on gpios.
//!
//! Safey: In general, even just using gpio pins is unsafe in Zephyr.  The gpio drivers are used
//! pervasively throughout Zephyr device drivers.  As such, most of the calls in this module are
//! unsafe.

use core::ffi::c_int;

use super::Unique;
use crate::raw;

/// Global instance to help make gpio in Rust slightly safer.
///
/// # Safety
///
/// To help with safety, the rust types use a global instance of a gpio-token.  Methods will
/// take a mutable reference to this, which will require either a single thread in the
/// application code, or something like a mutex or critical section to manage.  The operation
/// methods are still unsafe, because we have no control over what happens with the gpio
/// operations outside of Rust code, but this will help make the Rust usage at least better.
pub struct GpioToken(());

static GPIO_TOKEN: Unique = Unique::new();

impl GpioToken {
    /// Retrieves the gpio token.
    ///
    /// # Safety
    /// This is unsafe because lots of code in zephyr operates on the gpio drivers.  The user of the
    /// gpio subsystem, in general should either coordinate all gpio access across the system (the
    /// token coordinates this only within Rust code), or verify that the particular gpio driver and
    /// methods are thread safe.
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

// SAFETY: Gpio's can be shared with other threads.  Safety is maintained by the Token.
unsafe impl Send for Gpio {}

impl Gpio {
    /// Constructor, used by the devicetree generated code.
    ///
    /// TODO: Guarantee single instancing.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device) -> Option<Gpio> {
        if !unique.once() {
            return None;
        }
        Some(Gpio { device })
    }

    /// Verify that the device is ready for use.  At a minimum, this means the device has been
    /// successfully initialized.
    pub fn is_ready(&self) -> bool {
        unsafe { raw::device_is_ready(self.device) }
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

// SAFETY: GpioPin's can be shared with other threads.  Safety is maintained by the Token.
unsafe impl Send for GpioPin {}

impl GpioPin {
    /// Constructor, used by the devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        device: *const raw::device,
        pin: u32,
        dt_flags: u32,
    ) -> Option<GpioPin> {
        if !unique.once() {
            return None;
        }
        Some(GpioPin {
            pin: raw::gpio_dt_spec {
                port: device,
                pin: pin as raw::gpio_pin_t,
                dt_flags: dt_flags as raw::gpio_dt_flags_t,
            },
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
    ///
    /// # Safety
    ///
    /// The `_token` enforces single threaded use of gpios from Rust code.  However, many drivers
    /// within Zephyr use GPIOs, and to use gpios safely, the caller must ensure that there is
    /// either not simultaneous use, or the gpio driver in question is thread safe.
    pub unsafe fn configure(&mut self, _token: &mut GpioToken, extra_flags: raw::gpio_flags_t) {
        // TODO: Error?
        unsafe {
            raw::gpio_pin_configure(
                self.pin.port,
                self.pin.pin,
                self.pin.dt_flags as raw::gpio_flags_t | extra_flags,
            );
        }
    }

    /// Toggle pin level.
    ///
    /// # Safety
    ///
    /// The `_token` enforces single threaded use of gpios from Rust code.  However, many drivers
    /// within Zephyr use GPIOs, and to use gpios safely, the caller must ensure that there is
    /// either not simultaneous use, or the gpio driver in question is thread safe.
    pub unsafe fn toggle_pin(&mut self, _token: &mut GpioToken) {
        // TODO: Error?
        unsafe {
            raw::gpio_pin_toggle_dt(&self.pin);
        }
    }

    /// Set the logical level of the pin.
    pub unsafe fn set(&mut self, _token: &mut GpioToken, value: bool) {
        raw::gpio_pin_set_dt(&self.pin, value as c_int);
    }

    /// Read the logical level of the pin.
    pub unsafe fn get(&mut self, _token: &mut GpioToken) -> bool {
        match raw::gpio_pin_get_dt(&self.pin) {
            0 => false,
            1 => true,
            _ => panic!("TODO: Handle gpio get error"),
        }
    }
}
