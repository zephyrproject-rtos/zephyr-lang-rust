//! Interface to Zephyr 'rtio' infrastructure.

use core::ffi::c_void;

use crate::error::to_result_void;
use crate::object::{ObjectInit, ZephyrObject};
use crate::raw;

/// The underlying structure, holding the rtio, it's semaphores, and pools.
///
/// Note that putting these together in a single struct makes this "pleasant" to use from Rust, but
/// does make the end result incompatible with userspace.
#[repr(C)]
pub struct RtioData<const SQE_SZ: usize, const CQE_SZ: usize> {
    /// The overall rtio struct.
    rtio: raw::rtio,
    /// Sempahore used for the submission queue.
    #[cfg(CONFIG_RTIO_SUBMIT_SEM)]
    submit_sem: raw::k_sem,
    /// Semaphore used for the consumption queue.
    #[cfg(CONFIG_RTIO_CONSUME_SEM)]
    consume_sem: raw::k_sem,
    /// The SQE items.
    sqe_pool_items: [raw::rtio_iodev_sqe; SQE_SZ],
    /// The SQE pool itself.
    sqe_pool: raw::rtio_sqe_pool,
    /// The CQE items.
    cqe_pool_items: [raw::rtio_cqe; CQE_SZ],
    /// The pool of CQEs.
    cqe_pool: raw::rtio_cqe_pool,
}

/// Init based reference to the the underlying rtio object.
///
/// Note that this declaration will _not_ support userspace currently, as the object will not be
/// placed in an iterable linker section.  Also, the linker sevction will not work as the
/// ZephyrObject will have an attached atomic used to ensure proper initialization.
pub struct RtioObject<const SQE_SZ: usize, const CQE_SZ: usize>(
    pub(crate) ZephyrObject<RtioData<SQE_SZ, CQE_SZ>>,
);

unsafe impl<const SQE_SZ: usize, const CQE_SZ: usize> Sync for RtioObject<SQE_SZ, CQE_SZ> {}

impl<const SQE_SZ: usize, const CQE_SZ: usize> RtioObject<SQE_SZ, CQE_SZ> {
    /// Construct a new RTIO pool.
    ///
    /// Create a new RTIO object.  These objects generally need to be statically allocated.
    pub const fn new() -> Self {
        let this = <ZephyrObject<RtioData<SQE_SZ, CQE_SZ>>>::new_raw();
        RtioObject(this)
    }

    /// Acquire a submission object.
    pub fn sqe_acquire(&'static self) -> Option<Sqe> {
        let this = unsafe { self.0.get() };

        let ptr = unsafe { raw::rtio_sqe_acquire(&raw mut (*this).rtio) };

        if ptr.is_null() {
            None
        } else {
            Some(Sqe { item: ptr })
        }
    }

    /// Submit the work.
    pub fn submit(&'static self, wait: usize) -> crate::Result<()> {
        let this = unsafe { self.0.get() };

        unsafe { to_result_void(raw::rtio_submit(&raw mut (*this).rtio, wait as u32)) }
    }

    /// Consume a single completion.
    ///
    /// Will return the completion if available.  If returned, it will be released upon drop.
    pub fn cqe_consume(&'static self) -> Option<Cqe> {
        let this = unsafe { self.0.get() };

        let ptr = unsafe { raw::rtio_cqe_consume(&raw mut (*this).rtio) };

        if ptr.is_null() {
            None
        } else {
            Some(Cqe {
                item: ptr,
                rtio: unsafe { &raw mut (*this).rtio },
            })
        }
    }
}

impl<const SQE_SZ: usize, const CQE_SZ: usize> ObjectInit<RtioData<SQE_SZ, CQE_SZ>>
    for ZephyrObject<RtioData<SQE_SZ, CQE_SZ>>
{
    fn init(item: *mut RtioData<SQE_SZ, CQE_SZ>) {
        #[cfg(CONFIG_RTIO_SUBMIT_SEM)]
        unsafe {
            raw::k_sem_init(&raw mut (*item).submit_sem, 0, raw::K_SEM_MAX_LIMIT);
            (*item).rtio.submit_sem = &raw mut (*item).submit_sem;
            (*item).rtio.submit_count = 0;
        }
        #[cfg(CONFIG_RTIO_CONSUME_SEM)]
        unsafe {
            raw::k_sem_init(&raw mut (*item).consume_sem, 0, raw::K_SEM_MAX_LIMIT);
            (*item).rtio.consume_sem = &raw mut (*item).consume_sem;
        }
        unsafe {
            // TODO: Zephyr atomic init?
            (*item).rtio.cq_count = 0;
            (*item).rtio.xcqcnt = 0;

            // Set up the sqe pool.
            raw::mpsc_init(&raw mut (*item).sqe_pool.free_q);
            (*item).sqe_pool.pool_size = SQE_SZ as u16;
            (*item).sqe_pool.pool_free = SQE_SZ as u16;
            (*item).sqe_pool.pool = (*item).sqe_pool_items.as_mut_ptr();

            for p in &mut (*item).sqe_pool_items {
                raw::mpsc_push(&raw mut (*item).sqe_pool.free_q, &raw mut p.q);
            }

            // Set up the cqe pool
            raw::mpsc_init(&raw mut (*item).cqe_pool.free_q);
            (*item).cqe_pool.pool_size = CQE_SZ as u16;
            (*item).cqe_pool.pool_free = CQE_SZ as u16;
            (*item).cqe_pool.pool = (*item).cqe_pool_items.as_mut_ptr();

            for p in &mut (*item).cqe_pool_items {
                raw::mpsc_push(&raw mut (*item).cqe_pool.free_q, &raw mut p.q);
            }

            (*item).rtio.sqe_pool = &raw mut (*item).sqe_pool;
            (*item).rtio.cqe_pool = &raw mut (*item).cqe_pool;

            raw::mpsc_init(&raw mut (*item).rtio.sq);
            raw::mpsc_init(&raw mut (*item).rtio.cq);
        }
    }
}

/// A single Sqe.
///
/// TODO: How to bind the lifetime to the Rtio meaningfully, even though it is all static.
pub struct Sqe {
    item: *mut raw::rtio_sqe,
}

impl Sqe {
    /// Configure this SQE as a callback.
    pub fn prep_callback(
        &mut self,
        callback: raw::rtio_callback_t,
        arg0: *mut c_void,
        userdata: *mut c_void,
    ) {
        unsafe {
            raw::rtio_sqe_prep_callback(self.item, callback, arg0, userdata);
        }
    }

    /// Configure this SQE as a nop.
    pub fn prep_nop(&mut self, dev: *mut raw::rtio_iodev, userdata: *mut c_void) {
        unsafe {
            raw::rtio_sqe_prep_nop(self.item, dev, userdata);
        }
    }

    /// Add flags.
    pub fn or_flags(&mut self, flags: u16) {
        unsafe {
            (*self.item).flags |= flags;
        }
    }
}

/// A single Cqe.
pub struct Cqe {
    item: *mut raw::rtio_cqe,
    rtio: *mut raw::rtio,
}

impl Cqe {
    /// Retrieve the result of this operation.
    pub fn result(&self) -> i32 {
        unsafe { (*self.item).result }
    }
}

impl Drop for Cqe {
    fn drop(&mut self) {
        unsafe {
            raw::rtio_cqe_release(self.rtio, self.item);
        }
    }
}
