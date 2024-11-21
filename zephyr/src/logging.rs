//! Rust logging in Zephyr
//!
//! There are a few config settings in Zephyr that affect how logging is done.  Zephyr has multiple
//! choices of front ends and backends for the logging.  We try to work with a few of these.
//!
//! There are various tradeoffs in terms of code size, and processing time that affect how logging
//! is done.  In addition to the tradeoffs in Zephyr, there are also tradeoffs in the Rust world,
//! especially when it comes to formatting.
//!
//! For now, any use of logging in Rust will go through the `log` crate.  This brings in the
//! overhead of string formatting.  For the most part, due to a semantic mismatch between how Rust
//! does string formatting, and how C does it, we will not defer logging, but generate log strings
//! that will be sent as full strings to the Zephyr logging system.
//!
//! - `CONFIG_LOG`: Global enable of logging in Zephyr.
//! - `CONFIG_LOG_MODE_MINIMAL`: Indicates a minimal printk type of logging.
//!
//! At this time, we will provide two loggers.  One is based on using the underlying printk
//! mechanism, and if enabled, will send log messages directly to printk.  This roughly corresponds
//! to the minimal logging mode of Zephyr, but also works if logging is entirely disabled.
//!
//! The other log backend uses the logging infrastructure to log simple messages to the C logger.
//! At this time, these require allocation for the string formatting, although the allocation will
//! generally be short lived.  A good future task will be to make the string formatter format
//! directly into the log buffer used by Zephyr.

use log::{LevelFilter, Log, SetLoggerError};

cfg_if::cfg_if! {
    if #[cfg(all(CONFIG_PRINTK,
                 any(not(CONFIG_LOG),
                     all(CONFIG_LOG, CONFIG_LOG_MODE_MINIMAL))))]
    {
        // If we either have no logging, or it is minimal, and we have printk, we can do the printk
        // logging handler.
        mod impl_printk;
        pub use impl_printk::set_logger;
    } else if #[cfg(all(CONFIG_LOG,
                        not(CONFIG_LOG_MODE_MINIMAL),
                        CONFIG_RUST_ALLOC))]
    {
        // Otherwise, if we have logging and allocation, and not minimal, we can use the Zephyr
        // logging backend.  Doing this without allocation is currently a TODO:
        mod impl_zlog;
        pub use impl_zlog::set_logger;
    } else {
        /// No lagging is possible, provide an empty handler that does nothing.
        ///
        /// It could be nice to print a message somewhere, but this is only available when there is
        /// no where we can send messages.
        pub unsafe fn set_logger() -> Result<(), SetLoggerError> {
            Ok(())
        }
    }
}

// The Rust logging system has different entry points based on whether or not we are on a target
// with atomic pointers.  We will provide a single function for this, which will be safe or unsafe
// depending on this.  The safety has to do with initialization order, and as long as this is called
// before any other threads run, it should be safe.
//
// TODO: Allow the default level to be set through Kconfig.
cfg_if::cfg_if! {
    if #[cfg(target_has_atomic = "ptr")] {
        unsafe fn set_logger_internal(logger: &'static dyn Log) -> Result<(), SetLoggerError> {
            log::set_logger(logger)?;
            log::set_max_level(LevelFilter::Info);
            Ok(())
        }
    } else {
        unsafe fn set_logger_internal(logger: &'static dyn Log) -> Result<(), SetLoggerError> {
            log::set_logger_racy(logger)?;
            log::set_max_level_racy(LevelFilter::Info);
            Ok(())
        }
    }
}
