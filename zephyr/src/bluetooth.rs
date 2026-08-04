// Copyright (c) 2026
// SPDX-License-Identifier: Apache-2.0

//! Bluetooth support that is independent of Zephyr's native host.

#[cfg(feature = "bt-hci")]
pub mod hci_raw;
