// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use ::libc::ENOTSUP;
use ::num_traits::FromPrimitive;
use ::runtime::fail::Fail;
use ::std::convert::TryFrom;

//==============================================================================
// Structures
//==============================================================================

/// Ipv4 Protocol
/// TODO: Move this to IP layer, because it is common for IPv4 and IPv6.
#[repr(u8)]
#[derive(FromPrimitive, Copy, Clone, PartialEq, Eq, Debug)]
pub enum Ipv4Protocol {
    /// Internet Control Message Protocol
    ICMPv4 = 0x01,
    /// Transmission Control Protocol
    TCP = 0x06,
    /// User Datagram Protocol
    UDP = 0x11,
}

//==============================================================================
// Trait Implementations
//==============================================================================

/// TryFrom trait implementation.
impl TryFrom<u8> for Ipv4Protocol {
    type Error = Fail;

    fn try_from(n: u8) -> Result<Self, Fail> {
        match FromPrimitive::from_u8(n) {
            Some(n) => Ok(n),
            None => Err(Fail::new(ENOTSUP, "unsupported IPv4 protocol")),
        }
    }
}
