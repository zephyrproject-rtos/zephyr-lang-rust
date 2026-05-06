// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use log::info;

#[no_mangle]
extern "C" fn rust_main() {
    // SAFETY: `rust_main` runs once during application startup before any rust tasks
    // are spawned, so the global logger is initialized before concurrent use.
    unsafe {
        zephyr::set_logger().unwrap();
    }

    info!("Hello world from Rust on {}", zephyr::kconfig::CONFIG_BOARD);
}
