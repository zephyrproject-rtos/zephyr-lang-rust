//! A simple TLS helping tool.
//!
//! Until this crate implements general TLS support, similar to std, this simpletls module can
//! provide a simplified type of thread-local storage.

extern crate alloc;

use core::{ptr, sync::atomic::Ordering};

use alloc::boxed::Box;
use alloc::vec::Vec;
use zephyr_sys::{k_current_get, k_thread};

use crate::sync::{atomic::AtomicPtr, Mutex};

/// A container for simple thread local storage.
///
/// This will maintain a mapping between Zephyr threads and a value of type T.  Entries will have to
/// be added manually, generally when each thread is started.
///
/// Note that T must implement Copy, as it is not safe to retain references to the inner data
/// outside of this api.
///
/// T must also implement Send, since although 'get' always retrieves the current thread's data,
/// `insert` will typically need to move `T` across threads.
pub struct SimpleTls<T: Copy + Send> {
    map: Vec<(usize, T)>,
}

impl<T: Copy + Send> SimpleTls<T> {
    /// Create a new SimpleTls.
    pub const fn new() -> Self {
        Self { map: Vec::new() }
    }

    /// Insert a new association into the SimpleTls.
    ///
    /// If this thread has already been added, the value will be replaced.
    pub fn insert(&mut self, thread: *const k_thread, data: T) {
        let thread = thread as usize;

        match self.map.binary_search_by(|(id, _)| id.cmp(&thread)) {
            Ok(pos) => self.map[pos] = (thread, data), // Replace existing.
            Err(pos) => self.map.insert(pos, (thread, data)),
        }
    }

    /// Lookup the data associated with a given thread.
    pub fn get(&self) -> Option<T> {
        let thread = unsafe { k_current_get() } as usize;

        self.map
            .binary_search_by(|(id, _)| id.cmp(&thread))
            .ok()
            .map(|pos| self.map[pos].1)
    }
}

/// A helper to safely use these with static.
///
/// The StaticTls type has a constant constructor, and the same insert and get methods as the
/// underlying SimpleTls, with support for initializing the Mutex as needed.
// TODO: This should eventually make it into a more general lazy mechanism.
pub struct StaticTls<T: Copy + Send> {
    /// The container for the data.
    ///
    /// The AtomicPtr is either Null, or contains a raw pointer to the underlying Mutex holding the
    /// data.
    data: AtomicPtr<Mutex<SimpleTls<T>>>,
}

impl<T: Copy + Send> StaticTls<T> {
    /// Create a new StaticTls that is empty.
    pub const fn new() -> Self {
        Self {
            data: AtomicPtr::new(ptr::null_mut()),
        }
    } 

    /// Get the underlying Mutex out of the data, initializing it with an empty type if necessary.
    fn get_inner(&self) -> &Mutex<SimpleTls<T>> {
        let data = self.data.fetch_update(
            // TODO: These orderings are likely stronger than necessary.
            Ordering::SeqCst,
            Ordering::SeqCst,
            |ptr| {
                if ptr.is_null() {
                    // For null, we need to allocate a new one.
                    let data = Box::new(Mutex::new(SimpleTls::new()));
                    Some(Box::into_raw(data))
                } else {
                    // If there was already a value, just use it.
                    None
                }
            });
        let data = match data {
            Ok(_) => {
                // If the update stored something, it unhelpfully returns the old value, which was
                // the null pointer.  Since the pointer will only ever be updated once, it is safe
                // to use a relaxed load here.
                self.data.load(Ordering::Relaxed)
            }
            // If there was already a pointer, that is what we want.
            Err(ptr) => ptr,
        };

        // SAFETY: The stored data was updated at most once, by the above code, and we now have a
        // pointer to a valid leaked box holding the data.
        unsafe { &*data }
    }

    /// Insert a new association into the StaticTls.
    pub fn insert(&self, thread: *const k_thread, data: T) {
        let inner = self.get_inner();
        inner.lock().unwrap().insert(thread, data);
    }

    /// Lookup the data associated with a given thread.
    pub fn get(&self) -> Option<T> {
        let inner = self.get_inner();
        inner.lock().unwrap().get()
    }
}
