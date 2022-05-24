// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use super::{
    active_open::ActiveOpenSocket,
    established::EstablishedSocket,
    isn_generator::IsnGenerator,
    passive_open::PassiveSocket,
};
use crate::protocols::{
    arp::ArpPeer,
    ethernet2::{
        EtherType2,
        Ethernet2Header,
    },
    ip::{
        EphemeralPorts,
        IpProtocol,
    },
    ipv4::{
        Ipv4Endpoint,
        Ipv4Header,
    },
    tcp::{
        established::ControlBlock,
        operations::{
            AcceptFuture,
            ConnectFuture,
            ConnectFutureState,
            PopFuture,
            PushFuture,
        },
        segment::{
            TcpHeader,
            TcpSegment,
        },
    },
};
use ::futures::channel::mpsc;
use ::libc::{
    EADDRNOTAVAIL,
    EAGAIN,
    EBADF,
    EBADMSG,
    EBUSY,
    EINPROGRESS,
    EINVAL,
    ENOTCONN,
    ENOTSUP,
    EOPNOTSUPP,
};
use ::runtime::{
    fail::Fail,
    memory::{
        Buffer,
        DataBuffer,
    },
    QDesc,
    Runtime,
};
use ::std::{
    cell::{
        RefCell,
        RefMut,
    },
    collections::HashMap,
    rc::Rc,
    task::{
        Context,
        Poll,
    },
    time::Duration,
};

#[cfg(feature = "profiler")]
use perftools::timer;

//==============================================================================
// Enumerations
//==============================================================================

enum Socket {
    Inactive { local: Option<Ipv4Endpoint> },
    Listening { local: Ipv4Endpoint },
    Connecting { local: Ipv4Endpoint, remote: Ipv4Endpoint },
    Established { local: Ipv4Endpoint, remote: Ipv4Endpoint },
}

//==============================================================================
// Structures
//==============================================================================

pub struct Inner<RT: Runtime> {
    isn_generator: IsnGenerator,

    ephemeral_ports: EphemeralPorts,

    // FD -> local port
    sockets: HashMap<QDesc, Socket>,

    passive: HashMap<Ipv4Endpoint, PassiveSocket<RT>>,
    connecting: HashMap<(Ipv4Endpoint, Ipv4Endpoint), ActiveOpenSocket<RT>>,
    established: HashMap<(Ipv4Endpoint, Ipv4Endpoint), EstablishedSocket<RT>>,

    rt: RT,
    arp: ArpPeer<RT>,

    dead_socket_tx: mpsc::UnboundedSender<QDesc>,
}

pub struct TcpPeer<RT: Runtime> {
    pub(super) inner: Rc<RefCell<Inner<RT>>>,
}

//==============================================================================
// Associated FUnctions
//==============================================================================

impl<RT: Runtime> TcpPeer<RT> {
    pub fn new(rt: RT, arp: ArpPeer<RT>) -> Self {
        let (tx, rx) = mpsc::unbounded();
        let inner = Rc::new(RefCell::new(Inner::new(rt.clone(), arp, tx, rx)));
        Self { inner }
    }

    /// Opens a TCP socket.
    pub fn do_socket(&self, qd: QDesc) -> Result<(), Fail> {
        #[cfg(feature = "profiler")]
        timer!("tcp::socket");
        let mut inner: RefMut<Inner<RT>> = self.inner.borrow_mut();
        match inner.sockets.contains_key(&qd) {
            false => {
                let socket: Socket = Socket::Inactive { local: None };
                inner.sockets.insert(qd, socket);
                Ok(())
            },
            true => return Err(Fail::new(EBUSY, "queue descriptor in use")),
        }
    }

    pub fn bind(&self, fd: QDesc, addr: Ipv4Endpoint) -> Result<(), Fail> {
        let mut inner = self.inner.borrow_mut();
        if addr.get_port() >= EphemeralPorts::first_private_port() {
            return Err(Fail::new(EBADMSG, "Port number in private port range"));
        }

        // Check if address is already bound.
        for (_, socket) in &inner.sockets {
            match socket {
                Socket::Inactive { local: Some(local) }
                | Socket::Listening { local }
                | Socket::Connecting { local, remote: _ }
                | Socket::Established { local, remote: _ }
                    if *local == addr =>
                {
                    return Err(Fail::new(libc::EADDRINUSE, "address already in use"))
                },
                _ => (),
            }
        }

        match inner.sockets.get_mut(&fd) {
            Some(Socket::Inactive { ref mut local }) => match *local {
                Some(_) => return Err(Fail::new(libc::EINVAL, "socket is already bound to an address")),
                None => {
                    *local = Some(addr);
                    Ok(())
                },
            },
            _ => Err(Fail::new(EBADF, "invalid queue descriptor")),
        }
    }

    pub fn receive(&self, ip_header: &Ipv4Header, buf: Box<dyn Buffer>) -> Result<(), Fail> {
        self.inner.borrow_mut().receive(ip_header, buf)
    }

    pub fn listen(&self, fd: QDesc, backlog: usize) -> Result<(), Fail> {
        let mut inner = self.inner.borrow_mut();
        let local = match inner.sockets.get_mut(&fd) {
            Some(Socket::Inactive { local: Some(local) }) => *local,
            _ => return Err(Fail::new(EBADF, "invalid queue descriptor")),
        };
        // TODO: Should this move to bind?
        if inner.passive.contains_key(&local) {
            return Err(Fail::new(EADDRNOTAVAIL, "port already in use"));
        }

        let socket = PassiveSocket::new(local, backlog, inner.rt.clone(), inner.arp.clone());
        assert!(inner.passive.insert(local, socket).is_none());
        inner.sockets.insert(fd, Socket::Listening { local });
        Ok(())
    }

    /// Accepts an incoming connection.
    pub fn do_accept(&self, qd: QDesc, new_qd: QDesc) -> AcceptFuture<RT> {
        AcceptFuture::new(qd, new_qd, self.inner.clone())
    }

    /// Handles an incoming connection.
    pub fn poll_accept(&self, qd: QDesc, new_qd: QDesc, ctx: &mut Context) -> Poll<Result<QDesc, Fail>> {
        let mut inner_: RefMut<Inner<RT>> = self.inner.borrow_mut();
        let inner: &mut Inner<RT> = &mut *inner_;

        let local: &Ipv4Endpoint = match inner.sockets.get(&qd) {
            Some(Socket::Listening { local }) => local,
            Some(..) => return Poll::Ready(Err(Fail::new(EOPNOTSUPP, "socket not listening"))),
            None => return Poll::Ready(Err(Fail::new(EBADF, "bad file descriptor"))),
        };

        let passive: &mut PassiveSocket<RT> = inner.passive.get_mut(local).expect("sockets/local inconsistency");
        let cb: ControlBlock<RT> = match passive.poll_accept(ctx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(e)) => e,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
        };
        let established: EstablishedSocket<RT> = EstablishedSocket::new(cb, new_qd, inner.dead_socket_tx.clone());
        let key: (Ipv4Endpoint, Ipv4Endpoint) = (established.cb.get_local(), established.cb.get_remote());

        let socket: Socket = Socket::Established {
            local: established.cb.get_local(),
            remote: established.cb.get_remote(),
        };

        // TODO: Reset the connection if the following following check fails, instead of panicking.
        if inner.sockets.insert(new_qd, socket).is_some() {
            panic!("duplicate queue descriptor in sockets table");
        }

        // TODO: Reset the connection if the following following check fails, instead of panicking.
        if inner.established.insert(key, established).is_some() {
            panic!("duplicate queue descriptor in established sockets table");
        }

        Poll::Ready(Ok(new_qd))
    }

    pub fn connect(&self, fd: QDesc, remote: Ipv4Endpoint) -> ConnectFuture<RT> {
        let mut inner = self.inner.borrow_mut();

        let r = try {
            match inner.sockets.get_mut(&fd) {
                Some(Socket::Inactive { .. }) => (),
                _ => Err(Fail::new(EBADF, "invalid file descriptor"))?,
            }

            // TODO: We need to free these!
            let local_port = inner.ephemeral_ports.alloc()?;
            let local = Ipv4Endpoint::new(inner.rt.local_ipv4_addr(), local_port);

            let socket = Socket::Connecting { local, remote };
            inner.sockets.insert(fd, socket);

            let local_isn = inner.isn_generator.generate(&local, &remote);
            let key = (local, remote);
            let socket = ActiveOpenSocket::new(local_isn, local, remote, inner.rt.clone(), inner.arp.clone());
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

    pub fn poll_recv(&self, fd: QDesc, ctx: &mut Context) -> Poll<Result<Box<dyn Buffer>, Fail>> {
        let inner = self.inner.borrow_mut();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(Socket::Connecting { .. }) => return Poll::Ready(Err(Fail::new(EINPROGRESS, "socket connecting"))),
            Some(Socket::Inactive { .. }) => return Poll::Ready(Err(Fail::new(EBADF, "socket inactive"))),
            Some(Socket::Listening { .. }) => return Poll::Ready(Err(Fail::new(ENOTCONN, "socket listening"))),
            None => return Poll::Ready(Err(Fail::new(EBADF, "bad queue descriptor"))),
        };
        match inner.established.get(&key) {
            Some(ref s) => s.poll_recv(ctx),
            None => Poll::Ready(Err(Fail::new(ENOTCONN, "connection not established"))),
        }
    }

    pub fn push(&self, fd: QDesc, buf: Box<dyn Buffer>) -> PushFuture<RT> {
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

    pub fn pop(&self, fd: QDesc) -> PopFuture<RT> {
        PopFuture {
            fd,
            inner: self.inner.clone(),
        }
    }

    fn send(&self, fd: QDesc, buf: Box<dyn Buffer>) -> Result<(), Fail> {
        let inner = self.inner.borrow_mut();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => return Err(Fail::new(ENOTCONN, "connection not established")),
            None => return Err(Fail::new(EBADF, "bad queue descriptor")),
        };
        match inner.established.get(&key) {
            Some(ref s) => s.send(buf),
            None => Err(Fail::new(ENOTCONN, "connection not established")),
        }
    }

    /// Closes a TCP socket.
    pub fn do_close(&self, qd: QDesc) -> Result<(), Fail> {
        let mut inner: RefMut<Inner<RT>> = self.inner.borrow_mut();

        match inner.sockets.remove(&qd) {
            Some(Socket::Established { local, remote }) => {
                let key: (Ipv4Endpoint, Ipv4Endpoint) = (local, remote);
                match inner.established.get(&key) {
                    Some(ref s) => s.close()?,
                    None => return Err(Fail::new(ENOTCONN, "connection not established")),
                }
            },

            Some(..) => return Err(Fail::new(ENOTSUP, "close not implemented for listening sockets")),
            None => return Err(Fail::new(EBADF, "bad queue descriptor")),
        }

        Ok(())
    }

    pub fn remote_mss(&self, fd: QDesc) -> Result<usize, Fail> {
        let inner = self.inner.borrow();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => return Err(Fail::new(ENOTCONN, "connection not established")),
            None => return Err(Fail::new(EBADF, "bad queue descriptor")),
        };
        match inner.established.get(&key) {
            Some(ref s) => Ok(s.remote_mss()),
            None => Err(Fail::new(ENOTCONN, "connection not established")),
        }
    }

    pub fn current_rto(&self, fd: QDesc) -> Result<Duration, Fail> {
        let inner = self.inner.borrow();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => return Err(Fail::new(ENOTCONN, "connection not established")),
            None => return Err(Fail::new(EBADF, "bad queue descriptor")),
        };
        match inner.established.get(&key) {
            Some(ref s) => Ok(s.current_rto()),
            None => Err(Fail::new(ENOTCONN, "connection not established")),
        }
    }

    pub fn endpoints(&self, fd: QDesc) -> Result<(Ipv4Endpoint, Ipv4Endpoint), Fail> {
        let inner = self.inner.borrow();
        let key = match inner.sockets.get(&fd) {
            Some(Socket::Established { local, remote }) => (*local, *remote),
            Some(..) => return Err(Fail::new(ENOTCONN, "connection not established")),
            None => return Err(Fail::new(EBADF, "bad queue descriptor")),
        };
        match inner.established.get(&key) {
            Some(ref s) => Ok(s.endpoints()),
            None => Err(Fail::new(ENOTCONN, "connection not established")),
        }
    }
}

impl<RT: Runtime> Inner<RT> {
    fn new(
        rt: RT,
        arp: ArpPeer<RT>,
        dead_socket_tx: mpsc::UnboundedSender<QDesc>,
        _dead_socket_rx: mpsc::UnboundedReceiver<QDesc>,
    ) -> Self {
        Self {
            isn_generator: IsnGenerator::new(rt.rng_gen()),
            ephemeral_ports: EphemeralPorts::new(&rt),
            sockets: HashMap::new(),
            passive: HashMap::new(),
            connecting: HashMap::new(),
            established: HashMap::new(),
            rt,
            arp,
            dead_socket_tx,
        }
    }

    fn receive(&mut self, ip_hdr: &Ipv4Header, buf: Box<dyn Buffer>) -> Result<(), Fail> {
        let tcp_options = self.rt.tcp_options();
        let (mut tcp_hdr, data) = TcpHeader::parse(ip_hdr, buf, tcp_options.get_rx_checksum_offload())?;
        debug!("TCP received {:?}", tcp_hdr);
        let local = Ipv4Endpoint::new(ip_hdr.get_dest_addr(), tcp_hdr.dst_port);
        let remote = Ipv4Endpoint::new(ip_hdr.get_src_addr(), tcp_hdr.src_port);

        if remote.get_address().is_broadcast()
            || remote.get_address().is_multicast()
            || remote.get_address().is_unspecified()
        {
            return Err(Fail::new(EINVAL, "invalid address type"));
        }
        let key = (local, remote);

        if let Some(s) = self.established.get(&key) {
            debug!("Routing to established connection: {:?}", key);
            s.receive(&mut tcp_hdr, data);
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
        let remote_link_addr = self
            .arp
            .try_query(remote.get_address())
            .ok_or(Fail::new(EINVAL, "detination not in ARP cache"))?;

        let mut tcp_hdr = TcpHeader::new(local.get_port(), remote.get_port());
        tcp_hdr.rst = true;

        let segment = TcpSegment {
            ethernet2_hdr: Ethernet2Header::new(remote_link_addr, self.rt.local_link_addr(), EtherType2::Ipv4),
            ipv4_hdr: Ipv4Header::new(local.get_address(), remote.get_address(), IpProtocol::TCP),
            tcp_hdr,
            data: Box::new(DataBuffer::empty()),
            tx_checksum_offload: self.rt.tcp_options().get_rx_checksum_offload(),
        };
        self.rt.transmit(segment);

        Ok(())
    }

    pub(super) fn poll_connect_finished(&mut self, fd: QDesc, context: &mut Context) -> Poll<Result<(), Fail>> {
        let key = match self.sockets.get(&fd) {
            Some(Socket::Connecting { local, remote }) => (*local, *remote),
            Some(..) => return Poll::Ready(Err(Fail::new(EAGAIN, "socket not connecting"))),
            None => return Poll::Ready(Err(Fail::new(EBADF, "bad queue descriptor"))),
        };

        let result = {
            let socket = match self.connecting.get_mut(&key) {
                Some(s) => s,
                None => return Poll::Ready(Err(Fail::new(EAGAIN, "socket not connecting"))),
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
        self.sockets.insert(fd, Socket::Established { local, remote });

        Poll::Ready(Ok(()))
    }
}
