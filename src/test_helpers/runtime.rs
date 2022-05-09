// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{
    futures::operation::FutureOperation,
    test_helpers::Engine,
};
use ::arrayvec::ArrayVec;
use ::futures::FutureExt;
use ::rand::{
    distributions::{
        Distribution,
        Standard,
    },
    rngs::SmallRng,
    seq::SliceRandom,
    Rng,
    SeedableRng,
};
use ::runtime::{
    fail::Fail,
    logging,
    memory::{
        Bytes,
        BytesMut,
        MemoryRuntime,
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
    task::SchedulerRuntime,
    timer::{
        Timer,
        TimerRc,
        WaitFuture,
    },
    types::demi_sgarray_t,
    utils::UtilsRuntime,
    Runtime,
};
use ::scheduler::{
    Scheduler,
    SchedulerFuture,
    SchedulerHandle,
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
    rng: SmallRng,
    incoming: VecDeque<Bytes>,
    outgoing: VecDeque<Bytes>,
}

#[derive(Clone)]
pub struct TestRuntime {
    name: &'static str,
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
        name: &'static str,
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
            rng: SmallRng::from_seed([0; 32]),
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
        };
        Self {
            name,
            link_addr,
            ipv4_addr,
            inner: Rc::new(RefCell::new(inner)),
            scheduler: Scheduler::default(),
            arp_options,
            udp_options,
            tcp_options,
        }
    }

    pub fn pop_frame(&self) -> Bytes {
        self.inner
            .borrow_mut()
            .outgoing
            .pop_front()
            .expect("pop_front didn't return an outgoing frame")
    }

    pub fn pop_frame_unchecked(&self) -> Option<Bytes> {
        self.inner.borrow_mut().outgoing.pop_front()
    }

    pub fn push_frame(&self, buf: Bytes) {
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

impl MemoryRuntime for TestRuntime {
    type Buf = Bytes;

    fn into_sgarray(&self, _buf: Bytes) -> Result<demi_sgarray_t, Fail> {
        unreachable!()
    }

    fn alloc_sgarray(&self, _size: usize) -> Result<demi_sgarray_t, Fail> {
        unreachable!()
    }

    fn free_sgarray(&self, _sga: demi_sgarray_t) -> Result<(), Fail> {
        unreachable!()
    }

    fn clone_sgarray(&self, _sga: &demi_sgarray_t) -> Result<Bytes, Fail> {
        unreachable!()
    }
}

impl NetworkRuntime for TestRuntime {
    fn transmit(&self, pkt: impl PacketBuf<Bytes>) {
        let header_size = pkt.header_size();
        let body_size = pkt.body_size();

        let mut buf = BytesMut::zeroed(header_size + body_size).unwrap();
        pkt.write_header(&mut buf[..header_size]);
        if let Some(body) = pkt.take_body() {
            buf[header_size..].copy_from_slice(&body[..]);
        }
        self.inner.borrow_mut().outgoing.push_back(buf.freeze());
    }

    fn receive(&self) -> ArrayVec<Bytes, RECEIVE_BATCH_SIZE> {
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

impl UtilsRuntime for TestRuntime {
    fn rng_gen<T>(&self) -> T
    where
        Standard: Distribution<T>,
    {
        let mut inner = self.inner.borrow_mut();
        inner.rng.gen()
    }

    fn rng_shuffle<T>(&self, slice: &mut [T]) {
        let mut inner = self.inner.borrow_mut();
        slice.shuffle(&mut inner.rng);
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

impl Runtime for TestRuntime {}
