// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use super::runtime::DummyRuntime;
use ::crossbeam_channel::{
    Receiver,
    Sender,
};
use ::inetstack::InetStack;
use ::runtime::{
    logging,
    memory::{
        Buffer,
        DataBuffer,
    },
    network::{
        config::ArpConfig,
        types::MacAddress,
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

//==============================================================================
// Structures
//==============================================================================

pub struct DummyLibOS {}

//==============================================================================
// Associated Functons
//==============================================================================

impl DummyLibOS {
    /// Initializes the libOS.
    pub fn new(
        link_addr: MacAddress,
        ipv4_addr: Ipv4Addr,
        tx: Sender<DataBuffer>,
        rx: Receiver<DataBuffer>,
        arp: HashMap<Ipv4Addr, MacAddress>,
    ) -> InetStack<DummyRuntime> {
        let now: Instant = Instant::now();
        let arp_options: ArpConfig = ArpConfig::new(
            Some(Duration::from_secs(600)),
            Some(Duration::from_secs(1)),
            Some(2),
            Some(arp),
            Some(false),
        );
        let rt: DummyRuntime = DummyRuntime::new(now, link_addr, ipv4_addr, rx, tx, arp_options.clone());
        logging::initialize();
        InetStack::new(rt, link_addr, ipv4_addr, arp_options).unwrap()
    }

    /// Cooks a buffer.
    pub fn cook_data(size: usize) -> Box<dyn Buffer> {
        let fill_char: u8 = b'a';

        let mut buf: Box<dyn Buffer> = Box::new(DataBuffer::new(size).unwrap());
        for a in &mut buf[..] {
            *a = fill_char;
        }
        buf
    }
}
