// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr application support for Rust
//!
//! This crates provides the core functionality for applications written in Rust that run on top of
//! Zephyr.

#![no_std]
#![allow(unexpected_cfgs)]
#![deny(missing_docs)]

pub mod align;
pub mod error;
pub mod object;
pub mod sync;
pub mod sys;
pub mod time;

pub use error::{Error, Result};

/// Re-exported for local macro use.
pub use paste::paste;

// Bring in the generated kconfig module
pub mod kconfig {
    //! Zephyr Kconfig values.
    //!
    //! This module contains an auto-generated set of constants corresponding to the values of
    //! various Kconfig values during the build.
    //!
    //! **Note**: Unless you are viewing docs generated for a specific build, the values below are
    //! unlikely to directly correspond to those in a given build.

    // Don't enforce doc comments on the bindgen, as it isn't enforced within Zephyr.
    #![allow(missing_docs)]

    include!(concat!(env!("OUT_DIR"), "/kconfig.rs"));
}

pub mod devicetree {
    //! Zephyr device tree
    //!
    //! This is an auto-generated module that represents the device tree for a given build.  The
    //! hierarchy here should match the device tree, with an additional top-level module "labels"
    //! that contains submodules for all of the labels.
    //!
    //! **Note**: Unless you are viewing docs generated for a specific build, the values below are
    //! unlikely to directly correspond to those in a given build.

    // Don't enforce doc comments on the generated device tree.
    #![allow(missing_docs)]

    include!(concat!(env!("OUT_DIR"), "/devicetree.rs"));
}

// Ensure that Rust is enabled.
#[cfg(not(CONFIG_RUST))]
compile_error!("CONFIG_RUST must be set to build Rust in Zephyr");

// Printk is provided if it is configured into the build.
#[cfg(CONFIG_PRINTK)]
pub mod printk;

use core::panic::PanicInfo;

/// Override rust's panic.  This simplistic initial version just hangs in a loop.
#[panic_handler]
fn panic(info :&PanicInfo) -> ! {
    #[cfg(CONFIG_PRINTK)]
    {
        printkln!("panic: {}", info);
    }
    let _ = info;

    // Call into the wrapper for the system panic function.
    unsafe {
        extern "C" {
            fn rust_panic_wrap() -> !;
        }
        rust_panic_wrap();
    }
}

#[cfg(CONFIG_PRINTK)]
pub fn set_logger() {
    printk::set_printk_logger();
}

/// Re-export of zephyr-sys as `zephyr::raw`.
pub mod raw {
    pub use zephyr_sys::*;
}

/// Provide symbols used by macros in a crate-local namespace.
#[doc(hidden)]
pub mod _export {
    pub use core::format_args;

    use crate::{object::StaticKernelObject, sys::thread::StaticThreadStack};

    /// Type alias for the thread stack kernel object.
    pub type KStaticThreadStack = StaticKernelObject<StaticThreadStack>;
}

// Mark this as `pub` so the docs can be read.
// If allocation has been requested, provide the allocator.
#[cfg(CONFIG_RUST_ALLOC)]
pub mod alloc_impl;

// If we have allocation, we can also support logging.
#[cfg(CONFIG_RUST_ALLOC)]
pub mod log {
    #[cfg(CONFIG_LOG)]
    compile_error!("Rust with CONFIG_LOG is not yet supported");

    mod log_printk;

    pub use log_printk::*;
}
