//! Lightweight wrapper around Zephyr's `k_queue`.
//!
//! The underlying operations on the `k_queue` are all unsafe, as the model does not match the
//! borrowing model that Rust expects.  This module is mainly intended to be used by the
//! implementation of `zephyr::sys::channel`, which can be used without needing unsafe.

use core::ffi::c_void;
use core::fmt;

use zephyr_sys::{k_queue, k_queue_append, k_queue_get, k_queue_init};

use crate::object::{ObjectInit, ZephyrObject};
use crate::time::Timeout;

/// A wrapper around a Zephyr `k_queue` object.
pub struct Queue(pub(crate) ZephyrObject<k_queue>);

unsafe impl Sync for Queue {}
unsafe impl Send for Queue {}

impl Queue {
    /// Create a new Queue, dynamically allocated.
    ///
    /// This Queue can only be used from system threads.
    ///
    /// **Note**: When a Queue is dropped, any messages that have been added to the queue will be
    /// leaked.
    pub const fn new() -> Queue {
        Queue(<ZephyrObject<k_queue>>::new_raw())
    }

    /// Append an element to the end of a queue.
    ///
    /// This adds an element to the given [`Queue`].  Zephyr requires the
    /// first word of this message to be available for the OS to enqueue
    /// the message.  See [`Message`] for details on how this can be used
    /// safely.
    ///
    /// [`Message`]: crate::sync::channel::Message
    ///
    /// # Safety
    ///
    /// Zephyr has specific requirements on the memory given in data, which can be summarized as:
    /// - The memory must remain valid until after the data is returned, via recv.
    /// - The first `usize` in the pointed data will be mutated by Zephyr to manage structures.
    /// - This first field must not be modified by any code while the message is enqueued.
    ///
    /// These are easiest to satisfy by ensuring the message is Boxed, and owned by the queue
    /// system.
    pub unsafe fn send(&self, data: *mut c_void) {
        k_queue_append(self.0.get(), data)
    }

    /// Get an element from a queue.
    ///
    /// This routine removes the first data item from the [`Queue`].
    /// The timeout value can be [`Forever`] to block until there is a message, [`NoWait`] to check
    /// and immediately return if there is no message, or a [`Duration`] to indicate a specific
    /// timeout.
    ///
    /// [`Forever`]: crate::time::Forever
    /// [`NoWait`]: crate::time::NoWait
    /// [`Duration`]: crate::time::Duration
    ///
    /// # Safety
    ///
    /// Once an item is received from a queue, ownership is returned to the caller, and Zephyr no
    /// longer depends on it not being freed, or the first `usize` field being for its use.
    pub unsafe fn recv<T>(&self, timeout: T) -> *mut c_void
    where
        T: Into<Timeout>,
    {
        let timeout: Timeout = timeout.into();
        k_queue_get(self.0.get(), timeout.0)
    }
}

impl ObjectInit<k_queue> for ZephyrObject<k_queue> {
    fn init(item: *mut k_queue) {
        // SAFETY: ZephyrObject handles initialization and move prevention.
        unsafe {
            k_queue_init(item);
        }
    }
}

impl fmt::Debug for Queue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: Just getting the address to print.
        write!(f, "sys::Queue {:?}", unsafe { self.0.get() })
    }
}
