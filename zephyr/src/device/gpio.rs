//! Most devices in Zephyr operate on a `struct device`.  This provides untyped access to
//! devices.  We want to have stronger typing in the Zephyr interfaces, so most of these types
//! will be wrapped in another structure.  This wraps a Gpio device, and provides methods to
//! most of the operations on gpios.
//!
//! Safey: In general, even just using gpio pins is unsafe in Zephyr.  The gpio drivers are used
//! pervasively throughout Zephyr device drivers.  As such, most of the calls in this module are
//! unsafe.

use core::ffi::c_int;

use super::{NoStatic, Unique};
use crate::raw;

#[cfg(feature = "async-drivers")]
mod async_io {
    //! Async operations for gpio drivers.
    //!
    //! For now, we make an assumption that a gpio controller can contain up to 32 gpios, which is
    //! the largest number currently used, although this might change with 64-bit targest in the
    //! future.

    use core::{
        cell::UnsafeCell,
        future::Future,
        mem,
        sync::atomic::Ordering,
        task::{Poll, Waker},
    };

    use embassy_sync::waitqueue::AtomicWaker;
    use zephyr_sys::{
        device, gpio_add_callback, gpio_callback, gpio_init_callback, gpio_pin_interrupt_configure,
        gpio_pin_interrupt_configure_dt, gpio_port_pins_t, ZR_GPIO_INT_LEVEL_HIGH,
        ZR_GPIO_INT_LEVEL_LOW, ZR_GPIO_INT_MODE_DISABLE_ONLY,
    };

    use crate::sync::atomic::{AtomicBool, AtomicU32};

    use super::GpioPin;

    pub(crate) struct GpioStatic {
        /// The wakers for each of the gpios.
        wakers: [AtomicWaker; 32],
        /// Indicates when an interrupt has fired.  Used to definitively indicate the event has
        /// happened, so we can wake.
        fired: AtomicU32,
        /// Have we been initialized?
        installed: AtomicBool,
        /// The data for the callback itself.
        callback: UnsafeCell<gpio_callback>,
    }

    unsafe impl Sync for GpioStatic {}

    impl GpioStatic {
        pub(crate) const fn new() -> Self {
            Self {
                wakers: [const { AtomicWaker::new() }; 32],
                fired: AtomicU32::new(0),
                installed: AtomicBool::new(false),
                // SAFETY: `installed` will tell us this need to be installed.
                callback: unsafe { mem::zeroed() },
            }
        }

        /// Ensure that the callback has been installed.
        pub(super) fn fast_install(&self, port: *const device) {
            if !self.installed.load(Ordering::Acquire) {
                self.install(port);
            }
        }

        fn install(&self, port: *const device) {
            critical_section::with(|_| {
                if !self.installed.load(Ordering::Acquire) {
                    let cb = self.callback.get();
                    // SAFETY: We're in a critical section, so there should be no concurrent use,
                    // and there should not be any calls from the driver.
                    unsafe {
                        gpio_init_callback(cb, Some(Self::callback_handler), 0);
                        gpio_add_callback(port, cb);
                    }

                    self.installed.store(true, Ordering::Release);
                }
            })
        }

        /// Register (replacing) a given callback.
        pub(super) fn register(&self, pin: u8, waker: &Waker) {
            self.wakers[pin as usize].register(waker);

            // SAFETY: Inherently unsafe, due to how the Zephyr API is defined.
            // The API appears to assume coherent memory, which although untrue, probably is "close
            // enough" on the supported targets.
            // The main issue is to ensure that any race is resolved in the direction of getting the
            // callback more than needed, rather than missing.  In the context here, ensure the
            // waker is registered (which does use an atomic), before enabling the pin in the
            // callback structure.
            //
            // If it seems that wakes are getting missed, it might be the case that this needs some
            // kind of memory barrier.
            let cb = self.callback.get();
            unsafe {
                (*cb).pin_mask |= 1 << pin;
            }
        }

        extern "C" fn callback_handler(
            port: *const device,
            cb: *mut gpio_callback,
            mut pins: gpio_port_pins_t,
        ) {
            let data = unsafe {
                cb.cast::<u8>()
                    .sub(mem::offset_of!(Self, callback))
                    .cast::<Self>()
            };

            // For each pin we are informed of.
            while pins > 0 {
                let pin = pins.trailing_zeros();

                pins &= !(1 << pin);

                // SAFETY: Handling this correctly is a bit tricky, especially with the
                // un-coordinated 'pin-mask' value.
                //
                // For level-triggered interrupts, not disabling this will result in an interrupt
                // storm.
                unsafe {
                    // Disable the actual interrupt from the controller.
                    gpio_pin_interrupt_configure(port, pin as u8, ZR_GPIO_INT_MODE_DISABLE_ONLY);

                    // Remove the callback bit.  Unclear if this is actually useful.
                    (*cb).pin_mask &= !(1 << pin);

                    // Indicate that we have fired.
                    // AcqRel is sufficient for ordering across a single atomic.
                    (*data).fired.fetch_or(1 << pin, Ordering::AcqRel);

                    // After the interrupt is off, wake the handler.
                    (*data).wakers[pin as usize].wake();
                }
            }
        }

        /// Check if we have fired for a given pin.  Clears the status.
        pub(crate) fn has_fired(&self, pin: u8) -> bool {
            let value = self.fired.fetch_and(!(1 << pin), Ordering::AcqRel);
            value & (1 << pin) != 0
        }
    }

    impl GpioPin {
        /// Asynchronously wait for a gpio pin to become high.
        ///
        /// # Safety
        ///
        /// Safety of multiple GPIOs depends on the underlying controller.
        pub unsafe fn wait_for_high(&mut self) -> impl Future<Output = ()> + use<'_> {
            GpioWait::new(self, 1)
        }

        /// Asynchronously wait for a gpio pin to become low.
        ///
        /// # Safety
        ///
        /// Safety of multiple GPIOs depends on the underlying controller.
        pub unsafe fn wait_for_low(&mut self) -> impl Future<Output = ()> + use<'_> {
            GpioWait::new(self, 0)
        }
    }

    /// A future that waits for a gpio to become high.
    pub struct GpioWait<'a> {
        pin: &'a mut GpioPin,
        level: u8,
    }

    impl<'a> GpioWait<'a> {
        fn new(pin: &'a mut GpioPin, level: u8) -> Self {
            Self { pin, level }
        }
    }

    impl<'a> Future for GpioWait<'a> {
        type Output = ();

        fn poll(
            self: core::pin::Pin<&mut Self>,
            cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<Self::Output> {
            self.pin.data.fast_install(self.pin.pin.port);

            // Early detection of the event.  Also clears.
            // This should be non-racy as long as only one task at a time waits on the gpio.
            if self.pin.data.has_fired(self.pin.pin.pin) {
                return Poll::Ready(());
            }

            self.pin.data.register(self.pin.pin.pin, cx.waker());

            let mode = match self.level {
                0 => ZR_GPIO_INT_LEVEL_LOW,
                1 => ZR_GPIO_INT_LEVEL_HIGH,
                _ => unreachable!(),
            };

            unsafe {
                gpio_pin_interrupt_configure_dt(&self.pin.pin, mode);

                // Before sleeping, check if it fired, to avoid having to pend if it already
                // happened.
                if self.pin.data.has_fired(self.pin.pin.pin) {
                    return Poll::Ready(());
                }
            }

            Poll::Pending
        }
    }
}

#[cfg(not(feature = "async-drivers"))]
mod async_io {
    pub(crate) struct GpioStatic;

    impl GpioStatic {
        pub(crate) const fn new() -> Self {
            Self
        }
    }
}

pub(crate) use async_io::*;

/// A single instance of a zephyr device to manage a gpio controller.  A gpio controller
/// represents a set of gpio pins, that are generally operated on by the same hardware block.
pub struct Gpio {
    /// The underlying device itself.
    #[allow(dead_code)]
    pub(crate) device: *const raw::device,
    /// Our associated data, used for callbacks.
    pub(crate) data: &'static GpioStatic,
}

// SAFETY: Gpio's can be shared with other threads.  Safety is maintained by the Token.
unsafe impl Send for Gpio {}

impl Gpio {
    /// Constructor, used by the devicetree generated code.
    ///
    /// TODO: Guarantee single instancing.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        data: &'static GpioStatic,
        device: *const raw::device,
    ) -> Option<Gpio> {
        if !unique.once() {
            return None;
        }
        Some(Gpio { device, data })
    }

    /// Verify that the device is ready for use.  At a minimum, this means the device has been
    /// successfully initialized.
    pub fn is_ready(&self) -> bool {
        unsafe { raw::device_is_ready(self.device) }
    }
}

/// A GpioPin represents a single pin on a gpio device.
///
/// This is a lightweight wrapper around the Zephyr `gpio_dt_spec` structure.
#[allow(dead_code)]
pub struct GpioPin {
    pub(crate) pin: raw::gpio_dt_spec,
    pub(crate) data: &'static GpioStatic,
}

// SAFETY: GpioPin's can be shared with other threads.  Safety is maintained by the Token.
unsafe impl Send for GpioPin {}

impl GpioPin {
    /// Constructor, used by the devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        _static: &NoStatic,
        device: *const raw::device,
        device_static: &'static GpioStatic,
        pin: u32,
        dt_flags: u32,
    ) -> Option<GpioPin> {
        if !unique.once() {
            return None;
        }
        Some(GpioPin {
            pin: raw::gpio_dt_spec {
                port: device,
                pin: pin as raw::gpio_pin_t,
                dt_flags: dt_flags as raw::gpio_dt_flags_t,
            },
            data: device_static,
        })
    }

    /// Verify that the device is ready for use.  At a minimum, this means the device has been
    /// successfully initialized.
    pub fn is_ready(&self) -> bool {
        self.get_gpio().is_ready()
    }

    /// Get the underlying Gpio device.
    pub fn get_gpio(&self) -> Gpio {
        Gpio {
            device: self.pin.port,
            data: self.data,
        }
    }

    /// Configure a single pin.
    ///
    /// # Safety
    ///
    /// Concurrency safety is determined by the underlying driver.
    pub fn configure(&mut self, extra_flags: raw::gpio_flags_t) {
        // TODO: Error?
        unsafe {
            raw::gpio_pin_configure(
                self.pin.port,
                self.pin.pin,
                self.pin.dt_flags as raw::gpio_flags_t | extra_flags,
            );
        }
    }

    /// Toggle pin level.
    ///
    /// # Safety
    ///
    /// Concurrency safety is determined by the underlying driver.
    pub fn toggle_pin(&mut self) {
        // TODO: Error?
        unsafe {
            raw::gpio_pin_toggle_dt(&self.pin);
        }
    }

    /// Set the logical level of the pin.
    pub fn set(&mut self, value: bool) {
        unsafe {
            raw::gpio_pin_set_dt(&self.pin, value as c_int);
        }
    }

    /// Read the logical level of the pin.
    pub fn get(&mut self) -> bool {
        unsafe {
            match raw::gpio_pin_get_dt(&self.pin) {
                0 => false,
                1 => true,
                _ => panic!("TODO: Handle gpio get error"),
            }
        }
    }
}
