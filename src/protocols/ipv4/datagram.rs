// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use crate::protocols::ipv4::Ipv4Protocol;
use ::byteorder::{ByteOrder, NetworkEndian};
use ::libc::{EBADMSG, ENOTSUP};
use ::runtime::{fail::Fail, memory::Buffer};
use ::std::{
    convert::{TryFrom, TryInto},
    net::Ipv4Addr,
};

//==============================================================================
// Constants
//==============================================================================

/// Size of IPv4 Headers (in bytes)
pub const IPV4_HEADER_SIZE: usize = 20;

/// Default time to live value.
const DEFAULT_IPV4_TTL: u8 = 255;

/// IPv4 header length when no options are used (in 32-bit words).
const IPV4_IHL_NO_OPTIONS: u8 = 5;

/// Minimum size for an IPv4 datagram (in bytes).
const IPV4_MIN_SIZE: u16 = 20;

/// Version number for IPv4.
const IPV4_VERSION: u8 = 4;

//==============================================================================
// Structures
//==============================================================================

/// IPv4 Datagram Header
#[derive(Debug, Copy, Clone)]
pub struct Ipv4Header {
    /// Internet header version (4 bits).
    version: u8,
    /// Internet Header Length. (4 bits).
    ihl: u8,
    /// Differentiated Services Code Point (6 bits).
    dscp: u8,
    /// Explicit Congestion Notification (2 bits).
    ecn: u8,
    /// Total length of the packet including header and data (16 bits).
    total_length: u16,
    /// Identifies the to which datagram a fragment belongs to (16 bits).
    identification: u16,
    /// Version control flags (3 bits).
    flags: u8,
    /// Fragment offset indicates where in the datagram this fragment belongs to (16 bits).
    fragment_offset: u16,
    /// Time to Live indicates the maximum remaining time the datagram is allowed to be in the network (8 bits).
    ttl: u8,
    /// Protocol used in the data portion of the datagram (8 bits).
    protocol: Ipv4Protocol,
    /// Header-only checksum for error detection (16 bits).
    header_checksum: u16,
    // Source IP address (32 bits).
    src_addr: Ipv4Addr,
    /// Destination IP address (32 bits).
    dst_addr: Ipv4Addr,
}

//==============================================================================
// Associated Functions
//==============================================================================

/// Associated Functions for IPv4 Headers
impl Ipv4Header {
    /// Instantiates an empty IPv4 header.
    pub fn new(src_addr: Ipv4Addr, dst_addr: Ipv4Addr, protocol: Ipv4Protocol) -> Self {
        Self {
            version: IPV4_VERSION,
            ihl: IPV4_IHL_NO_OPTIONS,
            dscp: 0,
            ecn: 0,
            total_length: IPV4_MIN_SIZE,
            identification: 0,
            flags: 0,
            fragment_offset: 0,
            ttl: DEFAULT_IPV4_TTL,
            protocol,
            header_checksum: 0,
            src_addr,
            dst_addr,
        }
    }

    /// Computes the size of the target IPv4 header.
    pub fn compute_size(&self) -> usize {
        // We don't support IPv4 options, so this is always 20.
        IPV4_HEADER_SIZE
    }

    /// Parses a buffer into an IPv4 header and payload.
    pub fn parse<T: Buffer>(mut buf: T) -> Result<(Self, T), Fail> {
        if buf.len() < IPV4_HEADER_SIZE {
            return Err(Fail::new(EBADMSG, "datagram too small"));
        }
        let hdr_buf: &[u8] = &buf[..IPV4_HEADER_SIZE];

        let version: u8 = hdr_buf[0] >> 4;
        if version != IPV4_VERSION {
            return Err(Fail::new(ENOTSUP, "unsupported IP version"));
        }

        let ihl: u8 = hdr_buf[0] & 0xF;
        if ihl < IPV4_IHL_NO_OPTIONS {
            return Err(Fail::new(EBADMSG, "IPv4 IHL is too small"));
        }
        if ihl > IPV4_IHL_NO_OPTIONS {
            return Err(Fail::new(ENOTSUP, "IPv4 options are not supported"));
        }

        let dscp: u8 = hdr_buf[1] >> 2;
        let ecn: u8 = hdr_buf[1] & 3;

        let total_length: u16 = NetworkEndian::read_u16(&hdr_buf[2..4]);

        // The TOTALLEN is definitely malformed if it doesn't have room for our header.
        if (total_length as usize) < IPV4_HEADER_SIZE {
            return Err(Fail::new(EBADMSG, "IPv4 TOTALLEN smaller than header"));
        }
        if (total_length as usize) > buf.len() {
            return Err(Fail::new(
                EBADMSG,
                "IPv4 TOTALLEN greater than header + payload",
            ));
        }

        let identification: u16 = NetworkEndian::read_u16(&hdr_buf[4..6]);
        let flags: u8 = (NetworkEndian::read_u16(&hdr_buf[6..8]) >> 13) as u8;

        let fragment_offset: u16 = NetworkEndian::read_u16(&hdr_buf[6..8]) & 0x1fff;
        if fragment_offset != 0 {
            return Err(Fail::new(ENOTSUP, "IPv4 fragmentation is unsupported"));
        }

        let time_to_live: u8 = hdr_buf[8];
        let protocol: Ipv4Protocol = Ipv4Protocol::try_from(hdr_buf[9])?;

        let header_checksum: u16 = NetworkEndian::read_u16(&hdr_buf[10..12]);
        if header_checksum == 0xffff {
            return Err(Fail::new(EBADMSG, "IPv4 checksum is 0xFFFF"));
        }
        if header_checksum != Self::compute_checksum(hdr_buf) {
            return Err(Fail::new(EBADMSG, "Invalid IPv4 checksum"));
        }

        let src_addr: Ipv4Addr = Ipv4Addr::from(NetworkEndian::read_u32(&hdr_buf[12..16]));
        let dst_addr: Ipv4Addr = Ipv4Addr::from(NetworkEndian::read_u32(&hdr_buf[16..20]));

        // NB (sujayakar, 11/6/2020): I've noticed that Ethernet transmission is liable to add
        // padding zeros for small payloads, so we can't assert that the Ethernet payload we
        // receives exactly matches the header's TOTALLEN. Therefore, we may need to truncate off
        // padding bytes when they don't line up.
        let padding_bytes = buf.len() - (total_length as usize);
        buf.adjust(IPV4_HEADER_SIZE);
        buf.trim(padding_bytes);

        let header = Self {
            version,
            ihl,
            dscp,
            ecn,
            total_length,
            identification,
            flags,
            fragment_offset,
            ttl: time_to_live,
            protocol,
            header_checksum,
            src_addr,
            dst_addr,
        };
        Ok((header, buf))
    }

    /// Serializes the target IPv4 header.
    pub fn serialize(&self, buf: &mut [u8], payload_len: usize) {
        let buf: &mut [u8; IPV4_HEADER_SIZE] = buf.try_into().unwrap();
        buf[0] = (IPV4_VERSION << 4) | IPV4_IHL_NO_OPTIONS;
        buf[1] = (self.dscp << 2) | (self.ecn & 3);
        NetworkEndian::write_u16(&mut buf[2..4], (IPV4_HEADER_SIZE + payload_len) as u16);
        NetworkEndian::write_u16(&mut buf[4..6], self.identification);
        NetworkEndian::write_u16(
            &mut buf[6..8],
            (self.flags as u16) << 13 | self.fragment_offset & 0x1fff,
        );
        buf[8] = self.ttl;
        buf[9] = self.protocol as u8;

        // Skip the checksum (bytes 10..12) until we finish writing the header.
        buf[12..16].copy_from_slice(&self.src_addr.octets());
        buf[16..20].copy_from_slice(&self.dst_addr.octets());

        let checksum: u16 = Self::compute_checksum(buf);
        NetworkEndian::write_u16(&mut buf[10..12], checksum);
    }

    /// Returns the source address field stored in the target IPv4 header.
    pub fn get_src_addr(&self) -> Ipv4Addr {
        self.src_addr
    }

    /// Returns the destination address field stored in the target IPv4 header.
    pub fn get_dest_addr(&self) -> Ipv4Addr {
        self.dst_addr
    }

    /// Returns the protocol field stored in the target IPv4 header.
    pub fn get_protocol(&self) -> Ipv4Protocol {
        self.protocol
    }

    /// Computes the checksum of the target IPv4 header.
    fn compute_checksum(buf: &[u8]) -> u16 {
        let buf: &[u8; IPV4_HEADER_SIZE] = buf.try_into().expect("Invalid header size");
        let mut state: u32 = 0xffffu32;
        for i in 0..5 {
            state += NetworkEndian::read_u16(&buf[(2 * i)..(2 * i + 2)]) as u32;
        }
        // Skip the 5th u16 since octets 10-12 are the header checksum, whose value should be zero when
        // computing a checksum.
        for i in 6..10 {
            state += NetworkEndian::read_u16(&buf[(2 * i)..(2 * i + 2)]) as u32;
        }
        while state > 0xffff {
            state -= 0xffff;
        }
        !state as u16
    }
}
