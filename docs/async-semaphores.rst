Async Zephyr semaphores
======================

The ``async-semaphores`` Cargo feature enables ``Semaphore::take_async`` on the
Zephyr Embassy executor. It integrates native Zephyr semaphores with async tasks,
including semaphores given by C code or interrupt handlers. It requires
``CONFIG_POLL=y`` and ``CONFIG_RUST_ALLOC=y``. Run the executor in a supervisor
thread; userspace executors are not supported by this implementation.

Enable the feature on the application's ``zephyr`` dependency::

    zephyr = { version = "0.1.0", features = ["async-semaphores"] }

Declare a static semaphore and await a token from an async task::

    use zephyr::sys::sync::Semaphore;

    static READY: Semaphore = Semaphore::new(0, 1);

    async fn consumer() {
        READY.take_async().await;
        // Process the data made available by the producer.
    }

Rust producers call ``READY.give()``. To share this semaphore with C code, pass
``READY.as_raw()`` to the producer, which can call ``k_sem_give()`` normally.
Native code must not reinitialize the semaphore or modify its fields directly.

The executor waits in ``k_poll()`` on its normal wake semaphore and a snapshot of
pending semaphore registrations. Notification does not reserve a token: async
waiters retry a nonblocking take, and synchronous takers can acquire it first.
Multiple async waiters are supported, without a FIFO fairness guarantee.

Dropping a pending future removes its registration without consuming a token.
Timeouts can be composed with an async timer, for example using
``embassy_futures::select::select``. Resetting the semaphore leaves async waits
pending until a subsequent give, without returning an error from ``take_async``. Poll and drop these futures
from thread context.

Semaphores must have a static lifetime because an executor may still be polling
an earlier snapshot when a future is dropped or forgotten. For a C-owned
semaphore, construct a ``SemaphoreRef`` using its unsafe
``from_raw`` constructor after verifying that it is initialized and will remain
at the same address for the rest of the program. The handle can then be copied
and awaited safely, without taking ownership of the native object.

Each pending wait allocates bookkeeping, and the executor allocates a poll snapshot when there are registered waits. Without this
feature, the executor retains its original semaphore-only sleep path. With no
pending async semaphore waits, it also uses that sleep path.

The integration test in ``tests/async-semaphore`` exercises C interrupt producers,
multiple waiters, cancellation, timeouts, repeated notifications, and coexistence
with timer-based task wakeups and multiple executor threads.
