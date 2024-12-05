//! Lightweight wrapper around Zephyr's `k_queue`.
//!
//! The underlying operations on the `k_queue` are all unsafe, as the model does not match the
//! borrowing model that Rust expects.  This module is mainly intended to be used by the
//! implementation of `zephyr::sys::channel`, which can be used without needing unsafe.

use core::ffi::c_void;
use core::fmt;
#[cfg(CONFIG_RUST_ALLOC)]
use core::mem;

use zephyr_sys::{
    k_queue,
    k_queue_init,
    k_queue_append,
    k_queue_get,
};

#[cfg(CONFIG_RUST_ALLOC)]
use crate::error::Result;
use crate::object::{Fixed, StaticKernelObject, Wrapped};
use crate::time::Timeout;

/// A wrapper around a Zephyr `k_queue` object.
pub struct Queue {
    item: Fixed<k_queue>,
}

unsafe impl Sync for StaticKernelObject<k_queue> { }

unsafe impl Sync for Queue { }
unsafe impl Send for Queue { }

impl Queue {
    /// Create a new Queue, dynamically allocated.
    ///
    /// This Queue can only be used from system threads.
    ///
    /// **Note**: When a Queue is dropped, any messages that have been added to the queue will be
    /// leaked.
    #[cfg(CONFIG_RUST_ALLOC)]
    pub fn new() -> Result<Queue> {
        let item: Fixed<k_queue> = Fixed::new(unsafe { mem::zeroed() });
        unsafe {
            k_queue_init(item.get());
        }
        Ok(Queue { item })
    }

    /// Append an element to the end of a queue.
    ///
    /// This adds an element to the given [`Queue`].  Zephyr requires the
    /// first word of this message to be available for the OS to enqueue
    /// the message.  See [`Message`] for details on how this can be used
    /// safely.
    ///
    /// [`Message`]: crate::sync::channel::Message
    pub unsafe fn send(&self, data: *mut c_void) {
        k_queue_append(self.item.get(), data)
    }

    /// Get an element from a queue.
    ///
    /// This routine removes the first data item from the [`Queue`].
    /// The timeout value can be [`Forever`] to block until there is a message, [`NoWait`] to check
    /// and immediately return if there is no message, or a [`Duration`] to indicate a specific
    /// timeout.
    pub unsafe fn recv<T>(&self, timeout: T) -> *mut c_void
        where T: Into<Timeout>
    {
        let timeout: Timeout = timeout.into();
        k_queue_get(self.item.get(), timeout.0)
    }
}

impl Wrapped for StaticKernelObject<k_queue> {
    type T = Queue;

    type I = ();

    fn get_wrapped(&self, _arg: Self::I) -> Queue {
        let ptr = self.value.get();
        unsafe {
            k_queue_init(ptr);
        }
        Queue {
            item: Fixed::Static(ptr),
        }
    }
}

/// A statically defined Zephyr `k_queue`.
///
/// This should be declared as follows:
/// ```
/// kobj_define! {
///     static MY_QUEUE: StaticQueue;
/// }
///
/// let my_queue = MY_QUEUE.init_once(());
///
/// my_queue.send(...);
/// ```
pub type StaticQueue = StaticKernelObject<k_queue>;

impl fmt::Debug for Queue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Queue {:?}", self.item.get())
    }
}
