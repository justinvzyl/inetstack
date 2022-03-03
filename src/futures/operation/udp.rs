// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{operations::OperationResult, protocols::udp::UdpPopFuture};
use ::catwalk::FutureResult;
use ::runtime::{fail::Fail, memory::MemoryRuntime, QDesc};
use ::std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

//==============================================================================
// Constants & Structures
//==============================================================================

/// Operations on UDP Layer
pub enum UdpOperation<RT: MemoryRuntime> {
    Connect(QDesc, Result<(), Fail>),
    Push(QDesc, Result<(), Fail>),
    Pop(FutureResult<UdpPopFuture<RT>>),
}

//==============================================================================
// Associate Functions
//==============================================================================

impl<RT: MemoryRuntime> UdpOperation<RT> {
    pub fn expect_result(self) -> (QDesc, OperationResult<RT>) {
        match self {
            UdpOperation::Push(fd, Err(e)) | UdpOperation::Connect(fd, Err(e)) => {
                (fd, OperationResult::Failed(e))
            }
            UdpOperation::Connect(fd, Ok(())) => (fd, OperationResult::Connect),
            UdpOperation::Push(fd, Ok(())) => (fd, OperationResult::Push),

            UdpOperation::Pop(FutureResult {
                future,
                done: Some(Ok((addr, bytes))),
            }) => (future.get_qd(), OperationResult::Pop(addr, bytes)),
            UdpOperation::Pop(FutureResult {
                future,
                done: Some(Err(e)),
            }) => (future.get_qd(), OperationResult::Failed(e)),

            _ => panic!("Future not ready"),
        }
    }
}

//==============================================================================
// Trait Implementations
//==============================================================================

/// Future trait implementation for [UdpOperation]
impl<RT: MemoryRuntime> Future for UdpOperation<RT> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<()> {
        match self.get_mut() {
            UdpOperation::Connect(..) | UdpOperation::Push(..) => Poll::Ready(()),
            UdpOperation::Pop(ref mut f) => Future::poll(Pin::new(f), ctx),
        }
    }
}
