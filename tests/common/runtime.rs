// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use ::arrayvec::ArrayVec;
use ::catwalk::{Scheduler, SchedulerFuture, SchedulerHandle};
use ::crossbeam_channel::{self};
use ::rand::{
    distributions::{Distribution, Standard},
    rngs::SmallRng,
    seq::SliceRandom,
    Rng, SeedableRng,
};
use ::runtime::{
    fail::Fail,
    memory::{Bytes, BytesMut, MemoryRuntime},
    network::{
        config::{ArpConfig, TcpConfig, UdpConfig},
        consts::RECEIVE_BATCH_SIZE,
        types::{Ipv4Addr, MacAddress},
        NetworkRuntime, PacketBuf,
    },
    task::SchedulerRuntime,
    timer::{Timer, TimerRc, WaitFuture},
    types::{dmtr_sgarray_t, dmtr_sgaseg_t},
    utils::UtilsRuntime,
    Runtime,
};
use ::std::{
    cell::RefCell,
    collections::HashMap,
    mem, ptr,
    rc::Rc,
    slice,
    time::{Duration, Instant},
};

//==============================================================================
// Structures
//==============================================================================

/// Shared Dummy Runtime
struct SharedDummyRuntime {
    /// Clock
    timer: TimerRc,
    /// Random Number Generator
    rng: SmallRng,
    /// Incoming Queue of Packets
    incoming: crossbeam_channel::Receiver<Bytes>,
    /// Outgoing Queue of Packets
    outgoing: crossbeam_channel::Sender<Bytes>,
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
        now: Instant,
        link_addr: MacAddress,
        ipv4_addr: Ipv4Addr,
        incoming: crossbeam_channel::Receiver<Bytes>,
        outgoing: crossbeam_channel::Sender<Bytes>,
        arp: HashMap<Ipv4Addr, MacAddress>,
    ) -> Self {
        let arp_options: ArpConfig = ArpConfig::new(
            Some(Duration::from_secs(600)),
            Some(Duration::from_secs(1)),
            Some(2),
            Some(arp),
            Some(false),
        );

        let inner = SharedDummyRuntime {
            timer: TimerRc(Rc::new(Timer::new(now))),
            rng: SmallRng::from_seed([0; 32]),
            incoming,
            outgoing,
        };
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

/// Memory Runtime Trait Implementation for Dummy Runtime
impl MemoryRuntime for DummyRuntime {
    type Buf = Bytes;

    fn into_sgarray(&self, buf: Bytes) -> Result<dmtr_sgarray_t, Fail> {
        let buf_copy: Box<[u8]> = (&buf[..]).into();
        let ptr = Box::into_raw(buf_copy);
        let sgaseg = dmtr_sgaseg_t {
            sgaseg_buf: ptr as *mut _,
            sgaseg_len: buf.len() as u32,
        };
        Ok(dmtr_sgarray_t {
            sga_buf: ptr::null_mut(),
            sga_numsegs: 1,
            sga_segs: [sgaseg],
            sga_addr: unsafe { mem::zeroed() },
        })
    }

    fn alloc_sgarray(&self, size: usize) -> Result<dmtr_sgarray_t, Fail> {
        let allocation: Box<[u8]> = unsafe { Box::new_uninit_slice(size).assume_init() };
        let ptr = Box::into_raw(allocation);
        let sgaseg = dmtr_sgaseg_t {
            sgaseg_buf: ptr as *mut _,
            sgaseg_len: size as u32,
        };
        Ok(dmtr_sgarray_t {
            sga_buf: ptr::null_mut(),
            sga_numsegs: 1,
            sga_segs: [sgaseg],
            sga_addr: unsafe { mem::zeroed() },
        })
    }

    fn free_sgarray(&self, sga: dmtr_sgarray_t) -> Result<(), Fail> {
        assert_eq!(sga.sga_numsegs, 1);
        for i in 0..sga.sga_numsegs as usize {
            let seg = &sga.sga_segs[i];
            let allocation: Box<[u8]> = unsafe {
                Box::from_raw(slice::from_raw_parts_mut(
                    seg.sgaseg_buf as *mut _,
                    seg.sgaseg_len as usize,
                ))
            };
            drop(allocation);
        }
        Ok(())
    }

    fn clone_sgarray(&self, sga: &dmtr_sgarray_t) -> Result<Bytes, Fail> {
        let mut len = 0;
        for i in 0..sga.sga_numsegs as usize {
            len += sga.sga_segs[i].sgaseg_len;
        }
        let mut buf = BytesMut::zeroed(len as usize).unwrap();
        let mut pos = 0;
        for i in 0..sga.sga_numsegs as usize {
            let seg = &sga.sga_segs[i];
            let seg_slice = unsafe {
                slice::from_raw_parts(seg.sgaseg_buf as *mut u8, seg.sgaseg_len as usize)
            };
            buf[pos..(pos + seg_slice.len())].copy_from_slice(seg_slice);
            pos += seg_slice.len();
        }
        Ok(buf.freeze())
    }
}

/// Network Runtime Trait Implementation for Dummy Runtime
impl NetworkRuntime for DummyRuntime {
    fn transmit(&self, pkt: impl PacketBuf<Bytes>) {
        let header_size = pkt.header_size();
        let body_size = pkt.body_size();

        let mut buf = BytesMut::zeroed(header_size + body_size).unwrap();
        pkt.write_header(&mut buf[..header_size]);
        if let Some(body) = pkt.take_body() {
            buf[header_size..].copy_from_slice(&body[..]);
        }
        self.inner
            .borrow_mut()
            .outgoing
            .try_send(buf.freeze())
            .unwrap();
    }

    fn receive(&self) -> ArrayVec<Bytes, RECEIVE_BATCH_SIZE> {
        let mut out = ArrayVec::new();
        if let Some(buf) = self.inner.borrow_mut().incoming.try_recv().ok() {
            out.push(buf);
        }
        out
    }

    fn local_link_addr(&self) -> MacAddress {
        self.link_addr.clone()
    }

    fn local_ipv4_addr(&self) -> Ipv4Addr {
        self.ipv4_addr.clone()
    }

    fn tcp_options(&self) -> TcpConfig {
        self.tcp_options.clone()
    }

    fn udp_options(&self) -> UdpConfig {
        UdpConfig::default()
    }

    fn arp_options(&self) -> ArpConfig {
        self.arp_options.clone()
    }
}

/// Utils Runtime Trait Implementation for Dummy Runtime
impl UtilsRuntime for DummyRuntime {
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

/// Scheduler Runtime Trait Implementation for Dummy Runtime
impl SchedulerRuntime for DummyRuntime {
    type WaitFuture = WaitFuture<TimerRc>;

    fn advance_clock(&self, now: Instant) {
        self.inner.borrow_mut().timer.0.advance_clock(now);
    }

    fn wait(&self, duration: Duration) -> Self::WaitFuture {
        let inner = self.inner.borrow_mut();
        let now = inner.timer.0.now();
        inner
            .timer
            .0
            .wait_until(inner.timer.clone(), now + duration)
    }

    fn wait_until(&self, when: Instant) -> Self::WaitFuture {
        let inner = self.inner.borrow_mut();
        inner.timer.0.wait_until(inner.timer.clone(), when)
    }

    fn now(&self) -> Instant {
        self.inner.borrow().timer.0.now()
    }

    fn spawn<F: SchedulerFuture>(&self, future: F) -> SchedulerHandle {
        self.scheduler.insert(future)
    }

    fn schedule<F: SchedulerFuture>(&self, future: F) -> SchedulerHandle {
        self.scheduler.insert(future)
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

/// Runtime Trait Implementation for Dummy Runtime
impl Runtime for DummyRuntime {}
