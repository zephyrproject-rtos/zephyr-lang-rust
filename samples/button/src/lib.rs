// Copyright (c) 2025 Emil Hammarström <emil.a.hammarstrom@gmail.com>
// SPDX-License-Identifier: Apache-2.0

#![no_std]
// Sigh. The check config system requires that the compiler be told what possible config values
// there might be.  This is completely impossible with both Kconfig and the DT configs, since the
// whole point is that we likely need to check for configs that aren't otherwise present in the
// build.  So, this is just always necessary.
#![allow(unexpected_cfgs)]

use log::{debug, info, warn};

use zephyr::raw::{ZR_GPIO_OUTPUT_ACTIVE, ZR_GPIO_INPUT};
use zephyr::device::gpio::GpioPin;
use zephyr::devicetree;
use zephyr::{embassy::Executor};

use static_cell::StaticCell;
use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Ticker};

#[derive(Debug)]
enum IoEvent {
    ButtonPress,
    ButtonRelease,
    LEDTimer,
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();
static IO_CHANNEL: Channel<CriticalSectionRawMutex, IoEvent, 8> = Channel::new();

#[embassy_executor::task]
async fn led_blinker(mut led: Option<GpioPin>) -> ! {
    enum State {
        Blinking,
        ButtonHeld,
    }

    let mut state = State::Blinking;
    let receiver = IO_CHANNEL.receiver();
    loop {
        let evt = receiver.receive().await;
        debug!("Received {:?}", evt);
        match state {
            State::Blinking => {
                match evt {
                    IoEvent::LEDTimer => {
                        if let Some(led) = &mut led {
                            led.toggle_pin();
                        }
                    }
                    IoEvent::ButtonPress => {
                        state = State::ButtonHeld;
                        info!("Blinking => ButtonHeld");
                        if let Some(led) = &mut led {
                            led.set(true);
                        }
                    }
                    IoEvent::ButtonRelease => {
                        // Ignore
                    }
                }
            }
            State::ButtonHeld => {
                match evt {
                    IoEvent::LEDTimer => {
                        // Ignore
                    }
                    IoEvent::ButtonPress => {
                        // Ignore
                    }
                    IoEvent::ButtonRelease => {
                        state = State::Blinking;
                        info!("ButtondHeld => Blinking");
                        if let Some(led) = &mut led {
                            led.set(false);
                        }
                    }
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn button_listener(mut button: GpioPin) -> ! {
    let sender = IO_CHANNEL.sender();
    loop {
        unsafe { button.wait_for_low().await; }
        sender.send(IoEvent::ButtonPress).await;
        unsafe { button.wait_for_high().await; }
        sender.send(IoEvent::ButtonRelease).await;
    }
}

#[embassy_executor::task]
async fn led_timer() -> ! {
    let mut ticker = Ticker::every(Duration::from_secs(1));
    let sender = IO_CHANNEL.sender();
    loop {
        ticker.next().await;
        sender.send(IoEvent::LEDTimer).await;
    }
}

#[no_mangle]
extern "C" fn rust_main() {
    unsafe {
        zephyr::set_logger().unwrap();
    }

    info!("Starting blinky button");

    let sw0 = configure_button();
    let led0 = configure_led();

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        match led0 {
            Ok(led) => {
                spawner.spawn(led_timer()).unwrap();
                spawner.spawn(led_blinker(Some(led))).unwrap();
            },
            Err(e) => {
                warn!("Failed to configure LED: {}", e);
                spawner.spawn(led_blinker(None)).unwrap();
            },
        }

        match sw0 {
            Ok(button) => {
                spawner.spawn(button_listener(button)).unwrap();
            },
            Err(e) => warn!("Failed to configure button: {}", e),
        }
    });
}

#[cfg(dt = "aliases::sw0")]
fn configure_button() -> Result<GpioPin, &'static str> {
    let mut sw0 = devicetree::aliases::sw0::get_instance().unwrap();

    if !sw0.is_ready() {
        Err("Button is not ready")
    } else {
        sw0.configure(ZR_GPIO_INPUT);
        Ok(sw0)
    }
}

#[cfg(not(dt = "aliases::sw0"))]
fn configure_button() -> Result<GpioPin, &'static str> {
    Err("sw0 not configured")
}

#[cfg(not(dt = "aliases::led0"))]
fn configure_led() -> Result<GpioPin, &'static str> {
    let mut led0 = devicetree::aliases::led0::get_instance().unwrap();

    if !led0.is_ready() {
        Err("LED is not ready")
    } else {
        led0.configure(ZR_GPIO_OUTPUT_ACTIVE);
        Ok(led0)
    }
}

#[cfg(dt = "aliases::led0")]
fn configure_led() -> Result<GpioPin, &'static str> {
    Err("led0 not configured")
}
