// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! # Zephyr errors
//!
//! This module contains an `Error` and `Result` type for use in wrapped Zephyr calls.  Many
//! operations in Zephyr return an int result where negative values correspond with errnos.
//! Convert those to a `Result` type where the `Error` condition maps to errnos.
//!
//! Initially, this will just simply wrap the numeric error code, but it might make sense to make
//! this an enum itself, however, it would probably be better to auto-generate this enum instead of
//! trying to maintain the list manually.

use core::ffi::c_int;
use core::fmt;

// This is a little messy because the constants end up as u32 from bindgen, although the values are
// negative.

/// A Zephyr error.
///
/// Represents an error result returned within Zephyr.
pub struct Error(pub u32);

impl core::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "zephyr error errno:{}", self.0)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "zephyr error errno:{}", self.0)
    }
}

/// Wraps a value with a possible Zephyr error.
pub type Result<T> = core::result::Result<T, Error>;

/// Map a return result from Zephyr into an Result.
///
/// Negative return results being considered errors.
#[inline(always)]
pub fn to_result(code: c_int) -> Result<c_int> {
    if code < 0 {
        Err(Error(-code as u32))
    } else {
        Ok(code)
    }
}

/// Map a return result, with a void result.
#[inline(always)]
pub fn to_result_void(code: c_int) -> Result<()> {
    to_result(code).map(|_| ())
}
