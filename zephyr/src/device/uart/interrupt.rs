//! Buffered UART interrupt backend. Driver calls and wakes run outside state locks.

use super::{buffered::Shared, Uart};
use crate::{raw, Error, Result};
use core::{ffi::c_void, future::poll_fn};

struct IrqState {
    driving: bool,
    handling: bool,

    rx: bool,
    tx: bool,
    errors: bool,
}

/// Application-owned, single-use static storage for interrupt UART I/O.
///
/// RX/TX are byte queue capacities, each in 1..=i32::MAX. No heap is used.
/// Storage is not returned for reuse after shutdown, Drop, or failed conversion.
pub struct InterruptStorage<const RX: usize, const TX: usize> {
    shared: Shared<RX, TX, IrqState>,
}

impl<const RX: usize, const TX: usize> Default for InterruptStorage<RX, TX> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const RX: usize, const TX: usize> InterruptStorage<RX, TX> {
    /// Create empty storage. Give it a static lifetime before converting a UART.
    pub const fn new() -> Self {
        Self {
            shared: Shared::new(IrqState {
                driving: false,
                handling: false,
                rx: false,
                tx: false,
                errors: false,
            }),
        }
    }

    fn drive(&self, device: *const raw::device) {
        // IRQ control calls are ISR-safe and do not wait. Keep their dispatcher
        // from being preempted by an IRQ or a rescheduled workqueue callback
        // that needs this same dispatcher to quiesce. No metadata lock spans
        // a driver call, so a directly invoked callback can still access state.
        // Release the interrupt lock before invoking arbitrary user wakers.
        let changed = critical_section::with(|cs| self.drive_control(device, cs));

        if changed {
            let wake = self.shared.state.lock().unwrap().io.wake();
            wake.wake();
        }
    }

    fn drive_control(
        &self,
        device: *const raw::device,
        _cs: critical_section::CriticalSection<'_>,
    ) -> bool {
        let mut changed = false;
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.backend.driving {
                return false;
            }

            state.backend.driving = true;
        }

        loop {
            let action = {
                let mut state = self.shared.state.lock().unwrap();
                let rx = !state.io.closing;
                let tx = rx && !state.io.tx.is_empty();
                let b = &mut state.backend;

                if b.rx != rx {
                    Some((0, rx))
                } else if b.errors != rx {
                    Some((1, rx))
                } else if b.tx != tx {
                    Some((2, tx))
                } else {
                    b.driving = false;
                    None
                }
            };

            let Some((kind, enable)) = action else {
                break;
            };

            changed = true;

            // SAFETY: Construction establishes a static ready UART with the IRQ
            // API. One dispatcher serializes control changes; no state lock is
            // held because enabling may synchronously invoke our callback.
            unsafe {
                match (kind, enable) {
                    (0, true) => raw::uart_irq_rx_enable(device),
                    (0, false) => raw::uart_irq_rx_disable(device),
                    (1, true) => raw::uart_irq_err_enable(device),
                    (1, false) => raw::uart_irq_err_disable(device),
                    (2, true) => raw::uart_irq_tx_enable(device),
                    _ => raw::uart_irq_tx_disable(device),
                }
            }

            let mut state = self.shared.state.lock().unwrap();

            match kind {
                0 => state.backend.rx = enable,
                1 => state.backend.errors = enable,
                _ => state.backend.tx = enable,
            }
        }

        changed
    }

    fn handle(&self, device: *const raw::device) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.backend.handling || state.io.closing {
                return;
            }

            state.backend.handling = true;
        }

        // SAFETY: Invoked only by the registered UART IRQ callback. Each loop
        // refreshes status and all FIFO buffers are live local arrays.
        unsafe {
            loop {
                if self.shared.state.lock().unwrap().io.closing {
                    break;
                }

                raw::uart_irq_update(device);

                let errors = raw::uart_err_check(device);
                if errors < 0
                    && errors.unsigned_abs() != raw::ENOSYS
                    && errors.unsigned_abs() != raw::ENOTSUP
                {
                    self.shared
                        .state
                        .lock()
                        .unwrap()
                        .io
                        .fail(errors.unsigned_abs());
                    break;
                }

                if errors > 0 {
                    let code = if errors as u32 & raw::uart_rx_stop_reason_UART_ERROR_OVERRUN != 0 {
                        raw::EOVERFLOW
                    } else {
                        raw::EIO
                    };

                    self.shared
                        .state
                        .lock()
                        .unwrap()
                        .io
                        .rx_error
                        .get_or_insert(code);
                }

                let pending = raw::uart_irq_is_pending(device);
                if pending < 0 {
                    self.shared
                        .state
                        .lock()
                        .unwrap()
                        .io
                        .fail(pending.unsigned_abs());
                    break;
                }

                if pending == 0 {
                    break;
                }

                let mut progress = false;

                let ready = raw::uart_irq_rx_ready(device);
                if ready < 0 {
                    self.shared
                        .state
                        .lock()
                        .unwrap()
                        .io
                        .fail(ready.unsigned_abs());
                    break;
                }

                if ready > 0 {
                    loop {
                        let mut bytes = [0; 64];
                        let n = raw::uart_fifo_read(device, bytes.as_mut_ptr(), bytes.len() as i32);

                        if n < 0 || n as usize > bytes.len() {
                            self.shared.state.lock().unwrap().io.fail(if n < 0 {
                                n.unsigned_abs()
                            } else {
                                raw::EIO
                            });
                            break;
                        }

                        self.shared
                            .state
                            .lock()
                            .unwrap()
                            .io
                            .receive(&bytes[..n as usize]);
                        progress |= n != 0;

                        if n as usize != bytes.len() {
                            break;
                        }
                    }
                }

                let ready = raw::uart_irq_tx_ready(device);
                if ready < 0 {
                    self.shared
                        .state
                        .lock()
                        .unwrap()
                        .io
                        .fail(ready.unsigned_abs());
                    break;
                }

                if ready > 0 {
                    let mut bytes = [0; 64];
                    let n = self.shared.state.lock().unwrap().io.tx.peek(&mut bytes);

                    if n > 0 {
                        let sent = raw::uart_fifo_fill(device, bytes.as_ptr(), n as i32);

                        if sent < 0 || sent as usize > n {
                            self.shared.state.lock().unwrap().io.fail(if sent < 0 {
                                sent.unsigned_abs()
                            } else {
                                raw::EIO
                            });
                            break;
                        }

                        self.shared
                            .state
                            .lock()
                            .unwrap()
                            .io
                            .tx
                            .discard(sent as usize);
                        progress |= sent != 0;
                    }
                }

                if !progress {
                    break;
                }
            }
        }

        let wake = {
            let mut state = self.shared.state.lock().unwrap();
            state.backend.handling = false;
            state.io.wake()
        };

        self.drive(device);
        wake.wake();
    }

    fn stop(&self, device: *const raw::device) {
        let wake = self.shared.stop();
        self.drive(device);
        wake.wake();
    }
}

unsafe extern "C" fn callback<const RX: usize, const TX: usize>(
    device: *const raw::device,
    user: *mut c_void,
) {
    // SAFETY: Registration supplies single-use static storage of this exact type.
    let storage = unsafe { &*user.cast::<InterruptStorage<RX, TX>>() };
    storage.handle(device);
}

impl Uart {
    /// Consume the polling handle and enable buffered interrupt-driven async I/O.
    ///
    /// Requires a dedicated UART, not one owned by C callbacks/console. Capacities
    /// must be nonzero. On error the handle and static storage remain consumed.
    pub fn into_interrupt<const RX: usize, const TX: usize>(
        self,
        storage: &'static mut InterruptStorage<RX, TX>,
    ) -> Result<InterruptUart<RX, TX>> {
        self.ensure_ready()?;
        Shared::<RX, TX, IrqState>::validate()?;

        // SAFETY: The device is static and ready. Userdata points to uniquely
        // supplied static storage, never the movable handle or a future.
        let result = unsafe {
            raw::uart_irq_callback_user_data_set(
                self.device,
                Some(callback::<RX, TX>),
                (storage as *mut InterruptStorage<RX, TX>).cast(),
            )
        };

        if result < 0 {
            return Err(Error(result.unsigned_abs()));
        }

        storage.drive(self.device);

        if let Some(code) = storage.shared.state.lock().unwrap().io.stop_error {
            return Err(Error(code));
        }

        Ok(InterruptUart {
            device: self.device,
            storage,
        })
    }
}

/// Buffered interrupt-driven UART. Individual reads/writes are cancellation-safe.
///
/// Writes report accepted queue bytes, not wire completion. `flush` returns
/// ENOTSUP. Drop requests stop without waiting; use `shutdown` to confirm it.
pub struct InterruptUart<const RX: usize, const TX: usize> {
    device: *const raw::device,
    storage: &'static InterruptStorage<RX, TX>,
}

/// Borrowed receive half of an interrupt-driven UART.
pub struct InterruptRx<'a, const RX: usize, const TX: usize> {
    device: *const raw::device,
    storage: &'a InterruptStorage<RX, TX>,
}

/// Borrowed transmit half of an interrupt-driven UART.
pub struct InterruptTx<'a, const RX: usize, const TX: usize> {
    device: *const raw::device,
    storage: &'a InterruptStorage<RX, TX>,
}

impl<const RX: usize, const TX: usize> InterruptUart<RX, TX> {
    /// Borrow independent halves for concurrent RX and TX on one executor.
    pub fn split(&mut self) -> (InterruptRx<'_, RX, TX>, InterruptTx<'_, RX, TX>) {
        (
            InterruptRx {
                device: self.device,
                storage: self.storage,
            },
            InterruptTx {
                device: self.device,
                storage: self.storage,
            },
        )
    }

    /// Abort queued writes and stop wrapper IRQ processing, without claiming wire-idle.
    /// Cancelling this wait leaves the device closing; awaiting again resumes it.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.storage.stop(self.device);

        poll_fn(|cx| {
            self.storage.shared.stopped(cx, |b| {
                !b.driving && !b.handling && !b.rx && !b.tx && !b.errors
            })
        })
        .await
    }
}

impl<const RX: usize, const TX: usize> Drop for InterruptUart<RX, TX> {
    fn drop(&mut self) {
        self.storage.stop(self.device);
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::ErrorType for InterruptUart<RX, TX> {
    type Error = Error;
}

impl<const RX: usize, const TX: usize> embedded_io_async::ErrorType for InterruptRx<'_, RX, TX> {
    type Error = Error;
}

impl<const RX: usize, const TX: usize> embedded_io_async::ErrorType for InterruptTx<'_, RX, TX> {
    type Error = Error;
}

impl<const RX: usize, const TX: usize> embedded_io_async::Read for InterruptUart<RX, TX> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        poll_fn(|cx| self.storage.shared.read(cx, out)).await
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::Read for InterruptRx<'_, RX, TX> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        poll_fn(|cx| self.storage.shared.read(cx, out)).await
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::Write for InterruptUart<RX, TX> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        poll_fn(|cx| {
            let result = self.storage.shared.write(cx, bytes);
            self.storage.drive(self.device);

            result
        })
        .await
    }

    async fn flush(&mut self) -> Result<()> {
        Err(Error(raw::ENOTSUP))
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::Write for InterruptTx<'_, RX, TX> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        poll_fn(|cx| {
            let result = self.storage.shared.write(cx, bytes);
            self.storage.drive(self.device);

            result
        })
        .await
    }

    async fn flush(&mut self) -> Result<()> {
        Err(Error(raw::ENOTSUP))
    }
}
