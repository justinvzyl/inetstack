// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Defines functions to be called during the TCP connection termination process.

use super::{ControlBlock, State};
use crate::protocols::tcp::SeqNumber;
use ::futures::FutureExt;
use ::runtime::{fail::Fail, memory::Buffer, Runtime};
use ::std::rc::Rc;

//==============================================================================

// ToDo: Eliminate this function as it is extremely inefficient.  The FIN flag should be set on the last data packet
// sent to our peer, not sent as a separate packet.  Conceptually, and in sequence number terms, the FIN comes after
// the last byte of data we send to our peer on this connection.  The code that sends that last unsent data knows it is
// doing that (we should have a flag in the Control Block indicating that the user has called close()), and can simply
// set the FIN flag on that last data packet.  And if there is no outstanding unsent data still to send when the user
// calls close, we can immediately send a FIN (the send_ack routine should handle this).  And since the sending of this
// FIN can be handled by the regular send mechanism, it will also set the retransmission timer appropriately so we
// don't need to worry about the ToDo below.  There is no point to having this function at all (it also looks buggy).
//
async fn active_send_fin<RT: Runtime>(cb: Rc<ControlBlock<RT>>) -> Result<!, Fail> {
    loop {
        let (st, st_changed) = cb.get_state();

        // Wait until we receive a FIN.
        if st != State::ActiveClose {
            st_changed.await;
            continue;
        }

        // Wait for `sent_seq_no` to catch up to `unsent_seq_no` and
        // then send a FIN segment.
        let (sent_seq, sent_seq_changed) = cb.get_sent_seq_no();
        let (unsent_seq, _) = cb.get_unsent_seq_no();
        if sent_seq != unsent_seq {
            sent_seq_changed.await;
            continue;
        }

        // TODO: When do we retransmit this?
        let remote_link_addr = cb.arp().query(cb.get_remote().get_address()).await?;
        let mut header = cb.tcp_header();
        header.seq_num = sent_seq;
        header.fin = true;
        cb.emit(header, RT::Buf::empty(), remote_link_addr);

        cb.set_state(State::FinWait1);
    }
}

//==============================================================================

// ToDo: Eliminate this function as it is simply crazy time.  As currently implemented, it is waiting for our state
// to become one of three non-existant states (in the TCP spec at least).  Then it is waiting for ourselves to
// acknowledge receipt of all the data we've received from our peer, just so we can acknowledge receipt of a FIN from
// our peer.  If we wanted to fix this function, there is no need to wait for another part of ourselves to send an ACK,
// as we could simply ACK the remaining data and the FIN together in a single packet.  But there is no need for this
// function at all, as we can simply send the appropriate ACK for the FIN when we receive it.
//
async fn active_ack_fin<RT: Runtime>(cb: Rc<ControlBlock<RT>>) -> Result<!, Fail> {
    loop {
        let (st, st_changed) = cb.get_state();

        if st != State::FinWait3 && st != State::TimeWait1 && st != State::Closing1 {
            st_changed.await;
            continue;
        }

        // Wait for all data to be acknowledged.
        let (ack_seq, ack_seq_changed) = cb.get_ack_seq_no();
        let (recv_seq, _) = cb.get_recv_seq_no();
        if ack_seq != recv_seq {
            ack_seq_changed.await;
            continue;
        }

        // Send ACK segment for FIN.
        let remote_link_addr = cb.arp().query(cb.get_remote().get_address()).await?;
        let mut header = cb.tcp_header();

        // ACK replies to FIN are special as their ack sequence number should be set to +1 the
        // received seq number even though there is no payload.
        header.ack = true;
        header.ack_num = recv_seq + SeqNumber::from(1);
        cb.emit(header, RT::Buf::empty(), remote_link_addr);

        if st == State::Closing1 {
            cb.set_state(State::Closing2)
        } else {
            cb.set_state(State::TimeWait2);
        }
    }
}

//==============================================================================

// ToDo: Figure out what this function is supposed to be doing, and either fix it or elminate the need for it.
// Right now this function is waiting for our state to become the (non-existant in the spec) TimeWait2 state,
// and aborts the connection when that happens.  Which of course the code that sets our state to the TimeWait2 state
// could simply do itself.
//
// The ToDo line below hints at something that needs to be done somewhere, however.  And that is to have a 2 MSL timer
// keeping our five tuple out of action for twice the maximum segment lifetime.  It appears nothing currently does that.
//

/// Awaits until connection terminates by our four-way handshake.
async fn active_wait_2msl<RT: Runtime>(cb: Rc<ControlBlock<RT>>) -> Result<!, Fail> {
    loop {
        let (st, st_changed) = cb.get_state();

        if st != State::TimeWait2 {
            st_changed.await;
            continue;
        }

        // TODO: Wait for 2*MSL if active close.
        return Err(Fail::ConnectionAborted {});
    }
}

//==============================================================================

// ToDo: Eliminate this function for basically the same reasons that we should eliminate active_ack_fin above.
//
async fn passive_close<RT: Runtime>(cb: Rc<ControlBlock<RT>>) -> Result<!, Fail> {
    loop {
        let (st, st_changed) = cb.get_state();

        // Wait until we receive a FIN.
        if st != State::PassiveClose {
            st_changed.await;
            continue;
        }

        // Wait for all data to be acknowledged.
        let (ack_seq, ack_seq_changed) = cb.get_ack_seq_no();
        let (recv_seq, _) = cb.get_recv_seq_no();
        if ack_seq != recv_seq {
            ack_seq_changed.await;
            continue;
        }

        // Send ACK segment for FIN.
        let remote_link_addr = cb.arp().query(cb.get_remote().get_address()).await?;
        let mut header = cb.tcp_header();
        header.ack = true;
        header.ack_num = recv_seq + SeqNumber::from(1);
        cb.emit(header, RT::Buf::empty(), remote_link_addr);

        cb.set_state(State::CloseWait1);
    }
}

//==============================================================================

// ToDo: Eliminate this function for basically the same reasons that we should eliminate active_send_fin above.  It
// also appears to be waiting for us to enter the wrong state, but maybe that's what this bogus CloseWait2 state is.
//
async fn passive_send_fin<RT: Runtime>(cb: Rc<ControlBlock<RT>>) -> Result<!, Fail> {
    loop {
        let (st, st_changed) = cb.get_state();

        if st != State::CloseWait2 {
            st_changed.await;
            continue;
        }

        // Wait for `sent_seq_no` to catch up to `unsent_seq_no` and then send a FIN segment.
        let (sent_seq, sent_seq_changed) = cb.get_sent_seq_no();
        let (unsent_seq, _) = cb.get_unsent_seq_no();
        if sent_seq != unsent_seq {
            sent_seq_changed.await;
            continue;
        }

        let remote_link_addr = cb.arp().query(cb.get_remote().get_address()).await?;
        let mut header = cb.tcp_header();
        header.seq_num = sent_seq;
        header.fin = true;
        cb.emit(header, RT::Buf::empty(), remote_link_addr);

        cb.set_state(State::LastAck);
    }
}

//==============================================================================

// ToDo: Eliminate this function for basically the same reasons that we should eliminate active_wait_2msl above.
//
async fn passive_wait_fin_ack<RT: Runtime>(cb: Rc<ControlBlock<RT>>) -> Result<!, Fail> {
    loop {
        let (st, st_changed) = cb.get_state();
        if st != State::Closed {
            st_changed.await;
            continue;
        }

        return Err(Fail::ConnectionAborted {});
    }
}

// ToDo: Eliminate this function after eliminating all the above closer closures as separately noted above, as there
// will no longer be any need for it.
//

/// Launches various closures having to do with connection termination. Neither `active_ack_fin`
/// nor `active_send_fin` terminate so the only way to return is via `active_wait_2msl`.
pub async fn connection_terminated<RT: Runtime>(cb: Rc<ControlBlock<RT>>) -> Result<!, Fail> {
    futures::select_biased! {
        r = active_send_fin(cb.clone()).fuse() => r,
        r = active_ack_fin(cb.clone()).fuse() => r,
        r = active_wait_2msl(cb.clone()).fuse() => r,
        r = passive_close(cb.clone()).fuse() => r,
        r = passive_send_fin(cb.clone()).fuse() => r,
        r = passive_wait_fin_ack(cb.clone()).fuse() => r,
    }
}
