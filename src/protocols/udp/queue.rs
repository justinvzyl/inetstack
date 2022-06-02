// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use ::futures::{
    channel::mpsc::{
        self,
        UnboundedReceiver,
        UnboundedSender,
    },
    StreamExt,
};
use ::libc::EIO;
use ::runtime::fail::Fail;
use ::std::{
    cell::RefCell,
    net::SocketAddrV4,
    rc::Rc,
};

//==============================================================================
// Structures
//==============================================================================

/// Shared Queue Slot
pub struct SharedQueueSlot<T> {
    /// Local endpoint.
    pub local: SocketAddrV4,
    /// Remote endpoint.
    pub remote: SocketAddrV4,
    /// Associated data.
    pub data: T,
}

/// Shared Queue
///
/// TODO: Reuse this structure in TCP stack, for send/receive queues.
pub struct SharedQueue<T> {
    /// Send-side endpoint.
    tx: Rc<RefCell<UnboundedSender<T>>>,
    /// Receive-side endpoint.
    rx: Rc<RefCell<UnboundedReceiver<T>>>,
}

//==============================================================================
// Associated Functions
//==============================================================================

/// Associated Functions Shared Queues
impl<T> SharedQueue<T> {
    /// Instantiates a shared queue.
    pub fn new() -> Self {
        let (tx, rx): (UnboundedSender<T>, UnboundedReceiver<T>) = mpsc::unbounded();
        Self {
            tx: Rc::new(RefCell::new(tx)),
            rx: Rc::new(RefCell::new(rx)),
        }
    }

    /// Pushes a message to the target shared queue.
    pub fn push(&self, msg: T) -> Result<(), Fail> {
        match self.tx.borrow_mut().unbounded_send(msg) {
            Ok(_) => Ok(()),
            Err(_) => Err(Fail::new(EIO, "failed to push to shared queue")),
        }
    }

    /// Synchronously attempts to pop a message from the target shared queue.
    pub fn try_pop(&mut self) -> Result<Option<T>, Fail> {
        match self.rx.borrow_mut().try_next() {
            Ok(Some(msg)) => Ok(Some(msg)),
            Ok(None) => Err(Fail::new(EIO, "failed to pop from shared queue")),
            Err(_) => Ok(None),
        }
    }

    /// Asynchronously pops a message from the target shared queue.
    pub async fn pop(&mut self) -> Result<T, Fail> {
        match self.rx.borrow_mut().next().await {
            Some(msg) => Ok(msg),
            None => Err(Fail::new(EIO, "failed to pop from shared queue")),
        }
    }
}

//==============================================================================
// Trait Implementations
//==============================================================================

/// Clone Trait Implementation for Shared Queues
impl<T> Clone for SharedQueue<T> {
    /// Clones the target [SharedQueue].
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            rx: self.rx.clone(),
        }
    }
}
