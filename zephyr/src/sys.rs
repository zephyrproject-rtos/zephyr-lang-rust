// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr 'sys' module.
//!
//! The `zephyr-sys` crate contains the direct C bindings to the Zephyr API.  All of these are
//! unsafe.
//!
//! This module `zephyr::sys` contains thin wrappers to these C bindings, that can be used without
//! unsafe, but as unchanged as possible.

use zephyr_sys::k_timeout_t;

pub mod sync;

// These two constants are not able to be captured by bindgen.  It is unlikely that these values
// would change in the Zephyr headers, but there will be an explicit test to make sure they are
// correct.

/// Represents a timeout with an infinite delay.
///
/// Low-level Zephyr constant.  Calls using this value will wait as long as necessary to perform
/// the requested operation.
pub const K_FOREVER: k_timeout_t = k_timeout_t { ticks: -1 };

/// Represents a null timeout delay.
///
/// Low-level Zephyr Constant.  Calls using this value will not wait if the operation cannot be
/// performed immediately.
pub const K_NO_WAIT: k_timeout_t = k_timeout_t { ticks: 0 };
