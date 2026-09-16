// Copyright (c) 2026 Open Device Partnership and Contributors
// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Set the panicking behavior to log over printk if CONFIG_PRINTK is set,
//! then call the system panic function.
//!
//! This is intended to only be used by Rust applications that run on Zephyr,
//! not for general Rust applications.
//!
//! # Usage
//!
//! ```ignore
//! #![no_std]
//!
//! use zephyr_panic as _;
//!
//! #[no_mangle]
//! extern "C" fn rust_main() {
//!     panic!("Message is logged if CONFIG_PRINTK is set, then system panic function is called.");
//! }
//! ```
#![no_std]
#![allow(unexpected_cfgs)]

use core::panic::PanicInfo;

/// Override rust's panic.  This simplistic initial version just hangs in a loop.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    #[cfg(CONFIG_PRINTK)]
    {
        zephyr::printkln!("panic: {}", info);
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
