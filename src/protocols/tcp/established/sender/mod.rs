// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub mod congestion_ctrl;
mod rto;

use self::{congestion_ctrl as cc, rto::RtoCalculator};
use super::ControlBlock;
use crate::protocols::tcp::SeqNumber;
use ::libc::{EBADMSG, EBUSY, EINVAL, ENOBUFS};
use ::runtime::{
    fail::Fail,
    memory::Buffer,
    watched::{WatchFuture, WatchedValue},
    Runtime,
};
use ::std::{
    boxed::Box,
    cell::RefCell,
    collections::VecDeque,
    convert::TryInto,
    fmt,
    time::{Duration, Instant},
};

// Structure of entries on our unacknowledged queue.
// ToDo: We currently allocate these on the fly when we add a buffer to the queue.  Would be more efficient to have a
// buffer structure that held everything we need directly, thus avoiding this extra wrapper.
//
pub struct UnackedSegment<RT: Runtime> {
    pub bytes: RT::Buf,
    // Set to `None` on retransmission to implement Karn's algorithm.
    pub initial_tx: Option<Instant>,
}

/// Hard limit for unsent queue.
/// ToDo: Remove this.  We should limit the unsent queue by either having a (configurable) send buffer size (in bytes,
/// not segments) and rejecting send requests that exceed that, or by limiting the user's send buffer allocations.
const UNSENT_QUEUE_CUTOFF: usize = 1024;

pub struct Sender<RT: Runtime> {
    // ToDo: Just use Figure 4 from RFC 793 here.
    //
    //                    |------------window_size------------|
    //
    //               base_seq_no               sent_seq_no           unsent_seq_no
    //                    v                         v                      v
    // ... ---------------|-------------------------|----------------------| (unavailable)
    //       acknowledged        unacknowledged     ^        unsent
    //

    // ToDo: Rename this.  It appears to be SND.UNA.
    base_seq_no: WatchedValue<SeqNumber>,

    // RFC 793 calls this the "retransmission queue".
    unacked_queue: RefCell<VecDeque<UnackedSegment<RT>>>,

    // ToDo: Rename this.  It appears to be SND.NXT.
    sent_seq_no: WatchedValue<SeqNumber>,

    // This is the send buffer (user data we do not yet have window to send).
    unsent_queue: RefCell<VecDeque<RT::Buf>>,

    // ToDo: Remove this.
    unsent_seq_no: WatchedValue<SeqNumber>,

    // ToDo: Rename this.  It appears to be SND.WND.
    window_size: WatchedValue<u32>,

    // RFC 1323: Number of bits to shift advertised window, defaults to zero.
    window_scale: u8,

    // Maximum Segment Size currently in use for this connection.
    // ToDo: Revisit this once we support path MTU discovery.
    mss: usize,

    retransmit_deadline: WatchedValue<Option<Instant>>,

    // Retransmission Timeout (RTO) calculator.
    rto: RefCell<RtoCalculator>,

    congestion_ctrl: Box<dyn cc::CongestionControl<RT>>,
}

impl<RT: Runtime> fmt::Debug for Sender<RT> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Sender")
            .field("base_seq_no", &self.base_seq_no)
            .field("sent_seq_no", &self.sent_seq_no)
            .field("unsent_seq_no", &self.unsent_seq_no)
            .field("window_size", &self.window_size)
            .field("window_scale", &self.window_scale)
            .field("mss", &self.mss)
            .field("retransmit_deadline", &self.retransmit_deadline)
            .field("rto", &self.rto)
            .finish()
    }
}

impl<RT: Runtime> Sender<RT> {
    pub fn new(
        seq_no: SeqNumber,
        window_size: u32,
        window_scale: u8,
        mss: usize,
        cc_constructor: cc::CongestionControlConstructor<RT>,
        congestion_control_options: Option<cc::Options>,
    ) -> Self {
        Self {
            base_seq_no: WatchedValue::new(seq_no),
            unacked_queue: RefCell::new(VecDeque::new()),
            sent_seq_no: WatchedValue::new(seq_no),
            unsent_queue: RefCell::new(VecDeque::new()),
            unsent_seq_no: WatchedValue::new(seq_no),

            window_size: WatchedValue::new(window_size),
            window_scale,
            mss,

            retransmit_deadline: WatchedValue::new(None),
            rto: RefCell::new(RtoCalculator::new()),

            congestion_ctrl: cc_constructor(mss, seq_no, congestion_control_options),
        }
    }

    pub fn get_mss(&self) -> usize {
        self.mss
    }

    pub fn get_window_size(&self) -> (u32, WatchFuture<u32>) {
        self.window_size.watch()
    }

    pub fn get_base_seq_no(&self) -> (SeqNumber, WatchFuture<SeqNumber>) {
        self.base_seq_no.watch()
    }

    pub fn get_sent_seq_no(&self) -> (SeqNumber, WatchFuture<SeqNumber>) {
        self.sent_seq_no.watch()
    }

    pub fn modify_sent_seq_no(&self, f: impl FnOnce(SeqNumber) -> SeqNumber) {
        self.sent_seq_no.modify(f)
    }

    pub fn get_unsent_seq_no(&self) -> (SeqNumber, WatchFuture<SeqNumber>) {
        self.unsent_seq_no.watch()
    }

    pub fn get_retransmit_deadline(&self) -> (Option<Instant>, WatchFuture<Option<Instant>>) {
        self.retransmit_deadline.watch()
    }

    pub fn set_retransmit_deadline(&self, when: Option<Instant>) {
        self.retransmit_deadline.set(when);
    }

    pub fn pop_unacked_segment(&self) -> Option<UnackedSegment<RT>> {
        self.unacked_queue.borrow_mut().pop_front()
    }

    pub fn push_unacked_segment(&self, segment: UnackedSegment<RT>) {
        self.unacked_queue.borrow_mut().push_back(segment)
    }

    pub fn rto_estimate(&self) -> Duration {
        self.rto.borrow().estimate()
    }

    pub fn rto_record_failure(&self) {
        self.rto.borrow_mut().record_failure()
    }

    // This is the main TCP send routine.
    //
    pub fn send(&self, buf: RT::Buf, cb: &ControlBlock<RT>) -> Result<(), Fail> {
        // If the user is done sending (i.e. has called close on this connection), then they shouldn't be sending.
        //
        if cb.user_is_done_sending.get() {
            return Err(Fail::new(EINVAL, "Connection is closing"));
        }

        // ToDo: Review below comment for accuracy.
        // Our API supports send buffers up to usize (variable, depends upon architecture) in size.  While we could
        // allow for larger send buffers, it is simpler and more practical to limit a single send to 2 GiB, which is
        // also the maximum value a TCP can advertise as its receive window (with maximum window scaling).
        //
        // Review: Move this check up the stack (i.e. closer to the user)?
        //
        let mut buf_len: u32 = buf.len().try_into().map_err(|_| Fail::new(EINVAL, "buffer too large"))?;

        // ToDo: What we should do here:
        //
        // Conceptually, we should take the provided buffer and add it to the unsent queue.  Then calculate the amount
        // of data we're currently allowed to send (based off of the receiver's advertised window, our congestion
        // control algorithm, silly window syndrome avoidance algorithm, etc).  Finally, enter a loop where we compose
        // maximum sized segments from the data on the unsent queue and send them, saving a (conceptual) copy of the
        // sent data on the unacknowledged queue, until we run out of either window space or unsent data.
        //
        // Note that there are several shortcuts we can make to this conceptual approach to speed the common case.
        // Note also that this conceptual send code is almost identical to what our "background send" algorithm should
        // be doing, so we should just have a single function that we call from both places.
        //
        // The current code below just tries to send the provided buffer immediately (if allowed), otherwise it places
        // it on the unsent queue and that's it.
        //

        // Check for unsent data.
        if self.unsent_queue.borrow().is_empty() {

            // No unsent data queued up, so we can try to send this new buffer immediately.

            // Calculate amount of data in flight (SND.NXT - SND.UNA).
            let base_seq = self.base_seq_no.get();
            let sent_seq = self.sent_seq_no.get();
            let sent_data: u32 = (sent_seq - base_seq).into();

            // ToDo: What limits buffer len to MSS?
            let in_flight_after_send = sent_data + buf_len;

            // Before we get cwnd for the check, we prompt it to shrink it if the connection has been idle.
            self.congestion_ctrl.on_cwnd_check_before_send();
            let cwnd = self.congestion_ctrl.get_cwnd();

            // The limited transmit algorithm can increase the effective size of cwnd by up to 2MSS.
            let effective_cwnd = cwnd + self.congestion_ctrl.get_limited_transmit_cwnd_increase();

            let win_sz = self.window_size.get();

            if win_sz > 0
                && win_sz >= in_flight_after_send
                && effective_cwnd >= in_flight_after_send
            {
                if let Some(remote_link_addr) = cb.arp().try_query(cb.get_remote().get_address()) {
                    // This hook is primarily intended to record the last time we sent data, so we can later tell if
                    // the connection has been idle.
                    let rto: Duration = self.current_rto();
                    self.congestion_ctrl.on_send(rto, sent_data);

                    // Prepare the segment and send it.
                    let mut header = cb.tcp_header();
                    header.seq_num = sent_seq;
                    if buf_len == 0 {
                        // This buffer is the end-of-send marker.
                        debug_assert_eq!(cb.user_is_done_sending.get(), true);
                        // Set FIN and adjust sequence number consumption accordingly.
                        header.fin = true;
                        buf_len = 1;
                    }
                    cb.emit(header, buf.clone(), remote_link_addr);

                    // Update SND.NXT.
                    self.sent_seq_no.modify(|s| s + SeqNumber::from(buf_len));

                    // ToDo: We don't need to track this.
                    self.unsent_seq_no.modify(|s| s + SeqNumber::from(buf_len));

                    // Put the segment we just sent on the retransmission queue.
                    let unacked_segment = UnackedSegment {
                        bytes: buf,
                        initial_tx: Some(cb.rt().now()),
                    };
                    self.unacked_queue.borrow_mut().push_back(unacked_segment);

                    // Start the retransmission timer if it isn't already running.
                    if self.retransmit_deadline.get().is_none() {
                        let rto = self.rto.borrow().estimate();
                        self.retransmit_deadline.set(Some(cb.rt().now() + rto));
                    }

                    return Ok(());
                } else {
                    warn!("no ARP cache entry for send");
                }
            }
        }

        // Too fast.
        // ToDo: We need to fix this the correct way: limit our send buffer size to the amount we're willing to buffer.
        if self.unsent_queue.borrow().len() > UNSENT_QUEUE_CUTOFF {
            return Err(Fail::new(EBUSY, "too many packets to send"));
        }

        // Slow path: Delegating sending the data to background processing.
        self.unsent_queue.borrow_mut().push_back(buf);
        self.unsent_seq_no.modify(|s| s + SeqNumber::from(buf_len));

        Ok(())
    }

    // Process an "acceptable" acknowledgement received from our peer.
    // ToDo: Rename this to something more meaningful.  Or maybe move this back into mainline receive?
    pub fn remote_ack(&self, seg_ack: SeqNumber, now: Instant) -> Result<(), Fail> {
        // ToDo: What we're supposed to do here:
        // This acknowledges new data, so update SND.UNA = SEG.ACK
        //  + remove acknowledged data from the retransmission queue,
        //  + report any fully acknowledged buffers to the user (do we have an API to do this?),
        //  + manage the retransmission timer,
        //  + update the send window.
        //

        let base_seq_no = self.base_seq_no.get();
        let sent_seq_no = self.sent_seq_no.get();

        let bytes_outstanding: u32 = (sent_seq_no - base_seq_no).into();
        let bytes_acknowledged: u32 = (seg_ack - base_seq_no).into();

        if bytes_acknowledged > bytes_outstanding {
            return Err(Fail::new(EBADMSG, "ACK is outside of send window"));
        }

        // Inform congestion control about this ACK.
        let rto: Duration = self.current_rto();
        self.congestion_ctrl
            .on_ack_received(rto, base_seq_no, sent_seq_no, seg_ack);
        if bytes_acknowledged == 0 {
            return Ok(());
        }

        // Manage the retransmit timer.
        if seg_ack == sent_seq_no {
            // If we've acknowledged all sent data, turn off the retransmit timer.
            self.retransmit_deadline.set(None);
        } else {
            // Otherwise, set it to the current RTO.
            // ToDo: This looks wrong.  Why extend the deadline here?
            let deadline = now + self.rto.borrow().estimate();
            self.retransmit_deadline.set(Some(deadline));
        }

        // Remove now acknowledged data from the unacked queue and advance SND.UNA.
        // TODO: Do acks need to be on segment boundaries? How does this interact with repacketization?
        // Answer: No, ACKs need not be on segment boundaries.  We need to handle this properly.
        let mut bytes_remaining = bytes_acknowledged as usize;
        while let Some(segment) = self.unacked_queue.borrow_mut().pop_front() {
            if segment.bytes.len() > bytes_remaining {
                // TODO: We need to close the connection in this case.
                return Err(Fail::new(EBADMSG, "ACK isn't on segment boundary"));
            }
            bytes_remaining -= segment.bytes.len();

            // Add sample for RTO if not a retransmission
            // TODO: TCP timestamp support.
            if let Some(initial_tx) = segment.initial_tx {
                self.rto.borrow_mut().add_sample(now - initial_tx);
            }
            if bytes_remaining == 0 {
                break;
            }
        }
        self.base_seq_no
            .modify(|b| b + SeqNumber::from(bytes_acknowledged));
        let new_base_seq_no = self.base_seq_no.get();
        if new_base_seq_no < base_seq_no {
            // We've wrapped around, and so we need to do some bookkeeping
            // ToDo: Figure out what this is doing -- it's probably wrong.
            self.congestion_ctrl.on_base_seq_no_wraparound();
        }

        Ok(())
    }

    pub fn pop_one_unsent_byte(&self) -> Option<RT::Buf> {
        let mut queue = self.unsent_queue.borrow_mut();

        let buf = queue.front_mut()?;
        let mut cloned_buf = buf.clone();
        let buf_len = buf.len();

        // Pop one byte off the buf still in the queue and all but one of the bytes on our clone.
        buf.adjust(1);
        cloned_buf.trim(buf_len - 1);

        Some(cloned_buf)
    }

    pub fn pop_unsent(&self, max_bytes: usize) -> Option<RT::Buf> {
        // TODO: Use a scatter/gather array to coalesce multiple buffers into a single segment.
        let mut unsent_queue = self.unsent_queue.borrow_mut();
        let mut buf = unsent_queue.pop_front()?;
        let buf_len = buf.len();

        if buf_len > max_bytes {
            let mut cloned_buf = buf.clone();

            buf.adjust(max_bytes);
            cloned_buf.trim(buf_len - max_bytes);

            unsent_queue.push_front(buf);
            buf = cloned_buf;
        }
        Some(buf)
    }

    pub fn top_size_unsent(&self) -> Option<usize> {
        let unsent_queue = self.unsent_queue.borrow_mut();
        Some(unsent_queue.front()?.len())
    }

    pub fn update_remote_window(&self, window_size_hdr: u16) -> Result<(), Fail> {
        // TODO: Is this the right check?
        let window_size = (window_size_hdr as u32)
            .checked_shl(self.window_scale as u32)
            .ok_or(Fail::new(ENOBUFS, "window size overlow"))?;

        debug!(
            "Updating window size -> {} (hdr {}, scale {})",
            window_size, window_size_hdr, self.window_scale
        );
        self.window_size.set(window_size);

        Ok(())
    }

    pub fn remote_mss(&self) -> usize {
        self.mss
    }

    pub fn current_rto(&self) -> Duration {
        self.rto.borrow().estimate()
    }

    pub fn congestion_ctrl_watch_retransmit_now_flag(&self) -> (bool, WatchFuture<bool>) {
        self.congestion_ctrl.watch_retransmit_now_flag()
    }

    pub fn congestion_ctrl_on_fast_retransmit(&self) {
        self.congestion_ctrl.on_fast_retransmit()
    }

    pub fn congestion_ctrl_on_rto(&self, base_seq_no: SeqNumber) {
        self.congestion_ctrl.on_rto(base_seq_no)
    }

    pub fn congestion_ctrl_on_send(&self, rto: Duration, num_sent_bytes: u32) {
        self.congestion_ctrl.on_send(rto, num_sent_bytes)
    }

    pub fn congestion_ctrl_on_cwnd_check_before_send(&self) {
        self.congestion_ctrl.on_cwnd_check_before_send()
    }

    pub fn congestion_ctrl_watch_cwnd(&self) -> (u32, WatchFuture<u32>) {
        self.congestion_ctrl.watch_cwnd()
    }

    pub fn congestion_ctrl_watch_limited_transmit_cwnd_increase(&self) -> (u32, WatchFuture<u32>) {
        self.congestion_ctrl.watch_limited_transmit_cwnd_increase()
    }
}
