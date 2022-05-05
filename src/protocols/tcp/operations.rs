// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::peer::{Inner, TcpPeer};
use crate::operations::OperationResult;
use ::scheduler::FutureResult;
use ::runtime::{fail::Fail, QDesc, Runtime};
use ::std::{
    cell::RefCell,
    fmt,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

pub enum TcpOperation<RT: Runtime> {
    Accept(FutureResult<AcceptFuture<RT>>),
    Connect(FutureResult<ConnectFuture<RT>>),
    Pop(FutureResult<PopFuture<RT>>),
    Push(FutureResult<PushFuture<RT>>),
}

impl<RT: Runtime> From<AcceptFuture<RT>> for TcpOperation<RT> {
    fn from(f: AcceptFuture<RT>) -> Self {
        TcpOperation::Accept(FutureResult::new(f, None))
    }
}

impl<RT: Runtime> From<ConnectFuture<RT>> for TcpOperation<RT> {
    fn from(f: ConnectFuture<RT>) -> Self {
        TcpOperation::Connect(FutureResult::new(f, None))
    }
}

impl<RT: Runtime> From<PushFuture<RT>> for TcpOperation<RT> {
    fn from(f: PushFuture<RT>) -> Self {
        TcpOperation::Push(FutureResult::new(f, None))
    }
}

impl<RT: Runtime> From<PopFuture<RT>> for TcpOperation<RT> {
    fn from(f: PopFuture<RT>) -> Self {
        TcpOperation::Pop(FutureResult::new(f, None))
    }
}

impl<RT: Runtime> Future for TcpOperation<RT> {
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

impl<RT: Runtime> TcpOperation<RT> {
    pub fn expect_result(self) -> (QDesc, Option<QDesc>, OperationResult<RT::Buf>) {
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
            }) => (
                future.qd,
                Some(future.new_qd),
                OperationResult::Accept(new_qd),
            ),
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

pub struct ConnectFuture<RT: Runtime> {
    pub fd: QDesc,
    pub state: ConnectFutureState,
    pub inner: Rc<RefCell<Inner<RT>>>,
}

impl<RT: Runtime> fmt::Debug for ConnectFuture<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ConnectFuture({:?})", self.fd)
    }
}

impl<RT: Runtime> Future for ConnectFuture<RT> {
    type Output = Result<(), Fail>;

    fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
        let self_ = self.get_mut();
        match self_.state {
            ConnectFutureState::Failed(ref e) => Poll::Ready(Err(e.clone())),
            ConnectFutureState::InProgress => self_
                .inner
                .borrow_mut()
                .poll_connect_finished(self_.fd, context),
        }
    }
}

/// Accept Operation Descriptor
pub struct AcceptFuture<RT: Runtime> {
    /// Queue descriptor of listening socket.
    qd: QDesc,
    // Pre-booked queue descriptor for incoming connection.
    new_qd: QDesc,
    // Reference to associated inner TCP peer.
    inner: Rc<RefCell<Inner<RT>>>,
}

/// Associated Functions for Accept Operation Descriptors
impl<RT: Runtime> AcceptFuture<RT> {
    /// Creates a descriptor for an accept operation.
    pub fn new(qd: QDesc, new_qd: QDesc, inner: Rc<RefCell<Inner<RT>>>) -> Self {
        Self { qd, new_qd, inner }
    }
}

/// Debug Trait Implementation for Accept Operation Descriptors
impl<RT: Runtime> fmt::Debug for AcceptFuture<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AcceptFuture({:?})", self.qd)
    }
}

/// Future Trait Implementation for Accept Operation Descriptors
impl<RT: Runtime> Future for AcceptFuture<RT> {
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

pub struct PushFuture<RT: Runtime> {
    pub fd: QDesc,
    pub err: Option<Fail>,
    pub _marker: std::marker::PhantomData<RT>,
}

impl<RT: Runtime> fmt::Debug for PushFuture<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PushFuture({:?})", self.fd)
    }
}

impl<RT: Runtime> Future for PushFuture<RT> {
    type Output = Result<(), Fail>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context) -> Poll<Self::Output> {
        match self.get_mut().err.take() {
            None => Poll::Ready(Ok(())),
            Some(e) => Poll::Ready(Err(e)),
        }
    }
}

pub struct PopFuture<RT: Runtime> {
    pub fd: QDesc,
    pub inner: Rc<RefCell<Inner<RT>>>,
}

impl<RT: Runtime> fmt::Debug for PopFuture<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PopFuture({:?})", self.fd)
    }
}

impl<RT: Runtime> Future for PopFuture<RT> {
    type Output = Result<RT::Buf, Fail>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let self_ = self.get_mut();
        let peer = TcpPeer {
            inner: self_.inner.clone(),
        };
        peer.poll_recv(self_.fd, ctx)
    }
}
