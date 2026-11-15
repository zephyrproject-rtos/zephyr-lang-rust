// SPDX-License-Identifier: Apache-2.0
#![no_std]

use core::{
    future::{poll_fn, Future},
    pin::pin,
    task::Poll,
};
use embassy_futures::{
    join::join,
    select::{select, Either},
};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use zephyr::{
    embassy::Executor,
    printkln,
    sys::sync::{Semaphore, SemaphoreRef},
    time::NoWait,
};
use zephyr_panic as _;

static EXECUTOR: StaticCell<Executor> = StaticCell::new();
static SECOND_EXECUTOR: StaticCell<Executor> = StaticCell::new();
static CROSS: Semaphore = Semaphore::new(0, 1);
static CROSS_DONE: Semaphore = Semaphore::new(0, 1);

zephyr::kobj_define! {
    static SECOND_THREAD: StaticThread;
    static SECOND_STACK: ThreadStack<2048>;
}
static SEM: Semaphore = Semaphore::new(0, 8);
static HEARTBEAT: Semaphore = Semaphore::new(0, 1);
static OTHER: Semaphore = Semaphore::new(0, 1);

extern "C" {
    fn schedule_give(sem: *mut zephyr::raw::k_sem);
    fn schedule_reset_then_give(sem: *mut zephyr::raw::k_sem);
    fn schedule_stolen_then_give(sem: *mut zephyr::raw::k_sem);
    fn native_semaphore() -> *mut zephyr::raw::k_sem;
}

fn give_later() {
    // SAFETY: SEM is initialized and static. The C timer only calls k_sem_give.
    unsafe { schedule_give(SEM.as_raw()) };
}

async fn take_with_deadline(sem: &'static Semaphore) {
    assert!(matches!(
        select(sem.take_async(), Timer::after_secs(1)).await,
        Either::First(())
    ));
}

#[no_mangle]
extern "C" fn rust_main() {
    let mut thread = SECOND_THREAD
        .init_once(SECOND_STACK.init_once(()).unwrap())
        .unwrap();
    thread.set_priority(5);
    thread.spawn(|| {
        SECOND_EXECUTOR
            .init(Executor::new())
            .run(|spawner| spawner.spawn(cross_executor().unwrap()));
    });
    EXECUTOR
        .init(Executor::new())
        .run(|spawner| spawner.spawn(test().unwrap()));
}

#[embassy_executor::task]
async fn cross_executor() {
    for _ in 0..20 {
        CROSS.take_async().await;
        CROSS_DONE.give();
    }
}

#[embassy_executor::task]
async fn test() {
    // Ready fast path consumes exactly one token.
    SEM.give();
    SEM.give();
    SEM.take_async().await;
    assert_eq!(SEM.count_get(), 1);
    SEM.take_async().await;
    assert_eq!(SEM.count_get(), 0);

    // Cancellation unregisters and leaves the next token available.
    {
        let mut pending = pin!(SEM.take_async());
        poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    SEM.give();
    SEM.take(NoWait).unwrap();

    // A timeout cancels a registered wait without blocking other tasks.
    assert!(matches!(
        select(SEM.take_async(), Timer::after_millis(10)).await,
        Either::Second(())
    ));
    SEM.give();
    SEM.take(NoWait).unwrap();

    // C gives from a timer ISR, without invoking a Rust wake callback.
    give_later();
    take_with_deadline(&SEM).await;

    // A driver-owned native object needs no Rust-side reinitialization.
    // SAFETY: C owns an initialized K_SEM_DEFINE object for program lifetime.
    let native = unsafe { SemaphoreRef::from_raw(native_semaphore()) };
    // SAFETY: The native object is static; the timer only gives it.
    unsafe { schedule_give(native_semaphore()) };
    assert!(matches!(
        select(native.take_async(), Timer::after_secs(1)).await,
        Either::First(())
    ));

    // Reset leaves the async wait pending until a subsequent give.
    // SAFETY: The timer only uses semaphore API operations on a static object.
    unsafe { schedule_reset_then_give(SEM.as_raw()) };
    take_with_deadline(&SEM).await;

    // Notification does not reserve a token. A competing native taker wins
    // the first give; the async future must rearm for the second.
    // SAFETY: Both timers use semaphore API operations on a static object.
    unsafe { schedule_stolen_then_give(SEM.as_raw()) };
    take_with_deadline(&SEM).await;

    // Two waiters for one native semaphore each require their own token.
    join(
        join(take_with_deadline(&SEM), take_with_deadline(&SEM)),
        async {
            give_later();
            Timer::after_millis(30).await;
            give_later();
        },
    )
    .await;
    assert_eq!(SEM.count_get(), 0);

    // Independent semaphores and ordinary task wakeups coexist.
    join(take_with_deadline(&OTHER), async {
        Timer::after_millis(5).await;
        OTHER.give();
    })
    .await;

    // Repeated waits catch lost wakeups and stale poll state. A timer task
    // proves that waiting never blocks the executor thread.
    join(
        async {
            for _ in 0..50 {
                give_later();
                take_with_deadline(&SEM).await;
            }
        },
        async {
            Timer::after(Duration::from_millis(5)).await;
            HEARTBEAT.give();
        },
    )
    .await;
    HEARTBEAT.take(NoWait).unwrap();
    // Two independent executor threads exchange native semaphore wakeups.
    for _ in 0..20 {
        CROSS.give();
        take_with_deadline(&CROSS_DONE).await;
    }
    printkln!("Async semaphore tests passed");
}
