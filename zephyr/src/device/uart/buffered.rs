//! CPU-only queues and waiter bookkeeping, shared by both UART backends.

use crate::{raw, sync::SpinMutex, Error, Result};
use core::task::{Context, Poll, Waker};

pub(super) struct Queue<const N: usize> {
    bytes: [u8; N],
    head: usize,
    len: usize,
}

impl<const N: usize> Queue<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            head: 0,
            len: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    pub fn push(&mut self, input: &[u8]) -> usize {
        let n = input.len().min(N - self.len);

        for &byte in &input[..n] {
            self.bytes[(self.head + self.len) % N] = byte;
            self.len += 1;
        }

        n
    }

    pub fn peek(&self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len);

        for (i, byte) in out[..n].iter_mut().enumerate() {
            *byte = self.bytes[(self.head + i) % N];
        }

        n
    }

    pub fn discard(&mut self, n: usize) {
        let n = n.min(self.len);
        if n != 0 {
            self.head = (self.head + n) % N;
            self.len -= n;
        }
    }

    pub fn pop(&mut self, out: &mut [u8]) -> usize {
        let n = self.peek(out);
        self.discard(n);

        n
    }
}

pub(super) struct Io<const RX: usize, const TX: usize> {
    pub rx: Queue<RX>,
    pub tx: Queue<TX>,

    pub closing: bool,
    pub rx_error: Option<u32>,
    pub tx_error: Option<u32>,
    pub stop_error: Option<u32>,

    rx_waiter: Option<Waker>,
    tx_waiter: Option<Waker>,
    flush_waiter: Option<Waker>,
    stop_waiter: Option<Waker>,
}

pub(super) struct WakeBatch([Option<Waker>; 4]);

impl WakeBatch {
    pub fn wake(self) {
        for waker in self.0.into_iter().flatten() {
            waker.wake();
        }
    }
}

impl<const RX: usize, const TX: usize> Io<RX, TX> {
    const fn new() -> Self {
        Self {
            rx: Queue::new(),
            tx: Queue::new(),
            closing: false,
            rx_error: None,
            tx_error: None,
            stop_error: None,
            rx_waiter: None,
            tx_waiter: None,
            flush_waiter: None,
            stop_waiter: None,
        }
    }

    pub fn wake(&mut self) -> WakeBatch {
        WakeBatch([
            self.rx_waiter.take(),
            self.tx_waiter.take(),
            self.flush_waiter.take(),
            self.stop_waiter.take(),
        ])
    }

    pub fn receive(&mut self, bytes: &[u8]) {
        if !self.closing && self.rx_error.is_none() && self.rx.push(bytes) != bytes.len() {
            self.rx_error = Some(raw::EOVERFLOW);
        }
    }

    pub fn tx_failed(&mut self, code: u32) {
        self.tx.clear();
        self.tx_error.get_or_insert(code);
    }

    pub fn fail(&mut self, code: u32) {
        self.closing = true;
        self.rx_error.get_or_insert(code);
        self.tx_failed(code);
        self.stop_error.get_or_insert(code);
    }
}

pub(super) struct State<const RX: usize, const TX: usize, B> {
    pub io: Io<RX, TX>,
    pub backend: B,
}

pub(super) struct Shared<const RX: usize, const TX: usize, B> {
    pub state: SpinMutex<State<RX, TX, B>>,
}

impl<const RX: usize, const TX: usize, B> Shared<RX, TX, B> {
    pub const fn new(backend: B) -> Self {
        Self {
            state: SpinMutex::new(State {
                io: Io::new(),
                backend,
            }),
        }
    }

    pub fn validate() -> Result<()> {
        if RX == 0 || TX == 0 || RX > i32::MAX as usize || TX > i32::MAX as usize {
            Err(Error(raw::EINVAL))
        } else {
            Ok(())
        }
    }

    pub fn read(&self, cx: &mut Context<'_>, out: &mut [u8]) -> Poll<Result<usize>> {
        if out.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let waker = cx.waker().clone();
        let old;

        {
            let mut state = self.state.lock().unwrap();

            if let Some(code) = state.io.rx_error.take() {
                state.io.rx.clear();
                return Poll::Ready(Err(Error(code)));
            }

            if state.io.closing {
                return Poll::Ready(Err(Error(raw::ENODEV)));
            }

            let n = state.io.rx.pop(out);
            if n != 0 {
                return Poll::Ready(Ok(n));
            }

            old = state.io.rx_waiter.replace(waker);
        }

        drop(old);

        Poll::Pending
    }

    pub fn write(&self, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<Result<usize>> {
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let waker = cx.waker().clone();
        let old;

        {
            let mut state = self.state.lock().unwrap();

            if let Some(code) = state.io.tx_error.take() {
                return Poll::Ready(Err(Error(code)));
            }

            if state.io.closing {
                return Poll::Ready(Err(Error(raw::ENODEV)));
            }

            let n = state.io.tx.push(bytes);
            if n != 0 {
                return Poll::Ready(Ok(n));
            }

            old = state.io.tx_waiter.replace(waker);
        }

        drop(old);

        Poll::Pending
    }

    pub fn stop(&self) -> WakeBatch {
        let mut state = self.state.lock().unwrap();

        state.io.closing = true;
        state.io.tx.clear();

        state.io.wake()
    }

    pub fn stopped(&self, cx: &mut Context<'_>, done: impl FnOnce(&B) -> bool) -> Poll<Result<()>> {
        let waker = cx.waker().clone();
        let old;

        {
            let mut state = self.state.lock().unwrap();

            if let Some(code) = state.io.stop_error {
                return Poll::Ready(Err(Error(code)));
            }

            if done(&state.backend) {
                return Poll::Ready(Ok(()));
            }

            old = state.io.stop_waiter.replace(waker);
        }

        drop(old);

        Poll::Pending
    }

    pub fn flushed(&self, cx: &mut Context<'_>, idle: impl FnOnce(&B) -> bool) -> Poll<Result<()>> {
        let waker = cx.waker().clone();
        let old;

        {
            let mut state = self.state.lock().unwrap();

            if let Some(code) = state.io.tx_error.take() {
                return Poll::Ready(Err(Error(code)));
            }

            if state.io.closing {
                return Poll::Ready(Err(Error(raw::ENODEV)));
            }

            if state.io.tx.is_empty() && idle(&state.backend) {
                return Poll::Ready(Ok(()));
            }

            old = state.io.flush_waiter.replace(waker);
        }

        drop(old);

        Poll::Pending
    }
}
