// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    CongestionControl,
    FastRetransmitRecovery,
    LimitedTransmit,
    Options,
    SlowStartCongestionAvoidance,
};
use crate::protocols::tcp::SeqNumber;
use ::runtime::network::NetworkRuntime;
use ::std::fmt::Debug;

// Implementation of congestion control which does nothing.
#[derive(Debug)]
pub struct None {}

impl<RT: NetworkRuntime> CongestionControl<RT> for None {
    fn new(_mss: usize, _seq_no: SeqNumber, _options: Option<Options>) -> Box<dyn CongestionControl<RT>> {
        Box::new(Self {})
    }
}

impl<RT: NetworkRuntime> SlowStartCongestionAvoidance<RT> for None {}
impl<RT: NetworkRuntime> FastRetransmitRecovery<RT> for None {}
impl<RT: NetworkRuntime> LimitedTransmit<RT> for None {}
