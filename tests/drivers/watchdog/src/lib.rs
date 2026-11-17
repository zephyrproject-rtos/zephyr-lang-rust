// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![allow(unexpected_cfgs)]

use zephyr_panic as _;

#[cfg(CONFIG_WATCHDOG)]
mod tests {
    use core::ffi::{c_int, c_uint};

    use zephyr::device::watchdog::{Options, Reset, TimeoutConfig};
    use zephyr::devicetree::{aliases, test_watchdog, test_watchdog_other, test_watchdog_unready};
    use zephyr::error::Result;
    use zephyr::raw;

    extern "C" {
        fn test_wdt_result(result: c_int);
        fn test_wdt_calls() -> c_uint;
        fn test_wdt_check_timeout(min: u32, max: u32, flags: u8);
        fn test_wdt_check_options(options: u8);
        fn test_wdt_check_channel(channel: c_int);
    }

    fn expect_error<T>(result: Result<T>, errno: u32) {
        assert_eq!(result.err().map(|error| error.0), Some(errno));
    }

    fn set_result(result: c_int) {
        // SAFETY: Updates the fixture only; all tests run on the main thread.
        unsafe { test_wdt_result(result) };
    }

    fn calls() -> c_uint {
        // SAFETY: Reads the fixture only; all tests run on the main thread.
        unsafe { test_wdt_calls() }
    }

    pub fn run() {
        for (min, max) in [(0, 0), (1, 1), (2, 1)] {
            expect_error(TimeoutConfig::new(min, max, Reset::Soc), raw::EINVAL);
        }

        let mut watchdog = aliases::watchdog0::get_instance().unwrap();
        assert!(test_watchdog::get_instance().is_none());
        assert!(watchdog.is_ready());
        assert_eq!(calls(), 0);

        for (min, max, reset, flag) in [
            (0, 1, Reset::None, raw::WDT_FLAG_RESET_NONE),
            (10, 1000, Reset::CpuCore, raw::WDT_FLAG_RESET_CPU_CORE),
            (u32::MAX - 1, u32::MAX, Reset::Soc, raw::WDT_FLAG_RESET_SOC),
        ] {
            let timeout = TimeoutConfig::new(min, max, reset).unwrap();
            watchdog.install_timeout(&timeout).unwrap();
            // SAFETY: This synchronous helper only checks captured driver values.
            unsafe { test_wdt_check_timeout(min, max, flag as u8) };
        }

        let timeout = TimeoutConfig::new(0, 1000, Reset::Soc).unwrap();
        set_result(-(raw::EBUSY as c_int));
        expect_error(watchdog.install_timeout(&timeout), raw::EBUSY);
        for id in [0, c_int::MAX] {
            set_result(id);
            let channel = watchdog.install_timeout(&timeout).unwrap();
            set_result(0);
            watchdog.feed(channel).unwrap();
            // SAFETY: This synchronous helper only checks the captured channel ID.
            unsafe { test_wdt_check_channel(id) };
        }

        for pause_in_sleep in [false, true] {
            for pause_when_debugging in [false, true] {
                watchdog
                    .setup(Options {
                        pause_in_sleep,
                        pause_when_debugging,
                    })
                    .unwrap();
                let expected = u8::from(pause_in_sleep) | (u8::from(pause_when_debugging) << 1);
                // SAFETY: This synchronous helper only checks captured setup options.
                unsafe { test_wdt_check_options(expected) };
            }
        }
        set_result(-(raw::ENOTSUP as c_int));
        expect_error(watchdog.setup(Options::default()), raw::ENOTSUP);

        set_result(0);
        let channel = watchdog.install_timeout(&timeout).unwrap();
        set_result(-(raw::EAGAIN as c_int));
        expect_error(watchdog.feed(channel), raw::EAGAIN);
        assert_eq!(calls(), 1);
        set_result(-(raw::EPERM as c_int));
        expect_error(watchdog.disable(), raw::EPERM);
        set_result(0);

        {
            let mut other = test_watchdog_other::get_instance().unwrap();
            expect_error(other.feed(channel), raw::EINVAL);

            let mut unready = test_watchdog_unready::get_instance().unwrap();
            assert!(!unready.is_ready());
            expect_error(unready.install_timeout(&timeout), raw::ENODEV);
            expect_error(unready.setup(Options::default()), raw::ENODEV);
            expect_error(unready.feed(channel), raw::ENODEV);
            expect_error(unready.disable(), raw::ENODEV);
        }
        assert_eq!(calls(), 0);
        watchdog.disable().unwrap();
    }
}

#[no_mangle]
extern "C" fn rust_main() {
    #[cfg(CONFIG_WATCHDOG)]
    tests::run();
    zephyr::printkln!("Watchdog tests passed");
}
