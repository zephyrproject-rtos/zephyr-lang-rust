//! A Rust global allocator that uses the stdlib allocator in Zephyr
//!
//! The zephyr runtime is divided into three crates:
//! - [core](https://doc.rust-lang.org/stable/core/) is the "dependency-free" foundation of the
//!   standard library.  It is all of the parts of the standard library that have no dependencies
//!   outside of the language and the architecture itself.
//! - [alloc](https://doc.rust-lang.org/stable/alloc/) provides the parts of the standard library
//!   that depend on memory allocation.  This includes various types of smart pointers, atomically
//!   referenced counted pointers, and various collections.  This depends on the platform providing
//!   an allocator.
//! - [std](https://doc.rust-lang.org/stable/std/) is the rest of the standard library. It include
//!   both core and alloc, and then everthing else, including filesystem access, networking, etc.  It
//!   is notable, however, that the Rust standard library is fairly minimal.  A lot of functionality
//!   that other languages might include will be relegated to other crates, and the ecosystem and
//!   tooling around cargo make it as easy to use these as the standard library.
//!
//! For running application code on Zephyr, the core library (mostly) just works (the a caveat of a
//! little work needed to use atomics on platforms Zephyr supports but don't have atomic
//! instructions).  The std library is somewhat explicitly _not_ supported.  Although the intent is
//! to provide much of the functionality from std, Zephyr is different enough from the conventional
//! operating system std was built around that just porting it doesn't really give practical
//! results.  The result is either to complicated to make work, or too different from what is
//! typically done on Zephyr.  Supporting std could be a future project.
//!
//! This leaves alloc, which is mostly independent but is required to know about an allocator to
//! use.  This module provides an allocator for Rust that uses the underlying memory allocator
//! configured into Zephyr.
//!
//! Because a given embedded application may or may not want memory allocation, this is controlled
//! by the `CONFIG_RUST_ALLOC` Kconfig.  When this config is enabled, the alloc crate becomes
//! available to applications.
//!
//! Since alloc is typically used on Rust as a part of the std library, building in a no-std
//! environment requires that it be access explicitly.  Generally, alloc must be explicitly added
//! to every module that needs it.
//!
//! ```
//! extern crate alloc;
//!
//! use alloc::boxed::Box;
//!
//! let item = Box::new(5);
//! printkln!("box value {}", item);
//! ```
//!
//! The box holding the value 5 will be allocated by `Box::new`, and freed when the `item` goes out
//! of scope.

// This entire module is only used if CONFIG_RUST_ALLOC is enabled.
extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};

use alloc::alloc::handle_alloc_error;

/// Define size_t, as it isn't defined within the FFI.
#[allow(non_camel_case_types)]
type c_size_t = usize;

extern "C" {
    fn malloc(size: c_size_t) -> *mut u8;
    fn free(ptr: *mut u8);
}

/// An allocator that uses Zephyr's allocation primitives.
///
/// This is exported for documentation purposes, this module does contain an instance of the
/// allocator as well.
pub struct ZephyrAllocator;

unsafe impl GlobalAlloc for ZephyrAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        // The C allocation library assumes an alignment of 8.  For now, just panic if this cannot
        // be satistifed.
        if align > 8 {
            handle_alloc_error(layout);
        }

        malloc(size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        free(ptr)
    }
}

/// The global allocator built around the Zephyr malloc/free.
#[global_allocator]
pub static ZEPHYR_ALLOCATOR: ZephyrAllocator = ZephyrAllocator;
