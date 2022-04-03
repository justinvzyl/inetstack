// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    sender::congestion_ctrl,
    sender::Sender,
    sender::{congestion_ctrl::CongestionControlConstructor, UnackedSegment},
};
use crate::protocols::{
    arp::ArpPeer,
    ethernet2::{EtherType2, Ethernet2Header},
    ip::IpProtocol,
    ipv4::Ipv4Endpoint,
    ipv4::Ipv4Header,
    tcp::{
        segment::{TcpHeader, TcpSegment},
        SeqNumber,
    },
};
use ::libc::{EBADMSG, ENOMEM};
use ::runtime::{
    fail::Fail,
    memory::Buffer,
    network::types::MacAddress,
    watched::{WatchFuture, WatchedValue},
    Runtime,
};
use ::std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    convert::TryInto,
    rc::Rc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

// ToDo: Review this value (and its purpose).  It (2048 segments) of 8 KB jumbo packets would limit the unread data to
// just 16 MB.  If we don't want to lie, that is also about the max window size we should ever advertise.  Whereas TCP
// with the window scale option allows for window sizes of up to 1 GB.  This value appears to exist more because of the
// mechanism used to manage the receive queue (a VecDeque) than anything else.
const RECV_QUEUE_SZ: usize = 2048;

// ToDo: Review this value (and its purpose).  It (16 segments) seems awfully small (would make fast retransmit less
// useful), and this mechanism isn't the best way to protect ourselves against deliberate out-of-order segment attacks.
// Ideally, we'd limit out-of-order data to that which (along with the unread data) will fit in the receive window.
const MAX_OUT_OF_ORDER: usize = 16;

// TCP Connection State.
// Note: This ControlBlock structure is only used after we've reached the ESTABLISHED state, so states LISTEN,
// SYN_RCVD, and SYN_SENT aren't included here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Established,
    FinWait1,
    FinWait2,
    Closing,
    TimeWait,
    CloseWait,
    LastAck,
    Closed,
}

// ToDo: Consider incorporating this directly into ControlBlock.
struct Receiver<RT: Runtime> {
    //
    // Receive Sequence Space:
    //
    //                     |<---------------receive_buffer_size---------------->|
    //                     |                                                    |
    //                     |                         |<-----receive window----->|
    //                reader_next              receive_next       receive_next + receive window
    //                     v                         v                          v
    // ... ----------------|-------------------------|--------------------------|------------------------------
    //      read by user   |  received but not read  |    willing to receive    | future sequence number space
    //
    // Note: In RFC 793 terminology, receive_next is RCV.NXT, and "receive window" is RCV.WND.
    //

    // Sequence number of next byte of data in the unread queue.
    pub reader_next: Cell<SeqNumber>,

    // Sequence number of the next byte of data (or FIN) that we expect to receive.  In RFC 793 terms, this is RCV.NXT.
    pub receive_next: Cell<SeqNumber>,

    // Receive queue.  Contains in-order received (and acknowledged) data ready for the application to read.
    recv_queue: RefCell<VecDeque<RT::Buf>>,
}

impl<RT: Runtime> Receiver<RT> {
    pub fn new(reader_next: SeqNumber, receive_next: SeqNumber) -> Self {
        Self {
            reader_next: Cell::new(reader_next),
            receive_next: Cell::new(receive_next),
            recv_queue: RefCell::new(VecDeque::with_capacity(RECV_QUEUE_SZ)),
        }
    }

    pub fn pop(&self) -> Option<RT::Buf> {
        let buf: RT::Buf = self.recv_queue.borrow_mut().pop_front()?;
        self.reader_next.set(self.reader_next.get() + SeqNumber::from(buf.len() as u32));

        Some(buf)
    }

    pub fn push(&self, buf: RT::Buf) {
        let buf_len: u32 = buf.len() as u32;
        self.recv_queue.borrow_mut().push_back(buf);
        self.receive_next.set(self.receive_next.get() + SeqNumber::from(buf_len as u32));
    }

    // ToDo: This appears to add up all the bytes ready for reading in the recv_queue, and is called each time we get a
    // new segment.  Seems like it would be more efficient to keep a running count of the bytes in the queue that we
    // add/subtract from as we add/remove segments from the queue.
    pub fn size(&self) -> usize {
        self.recv_queue
            .borrow()
            .iter()
            .map(|b| b.len())
            .sum::<usize>()
    }
}

/// Transmission control block for representing our TCP connection.
pub struct ControlBlock<RT: Runtime> {
    local: Ipv4Endpoint,
    remote: Ipv4Endpoint,

    rt: Rc<RT>,

    // ToDo: We shouldn't be keeping anything datalink-layer specific at this level.  The IP layer should be holding
    // this along with other remote IP information (such as routing, path MTU, etc).
    arp: Rc<ArpPeer<RT>>,

    // Send-side state information.  ToDo: Consider incorporating this directly into ControlBlock.
    sender: Sender<RT>,

    // TCP Connection State.
    state: Cell<State>,

    ack_delay_timeout: Duration,

    ack_deadline: WatchedValue<Option<Instant>>,

    // This is our receive buffer size, which is also the maximum size of our receive window.
    // Note: The maximum possible advertised window is 1 GiB with window scaling and 64 KiB without.
    receive_buffer_size: u32,

    // ToDo: Review how this is used.  We could have separate window scale factors, so there should be one for the
    // receiver and one for the sender.
    // This is the receive-side window scale factor.
    // This is the number of bits to shift to convert to/from the scaled value, and has a maximum value of 14.
    // ToDo: Keep this as a u8?
    window_scale: u32,

    waker: RefCell<Option<Waker>>,

    // Queue of out-of-order segments.  This is where we hold onto data that we've received (because it was within our
    // receive window) but can't yet present to the user because we're missing some other data that comes between this
    // and what we've already presented to the user.
    //
    out_of_order: RefCell<VecDeque<(SeqNumber, RT::Buf)>>,

    // Receive-side state information.  ToDo: Consider incorporating this directly into ControlBlock.
    receiver: Receiver<RT>,

    // Whether the user has called close.
    pub user_is_done_sending: Cell<bool>,
}

//==============================================================================

impl<RT: Runtime> ControlBlock<RT> {
    pub fn new(
        local: Ipv4Endpoint,
        remote: Ipv4Endpoint,
        rt: RT,
        arp: ArpPeer<RT>,
        receiver_seq_no: SeqNumber,
        ack_delay_timeout: Duration,
        receiver_window_size: u32,
        receiver_window_scale: u32,
        sender_seq_no: SeqNumber,
        sender_window_size: u32,
        sender_window_scale: u8,
        sender_mss: usize,
        sender_cc_constructor: CongestionControlConstructor<RT>,
        sender_congestion_control_options: Option<congestion_ctrl::Options>,
    ) -> Self {
        let sender = Sender::new(
            sender_seq_no,
            sender_window_size,
            sender_window_scale,
            sender_mss,
            sender_cc_constructor,
            sender_congestion_control_options,
        );
        Self {
            local,
            remote,
            rt: Rc::new(rt),
            arp: Rc::new(arp),
            sender: sender,
            state: Cell::new(State::Established),
            ack_delay_timeout,
            ack_deadline: WatchedValue::new(None),
            receive_buffer_size: receiver_window_size,
            window_scale: receiver_window_scale,
            waker: RefCell::new(None),
            out_of_order: RefCell::new(VecDeque::new()),
            receiver: Receiver::new(receiver_seq_no, receiver_seq_no),
            user_is_done_sending: Cell::new(false),
        }
    }

    pub fn get_local(&self) -> Ipv4Endpoint {
        self.local
    }

    pub fn get_remote(&self) -> Ipv4Endpoint {
        self.remote
    }

    pub fn rt(&self) -> Rc<RT> {
        self.rt.clone()
    }

    // ToDo: Remove this.  ARP doesn't belong at this layer.
    pub fn arp(&self) -> Rc<ArpPeer<RT>> {
        self.arp.clone()
    }

    pub fn send(&self, buf: RT::Buf) -> Result<(), Fail> {
        self.sender.send(buf, self)
    }

    pub fn congestion_ctrl_watch_retransmit_now_flag(&self) -> (bool, WatchFuture<bool>) {
        self.sender.congestion_ctrl_watch_retransmit_now_flag()
    }

    pub fn congestion_ctrl_on_fast_retransmit(&self) {
        self.sender.congestion_ctrl_on_fast_retransmit()
    }

    pub fn congestion_ctrl_on_rto(&self, send_unacknowledged: SeqNumber) {
        self.sender.congestion_ctrl_on_rto(send_unacknowledged)
    }

    pub fn congestion_ctrl_on_send(&self, rto: Duration, num_sent_bytes: u32) {
        self.sender.congestion_ctrl_on_send(rto, num_sent_bytes)
    }

    pub fn congestion_ctrl_on_cwnd_check_before_send(&self) {
        self.sender.congestion_ctrl_on_cwnd_check_before_send()
    }

    pub fn congestion_ctrl_watch_cwnd(&self) -> (u32, WatchFuture<u32>) {
        self.sender.congestion_ctrl_watch_cwnd()
    }

    pub fn congestion_ctrl_watch_limited_transmit_cwnd_increase(&self) -> (u32, WatchFuture<u32>) {
        self.sender
            .congestion_ctrl_watch_limited_transmit_cwnd_increase()
    }

    pub fn get_mss(&self) -> usize {
        self.sender.get_mss()
    }

    // ToDo: Rename this for clarity?  There is both a send window size and a receive window size.
    pub fn get_window_size(&self) -> (u32, WatchFuture<u32>) {
        self.sender.get_window_size()
    }

    pub fn get_send_unacked(&self) -> (SeqNumber, WatchFuture<SeqNumber>) {
        self.sender.get_send_unacked()
    }

    pub fn get_unsent_seq_no(&self) -> (SeqNumber, WatchFuture<SeqNumber>) {
        self.sender.get_unsent_seq_no()
    }

    pub fn get_send_next(&self) -> (SeqNumber, WatchFuture<SeqNumber>) {
        self.sender.get_send_next()
    }

    pub fn modify_send_next(&self, f: impl FnOnce(SeqNumber) -> SeqNumber) {
        self.sender.modify_send_next(f)
    }

    pub fn get_retransmit_deadline(&self) -> (Option<Instant>, WatchFuture<Option<Instant>>) {
        self.sender.get_retransmit_deadline()
    }

    pub fn set_retransmit_deadline(&self, when: Option<Instant>) {
        self.sender.set_retransmit_deadline(when);
    }

    pub fn pop_unacked_segment(&self) -> Option<UnackedSegment<RT>> {
        self.sender.pop_unacked_segment()
    }

    pub fn push_unacked_segment(&self, segment: UnackedSegment<RT>) {
        self.sender.push_unacked_segment(segment)
    }

    pub fn rto_estimate(&self) -> Duration {
        self.sender.rto_estimate()
    }

    pub fn rto_record_failure(&self) {
        self.sender.rto_record_failure()
    }

    pub fn unsent_top_size(&self) -> Option<usize> {
        self.sender.top_size_unsent()
    }

    pub fn pop_unsent_segment(&self, max_bytes: usize) -> Option<RT::Buf> {
        self.sender.pop_unsent(max_bytes)
    }

    pub fn pop_one_unsent_byte(&self) -> Option<RT::Buf> {
        self.sender.pop_one_unsent_byte()
    }

    // This is the main TCP receive routine.
    //
    pub fn receive(&self, mut header: &mut TcpHeader, mut data: RT::Buf) {
        debug!(
            "{:?} Connection Receiving {} bytes + {:?}",
            self.state.get(),
            data.len(),
            header
        );

        // ToDo: We're probably getting "now" here in order to get a timestamp as close as possible to when we received
        // the packet.  However, this is wasteful if we don't take a path below that actually uses it.  Review this.
        let now: Instant = self.rt.now();

        // Check to see if the segment is acceptable sequence-wise (i.e. contains some data that fits within the receive
        // window, or is a non-data segment with a sequence number that falls within the window).  Unacceptable segments
        // should be ACK'd (unless they are RSTs), and then dropped.
        //
        // [From RFC 793]
        // There are four cases for the acceptability test for an incoming segment:
        //
        // Segment Receive  Test
        // Length  Window
        // ------- -------  -------------------------------------------
        //
        //   0       0     SEG.SEQ = RCV.NXT
        //
        //   0      >0     RCV.NXT =< SEG.SEQ < RCV.NXT+RCV.WND
        //
        //  >0       0     not acceptable
        //
        //  >0      >0     RCV.NXT =< SEG.SEQ < RCV.NXT+RCV.WND
        //              or RCV.NXT =< SEG.SEQ+SEG.LEN-1 < RCV.NXT+RCV.WND

        // Review: We don't need all of these intermediate variables in the fast path.  It might be more efficient to
        // rework this to calculate some of them only when needed, even if we need to (re)do it in multiple places.

        let mut seg_start: SeqNumber = header.seq_num;

        let mut seg_end: SeqNumber = seg_start;
        let mut seg_len: u32 = data.len() as u32;
        if header.syn {
            seg_len += 1;
        }
        if header.fin {
            seg_len += 1;
        }
        if seg_len > 0 {
            seg_end = seg_start + SeqNumber::from(seg_len - 1);
        }

        let receive_next: SeqNumber = self.receiver.receive_next.get();

        let after_receive_window: SeqNumber = receive_next + SeqNumber::from(self.get_receive_window_size());

        // Check if this segment fits in our receive window.
        // In the optimal case it starts at RCV.NXT, so we check for that first.
        //
        if seg_start != receive_next {
            // The start of this segment is not what we expected.  See if it comes before or after.
            //
            if seg_start < receive_next {
                // This segment contains duplicate data (i.e. data we've already received).
                // See if it is a complete duplicate, or if some of the data is new.
                //
                if seg_end < receive_next {
                    // This is an entirely duplicate (i.e. old) segment.  ACK (if not RST) and drop.
                    //
                    if !header.rst {
                        self.send_ack();
                    }
                    return;
                } else {
                    // Some of this segment's data is new.  Cut the duplicate data off of the front.
                    // If there is a SYN at the start of this segment, remove it too.
                    //
                    let mut duplicate = u32::from(receive_next - seg_start);
                    seg_start = seg_start + SeqNumber::from(duplicate);
                    seg_len -= duplicate;
                    if header.syn {
                        header.syn = false;
                        duplicate -= 1;
                    }
                    data.adjust(duplicate as usize);
                }
            } else {
                // This segment contains entirely new data, but is later in the sequence than what we're expecting.
                // See if any part of the data fits within our receive window.
                //
                if seg_start >= after_receive_window {
                    // This segment is completely outside of our window.  ACK (if not RST) and drop.
                    //
                    if !header.rst {
                        self.send_ack();
                    }
                    return;
                }

                // At least the beginning of this segment is in the window.  We'll check the end below.
            }
        }

        // The start of the segment is in the window.
        // Check that the end of the segment is in the window, and trim it down if it is not.
        //
        if seg_len > 0 && seg_end >= after_receive_window {
            let mut excess: u32 = u32::from(seg_end - after_receive_window);
            excess += 1;
            // ToDo: If we end up (after receive handling rewrite is complete) not needing seg_end and seg_len after
            // this, remove these two lines adjusting them as they're being computed needlessly.
            seg_end = seg_end - SeqNumber::from(excess);
            seg_len -= excess;
            if header.fin {
                header.fin = false;
                excess -= 1;
            }
            data.trim(excess as usize);
        }

        // Check the RST bit.
        if header.rst {
            // Our peer has given up.  Shut the connection down hard.
            match self.state.get() {
                // ToDo: Can we be in SynReceived here?  If so, // State::SynReceived => return,

                // Data transfer states.
                State::Established | State::FinWait1 | State::FinWait2 | State::CloseWait => {
                    // ToDo: Return all outstanding user Receive and Send requests with "reset" responses.
                    // ToDo: Flush all segment queues.

                    // Enter Closed state.
                    self.state.set(State::Closed);

                    // ToDo: Delete the ControlBlock.
                    return;
                },

                // Closing states.
                State::Closing | State::LastAck | State::TimeWait => {
                    // Enter Closed state.
                    self.state.set(State::Closed);

                    // ToDo: Delete the ControlBlock.
                    return;
                },

                // Should never happen.
                state => panic!("Bad TCP state {:?}", state),
            }

            // Note: We should never get here.
        }

        // Note: RFC 793 says to check security/compartment and precedence next, but those are largely deprecated.

        // Check the SYN bit.
        if header.syn {
            // Receiving a SYN here is an error.
            warn!("Received in-window SYN on established connection.");
            // ToDo: Send Reset.
            // ToDo: Return all outstanding Receive and Send requests with "reset" responses.
            // ToDo: Flush all segment queues.

            // Enter Closed state.
            self.state.set(State::Closed);

            // ToDo: Delete the ControlBlock.
            return;
        }

        // Check the ACK bit.
        if !header.ack {
            // All segments on established connections should be ACKs.  Drop this segment.
            warn!("Received non-ACK segment on established connection.");
            return;
        }

        // Process the ACK.
        // Note: We process valid ACKs while in any synchronized state, even though there shouldn't be anything to do
        // in some states (e.g. TIME-WAIT) as it is more wasteful to always check that we're not in TIME-WAIT.
        //
        // Start by checking that the ACK acknowledges something new.
        // ToDo: Cleanup send-side variable names and removed Watched types.
        //
        let (send_unacknowledged, _): (SeqNumber, _) = self.sender.get_send_unacked();
        let (send_next, _): (SeqNumber, _) = self.sender.get_send_next();

        if send_unacknowledged < header.ack_num {
            if header.ack_num <= send_next {
                // This segment acknowledges new data (possibly and/or FIN).
                //
                // ToDo: Fix below function.
                if let Err(e) = self.sender.remote_ack(header.ack_num, now) {
                    warn!("Ignoring remote ack for {:?}: {:?}", header, e);
                }

                // Some states require additional processing.
                // ToDo: Review order of checks here for efficiency.
                //
                match self.state.get() {
                    State::Established => (),  // Common case.  Nothing more to do.
                    State::FinWait1 => {
                        // If our FIN is now ACK'd, then enter FIN-WAIT-2.
                        if header.ack_num == send_next {
                            self.state.set(State::FinWait2);
                        }
                    },
                    State::Closing => {
                        // If our FIN is now ACK'd, then enter TIME-WAIT.
                        if header.ack_num == send_next {
                            self.state.set(State::TimeWait);
                        }
                    }
                    State::LastAck => {
                        // In LAST-ACK state we're just waiting for all of our sent data (including FIN) to be ACK'd.
                        // Note we may have to retransmit something, but we've sent it all (incl. FIN) at least once.
                        // Check if this ACK acknowledges our FIN.  If it does, we're done.
                        //
                        if header.ack_num == send_next {
                            self.state.set(State::Closed);

                            // ToDo: Delete the ControlBlock.

                            return;
                        }
                    },

                    _ => (),
                }

                // See if the send window should be updated.
                // ToDo: Fix below function.  Note that SEG.WND is an offset from SEG.ACK.
                if let Err(e) = self.sender.update_remote_window(header.window_size as u16) {
                    warn!("Invalid window size update for {:?}: {:?}", header, e);
                }
            } else {
                // This segment acknowledges data we have yet to send!?  Send an ACK and drop the segment.
                //
                warn!("Received segment acknowledging data we have yet to send!");
                self.send_ack();
                return;
            }
        } else {
            // Duplicate ACK (doesn't acknowledge anything new).  We can mostly ignore this, except for fast-retransmit.
            // ToDo: Implement fast-retransmit.  In which case, we'd increment our dup-ack counter here.
        }

        // ToDo: Check the URG bit.  If we decide to support this, how should we do it?
        if header.urg {
            warn!("Got packet with URG bit set!");
        }

        // Process the segment text (if any).
        if !data.is_empty() {
            if self.state.get() != State::Established {
                // ToDo: Review this warning.  TCP connections in FIN-WAIT-1 and FIN-WAIT-2 can still receive data.
                warn!("Receiver closed");
            }
            if let Err(e) = self.receive_data(seg_start, data, now) {
                warn!("Ignoring remote data for {:?}: {:?}", header, e);
            }

            // ToDo: Arrange for an ACK to be sent soon.
        }

        // ToDo: If new segment was out-of-order above, make sure we've cleared the FIN by now.  Likewise, if this new
        // segment filled a hole allowing a previous out-of-order segment to be processed, and it had a FIN, make sure
        // we've set the FIN bit by now.

        // Check the FIN bit.
        if header.fin {
            // ToDo: Signal the user "connection closing" and return any pending Receive requests.

            // Advance RCV.NXT over the FIN and send ACK (delayed?).
            //
            self.receiver.receive_next.set(self.receiver.receive_next.get() + SeqNumber::from(1));
            self.send_ack();

            match self.state.get() {
                State::Established => self.state.set(State::CloseWait),
                State::FinWait1 => {
                    // RFC 793 has a benign logic flaw.  It says "If our FIN has been ACKed (perhaps in this segment),
                    // then enter TIME-WAIT, start the time-wait timer, turn off the other timers;".  But if our FIN
                    // has been ACK'd, we'd be in FIN-WAIT-2 here as a result of processing that ACK (see ACK handling
                    // above) and will enter TIME-WAIT in the FIN-WAIT-2 case below.  So we can skip that clause and go
                    // straight to "otherwise enter the CLOSING state".
                    //
                    self.state.set(State::Closing);
                },
                State::FinWait2 => {
                    // Enter TIME-WAIT.
                    self.state.set(State::TimeWait);
                    // ToDo: Start the time-wait timer and turn off the other timers.
                },
                State::CloseWait | State::Closing | State::LastAck => (),  // Remain in current state.
                State::TimeWait => {
                    // ToDo: Remain in TIME-WAIT.  Restart the 2 MSL time-wait timeout.
                },
                state => panic!("Bad TCP state {:?}", state),  // Should never happen.
            }
        }

        // ToDo: Implement delayed ACKs.
        //
        //self.send_ack();
    }

    /// Handle the user's close request.
    ///
    /// In TCP parlance, a user's close request means "I have no more data to send".  The user may continue to receive
    /// data on this connection until their peer also closes.
    ///
    /// Note this routine will only be called for connections with a ControlBlock (i.e. in state ESTABLISHED or later).
    ///
    pub fn close(&self) -> Result<(), Fail> {
        // Check to see if close has already been called, as we should only do this once.
        //
        if self.user_is_done_sending.get() {
            // Review: Should we return an error here instead?  RFC 793 recommends a "connection closing" error.
            return Ok(());
        }

        // In the normal case, we'll be in either ESTABLISHED or CLOSE_WAIT here (depending upon whether we've received
        // a FIN from our peer yet).  Queue up a FIN to be sent, and attempt to send it immediately (if possible).  We
        // only change state to FIN-WAIT-1 or LAST_ACK after we've actually been able to send the FIN.
        //
        debug_assert!((self.state.get() == State::Established) || (self.state.get() == State::CloseWait));

        // Send a FIN.
        //
        let fin_buf: RT::Buf = Buffer::empty();
        self.send(fin_buf).expect("send failed");

        // Remember that the user has called close.
        //
        self.user_is_done_sending.set(true);

        Ok(())
    }

    /// Fetch a TCP header filling out various values based on our current state.
    /// ToDo: Fix the "filling out various values based on our current state" part to actually do that correctly.
    pub fn tcp_header(&self) -> TcpHeader {
        let mut header = TcpHeader::new(self.local.get_port(), self.remote.get_port());
        header.window_size = self.hdr_window_size();

        // Note that once we reach a synchronized state we always include a valid acknowledgement number.
        header.ack = true;
        header.ack_num = self.receiver.receive_next.get();

        // Return this header.
        header
    }

    /// Send an ACK to our peer, reflecting our current state.
    pub fn send_ack(&self) {
        let mut header: TcpHeader = self.tcp_header();

        // ToDo: Think about moving this to tcp_header() as well.
        let (seq_num, _): (SeqNumber, _) = self.get_send_next();
        header.seq_num = seq_num;

        // ToDo: Remove this if clause once emit() is fixed to not require the remote hardware addr (this should be
        // left to the ARP layer and not exposed to TCP).
        if let Some(remote_link_addr) = self.arp().try_query(self.remote.get_address()) {
            self.emit(header, RT::Buf::empty(), remote_link_addr);
        }
    }

    /// Transmit this message to our connected peer.
    ///
    pub fn emit(&self, header: TcpHeader, data: RT::Buf, remote_link_addr: MacAddress) {
        debug!("Sending {} bytes + {:?}", data.len(), header);

        // This routine should only ever be called to send TCP segments that contain a valid ACK value.
        debug_assert!(header.ack);

        let sent_fin: bool = header.fin;

        // Prepare description of TCP segment to send.
        // ToDo: Change this to call lower levels to fill in their header information, handle routing, ARPing, etc.
        let segment = TcpSegment {
            ethernet2_hdr: Ethernet2Header::new(
                remote_link_addr,
                self.rt.local_link_addr(),
                EtherType2::Ipv4,
            ),
            ipv4_hdr: Ipv4Header::new(
                self.local.get_address(),
                self.remote.get_address(),
                IpProtocol::TCP,
            ),
            tcp_hdr: header,
            data,
            tx_checksum_offload: self.rt.tcp_options().get_tx_checksum_offload(),
        };

        // Call the runtime to send the segment.
        self.rt.transmit(segment);

        // Post-send operations follow.
        // Review: We perform these after the send, in order to keep send latency as low as possible.

        // Since we sent an ACK, cancel any outstanding delayed ACK request.
        self.set_ack_deadline(None);

        // If we sent a FIN, update our protocol state.
        if sent_fin {
            match self.state.get() {
                // Active close.
                State::Established => self.state.set(State::FinWait1),
                // Passive close.
                State::CloseWait => self.state.set(State::LastAck),
                // We can legitimately retransmit the FIN in these states.  And we stay there until the FIN is ACK'd.
                State::FinWait1 | State::LastAck => {},
                // We shouldn't be sending a FIN from any other state.
                state => warn!("Sent FIN while in nonsensical TCP state {:?}", state),
            }
        }
    }

    pub fn remote_mss(&self) -> usize {
        self.sender.remote_mss()
    }

    pub fn rto_current(&self) -> Duration {
        self.sender.current_rto()
    }

    pub fn get_ack_deadline(&self) -> (Option<Instant>, WatchFuture<Option<Instant>>) {
        self.ack_deadline.watch()
    }

    pub fn set_ack_deadline(&self, when: Option<Instant>) {
        self.ack_deadline.set(when);
    }

    pub fn get_receive_window_size(&self) -> u32 {
        let bytes_unread: u32 = (self.receiver.receive_next.get() - self.receiver.reader_next.get()).into();
        self.receive_buffer_size - bytes_unread
    }

    pub fn hdr_window_size(&self) -> u16 {
        let window_size = self.get_receive_window_size();
        let hdr_window_size = (window_size >> self.window_scale)
            .try_into()
            .expect("Window size overflow");
        debug!(
            "Sending window size update -> {} (hdr {}, scale {})",
            (hdr_window_size as u32) << self.window_scale,
            hdr_window_size,
            self.window_scale
        );
        hdr_window_size
    }

    pub fn poll_recv(&self, ctx: &mut Context) -> Poll<Result<RT::Buf, Fail>> {
        // ToDo: Need to add a way to indicate that the other side closed (i.e. that we've received a FIN).
        // This code was checking for an empty receive queue by comparing sequence numbers, as in:
        //  if self.receiver.reader_next.get() == self.receiver.receive_next.get() {
        // But that will think data is available to be read once we've received a FIN, because FINs consume sequence
        // number space.
        //
        if self.receiver.recv_queue.borrow().is_empty() {
            *self.waker.borrow_mut() = Some(ctx.waker().clone());
            return Poll::Pending;
        }

        let segment = self
            .receiver
            .pop()
            .expect("poll_recv failed to pop data from receive queue");

        Poll::Ready(Ok(segment))
    }

    // ToDo: Improve following comment:
    // This routine appears to take an incoming TCP segment and either add it to the receiver's queue of data that is
    // ready to be read by the user (if the segment contains in-order data) or add it to the proper position in the
    // receiver's store of out-of-order data.  Also, in the in-order case, it updates our receiver's sequence number
    // corresponding to the minumum number allowed for new reception (RCV.NXT in RFC 793 terms).
    //
    pub fn receive_data(&self, seq_no: SeqNumber, buf: RT::Buf, now: Instant) -> Result<(), Fail> {
        let recv_seq_no: SeqNumber = self.receiver.receive_next.get();

        if seq_no > recv_seq_no {
            // This new segment comes after what we're expecting (i.e. the new segment arrived out-of-order).
            let mut out_of_order = self.out_of_order.borrow_mut();

            // Check if the new data segment's starting sequence number is already in the out-of-order store.
            // ToDo: We should check if any part of the new segment contains new data, and not just the start.
            for stored_segment in out_of_order.iter() {
                if stored_segment.0 == seq_no {
                    // Drop this segment as a duplicate.
                    // ToDo: We should ACK when we drop a segment.
                    return Err(Fail::new(EBADMSG, "out of order segment (duplicate)"));
                }
            }

            // Before adding more, if the out-of-order store contains too many entries, delete the later entries.
            while out_of_order.len() > MAX_OUT_OF_ORDER {
                out_of_order.pop_back();
            }

            // Add the new segment to the out-of-order store (the store is sorted by starting sequence number).
            let mut insert_index: usize = out_of_order.len();
            for index in 0..out_of_order.len() {
                if seq_no > out_of_order[index].0 {
                    insert_index = index;
                    break;
                }
            }
            if insert_index < out_of_order.len() {
                out_of_order[insert_index] = (seq_no, buf);
            } else {
                out_of_order.push_back((seq_no, buf));
            }

            // ToDo: There is a bug here.  We should send an ACK when we drop a segment.
            return Err(Fail::new(EBADMSG, "out of order segment (reordered)"));
        }

        // Check if we've already received this data (i.e. new segment contains duplicate data).
        // ToDo: There is a bug here.  The new segment could contain both old *and* new data.  Current code throws it
        // all away.  We need to check if any part of the new segment falls within our receive window.
        if seq_no < recv_seq_no {
            // ToDo: There is a bug here.  We should send an ACK if we drop the segment.
            return Err(Fail::new(EBADMSG, "out of order segment (duplicate)"));
        }

        // If we get here, the new segment begins with the sequence number we're expecting.
        // ToDo: Since this is the "good" case, we should have a fast-path check for it first above, instead of falling
        // through to it (performance improvement).

        let unread_bytes: usize = self.receiver.size();

        // This appears to drop segments if their total contents would exceed the receive window.
        // ToDo: There is a bug here.  The segment could also contain some data that fits within the window.  We should
        // still accept the data that fits within the window.
        // ToDo: We should restructure this to convert usize things to known (fixed) sizes, not the other way around.
        if unread_bytes + buf.len() > self.receive_buffer_size as usize {
            // ToDo: There is a bug here.  We should send an ACK if we drop the segment.
            return Err(Fail::new(ENOMEM, "full receive window"));
        }

        // Push the new segment data onto the end of the receive queue.
        let mut recv_seq_no: SeqNumber = recv_seq_no + SeqNumber::from(buf.len() as u32);
        self.receiver.push(buf);

        // Okay, we've successfully received some new data.  Check if any of the formerly out-of-order data waiting in
        // the out-of-order queue is now in-order.  If so, we can move it to the receive queue.
        let mut out_of_order = self.out_of_order.borrow_mut();
        while !out_of_order.is_empty() {
            if let Some(stored_entry) = out_of_order.front() {
                if stored_entry.0 == recv_seq_no {
                    // Move this entry's buffer from the out-of-order store to the receive queue.
                    // This data is now considered to be "received" by TCP, and included in our RCV.NXT calculation.
                    info!("Recovering out-of-order packet at {}", recv_seq_no);
                    if let Some(temp) = out_of_order.pop_front() {
                        recv_seq_no = recv_seq_no + SeqNumber::from(temp.1.len() as u32);
                        self.receiver.push(temp.1);
                    }
                } else {
                    // Since our out-of-order list is sorted, we can stop when the next segment is not in sequence.
                    break;
                }
            }
        }

        // ToDo: Review recent change to update control block copy of recv_seq_no upon each push to the receiver.
        // When receiving a retransmitted segment that fills a "hole" in the receive space, thus allowing a number
        // (potentially large number) of out-of-order segments to be added, we'll be modifying the TCB copy of the
        // recv_seq_no many times.  Since this potentially wakes a waker, we might want to wait until we've added all
        // the segments before we update the value.
        // Anyhow that recent change removes the need for the following two lines:
        // Update our receive sequence number (i.e. RCV_NXT) appropriately.
        // self.recv_seq_no.set(recv_seq_no);

        // This appears to be checking if something is waiting on this Receiver, and if so, wakes that thing up.
        // ToDo: Verify that this is the right place and time to do this.
        if let Some(w) = self.waker.borrow_mut().take() {
            w.wake()
        }

        // ToDo: How do we handle when the other side is in PERSIST state here?
        // ToDo: Fix above comment - there is no such thing as a PERSIST state in TCP.  Presumably, this comment means
        // to ask "how do we handle the situation where the other side is sending us zero window probes because it has
        // data to send and no open window to send into?".  The answer is: we should ACK zero-window probes.

        // Schedule an ACK for this receive (if one isn't already).
        // ToDo: Another bug.  If the delayed ACK timer is already running, we should cancel it and ACK immediately.
        if self.ack_deadline.get().is_none() {
            self.ack_deadline.set(Some(now + self.ack_delay_timeout));
        }

        Ok(())
    }
}
