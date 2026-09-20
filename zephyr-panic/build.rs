// Copyright (c) 2026 Open Device Partnership and Contributors
// SPDX-License-Identifier: Apache-2.0

// Pre-build code for the zephyr-panic module.

// This makes the values from the generated .config available as conditional compilation, so that
// the panic handler can optionally print using `printk` when it is configured into the build.

fn main() {
    zephyr_build::export_kconfig_bool_options();
}
