// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use ::arrayvec::ArrayVec;
use ::crossbeam_channel;
use ::runtime::{
    memory::{
        Buffer,
        DataBuffer,
    },
    network::{
        config::{
            ArpConfig,
            TcpConfig,
        },
        consts::RECEIVE_BATCH_SIZE,
        types::{
            Ipv4Addr,
            MacAddress,
        },
        NetworkRuntime,
        PacketBuf,
    },
    scheduler::Scheduler,
    task::SchedulerRuntime,
    timer::{
        TimerRc,
        WaitFuture,
    },
};
use ::std::{
    cell::RefCell,
    rc::Rc,
};

//==============================================================================
// Structures
//==============================================================================

/// Shared Dummy Runtime
struct SharedDummyRuntime {
    /// Random Number Generator
    /// Incoming Queue of Packets
    incoming: crossbeam_channel::Receiver<DataBuffer>,
    /// Outgoing Queue of Packets
    outgoing: crossbeam_channel::Sender<DataBuffer>,
}

/// Dummy Runtime
#[derive(Clone)]
pub struct DummyRuntime {
    /// Shared Member Fields
    inner: Rc<RefCell<SharedDummyRuntime>>,
    scheduler: Scheduler,
    link_addr: MacAddress,
    ipv4_addr: Ipv4Addr,
    tcp_options: TcpConfig,
    arp_options: ArpConfig,
}

//==============================================================================
// Associate Functions
//==============================================================================

/// Associate Functions for Dummy Runtime
impl DummyRuntime {
    /// Creates a Dummy Runtime.
    pub fn new(
        link_addr: MacAddress,
        ipv4_addr: Ipv4Addr,
        incoming: crossbeam_channel::Receiver<DataBuffer>,
        outgoing: crossbeam_channel::Sender<DataBuffer>,
        arp_options: ArpConfig,
    ) -> Self {
        let inner = SharedDummyRuntime { incoming, outgoing };
        Self {
            inner: Rc::new(RefCell::new(inner)),
            scheduler: Scheduler::default(),
            link_addr,
            ipv4_addr,
            tcp_options: TcpConfig::default(),
            arp_options,
        }
    }
}

//==============================================================================
// Trait Implementations
//==============================================================================

/// Network Runtime Trait Implementation for Dummy Runtime
impl NetworkRuntime for DummyRuntime {
    fn transmit(&self, pkt: impl PacketBuf) {
        let header_size = pkt.header_size();
        let body_size = pkt.body_size();

        let mut buf: DataBuffer = DataBuffer::new(header_size + body_size).unwrap();
        pkt.write_header(&mut buf[..header_size]);
        if let Some(body) = pkt.take_body() {
            buf[header_size..].copy_from_slice(&body[..]);
        }
        self.inner.borrow_mut().outgoing.try_send(buf).unwrap();
    }

    fn receive(&self) -> ArrayVec<Box<dyn Buffer>, RECEIVE_BATCH_SIZE> {
        let mut out = ArrayVec::new();
        if let Some(buf) = self.inner.borrow_mut().incoming.try_recv().ok() {
            let dbuf: Box<dyn Buffer> = Box::new(buf);
            out.push(dbuf);
        }
        out
    }
}

/// Scheduler Runtime Trait Implementation for Dummy Runtime
impl SchedulerRuntime for DummyRuntime {
    type WaitFuture = WaitFuture<TimerRc>;
}
