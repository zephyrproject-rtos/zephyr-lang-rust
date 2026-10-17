// Copyright (c) 2024 Linaro LTD
// Copyright (c) 2026 Google LLC
// SPDX-License-Identifier: Apache-2.0

//! # Zephyr errors
//!
//! This module contains an `Error` and `Result` type for use in wrapped Zephyr calls.  Many
//! operations in Zephyr return an int result where negative values correspond with errnos.
//! Convert those to a `Result` type where the `Error` condition maps to errnos.
//!
//! The portable set of errno values are available as associated constants such as [`Error::EINVAL`], with the
//! numeric values taken from `zephyr-sys` so they always match the libc the application is built
//! with.  This allows matching on errors by name instead of comparing raw numbers:
//!

use core::ffi::c_int;
use core::fmt;

// This is a little messy because the constants end up as u32 from bindgen, although the values are
// negative.

/// A Zephyr error.
///
/// Represents an error result returned within Zephyr.  The value is the positive errno (e.g. `22`
/// for `EINVAL`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Error(pub u32);

impl Error {
    /// Creates an `Error` from a raw errno value.
    ///
    /// Both the positive errno (`EINVAL`) and the negated form Zephyr APIs return (`-EINVAL`) are
    /// accepted.  Returns `None` for `0`, which is not an error.
    pub const fn from_raw(raw: i32) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw.unsigned_abs()))
        }
    }

    /// Returns the positive errno value (e.g. `22` for [`Error::EINVAL`]).
    pub const fn raw(self) -> i32 {
        self.0 as i32
    }

    /// Returns the negated errno value (e.g. `-22` for [`Error::EINVAL`]).
    // Useful when propagating an error back to C code.
    pub const fn as_neg(self) -> c_int {
        -(self.0 as c_int)
    }

    /// Returns the symbolic name of this error (e.g. `"EINVAL"`), or `None` if the value is not one
    /// of the known constants.
    ///
    /// For aliased values the canonical name is returned (e.g. [`Error::EWOULDBLOCK`] reports
    /// `"EAGAIN"`).
    pub fn name(self) -> Option<&'static str> {
        NAMES
            .iter()
            .find(|(value, _)| *value == self.0)
            .map(|(_, name)| *name)
    }
}

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

/// Extension methods on [`Result`] for the common "this particular errno is not a failure"
/// pattern.
pub trait ResultExt<T> {
    /// Turns `Err(errno)` into `Ok(T::default())`, leaving any other result untouched.
    ///
    /// ```ignore
    /// // Already enabled is not a failure.
    /// to_result_void(ret).ignore(Error::EALREADY)?;
    /// ```
    fn ignore(self, errno: Error) -> Result<T>
    where
        T: Default;

    /// Turns `Err(errno)` into `Ok(f())`, leaving any other result untouched.
    fn ignore_with(self, errno: Error, f: impl FnOnce() -> T) -> Result<T>;
}

impl<T> ResultExt<T> for Result<T> {
    fn ignore(self, errno: Error) -> Result<T>
    where
        T: Default,
    {
        self.ignore_with(errno, T::default)
    }

    fn ignore_with(self, errno: Error, f: impl FnOnce() -> T) -> Result<T> {
        match self {
            Err(e) if e == errno => Ok(f()),
            other => other,
        }
    }
}

/// One macro invocation is the single source of truth for both the associated constants and the
/// name table.  Values are sourced from `zephyr-sys`, so they track whichever libc Zephyr is
/// configured with.
///
/// Aliases (`EWOULDBLOCK`, ...) must be listed after their canonical name so that [`Error::name`]
/// returns the canonical one.
macro_rules! errnos {
    ($($name:ident),+ $(,)?) => {
        impl Error {
            $(
                #[doc = concat!("`", stringify!($name), "`")]
                pub const $name: Error = Error(crate::raw::$name);
            )+
        }

        static NAMES: &[(u32, &str)] = &[
            $((crate::raw::$name, stringify!($name)),)+
            #[cfg(not(target_os = "linux"))]
            (crate::raw::ENOTSUP, "ENOTSUP"),
            #[cfg(target_os = "linux")]
            (crate::raw::EOPNOTSUPP, "ENOTSUP"),
        ];
    };
}

impl Error {
    /// `ENOTSUP`
    ///
    /// On a Linux host (`native_sim`), glibc defines `ENOTSUP` as an alias of `EOPNOTSUPP`, so
    /// bindgen does not emit a separate constant for it.  On embedded targets it is a distinct
    /// errno number.
    #[cfg(not(target_os = "linux"))]
    pub const ENOTSUP: Error = Error(crate::raw::ENOTSUP);
    /// `ENOTSUP`
    ///
    /// On a Linux host (`native_sim`), glibc defines `ENOTSUP` as an alias of `EOPNOTSUPP`, so
    /// bindgen does not emit a separate constant for it.  On embedded targets it is a distinct
    /// errno number.
    #[cfg(target_os = "linux")]
    pub const ENOTSUP: Error = Error::EOPNOTSUPP;
}

// The portable subset defined by Zephyr's minimal libc (lib/libc/minimal/include/errno.h).  Zephyr
// core and drivers only return these, and every libc Zephyr supports defines them.
#[rustfmt::skip]
errnos! {
    EPERM, ENOENT, ESRCH, EINTR, EIO, ENXIO, E2BIG, ENOEXEC, EBADF, ECHILD,
    EAGAIN, ENOMEM, EACCES, EFAULT, ENOTBLK, EBUSY, EEXIST, EXDEV, ENODEV,
    ENOTDIR, EISDIR, EINVAL, ENFILE, EMFILE, ENOTTY, ETXTBSY, EFBIG, ENOSPC,
    ESPIPE, EROFS, EMLINK, EPIPE, EDOM, ERANGE, ENOMSG, EDEADLK, ENOLCK, ENOSTR,
    ENODATA, ETIME, ENOSR, EPROTO, EBADMSG, ENOSYS, ENOTEMPTY, ENAMETOOLONG,
    ELOOP, EOPNOTSUPP, EPFNOSUPPORT, ECONNRESET, ENOBUFS, EAFNOSUPPORT,
    EPROTOTYPE, ENOTSOCK, ENOPROTOOPT, ESHUTDOWN, ECONNREFUSED, EADDRINUSE,
    ECONNABORTED, ENETUNREACH, ENETDOWN, ETIMEDOUT, EHOSTDOWN, EHOSTUNREACH,
    EINPROGRESS, EALREADY, EDESTADDRREQ, EMSGSIZE, EPROTONOSUPPORT,
    ESOCKTNOSUPPORT, EADDRNOTAVAIL, ENETRESET, EISCONN, ENOTCONN, ETOOMANYREFS,
    EILSEQ, EOVERFLOW, ECANCELED,
    // Aliases: keep after their canonical names.
    EWOULDBLOCK,
}
