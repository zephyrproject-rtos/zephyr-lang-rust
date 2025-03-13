//! Thread support.
//!
//! Implement the friendly Thread types used by the `zephyr::thread` proc macro to declare new
//! threads.
//!
//! This is intended to be completely usable without alloc, while still allow threads to be
//! started with any arbitrary Send arguments.  Threads can be joined, and reused after they have
//! exited.  The model intentionally tries to be similar to how async tasks work in something like
//! Embassy, but some changes due to the different semantics of Zephyr threads.

use core::{
    cell::UnsafeCell,
    ffi::{c_int, c_void, CStr},
    mem,
    ptr::null_mut,
    sync::atomic::Ordering,
};

use portable_atomic::AtomicU8;
use zephyr_sys::{
    k_thread, k_thread_create, k_thread_entry_t, k_thread_join, k_thread_name_set,
    k_thread_priority_set, k_wakeup, z_thread_stack_element, ZR_STACK_ALIGN, ZR_STACK_RESERVED,
};

use crate::{
    align::AlignAs,
    sys::{K_FOREVER, K_NO_WAIT},
};

/// Adjust a given requested stack size up for the alignment.  This is just the stack, and the
/// reservation is explicitly included in the stack declaration below.
pub const fn stack_len(size: usize) -> usize {
    size.next_multiple_of(ZR_STACK_ALIGN)
}

/// States a Zephyr thread can be in.
#[repr(u8)]
pub enum ThreadState {
    /// A non running thread, that is free.
    Init,
    /// An allocated thread.  There is a ThreadHandle for this thread, but it has not been started.
    Allocated,
    /// A thread that is running, as far as we know.  Termination is not checked unless demanded.
    Running,
}

/// The holder of data that is to be shared with the target thread.
///
/// # Safety
///
/// The Option is kept in an UnsafeCell, and it's use governed by an atomic in the `TaskData`
/// below.  When the task is not initialized/not running, this should be set to None.  It will be
/// set to Some in a critical section during startup, where the critical section provides the
/// barrier.  Once the atomic is set to true, the thread owns this data.
///
/// The Send constraint force arguments passed to threads to be Send.
pub struct InitData<T: Send>(pub UnsafeCell<Option<T>>);

impl<T: Send> InitData<T> {
    /// Construct new Shared init state.
    pub const fn new() -> Self {
        Self(UnsafeCell::new(None))
    }
}

unsafe impl<T: Send> Sync for InitData<T> {}

/// The static data associated with each thread.  The stack is kept separate, as it is intended to
/// go into an uninitialized linker section.
pub struct ThreadData<T: Send> {
    init: InitData<T>,
    state: AtomicU8,
    thread: Thread,
}

impl<T: Send> ThreadData<T> {
    /// Construct new ThreadData.
    pub const fn new() -> Self {
        Self {
            init: InitData::new(),
            state: AtomicU8::new(ThreadState::Init as u8),
            thread: unsafe { Thread::new() },
        }
    }

    /// Acquire the thread, in preparation to run it.
    pub fn acquire_old<const SIZE: usize>(
        &'static self,
        args: T,
        stack: &'static ThreadStack<SIZE>,
        entry: k_thread_entry_t,
    ) {
        critical_section::with(|_| {
            // Relaxed is sufficient, as the critical section provides both synchronization and
            // a memory barrier.
            let old = self.state.load(Ordering::Relaxed);
            if old != ThreadState::Init as u8 {
                // TODO: This is where we should check for termination.
                panic!("Attempt to use a thread that is already in use");
            }
            self.state
                .store(ThreadState::Allocated as u8, Ordering::Relaxed);

            let init = self.init.0.get();
            unsafe {
                *init = Some(args);
            }
        });

        // For now, just directly start the thread.  We'll want to delay this so that parameters
        // (priority and/or flags) can be passed, as well as to have a handle to be able to join and
        // restart threads.
        let _tid = unsafe {
            k_thread_create(
                self.thread.0.get(),
                stack.data.get() as *mut z_thread_stack_element,
                stack.size(),
                entry,
                self.init.0.get() as *mut c_void,
                null_mut(),
                null_mut(),
                0,
                0,
                K_NO_WAIT,
            )
        };
    }

    /// Acquire a thread from the pool of threads, panicing if the pool is exhausted.
    pub fn acquire<const SIZE: usize>(
        pool: &'static [Self],
        stacks: &'static [ThreadStack<SIZE>],
        args: T,
        entry: k_thread_entry_t,
        priority: c_int,
        name: &'static CStr,
    ) -> ReadyThread {
        let id = Self::find_thread(pool);

        let init = pool[id].init.0.get();
        unsafe {
            *init = Some(args);
        }

        // Create the thread in Zephyr, in a non-running state.
        let tid = unsafe {
            k_thread_create(
                pool[id].thread.0.get(),
                stacks[id].data.get() as *mut z_thread_stack_element,
                SIZE,
                entry,
                pool[id].init.0.get() as *mut c_void,
                null_mut(),
                null_mut(),
                priority,
                0,
                K_FOREVER,
            )
        };

        // Set the name.
        unsafe {
            k_thread_name_set(tid, name.as_ptr());
        }

        ReadyThread { id: tid }
    }

    /// Scan the pool of threads, looking for an available thread.
    ///
    /// Returns the index of a newly allocated thread.  The thread will be marked 'Allocated' after
    /// this.
    fn find_thread(pool: &'static [Self]) -> usize {
        let id = critical_section::with(|_| {
            for (id, thread) in pool.iter().enumerate() {
                // Relaxed is sufficient, due to the critical section.
                let old = thread.state.load(Ordering::Relaxed);

                match old {
                    v if v == ThreadState::Init as u8 => {
                        // This is available.  Mark as allocated and return from the closure.
                        thread
                            .state
                            .store(ThreadState::Allocated as u8, Ordering::Relaxed);
                        return Some(id);
                    }
                    v if v == ThreadState::Allocated as u8 => {
                        // Allocate threads haven't started, so aren't available.
                    }
                    v if v == ThreadState::Running as u8 => {
                        // A running thread might be available if it has terminated.  We could
                        // improve performance here by not checking these until after the pool has
                        // been checked for Init threads.
                        if unsafe { k_thread_join(thread.thread.0.get(), K_NO_WAIT) } == 0 {
                            thread
                                .state
                                .store(ThreadState::Allocated as u8, Ordering::Relaxed);
                            return Some(id);
                        }
                    }
                    _ => unreachable!(),
                }
            }

            None
        });

        if let Some(id) = id {
            id
        } else {
            panic!("Attempt to use more threads than declared pool size");
        }
    }
}

/// A thread that has been set up and is ready to start.
///
/// Represents a thread that has been created, but not yet started.
pub struct ReadyThread {
    id: *mut k_thread,
}

impl ReadyThread {
    /// Change the priority of this thread before starting it.  The initial default priority was
    /// determined by the declaration of the thread.
    pub fn set_priority(&self, priority: c_int) {
        // SAFETY: ReadyThread should only exist for valid created threads.
        unsafe {
            k_thread_priority_set(self.id, priority);
        }
    }

    /// Start this thread.
    pub fn start(self) -> RunningThread {
        // SAFETY: ReadyThread should only exist for valid created threads.
        unsafe {
            // As per the docs, this should no longer be `k_thread_start`, but `k_wakeup` is fine
            // these days.
            k_wakeup(self.id);
        }

        RunningThread { id: self.id }
    }
}

/// A thread that has been started.
pub struct RunningThread {
    id: *mut k_thread,
}

impl RunningThread {
    /// Wait for this thread to finish executing.
    ///
    /// Will block until the thread has terminated.
    ///
    /// TODO: Allow a timeout?
    /// TODO: Should we try to return a value?
    pub fn join(&self) {
        unsafe {
            // TODO: Can we do something meaningful with the result?
            k_thread_join(self.id, K_FOREVER);

            // TODO: Ideally, we could put the thread state back to avoid the need for another join
            // check when re-allocating the thread.
        }
    }
}

/// A Zephyr stack declaration.
///
/// This isn't intended to be used directly, as it needs additional decoration about linker sections
/// and such.  Unlike the C declaration, the reservation is a separate field.  As long as the SIZE
/// is properly aligned, this should work without padding between the fields.
///
/// Generally, this should be placed in a noinit linker section to avoid having to initialize the
/// memory.
#[repr(C)]
pub struct ThreadStack<const SIZE: usize> {
    /// Align the stack itself according to the Kconfig determined alignment.
    #[allow(dead_code)]
    align: AlignAs<ZR_STACK_ALIGN>,
    /// The data of the stack itself.
    #[allow(dead_code)]
    pub data: UnsafeCell<[z_thread_stack_element; SIZE]>,
    /// Extra data, used by Zephyr.
    #[allow(dead_code)]
    extra: [z_thread_stack_element; ZR_STACK_RESERVED],
}

unsafe impl<const SIZE: usize> Sync for ThreadStack<SIZE> {}

impl<const SIZE: usize> ThreadStack<SIZE> {
    /// Construct a new ThreadStack
    ///
    /// # Safety
    ///
    /// This is unsafe as the memory remains uninitialized, and it is the responsibility of the
    /// caller to use the stack correctly.  The stack should be associated with a single thread.
    pub const fn new() -> Self {
        // SAFETY: Although this is declared as zeroed, the linker section actually used on the
        // stack can be used to place it in no-init memory.
        unsafe { mem::zeroed() }
    }

    /// Retrieve the size of this stack.
    pub const fn size(&self) -> usize {
        SIZE
    }
}

/// A zephyr thread.
///
/// This declares a single k_thread in Zephyr.
pub struct Thread(pub UnsafeCell<k_thread>);

// Threads are "sort of" thread safe.  But, this declaration is needed to be able to declare these
// statically, and all generated versions will protect the thread with a critical section.
unsafe impl Sync for Thread {}

impl Thread {
    /// Static allocation of a thread
    ///
    /// This makes the zero-initialized memory that can later be used as a thread.
    ///
    /// # Safety
    ///
    /// The caller is responsible for using operations such as `create` to construct the thread,
    /// according to the underlying semantics of the Zephyr operations.
    pub const unsafe fn new() -> Self {
        // SAFETY: Zero initialized to match thread declarations in the C macros.
        unsafe { mem::zeroed() }
    }
}
