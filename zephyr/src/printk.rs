// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Printk implementation for Rust.
//!
//! This uses the `k_str_out` syscall, which is part of printk to output to the console.

use core::fmt::{
    Arguments,
    Result,
    Write,
    write,
};

use log::{LevelFilter, Log, Metadata, Record};

/// Print to Zephyr's console, without a newline.
///
/// This macro uses the same syntax as std's
/// [`format!`](https://doc.rust-lang.org/stable/std/fmt/index.html), but writes to the Zephyr
/// console instead.
///
/// if `CONFIG_PRINTK_SYNC` is enabled, this locks during printing.  However, to avoid allocation,
/// and due to private accessors in the Zephyr printk implementation, the lock is only over groups
/// of a small buffer size.  This buffer must be kept fairly small, as it resides on the stack.
#[macro_export]
macro_rules! printk {
    ($($arg:tt)*) => {{
        $crate::printk::printk(format_args!($($arg)*));
    }};
}

/// Print to Zephyr's console, with a newline.
///
/// This macro uses the same syntax as std's
/// [`format!`](https://doc.rust-lang.org/stable/std/fmt/index.html), but writes to the Zephyr
/// console instead.
///
/// If `CONFIG_PRINTK_SYNC` is enabled, this locks during printing.  However, to avoid allocation,
/// and due to private accessors in the Zephyr printk implementation, the lock is only over groups
/// of a small buffer size.  This buffer must be kept fairly small, as it resides on the stack.
///
/// [`format!`]: alloc::format
#[macro_export]
macro_rules! printkln {
    ($($arg:tt)*) => {{
        $crate::printk::printkln(format_args!($($arg)*));
    }};
}

/// A simple log handler built around printk.
struct PrintkLogger;

impl Log for PrintkLogger {
    // Initially, everything is just available.
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    // Just print out the information.
    fn log(&self, record: &Record<'_>) {
        printkln!("{}:{}: {}",
            record.level(),
            record.target(),
            record.args());
    }

    // Nothing to do for flush.
    fn flush(&self) {
    }
}

static PRINTK_LOGGER: PrintkLogger = PrintkLogger;

// The cfg matches what is in the log crate, which doesn't use portable atomic, and assumes the
// racy init when not the case.
#[doc(hidden)]
#[cfg(target_has_atomic = "ptr")]
pub fn set_printk_logger() {
    log::set_logger(&PRINTK_LOGGER).unwrap();
    log::set_max_level(LevelFilter::Info);
}

#[doc(hidden)]
#[cfg(not(target_has_atomic = "ptr"))]
pub fn set_printk_logger() {
    unsafe {
        log::set_logger_racy(&PRINTK_LOGGER).unwrap();
        log::set_max_level_racy(LevelFilter::Info);
    }
}

// This could readily be optimized for the configuration where we don't have userspace, as well as
// when we do, and are not running in userspace.  This initial implementation will always use a
// string buffer, as it doesn't depend on static symbols in print.c.
//
// A consequence is that the CONFIG_PRINTK_SYNC is enabled, the synchonization will only occur at
// the granularity of these buffers rather than at the print message level.

/// The buffer size for syscall.  This is a tradeoff between efficiency (large buffers need fewer
/// syscalls) and needing more stack space.
const BUF_SIZE: usize = 32;

struct Context {
    // How many characters are used in the buffer.
    count: usize,
    // Bytes written.
    buf: [u8; BUF_SIZE],
}

fn utf8_byte_length(byte: u8) -> usize {
    if byte & 0b1000_0000 == 0 {
        // Single byte (0xxxxxxx)
        1
    } else if byte & 0b1110_0000 == 0b1100_0000 {
        // Two-byte sequence (110xxxxx)
        2
    } else if byte & 0b1111_0000 == 0b1110_0000 {
        // Three-byte sequence (1110xxxx)
        3
    } else if byte & 0b1111_1000 == 0b1111_0000 {
        // Four-byte sequence (11110xxx)
        4
    } else {
        // Continuation byte or invalid (10xxxxxx)
        1
    }
}

impl Context {
    fn add_byte(&mut self, b: u8) {
        // Ensure we have room for an entire UTF-8 sequence.
        if self.count + utf8_byte_length(b) > self.buf.len() {
            self.flush();
        }

        self.buf[self.count] = b;
        self.count += 1;
    }

    fn flush(&mut self) {
        if self.count > 0 {
            unsafe {
                zephyr_sys::k_str_out(self.buf.as_mut_ptr() as *mut i8, self.count);
            }
            self.count = 0;
        }
    }
}

impl Write for Context {
    fn write_str(&mut self, s: &str) -> Result {
        for b in s.bytes() {
            self.add_byte(b);
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn printk(args: Arguments<'_>) {
    let mut context = Context {
        count: 0,
        buf: [0; BUF_SIZE],
    };
    write(&mut context, args).unwrap();
    context.flush();
}

#[doc(hidden)]
pub fn printkln(args: Arguments<'_>) {
    let mut context = Context {
        count: 0,
        buf: [0; BUF_SIZE],
    };
    write(&mut context, args).unwrap();
    context.add_byte(b'\n');
    context.flush();
}
