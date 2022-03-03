// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use ::runtime::network::types::Port16;
use ::runtime::{fail::Fail, Runtime};
use ::std::convert::TryFrom;
use ::std::num::NonZeroU16;

//==============================================================================
// Constants
//==============================================================================

const FIRST_PRIVATE_PORT: u16 = 49152;
const LAST_PRIVATE_PORT: u16 = 65535;

//==============================================================================
// Structures
//==============================================================================

pub struct EphemeralPorts {
    ports: Vec<Port16>,
}

//==============================================================================
// Associate Functions
//==============================================================================

impl EphemeralPorts {
    pub fn new<RT: Runtime>(rt: &RT) -> Self {
        let mut ports: Vec<Port16> = Vec::<Port16>::new();
        for n in FIRST_PRIVATE_PORT..LAST_PRIVATE_PORT {
            let p: Port16 =
                Port16::new(NonZeroU16::new(n).expect("failed to allocate ephemeral port"));
            ports.push(p);
        }

        rt.rng_shuffle(&mut ports[..]);
        Self { ports }
    }

    pub fn first_private_port() -> Port16 {
        Port16::try_from(FIRST_PRIVATE_PORT).unwrap()
    }

    pub fn is_private(port: Port16) -> bool {
        u16::from(port) >= FIRST_PRIVATE_PORT
    }

    pub fn alloc(&mut self) -> Result<Port16, Fail> {
        self.ports.pop().ok_or(Fail::ResourceExhausted {
            details: "Out of private ports",
        })
    }

    pub fn free(&mut self, port: Port16) {
        self.ports.push(port);
    }
}
