//! LED driver, with support for PWMLED driver, with support for PWM.

use crate::raw;
use crate::error::{Result, to_result_void};

use super::Unique;

/// A simple led strip wrapper.
pub struct Leds {
    /// The underlying device itself.
    #[allow(dead_code)]
    pub(crate) device: *const raw::device,
    /// How many are configured in the DT.
    pub(crate) count: usize,
}

// This is send, safe with Zephyr.
unsafe impl Send for Leds { }

impl Leds {
    /// Constructor, used by the devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device, count: usize) -> Option<Leds> {
        if !unique.once() {
            return None;
        }

        Some(Leds { device, count })
    }

    /// Return the number of LEDS.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Set the brightness of one of the described LEDs
    pub unsafe fn set_brightness(&mut self, index: usize, value: u8) -> Result<()> {
        to_result_void(unsafe {
            raw::led_set_brightness(self.device,
                                    index as u32,
                                    value)
        })
    }
}
