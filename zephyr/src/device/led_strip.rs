//! Device wrappers for Zephyr LED strip controllers.

use super::{NoStatic, Unique};
use crate::{error::to_result_void, raw, Result};

/// Type alias for the Zephyr `led_rgb` struct used to describe a single pixel's RGB value.
///
/// Note that `led_strip_update_rgb` is documented to potentially overwrite the pixel buffer,
/// hence methods that update the strip take `&mut [LedRgb]`.
pub type LedRgb = raw::led_rgb;

/// A Zephyr LED strip device (e.g. a WS2812 addressable RGB strip).
///
/// This wrapper maps to Zephyr's `led_strip_*` API.
#[allow(dead_code)]
pub struct LedStrip {
    pub(crate) device: *const raw::device,
}

// SAFETY: `LedStrip` holds a raw pointer to a static Zephyr device structure with no
// thread-affine Rust state.
unsafe impl Send for LedStrip {}

impl LedStrip {
    /// Constructor, intended to be called by devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        _static: &NoStatic,
        device: *const raw::device,
    ) -> Option<LedStrip> {
        if !unique.once() {
            return None;
        }

        Some(LedStrip { device })
    }

    /// Verify that the underlying LED strip device is ready for use.
    pub fn is_ready(&self) -> bool {
        unsafe { raw::device_is_ready(self.device) }
    }

    /// Push an array of RGB pixel values to the strip.
    ///
    /// The driver may overwrite the contents of `pixels`.
    pub fn update_rgb(&mut self, pixels: &mut [LedRgb]) -> Result<()> {
        to_result_void(unsafe {
            raw::led_strip_update_rgb(self.device, pixels.as_mut_ptr(), pixels.len())
        })
    }

    /// Return the number of pixels (LEDs) in the strip.
    pub fn length(&self) -> usize {
        unsafe { raw::led_strip_length(self.device) }
    }
}
