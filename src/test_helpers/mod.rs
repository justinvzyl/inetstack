// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub mod engine;
pub mod runtime;

pub use self::runtime::TestRuntime;
pub use engine::Engine;

use ::runtime::{
    network::{
        config::{
            ArpConfig,
            TcpConfig,
            UdpConfig,
        },
        types::MacAddress,
    },
    scheduler::Scheduler,
    timer::{
        Timer,
        TimerRc,
    },
};
use ::std::{
    collections::HashMap,
    net::Ipv4Addr,
    time::{
        Duration,
        Instant,
    },
};
use std::rc::Rc;

//==============================================================================
// Constants
//==============================================================================

pub const RECEIVE_WINDOW_SIZE: usize = 1024;
pub const ALICE_MAC: MacAddress = MacAddress::new([0x12, 0x23, 0x45, 0x67, 0x89, 0xab]);
pub const ALICE_IPV4: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);
pub const BOB_MAC: MacAddress = MacAddress::new([0xab, 0x89, 0x67, 0x45, 0x23, 0x12]);
pub const BOB_IPV4: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 2);
pub const CARRIE_MAC: MacAddress = MacAddress::new([0xef, 0xcd, 0xab, 0x89, 0x67, 0x45]);
pub const CARRIE_IPV4: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 3);

//==============================================================================
// Types
//==============================================================================

pub type TestEngine = Engine;

//==============================================================================
// Standalone Functions
//==============================================================================

pub fn new_alice(now: Instant) -> Engine {
    let arp_options = ArpConfig::new(
        Some(Duration::from_secs(600)),
        Some(Duration::from_secs(1)),
        Some(2),
        Some(HashMap::new()),
        Some(false),
    );
    let udp_options = UdpConfig::default();
    let tcp_options = TcpConfig::default();
    let scheduler: Scheduler = Scheduler::default();
    let clock = TimerRc(Rc::new(Timer::new(now)));
    let rt = TestRuntime::new(
        scheduler.clone(),
        clock.clone(),
        arp_options.clone(),
        udp_options.clone(),
        tcp_options.clone(),
        ALICE_MAC,
        ALICE_IPV4,
    );
    Engine::new(
        rt,
        clock.clone(),
        scheduler.clone(),
        ALICE_MAC,
        ALICE_IPV4,
        arp_options,
        udp_options,
        tcp_options,
    )
    .unwrap()
}

pub fn new_bob(now: Instant) -> Engine {
    let arp_options = ArpConfig::new(
        Some(Duration::from_secs(600)),
        Some(Duration::from_secs(1)),
        Some(2),
        Some(HashMap::new()),
        Some(false),
    );
    let udp_options = UdpConfig::default();
    let tcp_options = TcpConfig::default();
    let scheduler: Scheduler = Scheduler::default();
    let clock = TimerRc(Rc::new(Timer::new(now)));
    let rt = TestRuntime::new(
        scheduler.clone(),
        clock.clone(),
        arp_options.clone(),
        udp_options.clone(),
        tcp_options.clone(),
        BOB_MAC,
        BOB_IPV4,
    );
    Engine::new(
        rt,
        clock.clone(),
        scheduler.clone(),
        BOB_MAC,
        BOB_IPV4,
        arp_options,
        udp_options,
        tcp_options,
    )
    .unwrap()
}

pub fn new_alice2(now: Instant) -> Engine {
    let mut arp: HashMap<Ipv4Addr, MacAddress> = HashMap::<Ipv4Addr, MacAddress>::new();
    arp.insert(ALICE_IPV4, ALICE_MAC);
    arp.insert(BOB_IPV4, BOB_MAC);
    let arp_options = ArpConfig::new(
        Some(Duration::from_secs(600)),
        Some(Duration::from_secs(1)),
        Some(2),
        Some(arp),
        Some(false),
    );
    let udp_options = UdpConfig::default();
    let tcp_options = TcpConfig::default();
    let scheduler: Scheduler = Scheduler::default();
    let clock = TimerRc(Rc::new(Timer::new(now)));
    let rt = TestRuntime::new(
        scheduler.clone(),
        clock.clone(),
        arp_options.clone(),
        udp_options.clone(),
        tcp_options.clone(),
        ALICE_MAC,
        ALICE_IPV4,
    );
    Engine::new(
        rt,
        clock.clone(),
        scheduler.clone(),
        ALICE_MAC,
        ALICE_IPV4,
        arp_options,
        udp_options,
        tcp_options,
    )
    .unwrap()
}

pub fn new_bob2(now: Instant) -> Engine {
    let mut arp: HashMap<Ipv4Addr, MacAddress> = HashMap::<Ipv4Addr, MacAddress>::new();
    arp.insert(BOB_IPV4, BOB_MAC);
    arp.insert(ALICE_IPV4, ALICE_MAC);
    let arp_options = ArpConfig::new(
        Some(Duration::from_secs(600)),
        Some(Duration::from_secs(1)),
        Some(2),
        Some(arp),
        Some(false),
    );
    let udp_options = UdpConfig::default();
    let tcp_options = TcpConfig::default();
    let scheduler: Scheduler = Scheduler::default();
    let clock = TimerRc(Rc::new(Timer::new(now)));

    let rt = TestRuntime::new(
        scheduler.clone(),
        clock.clone(),
        arp_options.clone(),
        udp_options.clone(),
        tcp_options.clone(),
        BOB_MAC,
        BOB_IPV4,
    );
    Engine::new(
        rt,
        clock.clone(),
        scheduler.clone(),
        BOB_MAC,
        BOB_IPV4,
        arp_options,
        udp_options,
        tcp_options,
    )
    .unwrap()
}

pub fn new_carrie(now: Instant) -> Engine {
    let arp_options = ArpConfig::new(
        Some(Duration::from_secs(600)),
        Some(Duration::from_secs(1)),
        Some(2),
        Some(HashMap::new()),
        Some(false),
    );
    let udp_options = UdpConfig::default();
    let tcp_options = TcpConfig::default();
    let scheduler: Scheduler = Scheduler::default();
    let clock = TimerRc(Rc::new(Timer::new(now)));

    let rt = TestRuntime::new(
        scheduler.clone(),
        clock.clone(),
        arp_options.clone(),
        udp_options.clone(),
        tcp_options.clone(),
        CARRIE_MAC,
        CARRIE_IPV4,
    );
    Engine::new(
        rt,
        clock.clone(),
        scheduler.clone(),
        CARRIE_MAC,
        CARRIE_IPV4,
        arp_options,
        udp_options,
        tcp_options,
    )
    .unwrap()
}
