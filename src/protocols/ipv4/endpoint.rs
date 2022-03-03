// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use ::runtime::network::types::{Ipv4Addr, Port16};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ipv4Endpoint {
    addr: Ipv4Addr,
    port: Port16,
}

/// Associate functions.
impl Ipv4Endpoint {
    /// Constructs a new [Ipv4Endpoint].
    pub fn new(addr: Ipv4Addr, port: Port16) -> Ipv4Endpoint {
        Ipv4Endpoint { addr, port }
    }

    /// Returns the [Ipv4Addr] associated to the target [Ipv4Endpoint].
    pub fn get_address(&self) -> Ipv4Addr {
        self.addr
    }

    /// Returns the [ip::Port] associated to the target [Ipv4Endpoint].
    pub fn get_port(&self) -> Port16 {
        self.port
    }
}
