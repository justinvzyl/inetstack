// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::runtime::DummyRuntime;
use ::catnip::Catnip;
use ::crossbeam_channel::{Receiver, Sender};
use ::runtime::{
    logging,
    memory::{Bytes, BytesMut, MemoryRuntime},
    network::types::MacAddress,
    types::dmtr_sgarray_t,
};
use ::std::{collections::HashMap, net::Ipv4Addr, time::Instant};

//==============================================================================
// Constants & Structures
//==============================================================================

pub struct DummyLibOS {}

impl DummyLibOS {
    /// Initializes the libOS.
    pub fn new(
        link_addr: MacAddress,
        ipv4_addr: Ipv4Addr,
        tx: Sender<Bytes>,
        rx: Receiver<Bytes>,
        arp: HashMap<Ipv4Addr, MacAddress>,
    ) -> Catnip<DummyRuntime> {
        let now = Instant::now();
        let rt = DummyRuntime::new(now, link_addr, ipv4_addr, rx, tx, arp);
        logging::initialize();
        Catnip::new(rt).unwrap()
    }

    /// Cooks a SGA buffer.
    pub fn cook_data(libos: &mut Catnip<DummyRuntime>, size: usize) -> dmtr_sgarray_t {
        let fill_char = b'a';

        let mut buf = BytesMut::zeroed(size).unwrap();
        for a in &mut buf[..] {
            *a = fill_char;
        }
        match libos.rt().into_sgarray(buf.freeze()) {
            Ok(sga) => sga,
            Err(e) => panic!("{:?}", e),
        }
    }

    /// Verifies the integrity of a buffer.
    pub fn check_data(sga: dmtr_sgarray_t) {
        assert_eq!(sga.sga_numsegs, 1);
        assert_eq!(sga.sga_segs[0].sgaseg_len, 32 as u32);
    }
}
