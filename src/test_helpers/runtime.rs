// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::test_helpers::Engine;
use ::arrayvec::ArrayVec;
use ::runtime::{
    logging,
    memory::{
        Buffer,
        DataBuffer,
    },
    network::{
        config::{
            ArpConfig,
            TcpConfig,
            UdpConfig,
        },
        consts::RECEIVE_BATCH_SIZE,
        types::MacAddress,
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
    collections::VecDeque,
    net::Ipv4Addr,
    rc::Rc,
};

//==============================================================================
// Types
//==============================================================================

pub type TestEngine = Engine;

//==============================================================================
// Structures
//==============================================================================

// TODO: Drop inner mutability pattern.
pub struct Inner {
    #[allow(unused)]
    timer: TimerRc,
    incoming: VecDeque<Box<dyn Buffer>>,
    outgoing: VecDeque<Box<dyn Buffer>>,
}

#[derive(Clone)]
pub struct TestRuntime {
    link_addr: MacAddress,
    ipv4_addr: Ipv4Addr,
    arp_options: ArpConfig,
    udp_options: UdpConfig,
    tcp_options: TcpConfig,
    inner: Rc<RefCell<Inner>>,
    scheduler: Scheduler,
}

//==============================================================================
// Associate Functions
//==============================================================================

impl TestRuntime {
    pub fn new(
        scheduler: Scheduler,
        clock: TimerRc,
        arp_options: ArpConfig,
        udp_options: UdpConfig,
        tcp_options: TcpConfig,
        link_addr: MacAddress,
        ipv4_addr: Ipv4Addr,
    ) -> Self {
        logging::initialize();

        let inner = Inner {
            timer: clock,
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
        };
        Self {
            link_addr,
            ipv4_addr,
            inner: Rc::new(RefCell::new(inner)),
            scheduler,
            arp_options,
            udp_options,
            tcp_options,
        }
    }

    pub fn pop_frame(&self) -> Box<dyn Buffer> {
        self.inner
            .borrow_mut()
            .outgoing
            .pop_front()
            .expect("pop_front didn't return an outgoing frame")
    }

    pub fn pop_frame_unchecked(&self) -> Option<Box<dyn Buffer>> {
        self.inner.borrow_mut().outgoing.pop_front()
    }

    pub fn push_frame(&self, buf: Box<dyn Buffer>) {
        self.inner.borrow_mut().incoming.push_back(buf);
    }

    pub fn poll_scheduler(&self) {
        // let mut ctx = Context::from_waker(noop_waker_ref());
        self.scheduler.poll();
    }
}

//==============================================================================
// Trait Implementations
//==============================================================================

impl NetworkRuntime for TestRuntime {
    fn transmit(&self, pkt: Box<dyn PacketBuf>) {
        let header_size = pkt.header_size();
        let body_size = pkt.body_size();

        let mut buf: Box<dyn Buffer> = Box::new(DataBuffer::new(header_size + body_size).unwrap());
        pkt.write_header(&mut buf[..header_size]);
        if let Some(body) = pkt.take_body() {
            buf[header_size..].copy_from_slice(&body[..]);
        }
        self.inner.borrow_mut().outgoing.push_back(buf);
    }

    fn receive(&self) -> ArrayVec<Box<dyn Buffer>, RECEIVE_BATCH_SIZE> {
        let mut out = ArrayVec::new();
        if let Some(buf) = self.inner.borrow_mut().incoming.pop_front() {
            out.push(buf);
        }
        out
    }
}

impl SchedulerRuntime for TestRuntime {
    type WaitFuture = WaitFuture<TimerRc>;
}
