// Copyright (c) 2026
// SPDX-License-Identifier: Apache-2.0

//! Blocking [`bt_hci`] transport support over Zephyr's HCI RAW channel.
//!
//! HCI RAW mode transfers ownership of the selected controller from Zephyr's
//! Bluetooth host to the caller.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt;
use core::mem::MaybeUninit;
use core::ptr;

use bt_hci::controller::blocking::TryError;
use bt_hci::transport::blocking::Transport;
use bt_hci::{ControllerToHostPacket, FromHciBytes, HostToControllerPacket, PacketKind};
use embedded_io::{ErrorKind, ErrorType};

use crate::raw;
use crate::sync::atomic::{AtomicBool, Ordering};
use crate::sys::{K_FOREVER, K_NO_WAIT};

/// Maximum H:4 packet size supported by the initial transport implementation.
pub const MAX_PACKET_SIZE: usize = 258;

/// Errors returned by [`HciRawTransport`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Zephyr rejected a RAW channel, queue, or controller operation.
    Zephyr(u32),
    /// The HCI RAW channel has already been enabled.
    AlreadyEnabled,
    /// The caller buffer cannot hold the complete H:4 packet.
    BufferTooSmall,
    /// The packet kind is not supported by this transport configuration.
    UnsupportedPacket(PacketKind),
    /// The bt-hci packet could not be decoded.
    Decode(bt_hci::FromHciBytesError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for Error {}

impl embedded_io::Error for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Zephyr(_) => ErrorKind::Other,
            Self::AlreadyEnabled => ErrorKind::AlreadyExists,
            Self::BufferTooSmall => ErrorKind::OutOfMemory,
            Self::UnsupportedPacket(_) | Self::Decode(_) => ErrorKind::InvalidInput,
        }
    }
}

struct RawFifo {
    inner: UnsafeCell<MaybeUninit<raw::k_fifo>>,
}

impl RawFifo {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    unsafe fn init(&self) -> *mut raw::k_fifo {
        let fifo = unsafe { (*self.inner.get()).as_mut_ptr() };
        unsafe { ptr::write(fifo, core::mem::zeroed()) };
        unsafe { raw::zr_hci_raw_fifo_init(fifo) };
        fifo
    }

    unsafe fn get(&self) -> *mut raw::k_fifo {
        unsafe { (*self.inner.get()).as_mut_ptr() }
    }
}

// SAFETY: the FIFO is initialized once before its address is passed to Zephyr,
// then accessed through Zephyr's synchronized FIFO API.
unsafe impl Sync for RawFifo {}

static RAW_FIFO: RawFifo = RawFifo::new();
static RAW_ENABLED: AtomicBool = AtomicBool::new(false);
static TRANSPORT: HciRawTransport = HciRawTransport { _private: () };

/// Process-wide blocking transport over Zephyr's HCI RAW channel.
pub struct HciRawTransport {
    _private: (),
}

impl HciRawTransport {
    /// Enable Zephyr HCI RAW mode and return its process-wide transport.
    ///
    /// This may be called exactly once. The returned transport is valid for
    /// the rest of the application lifetime.
    pub fn enable() -> Result<&'static Self, Error> {
        if RAW_ENABLED.swap(true, Ordering::AcqRel) {
            return Err(Error::AlreadyEnabled);
        }

        // SAFETY: RAW_FIFO has static storage and is initialized before its
        // pointer is retained by Zephyr.
        let fifo = unsafe { RAW_FIFO.init() };
        let result = unsafe { raw::bt_enable_raw(fifo) };
        if result < 0 {
            RAW_ENABLED.store(false, Ordering::Release);
            return Err(Error::Zephyr((-result) as u32));
        }

        Ok(&TRANSPORT)
    }

    /// Attempt to receive one controller packet without blocking.
    pub fn try_read<'a>(
        &self,
        rx: &'a mut [u8],
    ) -> Result<Option<ControllerToHostPacket<'a>>, Error> {
        match self.recv(rx, K_NO_WAIT) {
            Err(Error::Zephyr(code)) if code == raw::EAGAIN => Ok(None),
            result => result.map(Some),
        }
    }

    fn recv<'a>(
        &self,
        rx: &'a mut [u8],
        timeout: raw::k_timeout_t,
    ) -> Result<ControllerToHostPacket<'a>, Error> {
        // SAFETY: enable() initializes the FIFO before this transport is
        // exposed. The returned net_buf is released below.
        let buffer =
            unsafe { raw::zr_hci_raw_fifo_get(RAW_FIFO.get(), timeout) as *mut raw::net_buf };
        if buffer.is_null() {
            return Err(Error::Zephyr(raw::EAGAIN));
        }

        // SAFETY: Zephyr provides a valid net_buf with `len` bytes at `data`.
        let data = unsafe {
            let simple = (*buffer).__bindgen_anon_2.b.as_ref();
            core::slice::from_raw_parts(simple.data, usize::from(simple.len))
        };
        if data.len() > rx.len() {
            unsafe { raw::net_buf_unref(buffer) };
            return Err(Error::BufferTooSmall);
        }

        rx[..data.len()].copy_from_slice(data);
        unsafe { raw::net_buf_unref(buffer) };
        ControllerToHostPacket::from_hci_bytes_complete(&rx[..data.len()]).map_err(Error::Decode)
    }
}

impl ErrorType for HciRawTransport {
    type Error = Error;
}

impl Transport for HciRawTransport {
    fn read<'a>(
        &self,
        rx: &'a mut [u8],
    ) -> Result<ControllerToHostPacket<'a>, TryError<Self::Error>> {
        self.recv(rx, K_FOREVER).map_err(TryError::Error)
    }

    fn write<T: HostToControllerPacket>(&self, value: &T) -> Result<(), TryError<Self::Error>> {
        let kind = value_kind(T::KIND)?;
        if value.size() > MAX_PACKET_SIZE - 1 {
            return Err(TryError::Error(Error::BufferTooSmall));
        }

        let mut payload = [0u8; MAX_PACKET_SIZE - 1];
        let mut writer = SliceWriter::new(&mut payload);
        value.write_hci(&mut writer).map_err(TryError::Error)?;
        let payload_len = writer.len();
        drop(writer);

        // SAFETY: the HCI RAW subsystem owns the configured TX buffer pools.
        // bt_send() consumes the buffer on success; the error path frees it.
        let buffer = unsafe {
            raw::bt_buf_get_tx(
                kind,
                K_NO_WAIT,
                payload.as_ptr() as *const c_void,
                payload_len,
            )
        };
        if buffer.is_null() {
            return Err(TryError::Error(Error::Zephyr(raw::ENOMEM)));
        }

        let result = unsafe { raw::bt_send(buffer) };
        if result < 0 {
            unsafe { raw::net_buf_unref(buffer) };
            return Err(TryError::Error(Error::Zephyr((-result) as u32)));
        }

        Ok(())
    }
}

fn value_kind(kind: PacketKind) -> Result<raw::bt_buf_type, TryError<Error>> {
    match kind {
        PacketKind::Cmd => Ok(raw::bt_buf_type_BT_BUF_CMD),
        PacketKind::AclData => Ok(raw::bt_buf_type_BT_BUF_ACL_OUT),
        unsupported => Err(TryError::Error(Error::UnsupportedPacket(unsupported))),
    }
}

struct SliceWriter<'a> {
    buffer: &'a mut [u8],
    len: usize,
}

impl<'a> SliceWriter<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, len: 0 }
    }

    fn len(&self) -> usize {
        self.len
    }
}

impl ErrorType for SliceWriter<'_> {
    type Error = Error;
}

impl embedded_io::Write for SliceWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error> {
        let remaining = self.buffer.len().saturating_sub(self.len);
        if bytes.len() > remaining {
            return Err(Error::BufferTooSmall);
        }
        self.buffer[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
