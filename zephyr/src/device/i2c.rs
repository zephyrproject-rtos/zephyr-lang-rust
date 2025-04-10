//! Zpehyr I2C interface

use core::{ffi::c_int, marker::PhantomData};

use crate::{error::to_result, printkln, raw};

use super::{NoStatic, Unique};

/// A single I2C controller.
pub struct I2C {
    /// The underlying device itself.
    #[allow(dead_code)]
    pub(crate) device: *const raw::device,
}

unsafe impl Send for I2C {}

impl I2C {
    /// Constructor, used by the devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        _data: &'static NoStatic,
        device: *const raw::device,
    ) -> Option<Self> {
        if !unique.once() {
            return None;
        }
        Some(I2C { device })
    }

    /// Do a write/read.
    pub fn write_read(&mut self, write: &[u8], read: &mut [u8]) -> crate::Result<c_int> {
        let mut msg = [
            raw::i2c_msg {
                buf: write.as_ptr() as *mut _,
                len: write.len() as u32,
                flags: raw::ZR_I2C_MSG_WRITE,
            },
            raw::i2c_msg {
                buf: read.as_mut_ptr(),
                len: read.len() as u32,
                flags: raw::ZR_I2C_MSG_READ | raw::ZR_I2C_MSG_STOP,
            },
        ];
        let res = unsafe { to_result(raw::i2c_transfer(self.device, msg.as_mut_ptr(), 2, 0x42)) };

        printkln!("res: {} {}", msg[1].len, msg[1].flags);

        res
    }

    /// Add an i2c operation to the RTIO.
    ///
    /// TODO: Unclear how to indicate that the buffers must live long enough for the submittion.
    /// As it is, this is actually completely unsound.
    pub fn rtio_write_read(&mut self, write: &[u8], read: &mut [u8]) -> crate::Result<()> {
        let _msg = [
            raw::i2c_msg {
                buf: write.as_ptr() as *mut _,
                len: write.len() as u32,
                flags: raw::ZR_I2C_MSG_WRITE,
            },
            raw::i2c_msg {
                buf: read.as_mut_ptr(),
                len: read.len() as u32,
                flags: raw::ZR_I2C_MSG_READ | raw::ZR_I2C_MSG_STOP,
            },
        ];

        todo!()
    }
}

/// An i2c transaction.
pub struct ReadWrite<'a> {
    _phantom: PhantomData<&'a ()>,
    msgs: [raw::i2c_msg; 2],
}

impl<'a> ReadWrite<'a> {
    /// Construct a new read/write transaction.
    pub fn new(write: &'a [u8], read: &'a mut [u8]) -> Self {
        Self {
            _phantom: PhantomData,
            msgs: [
                raw::i2c_msg {
                    buf: write.as_ptr() as *mut _,
                    len: write.len() as u32,
                    flags: raw::ZR_I2C_MSG_WRITE,
                },
                raw::i2c_msg {
                    buf: read.as_mut_ptr(),
                    len: read.len() as u32,
                    flags: raw::ZR_I2C_MSG_READ | raw::ZR_I2C_MSG_STOP,
                },
            ],
        }
    }
}
