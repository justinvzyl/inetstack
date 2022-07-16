// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::protocols::tcp::SeqNumber;
#[allow(unused_imports)]
use std::{
    hash::Hasher,
    net::SocketAddr,
    num::Wrapping,
};

#[allow(dead_code)]
pub struct IsnGenerator {
    nonce: u32,
    counter: Wrapping<u16>,
}

impl IsnGenerator {
    pub fn new(nonce: u32) -> Self {
        Self {
            nonce,
            counter: Wrapping(0),
        }
    }

    #[cfg(test)]
    pub fn generate(&mut self, _local: &SocketAddr, _remote: &SocketAddr) -> SeqNumber {
        SeqNumber::from(0)
    }

    #[cfg(not(test))]
    pub fn generate(&mut self, local: &SocketAddr, remote: &SocketAddr) -> SeqNumber {
        use std::net::IpAddr;

        let crc: crc::Crc<u32> = crc::Crc::<u32>::new(&crc::CRC_32_CKSUM);
        let mut digest = crc.digest();
        let remote_octets = match remote.ip() {
            // todo: pass by reference?
            IpAddr::V4(ipv4) => ipv4.octets(),
            IpAddr::V6(_) => todo!(),
        };
        digest.update(&remote_octets);
        let remote_port: u16 = remote.port().into();
        digest.update(&remote_port.to_be_bytes());
        let local_octets = match local.ip() {
            // todo: pass by reference?
            IpAddr::V4(ipv4) => ipv4.octets(),
            IpAddr::V6(_) => todo!(),
        };
        digest.update(&local_octets);
        let local_port: u16 = local.port().into();
        digest.update(&local_port.to_be_bytes());
        digest.update(&self.nonce.to_be_bytes());
        let digest = digest.finalize();
        let isn = SeqNumber::from(digest + self.counter.0 as u32);
        self.counter += Wrapping(1);
        isn
    }
}
