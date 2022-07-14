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
    timer::{
        Timer,
        TimerRc,
    },
};
use ::std::{
    cell::RefCell,
    collections::VecDeque,
    net::Ipv4Addr,
    rc::Rc,
    time::Instant,
};

//==============================================================================
// Types
//==============================================================================

pub type TestEngine = Engine<TestRuntime>;

//==============================================================================
// Structures
//==============================================================================

// TODO: Drop inner mutability pattern.
pub struct Inner {
    #[allow(unused)]
    timer: TimerRc,
    incoming: VecDeque<Buffer>,
    outgoing: VecDeque<Buffer>,
}

#[derive(Clone)]
pub struct TestRuntime {
    link_addr: MacAddress,
    ipv4_addr: Ipv4Addr,
    arp_options: ArpConfig,
    udp_options: UdpConfig,
    tcp_options: TcpConfig,
    inner: Rc<RefCell<Inner>>,
    pub scheduler: Scheduler,
    pub clock: TimerRc,
}

//==============================================================================
// Associate Functions
//==============================================================================

impl TestRuntime {
    pub fn new(
        now: Instant,
        arp_options: ArpConfig,
        udp_options: UdpConfig,
        tcp_options: TcpConfig,
        link_addr: MacAddress,
        ipv4_addr: Ipv4Addr,
    ) -> Self {
        logging::initialize();

        let inner = Inner {
            timer: TimerRc(Rc::new(Timer::new(now))),
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
        };
        Self {
            link_addr,
            ipv4_addr,
            inner: Rc::new(RefCell::new(inner)),
            scheduler: Scheduler::default(),
            clock: TimerRc(Rc::new(Timer::new(now))),
            arp_options,
            udp_options,
            tcp_options,
        }
    }

    pub fn pop_frame(&self) -> Buffer {
        self.inner
            .borrow_mut()
            .outgoing
            .pop_front()
            .expect("pop_front didn't return an outgoing frame")
    }

    pub fn pop_frame_unchecked(&self) -> Option<Buffer> {
        self.inner.borrow_mut().outgoing.pop_front()
    }

    pub fn push_frame(&self, buf: Buffer) {
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
    fn transmit(&self, pkt: impl PacketBuf) {
        let header_size = pkt.header_size();
        let body_size = pkt.body_size();

        let mut buf: Buffer = Buffer::Heap(DataBuffer::new(header_size + body_size).unwrap());
        pkt.write_header(&mut buf[..header_size]);
        if let Some(body) = pkt.take_body() {
            buf[header_size..].copy_from_slice(&body[..]);
        }
        self.inner.borrow_mut().outgoing.push_back(buf);
    }

    fn receive(&self) -> ArrayVec<Buffer, RECEIVE_BATCH_SIZE> {
        let mut out = ArrayVec::new();
        if let Some(buf) = self.inner.borrow_mut().incoming.pop_front() {
            out.push(buf);
        }
        out
    }

    fn local_link_addr(&self) -> MacAddress {
        self.link_addr
    }

    fn local_ipv4_addr(&self) -> Ipv4Addr {
        self.ipv4_addr
    }

    fn tcp_options(&self) -> TcpConfig {
        self.tcp_options.clone()
    }

    fn udp_options(&self) -> UdpConfig {
        self.udp_options.clone()
    }

    fn arp_options(&self) -> ArpConfig {
        self.arp_options.clone()
    }
}
