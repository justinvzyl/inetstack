// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod datagram;
mod endpoint;

#[cfg(test)]
mod tests;

//==============================================================================
// Exports
//==============================================================================

pub use self::{
    datagram::{
        Ipv4Header,
        IPV4_HEADER_DEFAULT_SIZE,
    },
    endpoint::Ipv4Endpoint,
};
