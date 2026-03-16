//! Device wrappers for Zephyr LED controllers.

use super::{NoStatic, Unique};
use crate::{error::to_result_void, raw, Result};

/// A Zephyr LED controller device.
#[allow(dead_code)]
pub struct LedController {
    pub(crate) device: *const raw::device,
}

impl LedController {
    /// Constructor, intended to be called by devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        _static: &NoStatic,
        device: *const raw::device,
    ) -> Option<LedController> {
        if !unique.once() {
            return None;
        }

        Some(LedController { device })
    }

    /// Verify that the underlying LED controller is ready for use.
    pub fn is_ready(&self) -> bool {
        unsafe { raw::device_is_ready(self.device) }
    }
}

/// A single logical LED exposed by a Zephyr LED controller.
///
/// This wrapper maps to Zephyr's `led_*` API. Devicetree child nodes of a `pwm-leds` controller
/// become instances of this type.
#[allow(dead_code)]
pub struct Led {
    pub(crate) device: *const raw::device,
    pub(crate) index: u32,
}

impl core::fmt::Debug for Led {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Led")
            .field("device", &self.device)
            .field("index", &self.index)
            .finish()
    }
}

// SAFETY: `Led` holds a raw device pointer and an LED index with no thread-affine Rust state.
unsafe impl Send for Led {}

impl Led {
    /// Constructor, intended to be called by devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        _static: &NoStatic,
        device: *const raw::device,
        index: u32,
    ) -> Option<Led> {
        if !unique.once() {
            return None;
        }

        Some(Led { device, index })
    }

    /// Verify that the underlying LED controller is ready for use.
    pub fn is_ready(&self) -> bool {
        unsafe { raw::device_is_ready(self.device) }
    }

    /// Set the LED brightness as a percentage in the range `0..=100`.
    pub fn set_brightness(&mut self, value: u8) -> Result<()> {
        to_result_void(unsafe { raw::led_set_brightness(self.device, self.index, value) })
    }

    /// Turn the LED fully on.
    pub fn on(&mut self) -> Result<()> {
        to_result_void(unsafe { raw::led_on(self.device, self.index) })
    }

    /// Turn the LED fully off.
    pub fn off(&mut self) -> Result<()> {
        to_result_void(unsafe { raw::led_off(self.device, self.index) })
    }

    /// Blink the LED using millisecond delays.
    pub fn blink(&mut self, delay_on: u32, delay_off: u32) -> Result<()> {
        to_result_void(unsafe { raw::led_blink(self.device, self.index, delay_on, delay_off) })
    }
}
