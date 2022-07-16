// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod background;
pub mod congestion_control;
mod ctrlblk;
mod rto;
mod sender;

pub use self::ctrlblk::{
    ControlBlock,
    State,
};

use self::background::background;
use crate::{
    futures::FutureOperation,
    protocols::tcp::segment::TcpHeader,
};
use ::futures::{
    channel::mpsc,
    FutureExt,
};
use ::runtime::{
    fail::Fail,
    memory::Buffer,
    network::NetworkRuntime,
    scheduler::SchedulerHandle,
    task::SchedulerRuntime,
    QDesc,
};
use ::std::{
    net::SocketAddr,
    rc::Rc,
    task::{
        Context,
        Poll,
    },
    time::Duration,
};

pub struct EstablishedSocket<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> {
    pub cb: Rc<ControlBlock<RT>>,
    /// The background co-routines handles various tasks, such as retransmission and acknowledging.
    /// We annotate it as unused because the compiler believes that it is never called which is not the case.
    #[allow(unused)]
    background: SchedulerHandle,
}

impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> EstablishedSocket<RT> {
    pub fn new(cb: ControlBlock<RT>, fd: QDesc, dead_socket_tx: mpsc::UnboundedSender<QDesc>) -> Self {
        let cb = Rc::new(cb);
        let future = background(cb.clone(), fd, dead_socket_tx);
        let handle: SchedulerHandle = cb.rt().spawn(FutureOperation::Background::<RT>(future.boxed_local()));
        Self {
            cb: cb.clone(),
            background: handle,
        }
    }

    pub fn receive(&self, header: &mut TcpHeader, data: Buffer) {
        self.cb.receive(header, data)
    }

    pub fn send(&self, buf: Buffer) -> Result<(), Fail> {
        self.cb.send(buf)
    }

    pub fn poll_recv(&self, ctx: &mut Context) -> Poll<Result<Buffer, Fail>> {
        self.cb.poll_recv(ctx)
    }

    pub fn close(&self) -> Result<(), Fail> {
        self.cb.close()
    }

    pub fn remote_mss(&self) -> usize {
        self.cb.remote_mss()
    }

    pub fn current_rto(&self) -> Duration {
        self.cb.rto_estimate()
    }

    pub fn endpoints(&self) -> (SocketAddr, SocketAddr) {
        (self.cb.get_local(), self.cb.get_remote())
    }
}
