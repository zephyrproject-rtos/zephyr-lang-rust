//! UART event backend. Driver loans are separate from protected CPU queues.

use super::{buffered::Shared, Uart};
use crate::{raw, Error, Result};
use core::{cell::UnsafeCell, ffi::c_void, future::poll_fn};

const RX_TIMEOUT_US: i32 = 1_000;

#[repr(align(32))]
struct Buffer<const N: usize>(UnsafeCell<[u8; N]>);

impl<const N: usize> Buffer<N> {
    const fn new() -> Self {
        Self(UnsafeCell::new([0; N]))
    }

    fn ptr(&self) -> *mut u8 {
        self.0.get().cast()
    }
}

struct EventState {
    driving: bool,
    refilling: bool,
    callbacks: usize,

    rx_active: bool,
    rx_needed: bool,
    rx_owned: [bool; 2],
    rx_reported: [usize; 2],
    rx_stop_issued: bool,

    tx_owned: bool,
    tx_len: usize,
    tx_stop_issued: bool,
}

enum Action {
    StartRx(usize),
    StartTx(usize),
    StopRx,
    StopTx,
}

/// Single-use static storage for the Zephyr async/event UART backend.
///
/// RX/TX are software queue capacities in 1..=i32::MAX. In addition to those
/// queues this owns two RX-byte driver buffers and one TX-byte staging buffer:
/// approximately 3*RX + 2*TX payload bytes, plus synchronization/metadata.
/// Driver buffers are 32-byte aligned; hardware DMA/cache restrictions still
/// depend on the driver and memory placement. No caller-slice pointer goes to C.
/// Storage is never returned for reuse, including on failed initialization.
pub struct AsyncStorage<const RX: usize, const TX: usize> {
    shared: Shared<RX, TX, EventState>,
    rx: [Buffer<RX>; 2],
    tx: Buffer<TX>,
}

// SAFETY: CPU metadata/queues are protected by a Zephyr spinlock. Driver buffers
// are separate UnsafeCells with reserved/owned bits set before submission and
// cleared only by validated terminal events. CPU writes never overlap an active
// driver loan. Callback userdata and every submitted buffer have static lifetime.
unsafe impl<const RX: usize, const TX: usize> Sync for AsyncStorage<RX, TX> {}

impl<const RX: usize, const TX: usize> Default for AsyncStorage<RX, TX> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const RX: usize, const TX: usize> AsyncStorage<RX, TX> {
    /// Create empty storage. Supply a static mutable reference to `into_async`.
    pub const fn new() -> Self {
        Self {
            shared: Shared::new(EventState {
                driving: false,
                refilling: false,
                callbacks: 0,
                rx_active: false,
                rx_needed: false,
                rx_owned: [false; 2],
                rx_reported: [0; 2],
                rx_stop_issued: false,
                tx_owned: false,
                tx_len: 0,
                tx_stop_issued: false,
            }),
            rx: [Buffer::new(), Buffer::new()],
            tx: Buffer::new(),
        }
    }

    fn drive(&self, device: *const raw::device) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.backend.driving {
                return;
            }

            state.backend.driving = true;
        }

        let mut changed = false;

        loop {
            let action = {
                let mut state = self.shared.state.lock().unwrap();
                if state.io.closing {
                    let b = &mut state.backend;
                    if b.refilling {
                        b.driving = false;
                        None
                    } else if b.rx_active && !b.rx_stop_issued {
                        b.rx_stop_issued = true;
                        Some(Action::StopRx)
                    } else if b.tx_owned && !b.tx_stop_issued {
                        b.tx_stop_issued = true;
                        Some(Action::StopTx)
                    } else {
                        b.driving = false;
                        None
                    }
                } else if !state.backend.rx_active && !state.backend.rx_owned.iter().any(|&v| v) {
                    state.backend.rx_active = true;
                    state.backend.rx_owned[0] = true;
                    state.backend.rx_reported[0] = 0;
                    state.backend.rx_stop_issued = false;

                    Some(Action::StartRx(0))
                } else if !state.backend.tx_owned && !state.io.tx.is_empty() {
                    // SAFETY: The metadata lock excludes another submitter and
                    // tx_owned is false, so C no longer has a loan of this region.
                    let bytes = unsafe { &mut *self.tx.0.get() };
                    let n = state.io.tx.pop(bytes);

                    // Reservation also covers the submitting interval: success
                    // does not write this bit back after an inline completion.
                    state.backend.tx_owned = true;
                    state.backend.tx_len = n;
                    state.backend.tx_stop_issued = false;

                    Some(Action::StartTx(n))
                } else {
                    state.backend.driving = false;
                    None
                }
            };

            let Some(action) = action else {
                break;
            };

            changed = true;

            // SAFETY: The device is a static ready UART with the event API.
            // Buffers have been reserved before these calls and are static.
            // No state lock is held: the driver may call us synchronously.
            let result = unsafe {
                match action {
                    Action::StartRx(slot) => {
                        raw::uart_rx_enable(device, self.rx[slot].ptr(), RX, RX_TIMEOUT_US)
                    }
                    Action::StartTx(n) => raw::uart_tx(device, self.tx.ptr().cast_const(), n, -1),
                    Action::StopRx => raw::uart_rx_disable(device),
                    Action::StopTx => raw::uart_tx_abort(device),
                }
            };

            if result < 0 {
                let mut state = self.shared.state.lock().unwrap();
                let code = result.unsigned_abs();

                match action {
                    Action::StartRx(_) | Action::StartTx(_) => state.io.fail(code),
                    Action::StopRx
                        if code == raw::EFAULT
                            && !state.backend.rx_active
                            && !state.backend.rx_owned.iter().any(|&v| v) => {}
                    Action::StopTx if code == raw::EFAULT && !state.backend.tx_owned => {}
                    _ => {
                        state.io.stop_error.get_or_insert(code);
                    }
                }

                // A failed submission never fabricates a release. Stop requests
                // may obtain terminal events; otherwise static storage stays held.
            }
        }

        if changed {
            let wake = self.shared.state.lock().unwrap().io.wake();
            wake.wake();
        }
    }

    fn refill(&self, device: *const raw::device) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.backend.refilling {
                return;
            }

            state.backend.refilling = true;
        }

        loop {
            let slot = {
                let mut state = self.shared.state.lock().unwrap();
                if state.io.closing || !state.backend.rx_active || !state.backend.rx_needed {
                    state.backend.refilling = false;
                    None
                } else if let Some(slot) = state.backend.rx_owned.iter().position(|&v| !v) {
                    state.backend.rx_owned[slot] = true;
                    state.backend.rx_reported[slot] = 0;
                    state.backend.rx_needed = false;

                    Some(slot)
                } else {
                    state.backend.refilling = false;
                    None
                }
            };

            let Some(slot) = slot else {
                break;
            };

            // SAFETY: A released static slot was reserved above, before lending
            // it to C. This response is permitted from the event callback and
            // executes without the metadata lock, including for inline events.
            let result = unsafe { raw::uart_rx_buf_rsp(device, self.rx[slot].ptr(), RX) };

            if result < 0 {
                self.shared
                    .state
                    .lock()
                    .unwrap()
                    .io
                    .fail(result.unsigned_abs());
            }
        }
    }

    fn handle(&self, device: *const raw::device, event: &raw::uart_event) {
        self.shared.state.lock().unwrap().backend.callbacks += 1;

        match event.type_ {
            raw::uart_event_type_UART_RX_BUF_REQUEST => {
                self.shared.state.lock().unwrap().backend.rx_needed = true;
            }

            raw::uart_event_type_UART_RX_RDY => {
                // SAFETY: The event tag selects this union member. The reference
                // is used only during the callback; the pointer is checked below.
                let rx = unsafe { event.data.rx.as_ref() };
                let slot = self.rx.iter().position(|buffer| buffer.ptr() == rx.buf);

                let mut state = self.shared.state.lock().unwrap();

                if let Some(slot) = slot {
                    let end = rx.offset.checked_add(rx.len);

                    if state.backend.rx_owned[slot]
                        && end.is_some_and(|end| end <= RX)
                        && rx.offset == state.backend.rx_reported[slot]
                    {
                        state.backend.rx_reported[slot] = end.unwrap();

                        // SAFETY: The buffer is one of our live driver loans;
                        // the checked range is exactly the newly reported stable
                        // portion. C may still use the remaining disjoint region.
                        let bytes =
                            unsafe { core::slice::from_raw_parts(rx.buf.add(rx.offset), rx.len) };
                        state.io.receive(bytes);
                    } else {
                        state.io.fail(raw::EIO);
                    }
                } else {
                    state.io.fail(raw::EIO);
                }
            }

            raw::uart_event_type_UART_RX_BUF_RELEASED => {
                // SAFETY: The event tag selects the release union member.
                let released = unsafe { event.data.rx_buf.as_ref() };
                let slot = self
                    .rx
                    .iter()
                    .position(|buffer| buffer.ptr() == released.buf);

                let mut state = self.shared.state.lock().unwrap();

                if let Some(slot) = slot.filter(|&slot| state.backend.rx_owned[slot]) {
                    state.backend.rx_owned[slot] = false;
                } else {
                    state.io.fail(raw::EIO);
                }
            }

            raw::uart_event_type_UART_RX_DISABLED => {
                let mut state = self.shared.state.lock().unwrap();
                state.backend.rx_active = false;
                state.backend.rx_needed = false;

                if state.backend.rx_owned.iter().any(|&v| v) {
                    state.io.fail(raw::EIO);
                }
            }

            raw::uart_event_type_UART_RX_STOPPED => {
                // SAFETY: The event tag selects the stopped union member. The
                // final received range is handled by its subsequent RX_RDY event.
                let stopped = unsafe { event.data.rx_stop.as_ref() };

                let code = if stopped.reason & raw::uart_rx_stop_reason_UART_ERROR_OVERRUN != 0 {
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

            raw::uart_event_type_UART_TX_DONE | raw::uart_event_type_UART_TX_ABORTED => {
                // SAFETY: Both terminal TX tags select the tx union member.
                let tx = unsafe { event.data.tx.as_ref() };
                let mut state = self.shared.state.lock().unwrap();

                if state.backend.tx_owned
                    && tx.buf == self.tx.ptr().cast_const()
                    && tx.len <= state.backend.tx_len
                {
                    let complete = tx.len == state.backend.tx_len;

                    state.backend.tx_owned = false;
                    state.backend.tx_len = 0;

                    if !state.io.closing
                        && (event.type_ == raw::uart_event_type_UART_TX_ABORTED || !complete)
                    {
                        state.io.tx_failed(raw::EIO);
                    }
                } else {
                    state.io.fail(raw::EIO);
                }
            }

            _ => self.shared.state.lock().unwrap().io.fail(raw::EIO),
        }

        self.refill(device);
        self.drive(device);

        let wake = {
            let mut state = self.shared.state.lock().unwrap();
            state.backend.callbacks -= 1;
            state.io.wake()
        };

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
    event: *mut raw::uart_event,
    user: *mut c_void,
) {
    // SAFETY: Registration supplies single-use static storage of this exact type;
    // Zephyr keeps the tagged event valid for the duration of this callback.
    let (storage, event) = unsafe { (&*user.cast::<AsyncStorage<RX, TX>>(), &*event) };
    storage.handle(device, event);
}

impl Uart {
    /// Consume the polling handle and enable Zephyr event/DMA-backed async I/O.
    ///
    /// Requires a dedicated UART with the event API and appropriate DMA resources
    /// where applicable. RX uses a 1 ms inactivity-reporting timeout, subject to
    /// driver granularity. Capacities must be nonzero. On error the handle and
    /// storage remain consumed; potentially retained driver buffers are not reused.
    pub fn into_async<const RX: usize, const TX: usize>(
        self,
        storage: &'static mut AsyncStorage<RX, TX>,
    ) -> Result<AsyncUart<RX, TX>> {
        self.ensure_ready()?;
        Shared::<RX, TX, EventState>::validate()?;

        // SAFETY: The ready device is static. Userdata and buffers reside in
        // uniquely supplied static storage, never in the movable returned handle.
        let result = unsafe {
            raw::uart_callback_set(
                self.device,
                Some(callback::<RX, TX>),
                (storage as *mut AsyncStorage<RX, TX>).cast(),
            )
        };

        if result < 0 {
            return Err(Error(result.unsigned_abs()));
        }

        storage.drive(self.device);

        if let Some(code) = storage.shared.state.lock().unwrap().io.stop_error {
            return Err(Error(code));
        }

        Ok(AsyncUart {
            device: self.device,
            storage,
        })
    }
}

/// Buffered Zephyr event-backed UART with cancellation-safe individual reads/writes.
///
/// Writes count accepted bytes; flush confirms driver TX completion. Drop starts
/// cancellation without waiting. Static storage is retained for any late events.
pub struct AsyncUart<const RX: usize, const TX: usize> {
    device: *const raw::device,
    storage: &'static AsyncStorage<RX, TX>,
}

/// Borrowed receive half of an event-backed UART.
pub struct AsyncRx<'a, const RX: usize, const TX: usize> {
    device: *const raw::device,
    storage: &'a AsyncStorage<RX, TX>,
}

/// Borrowed transmit half of an event-backed UART.
pub struct AsyncTx<'a, const RX: usize, const TX: usize> {
    device: *const raw::device,
    storage: &'a AsyncStorage<RX, TX>,
}

impl<const RX: usize, const TX: usize> AsyncUart<RX, TX> {
    /// Borrow independent halves for concurrent RX and TX on one executor.
    pub fn split(&mut self) -> (AsyncRx<'_, RX, TX>, AsyncTx<'_, RX, TX>) {
        (
            AsyncRx {
                device: self.device,
                storage: self.storage,
            },
            AsyncTx {
                device: self.device,
                storage: self.storage,
            },
        )
    }

    /// Stop RX and abort queued/in-flight TX, awaiting buffer-release events.
    ///
    /// This does not drain TX or power off the peripheral. Cancelling leaves the
    /// handle closing; awaiting again resumes the same operation. Storage is not
    /// returned for reuse, including on driver errors or incomplete shutdown.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.storage.stop(self.device);

        poll_fn(|cx| {
            self.storage.shared.stopped(cx, |b| {
                !b.driving
                    && !b.refilling
                    && b.callbacks == 0
                    && !b.rx_active
                    && !b.tx_owned
                    && !b.rx_owned.iter().any(|&v| v)
            })
        })
        .await
    }
}

impl<const RX: usize, const TX: usize> Drop for AsyncUart<RX, TX> {
    fn drop(&mut self) {
        self.storage.stop(self.device);
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::ErrorType for AsyncUart<RX, TX> {
    type Error = Error;
}

impl<const RX: usize, const TX: usize> embedded_io_async::ErrorType for AsyncRx<'_, RX, TX> {
    type Error = Error;
}

impl<const RX: usize, const TX: usize> embedded_io_async::ErrorType for AsyncTx<'_, RX, TX> {
    type Error = Error;
}

impl<const RX: usize, const TX: usize> embedded_io_async::Read for AsyncUart<RX, TX> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        poll_fn(|cx| self.storage.shared.read(cx, out)).await
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::Read for AsyncRx<'_, RX, TX> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        poll_fn(|cx| self.storage.shared.read(cx, out)).await
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::Write for AsyncUart<RX, TX> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        poll_fn(|cx| {
            let result = self.storage.shared.write(cx, bytes);
            self.storage.drive(self.device);

            result
        })
        .await
    }

    async fn flush(&mut self) -> Result<()> {
        poll_fn(|cx| {
            self.storage
                .shared
                .flushed(cx, |b| !b.tx_owned && !b.driving)
        })
        .await
    }
}

impl<const RX: usize, const TX: usize> embedded_io_async::Write for AsyncTx<'_, RX, TX> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        poll_fn(|cx| {
            let result = self.storage.shared.write(cx, bytes);
            self.storage.drive(self.device);

            result
        })
        .await
    }

    async fn flush(&mut self) -> Result<()> {
        poll_fn(|cx| {
            self.storage
                .shared
                .flushed(cx, |b| !b.tx_owned && !b.driving)
        })
        .await
    }
}
