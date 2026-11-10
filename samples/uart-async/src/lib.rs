// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![allow(unexpected_cfgs)]

use core::{
    future::{poll_fn, Future},
    task::Poll,
};
use embassy_futures::join::join;
use embedded_io_async::{Read, Write};
use static_cell::StaticCell;
use zephyr::{embassy::Executor, printkln};
use zephyr_panic as _;

static EXECUTOR: StaticCell<Executor> = StaticCell::new();
const MESSAGE: &[u8] = b"Full-duplex Rust UART: \x00\x7f\x80\xff\r\n";

#[no_mangle]
extern "C" fn rust_main() {
    EXECUTOR.init(Executor::new()).run(|spawner| {
        spawner.spawn(demo().unwrap());
    });
}

#[embassy_executor::task]
async fn demo() {
    #[cfg(CONFIG_UART_INTERRUPT_DRIVEN)]
    run_interrupt().await;
    #[cfg(CONFIG_UART_ASYNC_API)]
    run_event().await;
    printkln!("Async UART loopback: all tests passed");
}

#[cfg(CONFIG_UART_INTERRUPT_DRIVEN)]
async fn run_interrupt() {
    use zephyr::device::uart::InterruptStorage;
    static STORAGE: StaticCell<InterruptStorage<64, 16>> = StaticCell::new();
    let uart = zephyr::devicetree::aliases::app_irq_uart::get_instance().unwrap();
    let mut uart = uart
        .into_interrupt(STORAGE.init(InterruptStorage::new()))
        .unwrap();
    {
        let (mut rx, mut tx) = uart.split();
        let mut cancelled_buf = [0; 1];
        {
            let mut pending = core::pin::pin!(rx.read(&mut cancelled_buf));
            assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx)))
                .await
                .is_pending());
        }
        let mut received = [0; MESSAGE.len()];
        let (sent, read) = join(tx.write_all(MESSAGE), rx.read_exact(&mut received)).await;
        sent.unwrap();
        read.unwrap();
        assert_eq!(received, MESSAGE);
        assert_eq!(rx.read(&mut []).await.unwrap(), 0);
        assert_eq!(tx.write(&[]).await.unwrap(), 0);
        assert_eq!(tx.flush().await.unwrap_err().0, zephyr::raw::ENOTSUP);
    }
    uart.shutdown().await.unwrap();
    printkln!("Async UART interrupt backend passed");
}

#[cfg(CONFIG_UART_ASYNC_API)]
async fn run_event() {
    use zephyr::device::uart::AsyncStorage;
    static STORAGE: StaticCell<AsyncStorage<64, 16>> = StaticCell::new();
    let uart = zephyr::devicetree::aliases::app_async_uart::get_instance().unwrap();
    let mut uart = uart.into_async(STORAGE.init(AsyncStorage::new())).unwrap();
    {
        let (mut rx, mut tx) = uart.split();
        let mut cancelled_buf = [0; 1];
        {
            let mut pending = core::pin::pin!(rx.read(&mut cancelled_buf));
            assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx)))
                .await
                .is_pending());
        }
        // The entire message is shorter than one driver RX buffer: receiving it
        // exercises inactivity reporting rather than waiting for that buffer to fill.
        let mut received = [0; MESSAGE.len()];
        let (sent, read) = join(tx.write_all(MESSAGE), rx.read_exact(&mut received)).await;
        sent.unwrap();
        read.unwrap();
        assert_eq!(received, MESSAGE);
        assert_eq!(rx.read(&mut []).await.unwrap(), 0);
        assert_eq!(tx.write(&[]).await.unwrap(), 0);
        tx.flush().await.unwrap();
    }
    uart.shutdown().await.unwrap();
    printkln!("Async UART event backend passed");
}
