//! Logging through Zephyr's log mechanism.
//!
//! This module implements a log handler for the [`log`] crate that logs messages through Zephyr's
//! logging infrastructure.
//!
//! Zephyr's logging is heavily built around a lot of C assumptions, and involves a lot of macros.
//! As such, this initial implemention will try to keep things simple, and format the message to an
//! allocated string, and send that off to the logging infrastructure, formatted as a "%s" message.
//! There are a lot of opportunities to improve this.

extern crate alloc;

use alloc::format;
use core::ffi::c_char;

use log::{Level, Log, Metadata, Record, SetLoggerError};

use crate::raw;

struct ZlogLogger;

impl Log for ZlogLogger {
    // For now, just always print messages.
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    /// Print out the log message.
    fn log(&self, record: &Record<'_>) {
        let level = match record.level() {
            Level::Error => raw::LOG_LEVEL_ERR,
            Level::Warn => raw::LOG_LEVEL_WRN,
            Level::Info => raw::LOG_LEVEL_INF,
            Level::Debug => raw::LOG_LEVEL_DBG,
            // Zephyr doesn't have a separate trace, so fold that into debug.
            Level::Trace => raw::LOG_LEVEL_DBG,
        };
        let mut msg = format!("{}: {}", record.target(), record.args());
        // Append a null so this is a valid C string.  This lets us avoid an additional allocation
        // and copying.
        msg.push('\x00');
        unsafe {
            rust_log_message(level, msg.as_ptr() as *const c_char);
        }
    }

    // Flush not needed.
    fn flush(&self) {}
}

extern "C" {
    fn rust_log_message(level: u32, msg: *const c_char);
}

static ZLOG_LOGGER: ZlogLogger = ZlogLogger;

/// Set the log handler to log messages through Zephyr's logging framework.
///
/// This is unsafe due to racy issues in the log framework on targets that do not support atomic
/// pointers.
pub unsafe fn set_logger() -> Result<(), SetLoggerError> {
    super::set_logger_internal(&ZLOG_LOGGER)
}
