// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

// This crate needs access to kconfig variables.  This is an example of how to do that.  The
// zephyr-build must be a build dependency.

fn main() {
    zephyr_build::export_bool_kconfig();
}
