//! Device wrappers for Zephyr I2C controllers and targets.

mod controller;
mod target;

pub use controller::{I2c, Operation};
pub use target::{I2cTarget, I2cTargetCallbacks, I2cTargetData};

// Re-export the raw callback types so users can write `extern "C"` handlers without reaching into
// `zephyr::raw` directly.
pub use crate::raw::{i2c_target_callbacks, i2c_target_config};
