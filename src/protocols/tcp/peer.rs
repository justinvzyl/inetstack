// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    active_open::ActiveOpenSocket, established::EstablishedSocket, isn_generator::IsnGenerator,
    passive_open::PassiveSocket,
};
use crate::protocols::{
    arp::ArpPeer,
    ethernet2::{EtherType2, Ethernet2Header},
    ip,
    ip::EphemeralPorts,
    ipv4::{Ipv4Endpoint, Ipv4Header, Ipv4Protocol2},
    tcp::{
        operations::{AcceptFuture, ConnectFuture, ConnectFutureState, PopFuture, PushFuture},
        segment::{TcpHeader, TcpSegment},
    },
};
use ::futures::channel::mpsc;
use ::runtime::{fail::Fail, memory::Buffer, queue::IoQueueDescriptor, Runtime};
use ::std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};
use std::collections::hash_map::Entry;
use std::collections::{HashSet, VecDeque};
use std::fmt::{Debug, Formatter};
use std::net::Ipv4Addr;

#[cfg(feature = "profiler")]
use perftools::timer;
use crate::protocols::tcp::established::State;

use serde::{Serialize, Deserialize};
use crate::protocols::tcp::cc::CongestionControl;
use crate::protocols::tcp::established::ControlBlock;
use crate::protocols::tcp::established::ctrlblk::ReceiveQueue;
use crate::protocols::tcp::established::sender::{Sender, UnackedSegment};
use crate::protocols::tcp::{cc, SeqNumber};

/// State needed for TCP Migration
#[derive(Serialize, Deserialize, Clone)]
/// TODO: Include out_of_order vec dequeue from control block. Will probably need that.
pub struct TcpState<RT: Runtime> {
    pub remote: Ipv4Endpoint,
    pub local: Ipv4Endpoint,
    pub receive_queue: VecDeque<RT::Buf>,
    pub unacked_queue: VecDeque<UnackedSegment<RT>>,
    pub retransmit_queue: VecDeque<RT::Buf>,
    // TODO: Do we need `our` and the `peers` mss?
    /// Maximum Segment Size
    pub sender_mss: usize,
    pub receiver_window_scale: u32,
    pub sender_window_scale: u8,
    pub max_window_size: u32,
    // RCV.NXT: Sequence numbers we have received and acknowledged.
    pub rcv_nxt: SeqNumber,
    // Current sequence number that peer has acknowledged up to.
    pub snd_una: SeqNumber,
    // Next sequence number for us to send.
    pub snd_nxt: SeqNumber,
    // SND.WND: Send Window
    pub snd_wnd: u32,

    // Not really part of the TCP expect but needed by ReceiveQueue
    pub base_seq_no: SeqNumber,
    pub recv_seq_no: SeqNumber
}

// We manually derive Debug to avoid requirement that RT also implement Debug.
impl<RT: Runtime> Debug for TcpState<RT> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpState").
            field("remote", &self.remote).
            field("local", &self.local).
            field("receive_queue", &self.receive_queue).
            field("unacked_queue", &self.unacked_queue).
            field("retransmit_queue", &self.retransmit_queue).
            field("sender_mss", &self.sender_mss).
            field("receiver_window_scale", &self.receiver_window_scale).
            field("sender_window_scale", &self.sender_window_scale).
            field("max_window_size", &self.max_window_size).
            field("rcv_nxt", &self.rcv_nxt).
            field("snd_una", &self.snd_una).
            field("snd_nxt", &self.snd_nxt).
            field("snd_wnd", &self.snd_wnd).
            field("base_seq_no", &self.base_seq_no).
            field("recv_seq_no", &self.recv_seq_no).

            finish()
    }
}

impl<RT: Runtime> PartialEq<Self> for TcpState<RT> {
    fn eq(&self, other: &Self) -> bool {
        (self.remote == other.remote) &&
            (self.local == other.local) &&
            (self.receive_queue == other.receive_queue) &&
            (self.unacked_queue == other.unacked_queue) &&
            (self.retransmit_queue == other.retransmit_queue) &&
            (self.sender_mss == other.sender_mss) &&
            (self.receiver_window_scale == other.receiver_window_scale) &&
            (self.sender_window_scale == other.sender_window_scale) &&
            (self.max_window_size == other.max_window_size) &&
            (self.rcv_nxt == other.rcv_nxt) &&
            (self.snd_una == other.snd_una) &&
            (self.snd_nxt == other.snd_nxt) &&
            (self.snd_wnd == other.snd_wnd) &&
            (self.base_seq_no == other.base_seq_no) &&
            (self.recv_seq_no == other.recv_seq_no)
    }
}

impl<RT: Runtime> Eq for TcpState<RT> {

}

impl<RT: Runtime> TcpState<RT> {
    pub fn new(remote: Ipv4Endpoint, local: Ipv4Endpoint, receive_queue: VecDeque<RT::Buf>, retransmit_queue: VecDeque<RT::Buf>, unacked_queue: VecDeque<UnackedSegment<RT>>, mss: usize, receiver_window_scale: u32, sender_window_scale: u8, rcv_wnd: u32, rcv_nxt: SeqNumber, snd_una: SeqNumber, snd_nxt: SeqNumber, snd_wnd: u32, base_seq_no: SeqNumber,
               recv_seq_no: SeqNumber) -> Self {
        TcpState { remote, local, receive_queue, unacked_queue, retransmit_queue, sender_mss: mss, receiver_window_scale, sender_window_scale, max_window_size: rcv_wnd, rcv_nxt, snd_una, snd_nxt, snd_wnd, base_seq_no, recv_seq_no }
    }
}


#[derive(Debug)]
pub enum Socket {
    Inactive {
        local: Option<Ipv4Endpoint>,
    },
    Listening {
        local: Ipv4Endpoint,
    },
    Connecting {
        local: Ipv4Endpoint,
        remote: Ipv4Endpoint,
    },
    Established {
        local: Ipv4Endpoint,
        remote: Ipv4Endpoint,
    },
    // This connection has been migrated out.
    MigratedOut {
        local: Ipv4Endpoint,
        remote: Ipv4Endpoint,
    }
}

pub struct Inner<RT: Runtime> {
    isn_generator: IsnGenerator,

    ephemeral_ports: EphemeralPorts,

    // FD -> local port
    pub(crate) sockets: HashMap<IoQueueDescriptor, Socket>,

    passive: HashMap<Ipv4Endpoint, PassiveSocket<RT>>,
    connecting: HashMap<(Ipv4Endpoint, Ipv4Endpoint), ActiveOpenSocket<RT>>,
    pub(crate) established: HashMap<(Ipv4Endpoint, Ipv4Endpoint), EstablishedSocket<RT>>,
    // Quick look up of IP addresses we have migrated in. Used by [Peer] to TODO
    migrated_in_connections: HashSet<Ipv4Addr>,
    rt: RT,
    arp: ArpPeer<RT>,

    pub(crate) dead_socket_tx: mpsc::UnboundedSender<IoQueueDescriptor>,
}

pub struct TcpPeer<RT: Runtime> {
    pub(crate) inner: Rc<RefCell<Inner<RT>>>,
}

impl<RT: Runtime> TcpPeer<RT> {
    pub fn new(rt: RT, arp: ArpPeer<RT>) -> Self {
        let (tx, rx) = mpsc::unbounded();
        let inner = Rc::new(RefCell::new(Inner::new(rt.clone(), arp, tx, rx)));
        Self { inner }
    }

    pub fn ip_migrated_in(&self, ip: &Ipv4Addr) -> bool{
        self.inner.borrow().migrated_in_connections.contains(ip)
    }

    /// Opens a TCP socket.
    pub fn do_socket(&self, fd: IoQueueDescriptor) {
        #[cfg(feature = "profiler")]
        timer!("tcp::socket");

        let mut inner = self.inner.borrow_mut();

        // Sanity check.
        assert_eq!(
            inner.sockets.contains_key(&fd),
            false,
            "file descriptor in use"
        );

        let socket = Socket::Inactive { local: None };
        inner.sockets.insert(fd, socket);
    }

    pub fn bind(&self, fd: IoQueueDescriptor, addr: Ipv4Endpoint) -> Result<(), Fail> {
        let mut inner = self.inner.borrow_mut();
        if addr.get_port() >= ip::Port::first_private_port() {
            return Err(Fail::Malformed {
                details: "Port number in private port range",
            });
        }
        match inner.sockets.get_mut(&fd) {
            Some(Socket::Inactive { ref mut local }) => {
                *local = Some(addr);
                Ok(())
            }
            _ => Err(Fail::Malformed {
                details: "Invalid file descriptor",
            }),
        }
    }

    pub fn receive(&self, ip_header: &Ipv4Header, buf: RT::Buf) -> Result<(), Fail> {
        self.inner.borrow_mut().receive(ip_header, buf)
    }

    pub fn listen(&self, fd: IoQueueDescriptor, backlog: usize) -> Result<(), Fail> {
        let mut inner = self.inner.borrow_mut();
        let local = match inner.sockets.get_mut(&fd) {
            Some(Socket::Inactive { local: Some(local) }) => *local,
            _ => {
                return Err(Fail::Malformed {
                    details: "Invalid file descriptor",
                })
            }
        };
        // TODO: Should this move to bind?
        if inner.passive.contains_key(&local) {
            return Err(Fail::ResourceBusy {
                details: "Port already in use",
            });
        }

        let socket = PassiveSocket::new(local, backlog, inner.rt.clone(), inner.arp.clone());
        assert!(inner.passive.insert(local, socket).is_none());
        inner.sockets.insert(fd, Socket::Listening { local });
        Ok(())
    }

    /// Accepts an incoming connection.
    pub fn do_accept(&self, fd: IoQueueDescriptor, newfd: IoQueueDescriptor) -> AcceptFuture<RT> {
        AcceptFuture {
            fd,
            newfd,
            inner: self.inner.clone(),
        }
    }

    /// Handles an incoming connection.
    pub fn poll_accept(
        &self,
        fd: IoQueueDescriptor,
        newfd: IoQueueDescriptor,
        ctx: &mut Context,
    ) -> Poll<Result<IoQueueDescriptor, Fail>> {
        let mut inner_ = self.inner.borrow_mut();
        let inner = &mut *inner_;

        let local = match inner.sockets.get(&fd) {
            Some(Socket::Listening { local }) => *local,
            Some(..) => {
                return Poll::Ready(Err(Fail::Malformed {
                    details: "Socket not listening",
                }))
            }
            None => return Poll::Ready(Err(Fail::Malformed { details: "Bad FD" })),
        };
        let passive = inner
            .passive
            .get_mut(&local)
            .expect("sockets/local inconsistency");
        let cb = match passive.poll_accept(ctx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(e)) => e,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
        };
        let established = EstablishedSocket::new(cb, newfd, inner.dead_socket_tx.clone());
        let key = (established.cb.get_local(), established.cb.get_remote());

        let socket = Socket::Established {
            local: established.cb.get_local(),
            remote: established.cb.get_remote(),
        };
        assert!(inner.sockets.insert(newfd, socket).is_none());
        assert!(inner.established.insert(key, established).is_none());

        Poll::Ready(Ok(newfd))
    }

    pub fn connect(&self, fd: IoQueueDescriptor, remote: Ipv4Endpoint) -> ConnectFuture<RT> {
        let mut inner = self.inner.borrow_mut();

        let r = try {
            match inner.sockets.get_mut(&fd) {
                Some(Socket::Inactive { .. }) => (),
                _ => Err(Fail::Malformed {
                    details: "Invalid file descriptor",
                })?,
            }

            // TODO: We need to free these!
            let local_port = inner.ephemeral_ports.alloc()?;
            let local = Ipv4Endpoint::new(inner.rt.local_ipv4_addr(), local_port);

            let socket = Socket::Connecting { local, remote };
            inner.sockets.insert(fd, socket);

            let local_isn = inner.isn_generator.generate(&local, &remote);
            let key = (local, remote);
            let socket = ActiveOpenSocket::new(
                local_isn,
                local,
                remote,
                inner.rt.clone(),
                inner.arp.clone(),
            );
            assert!(inner.connecting.insert(key, socket).is_none());
            fd
        };
        let state = match r {
            Ok(..) => ConnectFutureState::InProgress,
            Err(e) => ConnectFutureState::Failed(e),
        };
        ConnectFuture {
            fd,
            state,
            inner: self.inner.clone(),
        }
    }

    pub fn poll_recv(
        &self,
        fd: IoQueueDescriptor,
        ctx: &mut Context,
    ) -> Poll<Result<RT::Buf, Fail>> {
        let inner = self.inner.borrow_mut();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(Socket::Connecting { .. }) => {
                return Poll::Ready(Err(Fail::Malformed {
                    details: "poll_recv(): socket connecting",
                }))
            }
            Some(Socket::Inactive { .. }) => {
                return Poll::Ready(Err(Fail::Malformed {
                    details: "poll_recv(): socket inactive",
                }))
            }
            Some(Socket::Listening { .. }) => {
                return Poll::Ready(Err(Fail::Malformed {
                    details: "poll_recv(): socket listening",
                }))
            }
            Some(Socket::MigratedOut { .. }) => {
                return Poll::Ready(Err(Fail::ConnectionMigratedOut))
            }
            None => return Poll::Ready(Err(Fail::Malformed { details: "Bad FD" })),
        };
        match inner.established.get(&key) {
            Some(ref s) => s.poll_recv(ctx),
            None => Poll::Ready(Err(Fail::Malformed {
                details: "Socket not established",
            })),
        }
    }

    pub fn push(&self, fd: IoQueueDescriptor, buf: RT::Buf) -> PushFuture<RT> {
        let err = match self.send(fd, buf) {
            Ok(()) => None,
            Err(e) => Some(e),
        };
        PushFuture {
            fd,
            err,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn pop(&self, fd: IoQueueDescriptor) -> PopFuture<RT> {
        PopFuture {
            fd,
            inner: self.inner.clone(),
        }
    }

    fn send(&self, fd: IoQueueDescriptor, buf: RT::Buf) -> Result<(), Fail> {
        let inner = self.inner.borrow_mut();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => {
                return Err(Fail::Malformed {
                    details: "Socket not established",
                })
            }
            None => return Err(Fail::Malformed { details: "Bad FD" }),
        };
        match inner.established.get(&key) {
            Some(ref s) => s.send(buf),
            None => Err(Fail::Malformed {
                details: "Socket not established",
            }),
        }
    }

    /// Closes a TCP socket.
    pub fn do_close(&self, fd: IoQueueDescriptor) -> Result<(), Fail> {
        let inner = self.inner.borrow_mut();

        match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => {
                let key = (*local, *remote);
                match inner.established.get(&key) {
                    Some(ref s) => s.close()?,
                    None => {
                        return Err(Fail::Malformed {
                            details: "Socket not established",
                        })
                    }
                }
            }

            Some(..) => {
                return Err(Fail::Unsupported {
                    details: "implement close for listening sockets",
                })
            }
            None => return Err(Fail::Malformed { details: "Bad FD" }),
        }

        Ok(())
    }

    pub fn remote_mss(&self, fd: IoQueueDescriptor) -> Result<usize, Fail> {
        let inner = self.inner.borrow();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => {
                return Err(Fail::Malformed {
                    details: "Socket not established",
                })
            }
            None => return Err(Fail::Malformed { details: "Bad FD" }),
        };
        match inner.established.get(&key) {
            Some(ref s) => Ok(s.remote_mss()),
            None => Err(Fail::Malformed {
                details: "Socket not established",
            }),
        }
    }

    pub fn current_rto(&self, fd: IoQueueDescriptor) -> Result<Duration, Fail> {
        let inner = self.inner.borrow();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => {
                return Err(Fail::Malformed {
                    details: "Socket not established",
                })
            }
            None => return Err(Fail::Malformed { details: "Bad FD" }),
        };
        match inner.established.get(&key) {
            Some(ref s) => Ok(s.current_rto()),
            None => Err(Fail::Malformed {
                details: "Socket not established",
            }),
        }
    }

    pub fn endpoints(&self, fd: IoQueueDescriptor) -> Result<(Ipv4Endpoint, Ipv4Endpoint), Fail> {
        let inner = self.inner.borrow();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => {
                return Err(Fail::Malformed {
                    details: "Socket not established",
                })
            }
            None => return Err(Fail::Malformed { details: "Bad FD" }),
        };
        match inner.established.get(&key) {
            Some(ref s) => Ok(s.endpoints()),
            None => Err(Fail::Malformed {
                details: "Socket not established",
            }),
        }
    }

    pub fn migrate_in_tcp_connection(&mut self, state: TcpState<RT>, qd: IoQueueDescriptor) -> Result<(), Fail> {
        let mut inner = self.inner.borrow_mut();

        // Check if keys already exist first. This way we don't have to undo changes we make to
        // the state.

        if inner.established.contains_key(&(state.local, state.remote)) {
            debug!("Key already exists in established hashmap.");
            // TODO: Not sure if there is a better error to use here.
            return Err(Fail::ResourceBusy { details: "This connection already exists." })
        }

        // Connection should either not exist or have been migrated out (and now we are migrating
        // it back in).
        match inner.sockets.entry(qd) {
            Entry::Occupied(mut e) => {
                match e.get_mut() {
                    e@Socket::MigratedOut { .. } => {
                      *e = Socket::Established { local: state.local, remote: state.remote };
                    }
                    _ => {
                        debug!("Key already exists in sockets hashmap.");
                        return Err(Fail::BadFileDescriptor {})
                    }
                }
            }
            Entry::Vacant(v) => {
                let socket = Socket::Established { local: state.local, remote: state.remote };
                v.insert(socket);
            }
        }

        let receive_queue = ReceiveQueue::migrated_in(
            state.rcv_nxt,
            state.base_seq_no,
            state.recv_seq_no,
            state.receive_queue,
        );

        let sender = Sender::migrated_in(
            state.snd_una,
            state.snd_nxt,
            state.snd_wnd,
            state.sender_window_scale,
            state.sender_mss,
            cc::None::new,
            None,
            state.retransmit_queue,
            state.unacked_queue,
        );

        let cb = ControlBlock::imported_connection(
            state.local,
            state.remote,
            inner.rt.clone(),
            inner.arp.clone(),
            // TODO: Should this be the origin or destination TCP stack value?
            inner.rt.tcp_options().get_ack_delay_timeout(),
            receive_queue,
            state.max_window_size,
            state.receiver_window_scale,
            sender,
        );

        let established = EstablishedSocket::new(cb, qd, inner.dead_socket_tx.clone());

        if let Some(_) = inner.established.insert((state.local, state.remote), established) {
            // This condition should have been checked for at the beggining of this function.
            unreachable!();
        }

        // This IP might have already been migrated in. That's okay.
        inner.migrated_in_connections.insert(state.local.get_address());

        Ok(())
    }

    /// `zero_copy` if true, this will take the values of the queue instead of copying them. This
    /// should be faster, but it is no longer a side-effect free computation.
    pub fn get_tcp_state(&mut self, fd: IoQueueDescriptor, zero_copy: bool) -> Result<TcpState<RT>, Fail> {
        info!("Migrating out TCP State for {:?}!", fd);
        let details =  "We can only migrate out established connections.";

        let inner = self.inner.borrow_mut();

        match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => {
                let key = (*local, *remote);
                match inner.established.get(&key) {
                    Some(connection) => {
                        let cb = connection.cb.clone();
                        let mss = cb.get_sender_mss();
                        let (snd_wnd, _) = cb.get_snd_wnd();
                        let (snd_una, _) = cb.get_snd_una();

                        let (snd_nxt, _) = cb.get_snd_nxt();
                        let (rcv_nxt, _) = cb.get_rcv_nxt();
                        let receiver_window_scale = cb.get_receiver_window_scale();
                        let sender_window_scale = cb.sender.get_window_scale();
                        let max_window_size = cb.get_max_window_scale();

                        let base_seq_no: SeqNumber = cb.receive_queue.base_seq_no.get();
                        let recv_seq_no: SeqNumber = cb.receive_queue.recv_seq_no.get();

                        let receive_queue: VecDeque<RT::Buf> = cb.take_receive_queue();
                        let retrans_queue: VecDeque<RT::Buf> = cb.take_retransmission_queue();
                        let unack_queue: VecDeque<UnackedSegment<RT>> = cb.sender.take_unacked_queue();

                        let state = TcpState::new(*remote, *local, receive_queue, retrans_queue, unack_queue, mss, receiver_window_scale, sender_window_scale, max_window_size, rcv_nxt, snd_una, snd_nxt, snd_wnd, base_seq_no, recv_seq_no);

                        // debug!("TCP State: {:?}", state);
                        Ok(state)
                    }
                    None => {
                        return Err(Fail::Malformed {
                            details
                        })
                    }
                }
            }
            _ => {
                return Err(Fail::Malformed { details });
            },
        }
    }


    /// 1) Change status of our socket to MigratedOut
    /// 2) Change status of ControlBlock state to Migrated out.
    /// 3) Remove socket from Established hashmap.
    pub fn migrate_out_connection(&self, fd: &IoQueueDescriptor) -> Result<(), Fail> {
        let mut inner = self.inner.borrow_mut();
        let socket = inner.sockets.get_mut(fd);

        let (local, remote) = match socket {
            None => {
                debug!("No entry in `sockets` for fd: {:?}", fd);
                return Err(Fail::BadFileDescriptor {});
            }
            Some(Socket::Established { local, remote }) => {
                (*local, *remote)
            }
            s => panic!("Unsupported Socket variant: {:?} for migrating out.", s),
        };

        // 1) Change status of our socket to MigratedOut
        *socket.unwrap() = Socket::MigratedOut { local, remote };

        // 2) Change status of ControlBlock state to Migrated out.
        let key = (local, remote);
        let mut entry = match inner.established.entry(key) {
            Entry::Occupied(entry) => entry,
            Entry::Vacant(_) => Err(Fail::Malformed {
                details: "Socket not established",
            })?,
        };

        let established = entry.get_mut();
        let (state, _) = established.cb.get_state();
        match state {
            State::Established => {}
            s => panic!("We only migrate out established conn. Found:  {:?}", s),
        }

        // 3) Remove socket from Established hashmap.
        if let None = inner.established.remove(&key) {
                // This panic is okay. This should never happen and represents an internal error
                // in our implementation.
                panic!("Established socket somehow missing.");
        }

        Ok(())
    }
}

impl<RT: Runtime> Inner<RT> {
    fn new(
        rt: RT,
        arp: ArpPeer<RT>,
        dead_socket_tx: mpsc::UnboundedSender<IoQueueDescriptor>,
        _dead_socket_rx: mpsc::UnboundedReceiver<IoQueueDescriptor>,
    ) -> Self {
        Self {
            isn_generator: IsnGenerator::new(rt.rng_gen()),
            ephemeral_ports: EphemeralPorts::new(&rt),
            sockets: HashMap::new(),
            passive: HashMap::new(),
            connecting: HashMap::new(),
            established: HashMap::new(),
            migrated_in_connections: Default::default(),
            rt,
            arp,
            dead_socket_tx,
        }
    }

    fn receive(&mut self, ip_hdr: &Ipv4Header, buf: RT::Buf) -> Result<(), Fail> {
        let tcp_options = self.rt.tcp_options();
        let (tcp_hdr, data) = TcpHeader::parse(ip_hdr, buf, tcp_options.get_rx_checksum_offload())?;
        debug!("TCP received {:?}", tcp_hdr);
        let local = Ipv4Endpoint::new(ip_hdr.dst_addr(), tcp_hdr.dst_port);
        let remote = Ipv4Endpoint::new(ip_hdr.src_addr(), tcp_hdr.src_port);

        if remote.get_address().is_broadcast()
            || remote.get_address().is_multicast()
            || remote.get_address().is_unspecified()
        {
            return Err(Fail::Malformed {
                details: "Invalid address type",
            });
        }
        let key = (local, remote);

        if let Some(s) = self.established.get(&key) {
            debug!("Routing to established connection: {:?}", key);
            s.receive(&tcp_hdr, data);
            return Ok(());
        }
        if let Some(s) = self.connecting.get_mut(&key) {
            debug!("Routing to connecting connection: {:?}", key);
            s.receive(&tcp_hdr);
            return Ok(());
        }
        let (local, _) = key;
        if let Some(s) = self.passive.get_mut(&local) {
            debug!("Routing to passive connection: {:?}", local);
            return s.receive(ip_hdr, &tcp_hdr);
        }

        // The packet isn't for an open port; send a RST segment.
        debug!("Sending RST for {:?}, {:?}", local, remote);
        self.send_rst(&local, &remote)?;
        Ok(())
    }

    fn send_rst(&mut self, local: &Ipv4Endpoint, remote: &Ipv4Endpoint) -> Result<(), Fail> {
        // TODO: Make this work pending on ARP resolution if needed.
        let remote_link_addr =
            self.arp
                .try_query(remote.get_address())
                .ok_or(Fail::ResourceNotFound {
                    details: "RST destination not in ARP cache",
                })?;

        let mut tcp_hdr = TcpHeader::new(local.get_port(), remote.get_port());
        tcp_hdr.rst = true;

        let segment = TcpSegment {
            ethernet2_hdr: Ethernet2Header::new(
                remote_link_addr,
                self.rt.local_link_addr(),
                EtherType2::Ipv4,
            ),
            ipv4_hdr: Ipv4Header::new(
                local.get_address(),
                remote.get_address(),
                Ipv4Protocol2::Tcp,
            ),
            tcp_hdr,
            data: RT::Buf::empty(),
            tx_checksum_offload: self.rt.tcp_options().get_rx_checksum_offload(),
        };
        self.rt.transmit(segment);

        Ok(())
    }

    pub(super) fn poll_connect_finished(
        &mut self,
        fd: IoQueueDescriptor,
        context: &mut Context,
    ) -> Poll<Result<(), Fail>> {
        let key = match self.sockets.get(&fd) {
            Some(Socket::Connecting { local, remote }) => (*local, *remote),
            Some(..) => {
                return Poll::Ready(Err(Fail::Malformed {
                    details: "Socket not connecting",
                }))
            }
            None => return Poll::Ready(Err(Fail::Malformed { details: "Bad FD" })),
        };

        let result = {
            let socket = match self.connecting.get_mut(&key) {
                Some(s) => s,
                None => {
                    return Poll::Ready(Err(Fail::Malformed {
                        details: "Socket not connecting",
                    }))
                }
            };
            match socket.poll_result(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(r) => r,
            }
        };
        self.connecting.remove(&key);

        let cb = result?;
        let socket = EstablishedSocket::new(cb, fd, self.dead_socket_tx.clone());
        assert!(self.established.insert(key, socket).is_none());
        let (local, remote) = key;
        self.sockets
            .insert(fd, Socket::Established { local, remote });

        Poll::Ready(Ok(()))
    }
}
