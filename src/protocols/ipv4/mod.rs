// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod datagram;
mod endpoint;
mod protocol;

#[cfg(test)]
mod tests;

//==============================================================================
// Exports
//==============================================================================

pub use self::{
    datagram::{Ipv4Header, IPV4_HEADER_SIZE},
    endpoint::Ipv4Endpoint,
    protocol::Ipv4Protocol,
};
