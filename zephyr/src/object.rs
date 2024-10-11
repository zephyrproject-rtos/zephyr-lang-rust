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
//! There are some funny rules about references and mutable references to memory that is
//! inaccessible.  Basically, it is never valid to have a reference to something that isn't
//! accessible.  However, we can have `*mut ktype` or `*const ktype`.  In Rust, having multiple
//! `*mut` pointers live is undefined behavior.  Zephyr makes extensive use of shared mutable
//! pointers (or sometimes immutable).  We will not dereference these in Rust code, but merely pass
//! them to APIs in Zephyr that require them.
//!
//! Most kernel objects use mutable pointers, and it makes sense to require the wrapper structure
//! to be mutable to these accesses.  There a few cases, mostly generated ones that live in
//! read-only memory, notably device instances, that need const pointers.  These will be
//! represented by a separate wrapper.
//!
//! # Initialization tracking
//!
//! The Kconfig `CONFIG_RUST_CHECK_KOBJ_INIT` enabled extra checking in Rust-based kernel objects.
//! This will result in a panic if the objects are used before the underlying object has been
//! initialized.  The initialization must happen through the `StaticKernelObject::init_help`
//! method.
//!
//! TODO: Document how the wrappers work once we figure out how to implement them.

use core::{cell::UnsafeCell, mem};

use crate::sync::atomic::{AtomicUsize, Ordering};

/// A kernel object represented statically in Rust code.
///
/// These should not be declared directly by the user, as they generally need linker decorations to
/// be properly registered in Zephyr as kernel objects.  The object has the underlying Zephyr type
/// T, and the wrapper type W.
///
/// TODO: Handling const-defined alignment for these.
pub struct StaticKernelObject<T> {
    #[allow(dead_code)]
    /// The underlying zephyr kernel object.
    pub(crate) value: UnsafeCell<T>,
    /// Initialization status of this object.  Most objects will start uninitialized and be
    /// initialized manually.
    init: AtomicUsize,
}

/// Each can be wrapped appropriately.  The wrapped type is the instance that holds the raw pointer.
pub trait Wrapped {
    /// The wrapped type.  This is what `take()` on the StaticKernelObject will return after
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

impl<T> StaticKernelObject<T>
where
    StaticKernelObject<T>: Wrapped,
{
    /// Construct an empty of these objects, with the zephyr data zero-filled.  This is safe in the
    /// sense that Zephyr we track the initialization, and they start in the uninitialized state.
    pub const fn new() -> StaticKernelObject<T> {
        StaticKernelObject {
            value: unsafe { mem::zeroed() },
            init: AtomicUsize::new(KOBJ_UNINITIALIZED),
        }
    }

    /// Get the instance of the kernel object.
    ///
    /// Will return a single wrapped instance of this object.  This will invoke the initialization,
    /// and return `Some<Wrapped>` for the wrapped containment type.
    pub fn take(&self, args: <Self as Wrapped>::I) -> Option<<Self as Wrapped>::T> {
        if let Err(_) = self.init.compare_exchange(
            KOBJ_UNINITIALIZED,
            KOBJ_INITING,
            Ordering::AcqRel,
            Ordering::Acquire)
        {
            return None;
        }
        let result = self.get_wrapped(args);
        self.init.store(KOBJ_INITIALIZED, Ordering::Release);
        Some(result)
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
}
