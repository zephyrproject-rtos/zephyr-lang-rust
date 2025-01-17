//! Logging through printk
//!
//! This module implements a log handler (for the [`log`] crate) that logs messages through Zephyr's
//! printk mechanism.
//!
//! Currently, filtering is global, and set to Info.

use log::{Log, Metadata, Record, SetLoggerError};

use crate::printkln;

/// A simple log handler, built around printk.
struct PrintkLogger;

impl Log for PrintkLogger {
    // For now, everything is just available.
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    // Print out the log message, using printkln.
    //
    // Normal caveats behind printkln apply, if `RUST_PRINTK_SYNC` is not defined, then all message
    // printing will be racy.  Otherwise, the message will be broken into small chunks that are each
    // printed atomically.
    fn log(&self, record: &Record<'_>) {
        printkln!("{}:{}: {}", record.level(), record.target(), record.args());
    }

    // Flush is not needed.
    fn flush(&self) {}
}

static PRINTK_LOGGER: PrintkLogger = PrintkLogger;

/// Set the log handler to log messages through printk in Zephyr.
///
/// # Safety
///
/// This is unsafe due to racy issues in the log framework on targets that do not support atomic
/// pointers.  As long as this is called ever by a single thread, it is safe to use.
pub unsafe fn set_logger() -> Result<(), SetLoggerError> {
    super::set_logger_internal(&PRINTK_LOGGER)
}
