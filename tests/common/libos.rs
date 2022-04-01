// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use super::runtime::DummyRuntime;
use ::catnip::Catnip;
use ::crossbeam_channel::{Receiver, Sender};
use ::runtime::{
    logging,
    memory::{Bytes, BytesMut},
    network::types::MacAddress,
};
use ::std::{collections::HashMap, net::Ipv4Addr, time::Instant};

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
        tx: Sender<Bytes>,
        rx: Receiver<Bytes>,
        arp: HashMap<Ipv4Addr, MacAddress>,
    ) -> Catnip<DummyRuntime> {
        let now: Instant = Instant::now();
        let rt: DummyRuntime = DummyRuntime::new(now, link_addr, ipv4_addr, rx, tx, arp);
        logging::initialize();
        Catnip::new(rt).unwrap()
    }

    /// Cooks a buffer.
    pub fn cook_data(size: usize) -> Bytes {
        let fill_char: u8 = b'a';

        let mut buf: BytesMut = BytesMut::zeroed(size).unwrap();
        for a in &mut buf[..] {
            *a = fill_char;
        }
        buf.freeze()
    }
}
