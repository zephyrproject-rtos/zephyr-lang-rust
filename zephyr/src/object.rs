//! # Zephyr Kernel Objects
//!
//! Zephyr has a concept of a 'kernel object' that is handled a bit magically.  In kernel mode
//! threads, these are just pointers to the data structures that Zephyr uses to manage that item.
//! In userspace, they are still pointers, but those data structures aren't accessible to the
//! thread.  When making syscalls, the kernel validates that the objects are both valid kernel
//! objects and that the are supposed to be accessible to this thread.
//!
//! In many Zephyr apps, the kernel objects in the app are defined as static, using special macros.
//! These macros make sure that the objects get registered so that they are accessible to userspace
//! (at least after that access is granted).
//!
//! There are also kernel objects that are synthesized as part of the build.  Most notably, there
//! are ones generated by the device tree.
//!
//! ## Safety
//!
//! Zephyr has traditionally not focused on safety.  Early versions of project goals, in fact,
//! emphasized performance and small code size as priorities over runtime checking of safety.  Over
//! the years, this focus has changed a bit, and Zephyr does contain some additional checking, some
//! of which is optional.
//!
//! Zephyr is still constrained at compile time to checks that can be performed with the limits
//! of the C language.  With Rust, we have a much greater ability to enforce many aspects of safety
//! at compile time.  However, there is some complexity to doing this at the interface between the C
//! world and Rust.
//!
//! There are two types of kernel objects we deal with.  There are kernel objects that are allocated
//! by C code (often auto-generated) that should be accessible to Rust.  These are mostly `struct
//! device` values, and will be handled in a devices module.  The other type are objects that
//! application code wishes to declare statically, and use from Rust code.  That is the
//! responsibility of this module.  (There will also be support for more dynamic management of
//! kernel objects, but this will be handled later).
//!
//! Static kernel objects in Zephyr are declared as C top-level variables (where the keyword static
//! means something different).  It is the responsibility of the calling code to initialize these
//! items, make sure they are only initialized once, and to ensure that sharing of the object is
//! handled properly.  All of these are concerns we can handle in Rust.
//!
//! To handle initialization, we pair each kernel object with a single atomic value, whose zero
//! value indicates [`KOBJ_UNINITIALIZED`].  There are a few instances of values that can be placed
//! into uninitialized memory in a C declaration that will need to be zero initialized as a Rust
//! static.  The case of thread stacks is handled as a special case, where the initialization
//! tracking is kept separate so that the stack can still be placed in initialized memory.
//!
//! This state goes through two more values as the item is initialized, one indicating the
//! initialization is happening, and another indicating it has finished.
//!
//! For each kernel object, there will be two types.  One, having a name of the form StaticThing,
//! and the other having the form Thing.  The StaticThing will be used in a static declaration.
//! There is a [`kobj_define!`] macro that matches declarations of these values and adds the
//! necessary linker declarations to place these in the correct linker sections.  This is the
//! equivalent of the set of macros in C, such as `K_SEM_DEFINE`.
//!
//! This StaticThing will have a single method [`init_once`] which accepts a single argument of a
//! type defined by the object.  For most objects, it will just be an empty tuple `()`, but it can
//! be whatever initializer is needed for that type by Zephyr.  Semaphores, for example, take the
//! initial value and the limit.  Threads take as an initializer the stack to be used.
//!
//! This `init_once` will initialize the Zephyr object and return the `Thing` item that will have
//! the methods on it to use the object.  Attributes such as `Sync`, and `Clone` will be defined
//! appropriately so as to match the semantics of the underlying Zephyr kernel object.  Generally
//! this `Thing` type will simply be a container for a direct pointer, and thus using and storing
//! these will have the same characteristics as it would from C.
//!
//! Rust has numerous strict rules about mutable references, namely that it is not safe to have more
//! than one mutable reference.  The language does allow multiple `*mut ktype` references, and their
//! safety depends on the semantics of what is pointed to.  In the case of Zephyr, some of these are
//! intentionally thread safe (for example, things like `k_sem` which have the purpose of
//! synchronizing between threads).  Others are not, and that is mirrored in Rust by whether or not
//! `Clone` and/or `Sync` are implemented.  Please see the documentation of individual entities for
//! details for that object.
//!
//! In general, methods on `Thing` will require `&mut self` if there is any state to manage.  Those
//! that are built around synchronization primitives, however, will generally use `&self`.  In
//! general, objects that implement `Clone` will use `&self` because there would be no benefit to
//! mutable self when the object could be cloned.
//!
//! [`kobj_define!`]: crate::kobj_define
//! [`init_once`]: StaticKernelObject::init_once

#[cfg(CONFIG_RUST_ALLOC)]
extern crate alloc;

use core::{cell::UnsafeCell, mem};

#[cfg(CONFIG_RUST_ALLOC)]
use core::pin::Pin;

#[cfg(CONFIG_RUST_ALLOC)]
use alloc::boxed::Box;

use crate::sync::atomic::{AtomicUsize, Ordering};

/// ## Init/move safe objects
///
/// In Rust code, many language features are designed around Rust's "move semantics".  Because of
/// the borrow checker, the Rust compiler has full knowledge of when it is safe to move an object in
/// memory, as it will know that there are no references to it.
///
/// However, most kernel objects in Zephyr contain self-referential pointers to those objects.  The
/// traditional way to handle this in Rust is to use `Pin`.  However, besides Pin being awkward to
/// use, it is generally assumed that the underlying objects will be dynamically allocated.  It is
/// desirable to allow as much functionality of Zephyr to be used without explicitly requiring
/// alloc.
///
/// The original solution (Wrapped), used a type `Fixed` that either referenced a static, or
/// contained a `Pin<Box<T>>` of the Zephyr kernel object.  This introduces overhead for both the
/// enum as well as the actual reference itself.
///
/// Some simple benchmarking has determined that it is just as efficient, or even slightly more so,
/// to represent each object instead as a `UnsafeCell` contaning the Zephyr object, and an atomic
/// pointer that can be used to determine the state of the object.
///
/// This type is not intended for general use, but for the implementation of wrappers for Zephyr
/// types that require non-move semantics.
///
/// # Safety
///
/// The Zephyr APIs require that once objects have been initialized, they are not moved in memory.
/// To avoid the inconvenience of managing 'Pin' for most of these, we rely on a run-time detection
/// both of initialization and non-movement.  It is fairly easy, as a user of an object in Rust to
/// avoid moving it.  Generally, in an embedded system, objects will either live on the stack of a
/// single persistent thread, or will be statically allocated.  Both of these cases will result in
/// objects that don't move.  However, we want initialization to be able to happen on first _use_
/// not when the constructor runs, because the semantics of most constructors invovles a move (even
/// if that is often optimized away).
///
/// Note that this does not solve the case of objects that must not be moved even after the object
/// has a single Rust reference (threads, and work queues, notably, or timers with active
/// callbacks).
///
/// To do this, each object is paired with an Atomic pointer.  The pointer can exist in three state:
/// - null: The object has not been initialized.  It is safe to move the object at this time.
/// - pointer that equals the addres of the object itself.  Object has been initialized, and can be
///   used.  It must not be moved.
/// - pointer that doesn't match the object.  This indicates that the object was moved, and is
///   invalid.  We treat this as a panic condition.
pub struct ZephyrObject<T> {
    state: AtomicUsize,
    object: UnsafeCell<T>,
}

impl<T> ZephyrObject<T>
where
    ZephyrObject<T>: ObjectInit<T>,
{
    /// Construct a new Zephyr Object.
    ///
    /// The 'init' function will be given a reference to the object.  For objects that have
    /// initialization parameters (specifically Semaphores), this can be used to hold those
    /// parameters until the real init is called upon first use.
    ///
    /// The 'setup' function must not assume the address given to it will persist.  The object can
    /// be freely moved by Rust until the 'init' call has been called, which happens on first use.
    pub const fn new_raw() -> Self {
        Self {
            state: AtomicUsize::new(0),
            // SAFETY: It is safe to assume Zephyr objects can be zero initialized before calling
            // their init.  The atomic above will ensure that this is not used by any API other than
            // the init call until it has been initialized.
            object: UnsafeCell::new(unsafe { mem::zeroed() }),
        }
    }

    /// Get a reference, _without initializing_ the item.
    ///
    /// This is useful during a const constructor to be able to stash values in the item.
    pub const fn get_uninit(&self) -> *mut T {
        self.object.get()
    }

    /// Get a reference to the underlying zephyr object, ensuring it has been initialized properly.
    /// The method is unsafe, because the caller must ensure that the lifetime of `&self` is long
    /// enough for the use of the raw pointer.
    ///
    /// # Safety
    ///
    /// If the object has not been initialized, It's 'init' method will be called.  If the object
    /// has been moved since `init` was called, this will panic.  Otherwise, the caller must ensure
    /// that the use of the returned pointer does not outlive the `&self`.
    ///
    /// The 'init' method will be called within a critical section, so should be careful to not
    /// block, or take extra time.
    pub unsafe fn get(&self) -> *mut T {
        let addr = self.object.get();

        // First, try reading the atomic relaxed.  This makes the common case of the object being
        // initialized faster, and we can double check after.
        match self.state.load(Ordering::Relaxed) {
            // Uninitialized.  Falls through to the slower init case.
            0 => (),
            // Initialized, and object has not been moved.
            ptr if ptr == addr as usize => return addr,
            _ => {
                // Object was moved after init.
                panic!("Object moved after init");
            }
        }

        // Perform the possible initialization within a critical section to avoid a race and double
        // initialization.
        critical_section::with(|_| {
            // Reload, with Acquire ordering to see a determined value.
            let state = self.state.load(Ordering::Acquire);

            // If the address does match, an initialization got in before the critical section.
            if state == addr as usize {
                // Note, this returns from the closure, not the function, but this is ok, as the
                // critical section result is the return result of the whole function.
                return addr;
            } else if state != 0 {
                // Initialization got in, and then it was moved.  This shouldn't happen without
                // unsafe code, but it is easy to detect.
                panic!("Object moved after init");
            }

            // Perform the initialization.
            <Self as ObjectInit<T>>::init(addr);

            self.state.store(addr as usize, Ordering::Release);

            addr
        })
    }
}

/// All `ZephyrObject`s must implement `ObjectInit` in order for first use to be able to initialize
/// the object.
pub trait ObjectInit<T> {
    /// Initialize the object.
    ///
    /// This is called upon first use.  The address given may (and generally will) be different than
    /// the initial address given to the `setup` call in the [`ZephyrObject::new_raw`] constructor.
    /// After this is called, all subsequent calls to [`ZephyrObject::get`] will return the same
    /// address, or panic.
    fn init(item: *mut T);
}

// The kernel object itself must be wrapped in `UnsafeCell` in Rust.  This does several thing, but
// the primary feature that we want to declare to the Rust compiler is that this item has "interior
// mutability".  One impact will be that the default linker section will be writable, even though
// the object will not be declared as mutable.  It also affects the compiler as it will avoid things
// like aliasing and such on the data, as it will know that it is potentially mutable.  In our case,
// the mutations happen from C code, so this is less important than the data being placed in the
// proper section.  Many will have the link section overridden by the `kobj_define` macro.

/// ## Old Wrapped objects
///
/// The wrapped objects was the original approach to managing Zephyr objects.
///
/// Define the Wrapping of a kernel object.
///
/// This trait defines the association between a static kernel object and the two associated Rust
/// types: `StaticThing` and `Thing`.  In the general case: there should be:
/// ```
/// impl Wrapped for StaticKernelObject<kobj> {
///     type T = Thing,
///     type I = (),
///     fn get_wrapped(&self, args: Self::I) -> Self::T {
///         let ptr = self.value.get();
///         // Initizlie the kobj using ptr and possible the args.
///         Thing { ptr }
///     }
/// }
/// ```
pub trait Wrapped {
    /// The wrapped type.  This is what `init_once()` on the StaticKernelObject will return after
    /// initialization.
    type T;

    /// The wrapped type also requires zero or more initializers.  Which are represented by this
    /// type.
    type I;

    /// Initialize this kernel object, and return the wrapped pointer.
    fn get_wrapped(&self, args: Self::I) -> Self::T;
}

/// A state indicating an uninitialized kernel object.
///
/// This must be zero, as kernel objects will
/// be represetned as zero-initialized memory.
pub const KOBJ_UNINITIALIZED: usize = 0;

/// A state indicating a kernel object that is being initialized.
pub const KOBJ_INITING: usize = 1;

/// A state indicating a kernel object that has completed initialization.  This also means that the
/// take has been called.  And shouldn't be allowed additional times.
pub const KOBJ_INITIALIZED: usize = 2;

/// A kernel object represented statically in Rust code.
///
/// These should not be declared directly by the user, as they generally need linker decorations to
/// be properly registered in Zephyr as kernel objects.  The object has the underlying Zephyr type
/// T, and the wrapper type W.
///
/// Kernel objects will have their `StaticThing` implemented as `StaticKernelObject<kobj>` where
/// `kobj` is the type of the underlying Zephyr object.  `Thing` will usually be a struct with a
/// single field, which is a `*mut kobj`.
///
/// TODO: Can we avoid the public fields with a const new method?
///
/// TODO: Handling const-defined alignment for these.
pub struct StaticKernelObject<T> {
    #[allow(dead_code)]
    /// The underlying zephyr kernel object.
    pub value: UnsafeCell<T>,
    /// Initialization status of this object.  Most objects will start uninitialized and be
    /// initialized manually.
    pub init: AtomicUsize,
}

impl<T> StaticKernelObject<T>
where
    StaticKernelObject<T>: Wrapped,
{
    /// Construct an empty of these objects, with the zephyr data zero-filled.
    ///
    /// # Safety
    ///
    /// This is safe in the sense that Zephyr we track the initialization, they start in the
    /// uninitialized state, and the zero value of the initialize atomic indicates that it is
    /// uninitialized.
    pub const unsafe fn new() -> StaticKernelObject<T> {
        StaticKernelObject {
            value: unsafe { mem::zeroed() },
            init: AtomicUsize::new(KOBJ_UNINITIALIZED),
        }
    }

    /// Get the instance of the kernel object.
    ///
    /// Will return a single wrapped instance of this object.  This will invoke the initialization,
    /// and return `Some<Wrapped>` for the wrapped containment type.
    ///
    /// If it is called an additional time, it will return None.
    pub fn init_once(&self, args: <Self as Wrapped>::I) -> Option<<Self as Wrapped>::T> {
        if self
            .init
            .compare_exchange(
                KOBJ_UNINITIALIZED,
                KOBJ_INITING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return None;
        }
        let result = self.get_wrapped(args);
        self.init.store(KOBJ_INITIALIZED, Ordering::Release);
        Some(result)
    }
}

/// Objects that can be fixed or allocated.
///
/// When using Rust threads from userspace, the `kobj_define` declarations and the complexity behind
/// it is required.  If all Rust use of kernel objects is from system threads, and dynamic memory is
/// available, kernel objects can be freeallocated, as long as the allocations themselves are
/// pinned.  This `Fixed` encapsulates both of these.
pub enum Fixed<T> {
    /// Objects that have been statically declared and just pointed to.
    Static(*mut T),
    /// Objects that are owned by the wrapper, and contained here.
    #[cfg(CONFIG_RUST_ALLOC)]
    Owned(Pin<Box<UnsafeCell<T>>>),
}

impl<T> Fixed<T> {
    /// Get the raw pointer out of the fixed object.
    ///
    /// Returns the `*mut T` pointer held by this object.  It is either just the static pointer, or
    /// the pointer outside of the unsafe cell holding the dynamic kernel object.
    pub fn get(&self) -> *mut T {
        match self {
            Fixed::Static(ptr) => *ptr,
            #[cfg(CONFIG_RUST_ALLOC)]
            Fixed::Owned(item) => item.get(),
        }
    }

    /// Construct a new fixed from an allocation.  Note that the object will not be fixed in memory,
    /// until _after_ this returns, and it should not be initialized until then.
    #[cfg(CONFIG_RUST_ALLOC)]
    pub fn new(item: T) -> Fixed<T> {
        Fixed::Owned(Box::pin(UnsafeCell::new(item)))
    }
}

/// Declare a static kernel object.  This helps declaring static values of Zephyr objects.
///
/// This can typically be used as:
/// ```
/// kobj_define! {
///     static A_MUTEX: StaticMutex;
///     static MUTEX_ARRAY: [StaticMutex; 4];
/// }
/// ```
#[macro_export]
macro_rules! kobj_define {
    ($v:vis static $name:ident: $type:tt; $($rest:tt)*) => {
        $crate::_kobj_rule!($v, $name, $type);
        $crate::kobj_define!($($rest)*);
    };
    ($v:vis static $name:ident: $type:tt<$size:ident>; $($rest:tt)*) => {
        $crate::_kobj_rule!($v, $name, $type<$size>);
        $crate::kobj_define!($($rest)*);
    };
    ($v:vis static $name:ident: $type:tt<$size:literal>; $($rest:tt)*) => {
        $crate::_kobj_rule!($v, $name, $type<$size>);
        $crate::kobj_define!($($rest)*);
    };
    ($v:vis static $name:ident: $type:tt<{$size:expr}>; $($rest:tt)*) => {
        $crate::_kobj_rule!($v, $name, $type<{$size}>);
        $crate::kobj_define!($($rest)*);
    };
    () => {};
}

#[doc(hidden)]
#[macro_export]
macro_rules! _kobj_rule {
    // static NAME: StaticSemaphore;
    ($v:vis, $name:ident, StaticSemaphore) => {
        #[link_section = concat!("._k_sem.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: $crate::sys::sync::StaticSemaphore =
            unsafe { ::core::mem::zeroed() };
    };

    // static NAMES: [StaticSemaphore; COUNT];
    ($v:vis, $name:ident, [StaticSemaphore; $size:expr]) => {
        #[link_section = concat!("._k_sem.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: [$crate::sys::sync::StaticSemaphore; $size] =
            unsafe { ::core::mem::zeroed() };
    };

    // static NAME: StaticMutex
    ($v:vis, $name:ident, StaticMutex) => {
        #[link_section = concat!("._k_mutex.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: $crate::sys::sync::StaticMutex =
            unsafe { $crate::sys::sync::StaticMutex::new() };
    };

    // static NAMES: [StaticMutex; COUNT];
    ($v:vis, $name:ident, [StaticMutex; $size:expr]) => {
        #[link_section = concat!("._k_mutex.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: [$crate::sys::sync::StaticMutex; $size] =
            // This isn't Copy, intentionally, so initialize the whole thing with zerored memory.
            // Relying on the atomic to be 0 for the uninitialized state.
            // [$crate::sys::sync::StaticMutex::new(); $size];
            unsafe { ::core::mem::zeroed() };
    };

    // static NAME: StaticCondvar;
    ($v:vis, $name:ident, StaticCondvar) => {
        #[link_section = concat!("._k_condvar.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: $crate::sys::sync::StaticCondvar =
            unsafe { $crate::sys::sync::StaticCondvar::new() };
    };

    // static NAMES: [StaticCondvar; COUNT];
    ($v:vis, $name:ident, [StaticCondvar; $size:expr]) => {
        #[link_section = concat!("._k_condvar.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: [$crate::sys::sync::StaticCondvar; $size] =
            // This isn't Copy, intentionally, so initialize the whole thing with zerored memory.
            // Relying on the atomic to be 0 for the uninitialized state.
            // [$crate::sys::sync::StaticMutex::new(); $size];
            unsafe { ::core::mem::zeroed() };
    };

    // static THREAD: staticThread;
    ($v:vis, $name:ident, StaticThread) => {
        // Since the static object has an atomic that we assume is initialized, we cannot use the
        // default linker section Zephyr uses for Thread, as that is uninitialized.  This will put
        // it in .bss, where it is zero initialized.
        $v static $name: $crate::sys::thread::StaticThread =
            unsafe { ::core::mem::zeroed() };
    };

    // static THREAD: [staticThread; COUNT];
    ($v:vis, $name:ident, [StaticThread; $size:expr]) => {
        // Since the static object has an atomic that we assume is initialized, we cannot use the
        // default linker section Zephyr uses for Thread, as that is uninitialized.  This will put
        // it in .bss, where it is zero initialized.
        $v static $name: [$crate::sys::thread::StaticThread; $size] =
            unsafe { ::core::mem::zeroed() };
    };

    // Use indirection on stack initializers to handle some different cases in the Rust syntax.
        ($v:vis, $name:ident, ThreadStack<$size:literal>) => {
        $crate::_kobj_stack!($v, $name, $size);
    };
    ($v:vis, $name:ident, ThreadStack<$size:ident>) => {
        $crate::_kobj_stack!($v, $name, $size);
    };
    ($v:vis, $name:ident, ThreadStack<{$size:expr}>) => {
        $crate::_kobj_stack!($v, $name, $size);
    };

    // Array of stack object versions.
    ($v:vis, $name:ident, [ThreadStack<$size:literal>; $asize:expr]) => {
        $crate::_kobj_stack!($v, $name, $size, $asize);
    };
    ($v:vis, $name:ident, [ThreadStack<$size:ident>; $asize:expr]) => {
        $crate::_kobj_stack!($v, $name, $size, $asize);
    };
    ($v:vis, $name:ident, [ThreadStack<{$size:expr}>; $asize:expr]) => {
        $crate::_kobj_stack!($v, $name, $size, $asize);
    };

    // Queues.
    ($v:vis, $name: ident, StaticQueue) => {
        #[link_section = concat!("._k_queue.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: $crate::sys::queue::StaticQueue =
            unsafe { ::core::mem::zeroed() };
    };

    ($v:vis, $name: ident, [StaticQueue; $size:expr]) => {
        #[link_section = concat!("._k_queue.static.", stringify!($name), ".", file!(), line!())]
        $v static $name: [$crate::sys::queue::StaticQueue; $size] =
            unsafe { ::core::mem::zeroed() };
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _kobj_stack {
    ($v:vis, $name: ident, $size:expr) => {
        $crate::paste! {
            // The actual stack itself goes into the no-init linker section.  We'll use the user_name,
            // with _REAL appended, to indicate the real stack.
            #[link_section = concat!(".noinit.", stringify!($name), ".", file!(), line!())]
            $v static [< $name _REAL >]: $crate::sys::thread::RealStaticThreadStack<{$crate::sys::thread::stack_len($size)}> =
                unsafe { ::core::mem::zeroed() };

            // The proxy object used to ensure initialization is placed in initialized memory.
            $v static $name: $crate::_export::KStaticThreadStack =
                $crate::_export::KStaticThreadStack::new_from(&[< $name _REAL >]);
        }
    };

    // This initializer needs to have the elements of the array initialized to fixed elements of the
    // `RealStaticThreadStack`.  Unfortunately, methods such as [`each_ref`] on the array are not
    // const and can't be used in a static initializer.  We could use a recursive macro definition
    // to perform the initialization, but this would require the array size to only be an integer
    // literal (constants aren't calculated until after macro expansion).  It may also be possible
    // to write a constructor for the array as a const fn, which would greatly simplify the
    // initialization here.
    ($v:vis, $name: ident, $size:expr, $asize:expr) => {
        $crate::paste! {
            // The actual stack itself goes into the no-init linker section.  We'll use the user_name,
            // with _REAL appended, to indicate the real stack.
            #[link_section = concat!(".noinit.", stringify!($name), ".", file!(), line!())]
            $v static [< $name _REAL >]:
                [$crate::sys::thread::RealStaticThreadStack<{$crate::sys::thread::stack_len($size)}>; $asize] =
                unsafe { ::core::mem::zeroed() };

            $v static $name:
                [$crate::_export::KStaticThreadStack; $asize] =
                $crate::_export::KStaticThreadStack::new_from_array(&[< $name _REAL >]);
        }
    };
}
