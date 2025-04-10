//! Support for Embassy on Rust+Zephyr
//!
//! [Embassy](https://embassy.dev/) is: "The next-generation framework for embedded applications".
//! From a typical RTOS perspective it is perhaps a little difficult to explain what exactly it is,
//! and why it makes sense to discuss it in the context of supporting Rust on Zephyr.
//!
//! At a core level, Embassy is a set of crates that implement various functionality that is used
//! when writing bare metal applications in Rust.  Combined, these provide most of the functionality
//! that is needed for an embedded application.  However, the crates are largely independent, and as
//! such find use when combined with Zephyr.
//!
//! ## Executor
//!
//! A significant aspect of Embassy's functionality revolves around providing one or more executors
//! for coordinating async code in Rust.  The Rust language transforms code annotated with
//! async/await into state machines that allow these operations to be run cooperatively.  A bare
//! metal system with one or more of these executors managing async tasks can indeed solve many of
//! the types of scheduling solutions needed for embedded systems.
//!
//! Although Zephyr does have a thread scheduler, there are still some advantages to running an
//! executor on one or more Zephyr threads:
//!
//! - Because the async code is transformed into a state machine, this code only uses stack while
//!   evaluating to the next stopping point.  This allows a large number of async operations to
//!   happen on a single thread, without requiring additional stack.  The state machines themselves
//!   do take memory, but this usage is known at compile time (with the stable Rust compiler, it is
//!   allocated from a pool, and with the nightly compiler, can be completely compile time
//!   determined).
//! - Context switches between async threads can be very fast.  When running a single executor
//!   thread, there is no need for locking for data that is entirely kept within that thread, and
//!   these context switches have similar cost to a function call.  Even with multiple threads
//!   involved, many switches will happen on the same underlying Zephyr thread, reducing the need to
//!   reschedule.
//! - Embassy provides a lot of mechanisms for coordinating between these tasks, all that work in
//!   the context of async/await.  Some may be thought of as redundant with Zephyr primitives, but
//!   they serve a different purpose, and provide more streamlined coordination for things entirely
//!   within the Rust world.
//!
//! ## Use
//!
//! To best use this module, it is best to look at the various examples under `samples/embassy*` in
//! this repo.  Some of the embassy crates, especially embassy-executor have numerous features that
//! must be configured correctly for proper operation.  To use the 'executor-thread' feature, it is
//! also necessary to configure embassy for the proper platform.  Future versions of the Cmake files
//! for Rust on Zephyr may provide assistance with this, but for now, this does limit a given
//! application to running on a specific architecture.  For using the `executor-zephyr` feature
//! provided by this module, it easier to allow the code to run on multiple platforms.
//!
//! The following features in the `zephyr` crate configure what is supported:
//!
//! - **`executor-zephyr`**: This implements an executor that uses a Zephyr semaphore to suspend the
//!   executor thread when there is no work to perform. This feature is incompatible with either
//!   `embassy-thread` or `embassy-interrupt` in the `embassy-executor` crate.
//! - **`embassy-time-driver`**: This feature causes the `zephyr` crate to provide a time driver to
//!   Embassy.  This driver uses a single `k_timer` in Zephyr to wake async operations that are
//!   dependent on time.  This enables the `embassy-time` crate's functionality to be used freely
//!   within async tasks on Zephyr.
//!
//! Future versions of this support will provide async interfaces to various driver systems in
//! Zephyr, allowing the use of Zephyr drivers freely from async code.
//!
//! It is perfectly permissible to use the `executor-thread` feature from embassy-executor on
//! Zephyr, within the following guidelines:
//!
//! - The executor is incompatible with the async executor provided within [`crate::kio`], and
//!   because there are no features to enable this, this functions will still be accessible.  Be
//!   careful.  You should enable `no-kio` in the zephyr crate to hide these functions.
//! - This executor does not coordinate with the scheduler on Zephyr, but uses an
//!   architecture-specific mechanism when there is no work. On Cortex-M, this is the 'wfe'
//!   instruction, on riscv32, the 'wfi' instruction.  This means that no tasks of lower priority
//!   will ever run, so this should only be started from the lowest priority task on the system.
//! - Because the 'idle' thread in Zephyr will never run, some platforms will not enter low power
//!   mode, when the system is idle.  This is very platform specific.
//!
//! ## Caveats
//!
//! The executor provided by Embassy is fundamentally incompatible with the executor provided by
//! this crate's [`crate::kio`] and [`crate::work::futures`].  Trying to use the functionality
//! provided by operations, such as [`Semaphore::take_async`], will generally result in a panic.
//! These routines are conditionally compiled out when `executor-zephyr` is enabled, but there is no
//! way for this crate to detect the use of embassy's `executor-threaded`.  Combining these will
//! result in undefined behavior, likely difficult to debug crashes.
//!
//! [`Semaphore::take_async`]: crate::sys::sync::Semaphore::take_async

#[cfg(feature = "time-driver")]
mod time_driver;

#[cfg(feature = "executor-zephyr")]
pub use executor::Executor;
#[cfg(feature = "executor-zephyr")]
mod executor;
