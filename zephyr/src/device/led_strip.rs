//! Simple led strip driver

use crate::raw;
use crate::error::{Result, to_result_void};

use super::Unique;

/// A simple led strip wrapper.
pub struct LedStrip {
    /// The underlying device itself.
    #[allow(dead_code)]
    pub(crate) device: *const raw::device,
}

// This is send, safe with Zephyr.
unsafe impl Send for LedStrip { }

impl LedStrip {
    /// Constructor, used by the devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device) -> Option<LedStrip> {
        if !unique.once() {
            return None;
        }

        Some(LedStrip { device })
    }

    /// Return the number of LEDS in the chain.
    pub fn chain_len(&self) -> usize {
        unsafe { raw::led_strip_length(self.device) as usize}
    }

    /// Update the state of the LEDs.
    ///
    /// It is unclear from the API docs whether this is supposed to be an array of rgb_led
    /// (which is what the samples assume), or packed rgb data.
    pub unsafe fn update(&mut self, channels: &[raw::led_rgb]) -> Result<()> {
        to_result_void(unsafe {
            raw::led_strip_update_rgb(self.device,
                channels.as_ptr() as *mut _,
                channels.len())
        })
    }
}
