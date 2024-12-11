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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
