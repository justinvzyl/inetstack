// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use ::runtime::{
    fail::Fail,
    network::NetworkRuntime,
    task::SchedulerRuntime,
    utils::UtilsRuntime,
};

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
    pub fn new<RT: SchedulerRuntime + UtilsRuntime + NetworkRuntime + Clone + 'static>(rt: &RT) -> Self {
        let mut ports: Vec<u16> = Vec::<u16>::new();
        for port in FIRST_PRIVATE_PORT..LAST_PRIVATE_PORT {
            ports.push(port);
        }

        rt.rng_shuffle(&mut ports[..]);
        Self { ports }
    }

    pub fn first_private_port() -> u16 {
        FIRST_PRIVATE_PORT
    }

    pub fn is_private(port: u16) -> bool {
        port >= FIRST_PRIVATE_PORT
    }

    pub fn alloc_any(&mut self) -> Result<u16, Fail> {
        self.ports.pop().ok_or(Fail::new(
            libc::EADDRINUSE,
            "all port numbers in the ephemeral port range are currently in use",
        ))
    }

    pub fn free(&mut self, port: u16) {
        self.ports.push(port);
    }
}
