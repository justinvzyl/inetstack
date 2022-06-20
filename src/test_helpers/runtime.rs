// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{
    futures::operation::FutureOperation,
    test_helpers::Engine,
};
use ::arrayvec::ArrayVec;
use ::futures::FutureExt;
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
    scheduler::{
        Scheduler,
        SchedulerFuture,
        SchedulerHandle,
    },
    task::SchedulerRuntime,
    timer::{
        Timer,
        TimerRc,
        WaitFuture,
    },
};
use ::std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    net::Ipv4Addr,
    rc::Rc,
    time::{
        Duration,
        Instant,
    },
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
    scheduler: Scheduler,
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

impl SchedulerRuntime for TestRuntime {
    type WaitFuture = WaitFuture<TimerRc>;

    fn advance_clock(&self, now: Instant) {
        self.inner.borrow_mut().timer.0.advance_clock(now);
    }

    fn wait(&self, duration: Duration) -> Self::WaitFuture {
        let inner = self.inner.borrow_mut();
        let now = inner.timer.0.now();
        inner.timer.0.wait_until(inner.timer.clone(), now + duration)
    }

    fn wait_until(&self, when: Instant) -> Self::WaitFuture {
        let inner = self.inner.borrow_mut();
        inner.timer.0.wait_until(inner.timer.clone(), when)
    }

    fn now(&self) -> Instant {
        self.inner.borrow().timer.0.now()
    }

    fn spawn<F: Future<Output = ()> + 'static>(&self, future: F) -> SchedulerHandle {
        match self
            .scheduler
            .insert(FutureOperation::Background::<TestRuntime>(future.boxed_local()))
        {
            Some(handle) => handle,
            None => panic!("could not insert future in scheduling queue"),
        }
    }

    fn schedule<F: SchedulerFuture>(&self, future: F) -> SchedulerHandle {
        match self.scheduler.insert(future) {
            Some(handle) => handle,
            None => panic!("could not insert future in scheduling queue"),
        }
    }

    fn get_handle(&self, key: u64) -> Option<SchedulerHandle> {
        self.scheduler.from_raw_handle(key)
    }

    fn take(&self, handle: SchedulerHandle) -> Box<dyn SchedulerFuture> {
        self.scheduler.take(handle)
    }

    fn poll(&self) {
        self.scheduler.poll()
    }
}
