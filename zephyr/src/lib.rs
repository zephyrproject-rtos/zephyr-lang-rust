// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr application support for Rust
//!
//! This crates provides the core functionality for applications written in Rust that run on top of
//! Zephyr.  The goal is to bridge the two worlds.  The functionality provided here shouldn't be too
//! distant from how Zephyr does things.  But, it should be "rusty" enough that Rust developers feel
//! comfortable using it.
//!
//! Some functionality:
//!
//! - [`time`]: The time module provides a [`Instant`] and [`Duration`] type that are similar to those
//!   in `std`, but are tailored for embedded systems.  These are bridged through traits, so that most
//!   API calls that offer a timeout will accept either an `Instant` or a `Duration`.
//! - [`sync`]: This crate provides various synchronization primitives that can be used to coordinate
//!   between threads.  These include
//!   - [`sync::atomic`]: Provides the same functionality as [`std::sync::atomic`], but on targets
//!     with limited synchronization primtives, re-exports features from the 'portable-atomic'
//!     crate, which can emulate atomics using critical sections.
//!   - [`sync::channel`]: Channel based synchronization built around the `k_queue` channels
//!     provided by Zephyr.  This provides both `alloc`-based unbounded channels, and bounded
//!     channels that pre-allocate.
//!   - [`sync::Mutex`]/[`sync::Condvar`]: `std`-style Mutexes and condition variables, where the
//!     Mutex protects some piece of data using Rust's features.
//!   - [`sync::SpinMutex`]: A Mutex that protects a piece of data, but does so using a spinlock.
//!     This is useful where data needs to be used exclusively, but without other types of
//!     synchronization.
//!   - [`sync::Arc`]: Atomic reference counted pointers.  Mostly like [`std::sync::Arc`] but supports
//!     all targets that support Rust on Zephyr.
//! - [`sys`]: More direct interfaces to Zephyr's primitives.  Most of the operations in `sync` are
//!   built on these.  These interfaces are 'safe', as in they can be used without the `unsafe`
//!   keyword, but the interfaces in `sync` are much more useful from Rust programs.  Although most
//!   things here won\t typically be needed, two standout:
//!   - [`sys::thread`]: A fairly direct, but safe, interface to create Zephyr threads.  At this
//!     point, this is the primary way to create threads in Rust code (see also [`work`] which
//!     supports multiple contexts using Zephyr's work queues.
//!   - [`sys::sync::Semaphore`]: The primitive Semaphore type from Zephyr.  This is the one lower
//!     level operation that is still quite useful in regular code.
//! - [`timer`]: Rust interfaces to Zephyr timers.  These timers can be used either by registering a
//!   callback, or polled or waited for for an elapsed time.
//! - [`work`]: Zephyr work queues for Rust.  The [`work::WorkQueueBuilder`] and resulting
//!   [`work::WorkQueue`] allow creation of Zephyr work queues to be used from Rust.  The
//!   [`work::Work`] item had an action that will be invoked by the work queue, and can be manually
//!   submitted when needed.
//! - [`kio`]: An implementation of an async executor built around triggerable work queues in
//!   Zephyr.  Although there is a bit more overhead to this executor, it is compatible with many of
//!   the Zephyr synchronization types, and many of these [`sys::sync::Semaphore`], and
//!   [`sync::channel`] will provide `_async` variants of most of the blocking operations.  These
//!   will return a `Future`, and can be used from async code started by the [`spawn`] function.
//!   In addition, because Zephyr's work queues do not work well with Zephyr's Mutex type, this is
//!   also a [`kio::sync::Mutex`] type that works with async.
//! - [`logging`]: A logging backend for Rust on Zephyr.  This will log to either `printk` or
//!   through Zephyr's logging framework.
//!
//! [`Instant`]: time::Instant
//! [`Duration`]: time::Duration
//! [`std::sync::atomic`]: https://doc.rust-lang.org/std/sync/atomic/
//! [`std::sync::Arc`]: https://doc.rust-lang.org/std/sync/struct.Arc.html
//! [`spawn`]: kio::spawn
//!
//! In addition to the above, the [`kconfig`] and [`devicetree`] provide a reflection of the kconfig
//! settings and device tree that were used for a specific build.  As such, the documentation
//! provided online is not likely to be that useful, and for these, it is best to generate the
//! documentation for a specific build:
//! ```bash
//! $ west rustdoc
//! ```
//!
//! Note, however, that the `kconfig` module only provides Kconfig **values**, and doesn't provide a
//! mechanmism to base conditional compilation.  For that, please see the
//! [zephyr-build](../../std/zephyr_build/index.html) crate, which provides routines that can be
//! called from a `build.rs` file to make these settings available.

#![no_std]
#![allow(unexpected_cfgs)]
#![deny(missing_docs)]

pub mod align;
pub mod device;
pub mod error;
#[cfg(CONFIG_RUST_ALLOC)]
pub mod kio;
pub mod logging;
pub mod object;
#[cfg(CONFIG_RUST_ALLOC)]
pub mod simpletls;
pub mod sync;
pub mod sys;
pub mod time;
#[cfg(CONFIG_RUST_ALLOC)]
pub mod timer;
#[cfg(CONFIG_RUST_ALLOC)]
pub mod work;

pub use error::{Error, Result};

pub use logging::set_logger;

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
fn panic(info: &PanicInfo) -> ! {
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

#[cfg(CONFIG_RUST_ALLOC)]
pub mod task {
    //! Provides the portable-atomic version of `alloc::task::Wake`, which uses the compatible
    //! versionm of Arc.

    pub use portable_atomic_util::task::Wake;
}
