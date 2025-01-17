// Copyright (c) 2023 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Synchronizer using channels
//!
//! Synchronize between the philosophers using channels to communicate with a thread that handles
//! the messages.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

use zephyr::sync::channel::{self, Receiver, Sender};
use zephyr::{kobj_define, sync::Arc};

use crate::{ForkSync, NUM_PHIL};

/// An implementation of ForkSync that uses a server commnicated with channels to perform the
/// synchronization.
#[derive(Debug)]
struct ChannelSync {
    command: Sender<Command>,
    reply_send: Sender<()>,
    reply_recv: Receiver<()>,
}

#[derive(Debug)]
enum Command {
    Acquire(usize, Sender<()>),
    Release(usize),
}

/// This implements a single Fork on the server side for the ChannelSync.
#[derive(Default)]
enum ChannelFork {
    /// The fork is free,
    #[default]
    Free,
    /// The work is in use, nobody is waiting.
    InUse,
    /// The fork is in use, and someone is waiting on it.
    InUseWait(Sender<()>),
}

impl ChannelFork {
    /// Attempt to aquire the work.  If it is free, reply to the sender, otherwise, track them to
    /// reply to them when the fork is freed up.
    fn acquire(&mut self, reply: Sender<()>) {
        // For debugging, just stop here, and wait for a stack report.
        let next = match *self {
            ChannelFork::Free => {
                // Reply immediately that this fork is free.
                reply.send(()).unwrap();
                ChannelFork::InUse
            }
            ChannelFork::InUse => {
                // The fork is being used, become the waiter.
                ChannelFork::InUseWait(reply)
            }
            ChannelFork::InUseWait(_) => {
                // There is already a wait.  Something has gone wrong as this should never happen.
                panic!("Mutliple waiters on fork");
            }
        };
        *self = next;
    }

    /// Release the fork.  This is presumably sent from the same sender that requested it, although
    /// this is not checked.
    fn release(&mut self) {
        let next = match self {
            ChannelFork::Free => {
                // An error case, the fork is not in use, it shouldn't be freed.
                panic!("Release of fork that is not in use");
            }
            ChannelFork::InUse => {
                // The fork is in use, and nobody else is waiting.
                ChannelFork::Free
            }
            ChannelFork::InUseWait(waiter) => {
                // The fork is in use by us, and someone else is waiting.  Tell the other waiter
                // they now have the work.
                waiter.send(()).unwrap();
                ChannelFork::InUse
            }
        };
        *self = next;
    }
}

impl ChannelSync {
    pub fn new(command: Sender<Command>, reply: (Sender<()>, Receiver<()>)) -> ChannelSync {
        ChannelSync {
            command,
            reply_send: reply.0,
            reply_recv: reply.1,
        }
    }
}

/// Generate a syncer out of a ChannelSync.
#[allow(dead_code)]
pub fn get_channel_syncer() -> Vec<Arc<dyn ForkSync>> {
    let (cq_send, cq_recv);
    let reply_queues;

    if cfg!(CONFIG_USE_BOUNDED_CHANNELS) {
        // Use only one message, so that send will block, to ensure that works.
        (cq_send, cq_recv) = channel::bounded(1);
        reply_queues = [(); NUM_PHIL].each_ref().map(|()| channel::bounded(1));
    } else {
        (cq_send, cq_recv) = channel::unbounded();
        reply_queues = [(); NUM_PHIL].each_ref().map(|()| channel::unbounded());
    }

    let syncer = reply_queues.into_iter().map(|rqueue| {
        let item = Box::new(ChannelSync::new(cq_send.clone(), rqueue)) as Box<dyn ForkSync>;
        Arc::from(item)
    });

    let channel_child = CHANNEL_THREAD
        .init_once(CHANNEL_STACK.init_once(()).unwrap())
        .unwrap();
    channel_child.spawn(move || {
        channel_thread(cq_recv);
    });

    syncer.collect()
}

/// The thread that handles channel requests.
///
/// Spawned when we are using the channel syncer.
fn channel_thread(cq_recv: Receiver<Command>) {
    let mut forks = [(); NUM_PHIL].each_ref().map(|_| ChannelFork::default());

    loop {
        match cq_recv.recv().unwrap() {
            Command::Acquire(fork, reply) => {
                forks[fork].acquire(reply);
            }
            Command::Release(fork) => {
                forks[fork].release();
            }
        }
    }
}

impl ForkSync for ChannelSync {
    fn take(&self, index: usize) {
        self.command
            .send(Command::Acquire(index, self.reply_send.clone()))
            .unwrap();
        // When the reply comes, we know we have the resource.
        self.reply_recv.recv().unwrap();
    }

    fn release(&self, index: usize) {
        self.command.send(Command::Release(index)).unwrap();
        // Release does not have a reply.
    }
}

kobj_define! {
    static CHANNEL_STACK: ThreadStack<2054>;
    static CHANNEL_THREAD: StaticThread;
}
