// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::printkln;

// Bring in the Zephyr panic handler.
use zephyr_panic as _;

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("Hello world from Rust on {}", zephyr::kconfig::CONFIG_BOARD);
}
