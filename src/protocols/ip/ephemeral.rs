// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use ::libc::EAGAIN;
use ::runtime::{
    fail::Fail,
    Runtime,
};
use ::std::convert::TryFrom;

//==============================================================================
// Constants
//==============================================================================

const FIRST_PRIVATE_PORT: u16 = 49152;
const LAST_PRIVATE_PORT: u16 = 65535;

//==============================================================================
// Structures
//==============================================================================

pub struct EphemeralPorts {
    ports: Vec<u16>,
}

//==============================================================================
// Associate Functions
//==============================================================================

impl EphemeralPorts {
    pub fn new<RT: Runtime>(rt: &RT) -> Self {
        let mut ports: Vec<u16> = Vec::<u16>::new();
        for port in FIRST_PRIVATE_PORT..LAST_PRIVATE_PORT {
            ports.push(port);
        }

        rt.rng_shuffle(&mut ports[..]);
        Self { ports }
    }

    pub fn first_private_port() -> u16 {
        u16::try_from(FIRST_PRIVATE_PORT).unwrap()
    }

    pub fn is_private(port: u16) -> bool {
        u16::from(port) >= FIRST_PRIVATE_PORT
    }

    pub fn alloc(&mut self) -> Result<u16, Fail> {
        self.ports.pop().ok_or(Fail::new(EAGAIN, "out of private ports"))
    }

    pub fn free(&mut self, port: u16) {
        self.ports.push(port);
    }
}
