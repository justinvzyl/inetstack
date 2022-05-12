// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub mod established;
pub mod setup;
mod tcp_migration;

use crate::protocols::{
    ethernet2::{EtherType2, Ethernet2Header},
    ipv4::Ipv4Header,
    tcp::{segment::TcpHeader, SeqNumber},
};
use ::runtime::{memory::Bytes, network::types::MacAddress};
use ::std::net::Ipv4Addr;

/// Extracts headers of a TCP packet.
fn extract_headers(bytes: Bytes) -> (Ethernet2Header, Ipv4Header, TcpHeader, Bytes) {
    let (eth2_header, eth2_payload) = Ethernet2Header::parse(bytes).unwrap();
    let (ipv4_header, ipv4_payload) = Ipv4Header::parse(eth2_payload).unwrap();
    let (tcp_header, tcp_payload) = TcpHeader::parse(&ipv4_header, ipv4_payload, false).unwrap();

    (eth2_header, ipv4_header, tcp_header, tcp_payload)
}

/// Checks for a data packet.
pub fn check_packet_data(
    bytes: Bytes,
    eth2_src_addr: MacAddress,
    eth2_dst_addr: MacAddress,
    ipv4_src_addr: Ipv4Addr,
    ipv4_dst_addr: Ipv4Addr,
    window_size: u16,
    seq_num: SeqNumber,
    ack_num: Option<SeqNumber>,
) -> usize {
    let (eth2_header, ipv4_header, tcp_header, tcp_payload) = extract_headers(bytes);
    check_eth_and_ip_headers(
        (eth2_src_addr, eth2_dst_addr),
        (ipv4_src_addr, ipv4_dst_addr),
        eth2_header,
        ipv4_header,
    );

    assert_ne!(tcp_payload.len(), 0);
    assert_eq!(tcp_header.window_size, window_size);
    assert_eq!(tcp_header.seq_num, seq_num);
    if let Some(ack_num) = ack_num {
        assert_eq!(tcp_header.ack, true);
        assert_eq!(tcp_header.ack_num, ack_num);
    }

    tcp_payload.len()
}

/// Checks for a pure ACK packet.
/// TODO: Perhaps rename this, as the term "pure ACK" isn't normally used to describe anything in TCP.  The original
/// version of this function compared the header sequence number field to zero (as if it wasn't set to anything),
/// which is incorrect (i.e. it was checking for incorrect behavior).  For an established connection, the current
/// sequence number should always reflect the current SND.NXT (send next).
pub fn check_packet_pure_ack(
    bytes: Bytes,
    eth2_src_addr: MacAddress,
    eth2_dst_addr: MacAddress,
    ipv4_src_addr: Ipv4Addr,
    ipv4_dst_addr: Ipv4Addr,
    window_size: u16,
    ack_num: SeqNumber,
) {
    let (eth2_header, ipv4_header, tcp_header, tcp_payload) = extract_headers(bytes);
    check_eth_and_ip_headers(
        (eth2_src_addr, eth2_dst_addr),
        (ipv4_src_addr, ipv4_dst_addr),
        eth2_header,
        ipv4_header,
    );

    assert_eq!(tcp_payload.len(), 0);
    assert_eq!(tcp_header.window_size, window_size);
    assert_eq!(tcp_header.ack, true);
    assert_eq!(tcp_header.ack_num, ack_num);
}

fn check_eth_and_ip_headers(
    eth2_addrs: (MacAddress, MacAddress),
    ipv4_addrs: (Ipv4Addr, Ipv4Addr),
    eth2_header: Ethernet2Header,
    ipv4_header: Ipv4Header,
) {
    assert_eq!(eth2_header.src_addr(), eth2_addrs.0);
    // Client sending to origin's MAC address instead of new-dest MAC address. This is okay.
    // assert_eq!(eth2_header.dst_addr(), eth2_addrs.1);
    assert_eq!(eth2_header.ether_type(), EtherType2::Ipv4);

    // assert_eq!(ipv4_header.src_addr(), ipv4_addrs.0);
    // assert_eq!(ipv4_header.dst_addr(), ipv4_addrs.1);
}
