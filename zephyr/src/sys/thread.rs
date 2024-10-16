//! Zephyr low level threads
//!
//! This is a fairly low level (but still safe) interface to Zephyr threads.  This is intended to
//! work the same way as threads are typically done on Zephyr systems, where the threads and their
//! stacks are statically allocated, a code is called to initialize them.
//!
//! In addition, there are some convenience operations available that require allocation to be
//! available.
//!
//! ## Usage
//!
//! Each thread needs a stack associated with it.  The stack and the thread should be defined as
//! follows:
//! ```
//! kobj_defined! {
//!     static MY_THREAD: StaticThread;
//!     static MY_THREAD_STACK: StaticThreadStack<2048>;
//! }
//! ```
//!
//! Each of these has a [`init_once`] method that returns the single usable instance.  The
//! StaticThread takes the stack retrieved by take as its argument.  This will return a
//! ThreadStarter, where various options can be set on the thread, and then it started with one of
//! `spawn`, or `simple_spawn` (spawn requires `CONFIG_RUST_ALLOC`).
//!
//! Provided that `CONFIG_RUST_ALLOC` has been enabled (recommended): the read can be initialized as
//! follows:
//! ```
//! let mut thread = MY_THREAD.init_once(MY_THREAD_STACK.init_once(()).unwrap()).unwrap();
//! thread.set_priority(5);
//! let child = thread.spawn(|| move {
//!     // thread code...
//! });
//! ```
//!
//! [`init_once`]: StaticKernelObject::init_once

#[cfg(CONFIG_RUST_ALLOC)]
extern crate alloc;

#[cfg(CONFIG_RUST_ALLOC)]
use alloc::boxed::Box;
use core::{cell::UnsafeCell, ffi::{c_int, c_void}, mem};

use zephyr_sys::{
    k_thread,
    k_thread_entry_t,
    k_thread_create,
    z_thread_stack_element,
    ZR_STACK_ALIGN,
    ZR_STACK_RESERVED,
};
use super::K_NO_WAIT;

use crate::{align::AlignAs, object::{StaticKernelObject, Wrapped}, sync::atomic::AtomicUsize};

/// Adjust the stack size for alignment.  Note that, unlike the C code, we don't include the
/// reservation in this, as it has its own fields in the struct.
pub const fn stack_len(size: usize) -> usize {
    size.next_multiple_of(ZR_STACK_ALIGN)
}

#[doc(hidden)]
/// A Zephyr stack declaration.
///
/// It isn't meant to be used directly, as it needs additional decoration about linker sections and
/// such.  Unlike the C declaration, the reservation is a separate field.  As long as the SIZE is
/// properly aligned, this should work without padding between the fields.
pub struct RealStaticThreadStack<const SIZE: usize> {
    #[allow(dead_code)]
    align: AlignAs<ZR_STACK_ALIGN>,
    #[allow(dead_code)]
    pub data: UnsafeCell<[z_thread_stack_element; SIZE]>,
    #[allow(dead_code)]
    extra: [z_thread_stack_element; ZR_STACK_RESERVED],
}

unsafe impl<const SIZE: usize> Sync for RealStaticThreadStack<SIZE> {}

/// The dynamic stack value, which wraps the underlying stack.
///
/// TODO: constructor instead of private.
pub struct ThreadStack {
    /// Private
    pub base: *mut z_thread_stack_element,
    /// Private
    pub size: usize,
}

#[doc(hidden)]
pub struct StaticThreadStack {
    pub base: *mut z_thread_stack_element,
    pub size: usize,
}

unsafe impl Sync for StaticKernelObject<StaticThreadStack> {}

/*
// Let's make sure I can declare some of this.
pub static TEST_STACK: RealStaticThreadStack<1024> = unsafe { ::core::mem::zeroed() };
pub static TEST: StaticKernelObject<StaticThreadStack> = StaticKernelObject {
    value: UnsafeCell::new(StaticThreadStack {
        base: TEST_STACK.data.get() as *mut z_thread_stack_element,
        size: 1024,
    }),
    init: AtomicUsize::new(0),
};

pub fn doit() {
    TEST.init_once(());
}
*/

impl Wrapped for StaticKernelObject<StaticThreadStack> {
    type T = ThreadStack;
    type I = ();
    fn get_wrapped(&self, _args: Self::I) -> Self::T {
        // This is a bit messy.  Whee.
        let stack = self.value.get();
        let stack = unsafe { &*stack };
        ThreadStack {
            base: stack.base,
            size: stack.size,
        }
    }
}

impl StaticKernelObject<StaticThreadStack> {
    /// Construct a StaticThreadStack object.
    ///
    /// This is not intended to be directly called, but is used by the [`kobj_define`] macro.
    #[doc(hidden)]
    pub const fn new_from<const SZ: usize>(real: &RealStaticThreadStack<SZ>) -> Self {
        Self {
            value: UnsafeCell::new(StaticThreadStack {
                base: real.data.get() as *mut z_thread_stack_element,
                size: SZ,
            }),
            init: AtomicUsize::new(0),
        }
    }

    /// Construct an array of StaticThreadStack kernel objects, based on the same sized array of the
    /// RealStaticThreadStack objects.
    ///
    /// This is not intended to be directly called, but is used by the [`kobj_define`] macro.
    #[doc(hidden)]
    pub const fn new_from_array<const SZ: usize, const N: usize>(
        real: &[RealStaticThreadStack<SZ>; N],
    ) -> [Self; N] {
        // Rustc currently doesn't support iterators in constant functions, but it does allow simple
        // looping.  Since the value is not Copy, we need to use the MaybeUninit with a bit of
        // unsafe.  This initialization is safe, as we loop through all of the entries, giving them
        // a value.
        //
        // In addition, MaybeUninit::uninit_array is not stable, so do this the old unsafe way.
        // let mut res: [MaybeUninit<Self>; N] = MaybeUninit::uninit_array();
        // Note that `mem::uninitialized` in addition to being deprecated, isn't const.  But, since
        // this is a const computation, zero-filling shouldn't hurt anything.
        let mut res: [Self; N] = unsafe { mem::zeroed() };
        let mut i = 0;
        while i < N {
            res[i] = Self::new_from(&real[i]);
            i += 1;
        }
        res
    }
}

/// A single Zephyr thread.
///
/// This wraps a `k_thread` type within Rust.  This value is returned from
/// [`StaticThread::init_once`] and represents an initialized thread that hasn't been started.
pub struct Thread {
    raw: *mut k_thread,
    stack: ThreadStack,

    /// The initial priority of this thread.
    priority: c_int,
    /// Options given to thread creation.
    options: u32,
}

/// A statically defined thread.
pub type StaticThread = StaticKernelObject<k_thread>;

unsafe impl Sync for StaticThread {}

impl Wrapped for StaticKernelObject<k_thread> {
    type T = Thread;
    type I = ThreadStack;
    fn get_wrapped(&self, stack: Self::I) -> Self::T {
        Thread {
            raw: self.value.get(),
            stack,
            priority: 0,
            options: 0,
        }
    }
}

impl Thread {
    /// Set the priority the thread will be created at.
    pub fn set_priority(&mut self, priority: c_int) {
        self.priority = priority;
    }

    /// Set the value of the options passed to thread creation.
    pub fn set_options(&mut self, options: u32) {
        self.options = options;
    }

    /// Simple thread spawn.  This is unsafe because of the raw values being used.  This can be
    /// useful in systems without an allocator defined.
    pub unsafe fn simple_spawn(self,
                               child: k_thread_entry_t,
                               p1: *mut c_void,
                               p2: *mut c_void,
                               p3: *mut c_void)
    {
        k_thread_create(
            self.raw,
            self.stack.base,
            self.stack.size,
            child,
            p1,
            p2,
            p3,
            self.priority,
            self.options,
            K_NO_WAIT);
    }

    #[cfg(CONFIG_RUST_ALLOC)]
    /// Spawn a thread, with a closure.
    ///
    /// This requires allocation to be able to safely pass the closure to the other thread.
    pub fn spawn<F: FnOnce() + Send + 'static>(&self, child: F) {
        use core::ptr::null_mut;

        let child: closure::Closure = Box::new(child);
        let child = Box::into_raw(Box::new(closure::ThreadData {
            closure: child,
        }));
        unsafe {
            k_thread_create(
                self.raw,
                self.stack.base,
                self.stack.size,
                Some(closure::child),
                child as *mut c_void,
                null_mut(),
                null_mut(),
                self.priority,
                self.options,
                K_NO_WAIT);
        }
    }
}

/*
use zephyr_sys::{
    k_thread, k_thread_create, k_thread_start, z_thread_stack_element, ZR_STACK_ALIGN, ZR_STACK_RESERVED
};

use core::{cell::UnsafeCell, ffi::c_void, ptr::null_mut};

use crate::{align::AlignAs, object::{KobjInit, StaticKernelObject}};

#[cfg(CONFIG_RUST_ALLOC)]
extern crate alloc;
#[cfg(CONFIG_RUST_ALLOC)]
use alloc::boxed::Box;
#[cfg(CONFIG_RUST_ALLOC)]
use core::mem::ManuallyDrop;

use super::K_FOREVER;

/// Adjust the stack size for alignment.  Note that, unlike the C code, we don't include the
/// reservation in this, as it has its own fields in the struct.
pub const fn stack_len(size: usize) -> usize {
    size.next_multiple_of(ZR_STACK_ALIGN)
}

/// A Zephyr stack declaration.  It isn't meant to be used directly, as it needs additional
/// decoration about linker sections and such.  Unlike the C declaration, the reservation is a
/// separate field.  As long as the SIZE is properly aligned, this should work without padding
/// between the fields.
pub struct ThreadStack<const SIZE: usize> {
    #[allow(dead_code)]
    align: AlignAs<ZR_STACK_ALIGN>,
    data: UnsafeCell<[z_thread_stack_element; SIZE]>,
    #[allow(dead_code)]
    extra: [z_thread_stack_element; ZR_STACK_RESERVED],
}

unsafe impl<const SIZE: usize> Sync for ThreadStack<SIZE> {}

impl<const SIZE: usize> ThreadStack<SIZE> {
    /// Get the size of this stack.  This is the size, minus any reservation.  This is called `size`
    /// to avoid any confusion with `len` which might return the actual size of the stack.
    pub fn size(&self) -> usize {
        SIZE
    }

    /// Return the stack base needed as the argument to various zephyr calls.
    pub fn base(&self) -> *mut z_thread_stack_element {
        self.data.get() as *mut z_thread_stack_element
    }

    /// Return the token information for this stack, which is a base and size.
    pub fn token(&self) -> StackToken {
        StackToken { base: self.base(), size: self.size() }
    }
}

/// Declare a variable, of a given name, representing the stack for a thread.
#[macro_export]
macro_rules! kernel_stack_define {
    ($name:ident, $size:expr) => {
        #[link_section = concat!(".noinit.", stringify!($name), ".", file!(), line!())]
        static $name: $crate::sys::thread::ThreadStack<{$crate::sys::thread::stack_len($size)}>
            = unsafe { ::core::mem::zeroed() };
    };
}

/// A single Zephyr thread.
///
/// This wraps a `k_thread` type within Zephyr.  This value is returned
/// from the `StaticThread::spawn` method, to allow control over the start
/// of the thread.  The [`start`] method should be used to start the
/// thread.
///
/// [`start`]: Thread::start
pub struct Thread {
    raw: *mut k_thread,
}

unsafe impl Sync for StaticKernelObject<k_thread> { }

impl KobjInit<k_thread, Thread> for StaticKernelObject<k_thread> {
    fn wrap(ptr: *mut k_thread) -> Thread {
        Thread { raw: ptr }
    }
}

// Public interface to threads.
impl Thread {
    /// Start execution of the given thread.
    pub fn start(&self) {
        unsafe { k_thread_start(self.raw) }
    }
}

/// Declare a global static representing a thread variable.
#[macro_export]
macro_rules! kernel_thread_define {
    ($name:ident) => {
        // Since the static object has an atomic that we assume is initialized, let the compiler put
        // this in the data section it finds appropriate (probably .bss if it is initialized to zero).
        // This only matters when the objects are being checked.
        // TODO: This doesn't seem to work with the config.
        // #[cfg_attr(not(CONFIG_RUST_CHECK_KOBJ_INIT),
        //            link_section = concat!(".noinit.", stringify!($name), ".", file!(), line!()))]
        static $name: $crate::object::StaticKernelObject<$crate::raw::k_thread> =
            $crate::object::StaticKernelObject::new();
        // static $name: $crate::sys::thread::Thread = unsafe { ::core::mem::zeroed() };
    };
}

/// For now, this "token" represents the somewhat internal information about thread.
/// What we really want is to make sure that stacks and threads go together.
pub struct StackToken {
    base: *mut z_thread_stack_element,
    size: usize,
}

// This isn't really safe at all, as these can be initialized.  It is unclear how, if even if it is
// possible to implement safe static threads and other data structures in Zephyr.

/// A Statically defined Zephyr `k_thread` object to be used from Rust.
/// 
/// This should be used in a manner similar to:
/// ```
/// const MY_STACK_SIZE: usize = 4096;
///
/// kobj_define! {
///     static MY_THREAD: StaticThread;
///     static MY_STACK: ThreadStack<MY_STACK_SIZE>;
/// }
///
/// let thread = MY_THREAD.spawn(MY_STACK.token(), move || {
///     // Body of thread.
/// });
/// thread.start();
/// ```
pub type StaticThread = StaticKernelObject<k_thread>;

// The thread itself assumes we've already initialized, so this method is on the wrapper.
impl StaticThread {
    /// Spawn this thread to the given external function.  This is a simplified version that doesn't
    /// take any arguments.  The child runs immediately.
    pub fn simple_spawn(&self, stack: StackToken, child: fn() -> ()) -> Thread {
        self.init_help(|raw| {
            unsafe {
                k_thread_create(
                    raw,
                    stack.base,
                    stack.size,
                    Some(simple_child),
                    child as *mut c_void,
                    null_mut(),
                    null_mut(),
                    5,
                    0,
                    K_FOREVER,
                );
            }
        });
        self.get()
    }

    #[cfg(CONFIG_RUST_ALLOC)]
    /// Spawn a thread, running a closure.  The closure will be boxed to give to the new thread.
    /// The new thread runs immediately.
    pub fn spawn<F: FnOnce() + Send + 'static>(&self, stack: StackToken, child: F) -> Thread {
        let child: closure::Closure = Box::new(child);
        let child = Box::into_raw(Box::new(closure::ThreadData {
            closure: ManuallyDrop::new(child),
        }));
        self.init_help(move |raw| {
            unsafe {
                k_thread_create(
                    raw,
                    stack.base,
                    stack.size,
                    Some(closure::child),
                    child as *mut c_void,
                    null_mut(),
                    null_mut(),
                    5,
                    0,
                    K_FOREVER,
                );
            }
        });
        self.get()
    }
}

unsafe extern "C" fn simple_child(
    arg: *mut c_void,
    _p2: *mut c_void,
    _p3: *mut c_void,
) {
    let child: fn() -> () = core::mem::transmute(arg);
    (child)();
}
*/

#[cfg(CONFIG_RUST_ALLOC)]
/// Handle the closure case.  This invokes a double box to rid us of the fat pointer.  I'm not sure
/// this is actually necessary.
mod closure {
    use super::Box;
    use core::ffi::c_void;

    pub type Closure = Box<dyn FnOnce()>;

    pub struct ThreadData {
        pub closure: Closure,
    }

    pub unsafe extern "C" fn child(child: *mut c_void, _p2: *mut c_void, _p3: *mut c_void) {
        let thread_data: Box<ThreadData> = unsafe { Box::from_raw(child as *mut ThreadData) };
        let closure = (*thread_data).closure;
        closure();
    }
}
