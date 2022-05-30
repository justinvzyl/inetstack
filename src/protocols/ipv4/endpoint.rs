// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use ::runtime::network::types::Ipv4Addr;

//==============================================================================
// Structures
//==============================================================================

/// IPv4 Endpoint
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ipv4Endpoint {
    /// IPv4 address.
    addr: Ipv4Addr,
    /// Port number.
    port: u16,
}

//==============================================================================
// Associate Functions
//==============================================================================

/// Associate functions.
impl Ipv4Endpoint {
    /// Constructs a new [Ipv4Endpoint].
    pub fn new(addr: Ipv4Addr, port: u16) -> Ipv4Endpoint {
        Ipv4Endpoint { addr, port }
    }

    /// Returns the [Ipv4Addr] associated to the target [Ipv4Endpoint].
    pub fn get_address(&self) -> Ipv4Addr {
        self.addr
    }

    /// Returns the [ip::Port] associated to the target [Ipv4Endpoint].
    pub fn get_port(&self) -> u16 {
        self.port
    }
}
