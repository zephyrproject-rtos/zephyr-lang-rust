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

// SAFETY: The device pointer refers to a statically allocated Zephyr device, and the `led_strip_*`
// calls are not tied to the thread that obtained it, so moving a `LedStrip` to another thread is
// sound. The `Unique` passed to `new` ensures only one `LedStrip` exists per device, and `LedStrip`
// is not `Sync`, so the strip is only driven from one thread at a time.
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
    /// `pixels[0]` is the first LED in the strip. If `pixels` is shorter than the strip, only the
    /// first `pixels.len()` LEDs are updated; what the remaining LEDs show depends on the driver
    /// (WS2812 LEDs keep their previous value). If `pixels` is longer than the strip, this returns
    /// an `ERANGE` error.
    ///
    /// The driver may overwrite the contents of `pixels`.
    pub fn update_rgb(&mut self, pixels: &mut [LedRgb]) -> Result<()> {
        to_result_void(unsafe {
            raw::led_strip_update_rgb(self.device, pixels.as_mut_ptr(), pixels.len())
        })
    }

    /// Push an array of individual channel values to the strip.
    ///
    /// Each byte controls one individually addressable channel or LED, in strip order. Drivers that
    /// do not support this return an `ENOSYS` error.
    ///
    /// The driver may overwrite the contents of `channels`.
    pub fn update_channels(&mut self, channels: &mut [u8]) -> Result<()> {
        to_result_void(unsafe {
            raw::led_strip_update_channels(self.device, channels.as_mut_ptr(), channels.len())
        })
    }

    /// Return the number of pixels (LEDs) in the strip.
    pub fn length(&self) -> usize {
        unsafe { raw::led_strip_length(self.device) }
    }
}
