// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![allow(unexpected_cfgs)]

use zephyr_panic as _;

#[cfg(CONFIG_WATCHDOG)]
mod tests {
    use core::ffi::{c_int, c_uint, c_void};

    use zephyr::device::watchdog::{Options, Reset, TimeoutConfig, Watchdog, WatchdogChannel};
    use zephyr::devicetree::{aliases, test_watchdog, test_watchdog_other, test_watchdog_unready};
    use zephyr::error::Result;
    use zephyr::raw;

    extern "C" {
        fn test_wdt_irq_offload(
            routine: unsafe extern "C" fn(*const c_void),
            parameter: *const c_void,
        );
        fn test_wdt_result(result: c_int);
        fn test_wdt_calls() -> c_uint;
        fn test_wdt_check_timeout(min: u32, max: u32, flags: u8);
        fn test_wdt_check_options(options: u8);
        fn test_wdt_check_channel(channel: c_int);
        fn test_wdt_hold_feed();
        fn test_wdt_wait_for_feed();
        fn test_wdt_release_feed();
    }

    fn expect_error<T>(result: Result<T>, errno: u32) {
        assert_eq!(result.err().map(|error| error.0), Some(errno));
    }

    fn set_result(result: c_int) {
        // SAFETY: Called only when no worker is using the fixture.
        unsafe { test_wdt_result(result) };
    }

    fn calls() -> c_uint {
        // SAFETY: Workers are joined or blocked before reading the fixture.
        unsafe { test_wdt_calls() }
    }

    unsafe extern "C" fn check_isr(parameter: *const c_void) {
        // SAFETY: irq_offload synchronously borrows the caller's handle references.
        let (watchdog, channel) = unsafe {
            &mut *parameter
                .cast_mut()
                .cast::<(&mut Watchdog, &mut WatchdogChannel)>()
        };
        let timeout = TimeoutConfig::new(0, 1000, Reset::Soc).unwrap();
        expect_error(watchdog.install_timeout(&timeout), raw::EPERM);
        expect_error(watchdog.setup(Options::default()), raw::EPERM);
        expect_error(watchdog.disable(), raw::EPERM);
        expect_error(channel.feed(), raw::EPERM);
    }

    #[zephyr::thread(stack_size = 2048)]
    fn feed_channel(mut channel: WatchdogChannel) {
        channel.feed().unwrap();
    }

    #[zephyr::thread(stack_size = 2048)]
    fn disable_watchdog(mut watchdog: Watchdog) {
        watchdog.disable().unwrap();
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
            let _ = watchdog.install_timeout(&timeout).unwrap();
            // SAFETY: This synchronous helper only checks captured driver values.
            unsafe { test_wdt_check_timeout(min, max, flag as u8) };
        }

        let timeout = TimeoutConfig::new(0, 1000, Reset::Soc).unwrap();
        set_result(-(raw::EBUSY as c_int));
        expect_error(watchdog.install_timeout(&timeout), raw::EBUSY);
        for id in [0, c_int::MAX] {
            set_result(id);
            let mut channel = watchdog.install_timeout(&timeout).unwrap();
            set_result(0);
            channel.feed().unwrap();
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
        let mut channel = watchdog.install_timeout(&timeout).unwrap();
        let before = calls();
        let mut context = (&mut watchdog, &mut channel);
        // SAFETY: The handles remain exclusively borrowed until the ISR returns.
        unsafe { test_wdt_irq_offload(check_isr, core::ptr::from_mut(&mut context).cast()) };
        assert_eq!(calls(), before);
        set_result(-(raw::EAGAIN as c_int));
        expect_error(channel.feed(), raw::EAGAIN);
        assert_eq!(calls(), 1);
        set_result(-(raw::EPERM as c_int));
        expect_error(watchdog.disable(), raw::EPERM);
        set_result(0);
        channel.feed().unwrap();
        watchdog.disable().unwrap();

        let replacement = watchdog.install_timeout(&timeout).unwrap();
        set_result(0);
        expect_error(channel.feed(), raw::EINVAL);
        assert_eq!(calls(), 0);

        let mut other_channel = {
            let mut other = test_watchdog_other::get_instance().unwrap();
            other.install_timeout(&timeout).unwrap()
        };
        other_channel.feed().unwrap();

        let before = calls();
        let mut unready = test_watchdog_unready::get_instance().unwrap();
        assert!(!unready.is_ready());
        expect_error(unready.install_timeout(&timeout), raw::ENODEV);
        expect_error(unready.setup(Options::default()), raw::ENODEV);
        expect_error(unready.disable(), raw::ENODEV);
        assert_eq!(calls(), before);

        set_result(7);
        let mut peer = watchdog.install_timeout(&timeout).unwrap();
        set_result(0);
        watchdog.setup(Options::default()).unwrap();
        // SAFETY: Arm the fixture before starting the worker.
        unsafe { test_wdt_hold_feed() };
        let worker = feed_channel(replacement).start();
        // SAFETY: Wait for the worker to block inside the driver with the lock held.
        unsafe { test_wdt_wait_for_feed() };
        let before = calls();
        expect_error(peer.feed(), raw::EAGAIN);
        expect_error(watchdog.install_timeout(&timeout), raw::EAGAIN);
        expect_error(watchdog.setup(Options::default()), raw::EAGAIN);
        expect_error(watchdog.disable(), raw::EAGAIN);
        assert_eq!(calls(), before);
        other_channel.feed().unwrap();
        // SAFETY: Release the blocked worker before joining it.
        unsafe { test_wdt_release_feed() };
        worker.join().unwrap();
        peer.feed().unwrap();
        // SAFETY: The worker has exited, so the captured ID is stable.
        unsafe { test_wdt_check_channel(7) };

        disable_watchdog(watchdog).start().join().unwrap();
        set_result(0);
        expect_error(peer.feed(), raw::EINVAL);
        assert_eq!(calls(), 0);
        other_channel.feed().unwrap();
    }
}

#[no_mangle]
extern "C" fn rust_main() {
    #[cfg(CONFIG_WATCHDOG)]
    tests::run();
    zephyr::printkln!("Watchdog tests passed");
}
