// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::peer::{
    Inner,
    TcpPeer,
};
use crate::operations::OperationResult;
use ::runtime::{
    fail::Fail,
    memory::Buffer,
    network::NetworkRuntime,
    scheduler::FutureResult,
    task::SchedulerRuntime,
    QDesc,
};
use ::std::{
    cell::RefCell,
    fmt,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{
        Context,
        Poll,
    },
};

pub enum TcpOperation<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> {
    Accept(FutureResult<AcceptFuture<RT>>),
    Connect(FutureResult<ConnectFuture<RT>>),
    Pop(FutureResult<PopFuture<RT>>),
    Push(FutureResult<PushFuture>),
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> From<AcceptFuture<RT>> for TcpOperation<RT> {
    fn from(f: AcceptFuture<RT>) -> Self {
        TcpOperation::Accept(FutureResult::new(f, None))
    }
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> From<ConnectFuture<RT>> for TcpOperation<RT> {
    fn from(f: ConnectFuture<RT>) -> Self {
        TcpOperation::Connect(FutureResult::new(f, None))
    }
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> From<PushFuture> for TcpOperation<RT> {
    fn from(f: PushFuture) -> Self {
        TcpOperation::Push(FutureResult::new(f, None))
    }
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> From<PopFuture<RT>> for TcpOperation<RT> {
    fn from(f: PopFuture<RT>) -> Self {
        TcpOperation::Pop(FutureResult::new(f, None))
    }
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> Future for TcpOperation<RT> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<()> {
        match self.get_mut() {
            TcpOperation::Accept(ref mut f) => Future::poll(Pin::new(f), ctx),
            TcpOperation::Connect(ref mut f) => Future::poll(Pin::new(f), ctx),
            TcpOperation::Push(ref mut f) => Future::poll(Pin::new(f), ctx),
            TcpOperation::Pop(ref mut f) => Future::poll(Pin::new(f), ctx),
        }
    }
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> TcpOperation<RT> {
    pub fn expect_result(self) -> (QDesc, Option<QDesc>, OperationResult) {
        match self {
            // Connect operation.
            TcpOperation::Connect(FutureResult {
                future,
                done: Some(Ok(())),
            }) => (future.fd, None, OperationResult::Connect),
            TcpOperation::Connect(FutureResult {
                future,
                done: Some(Err(e)),
            }) => (future.fd, None, OperationResult::Failed(e)),

            // Accept operation.
            TcpOperation::Accept(FutureResult {
                future,
                done: Some(Ok(new_qd)),
            }) => (future.qd, Some(future.new_qd), OperationResult::Accept(new_qd)),
            TcpOperation::Accept(FutureResult {
                future,
                done: Some(Err(e)),
            }) => (future.qd, Some(future.new_qd), OperationResult::Failed(e)),

            // Push operation
            TcpOperation::Push(FutureResult {
                future,
                done: Some(Ok(())),
            }) => (future.fd, None, OperationResult::Push),
            TcpOperation::Push(FutureResult {
                future,
                done: Some(Err(e)),
            }) => (future.fd, None, OperationResult::Failed(e)),

            // Pop Operation.
            TcpOperation::Pop(FutureResult {
                future,
                done: Some(Ok(bytes)),
            }) => (future.fd, None, OperationResult::Pop(None, bytes)),
            TcpOperation::Pop(FutureResult {
                future,
                done: Some(Err(e)),
            }) => (future.fd, None, OperationResult::Failed(e)),

            _ => panic!("Future not ready"),
        }
    }
}

pub enum ConnectFutureState {
    Failed(Fail),
    InProgress,
}

pub struct ConnectFuture<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> {
    pub fd: QDesc,
    pub state: ConnectFutureState,
    pub inner: Rc<RefCell<Inner<RT>>>,
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> fmt::Debug for ConnectFuture<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ConnectFuture({:?})", self.fd)
    }
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> Future for ConnectFuture<RT> {
    type Output = Result<(), Fail>;

    fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
        let self_ = self.get_mut();
        match self_.state {
            ConnectFutureState::Failed(ref e) => Poll::Ready(Err(e.clone())),
            ConnectFutureState::InProgress => self_.inner.borrow_mut().poll_connect_finished(self_.fd, context),
        }
    }
}

/// Accept Operation Descriptor
pub struct AcceptFuture<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> {
    /// Queue descriptor of listening socket.
    qd: QDesc,
    // Pre-booked queue descriptor for incoming connection.
    new_qd: QDesc,
    // Reference to associated inner TCP peer.
    inner: Rc<RefCell<Inner<RT>>>,
}

/// Associated Functions for Accept Operation Descriptors
impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> AcceptFuture<RT> {
    /// Creates a descriptor for an accept operation.
    pub fn new(qd: QDesc, new_qd: QDesc, inner: Rc<RefCell<Inner<RT>>>) -> Self {
        Self { qd, new_qd, inner }
    }
}

/// Debug Trait Implementation for Accept Operation Descriptors
impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> fmt::Debug for AcceptFuture<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AcceptFuture({:?})", self.qd)
    }
}

/// Future Trait Implementation for Accept Operation Descriptors
impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> Future for AcceptFuture<RT> {
    type Output = Result<QDesc, Fail>;

    /// Polls the underlying accept operation.
    fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
        let self_: &mut AcceptFuture<RT> = self.get_mut();
        // TODO: The following design pattern looks ugly. We should move poll_accept to the inner structure.
        let peer: TcpPeer<RT> = TcpPeer::<RT> {
            inner: self_.inner.clone(),
        };
        peer.poll_accept(self_.qd, self_.new_qd, context)
    }
}

pub struct PushFuture {
    pub fd: QDesc,
    pub err: Option<Fail>,
}

impl fmt::Debug for PushFuture {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PushFuture({:?})", self.fd)
    }
}

impl Future for PushFuture {
    type Output = Result<(), Fail>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context) -> Poll<Self::Output> {
        match self.get_mut().err.take() {
            None => Poll::Ready(Ok(())),
            Some(e) => Poll::Ready(Err(e)),
        }
    }
}

pub struct PopFuture<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> {
    pub fd: QDesc,
    pub inner: Rc<RefCell<Inner<RT>>>,
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> fmt::Debug for PopFuture<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PopFuture({:?})", self.fd)
    }
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> Future for PopFuture<RT> {
    type Output = Result<Box<dyn Buffer>, Fail>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let self_ = self.get_mut();
        let peer = TcpPeer {
            inner: self_.inner.clone(),
        };
        peer.poll_recv(self_.fd, ctx)
    }
}
