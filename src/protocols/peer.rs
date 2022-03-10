// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::protocols::{
    arp::ArpPeer,
    icmpv4::Icmpv4Peer,
    ipv4::{Ipv4Header, Ipv4Protocol2},
    tcp::TcpPeer,
    udp::UdpPeer,
};
use ::libc::ENOTCONN;
use ::runtime::{fail::Fail, network::types::MacAddress, Runtime};
use ::std::{future::Future, net::Ipv4Addr, time::Duration};

#[cfg(test)]
use runtime::QDesc;

pub struct Peer<RT: Runtime> {
    rt: RT,
    icmpv4: Icmpv4Peer<RT>,
    pub tcp: TcpPeer<RT>,
    pub udp: UdpPeer<RT>,
}

impl<RT: Runtime> Peer<RT> {
    pub fn new(rt: RT, arp: ArpPeer<RT>) -> Peer<RT> {
        let local_link_addr: MacAddress = rt.local_link_addr();
        let local_ipv4_addr: Ipv4Addr = rt.local_ipv4_addr();
        let udp_offload_checksum: bool = rt.udp_options().get_tx_checksum_offload();
        let udp = UdpPeer::new(
            rt.clone(),
            local_link_addr,
            local_ipv4_addr,
            udp_offload_checksum,
            arp.clone(),
        );
        let icmpv4 = Icmpv4Peer::new(rt.clone(), arp.clone());
        let tcp = TcpPeer::new(rt.clone(), arp);
        Peer {
            rt,
            icmpv4,
            tcp,
            udp,
        }
    }

    pub fn receive(&mut self, buf: RT::Buf) -> Result<(), Fail> {
        let (header, payload) = Ipv4Header::parse(buf)?;
        debug!("Ipv4 received {:?}", header);
        if header.dst_addr() != self.rt.local_ipv4_addr() && !header.dst_addr().is_broadcast() {
            return Err(Fail::new(ENOTCONN, "invalid destination address"));
        }
        match header.protocol() {
            Ipv4Protocol2::Icmpv4 => self.icmpv4.receive(&header, payload),
            Ipv4Protocol2::Tcp => self.tcp.receive(&header, payload),
            Ipv4Protocol2::Udp => self.udp.do_receive(&header, payload),
        }
    }

    pub fn ping(
        &mut self,
        dest_ipv4_addr: Ipv4Addr,
        timeout: Option<Duration>,
    ) -> impl Future<Output = Result<Duration, Fail>> {
        self.icmpv4.ping(dest_ipv4_addr, timeout)
    }
}

#[cfg(test)]
impl<RT: Runtime> Peer<RT> {
    pub fn tcp_mss(&self, fd: QDesc) -> Result<usize, Fail> {
        self.tcp.remote_mss(fd)
    }

    pub fn tcp_rto(&self, fd: QDesc) -> Result<Duration, Fail> {
        self.tcp.current_rto(fd)
    }
}
