//! A simple TLS helping tool.
//!
//! Until this crate implements general TLS support, similar to std, this simpletls module can
//! provide a simplified type of thread-local storage.

extern crate alloc;

use alloc::vec::Vec;
use zephyr_sys::{k_current_get, k_thread};

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
