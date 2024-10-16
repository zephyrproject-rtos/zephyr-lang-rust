// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! # Zephyr low-level synchronization primities.
//!
//! The `zephyr-sys` crate contains direct calls into the Zephyr C API.  This interface, however,
//! cannot be used from safe Rust.  This crate attempts to be as direct an interface to some of
//! these synchronization mechanisms, but without the need for unsafe.  The other module
//! `crate::sync` provides higher level interfaces that help manage synchronization in coordination
//! with Rust's borrowing and sharing rules, and will generally provide much more usable
//! interfaces.
//!
//! # Kernel objects
//!
//! Zephyr's primitives work with the concept of a kernel object.  These are the data structures
//! that are used by the Zephyr kernel to coordinate the operation of the primitives.  In addition,
//! they are where the protection barrier provided by `CONFIG_USERSPACE` is implemented.  In order
//! to use these primitives from a userspace thread two things must happen:
//!
//! - The kernel objects must be specially declared.  All kernel objects in Zephyr will be built,
//!   at compile time, into a perfect hash table that is used to validate them.  The special
//!   declaration will take care of this.
//! - The objects must be granted permission to be used by the userspace thread.  This can be
//!   managed either by specifically granting permission, or by using inheritance when creating the
//!   thread.
//!
//! At this time, only the first mechanism is implemented, and all kernel objects should be
//! declared using the `crate::kobj_define!` macro.  These then must be initialized, and then the
//! special method `.get()` called, to retrieve the Rust-style value that is used to manage them.
//! Later, there will be a pool mechanism to allow these kernel objects to be allocated and freed
//! from a pool, although the objects will still be statically allocated.

pub mod mutex;
pub mod semaphore;

pub use mutex::{
    Condvar,
    StaticCondvar,
    Mutex,
    StaticMutex,
};
pub use semaphore::{
    Semaphore,
    StaticSemaphore,
};
