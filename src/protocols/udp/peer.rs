// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//==============================================================================
// Imports
//==============================================================================

use super::{
    datagram::{
        UdpDatagram,
        UdpHeader,
    },
    futures::UdpPopFuture,
    queue::{
        SharedQueue,
        SharedQueueSlot,
    },
};
use crate::{
    futures::FutureOperation,
    protocols::{
        arp::ArpPeer,
        ethernet2::{
            EtherType2,
            Ethernet2Header,
        },
        ip::{
            EphemeralPorts,
            IpProtocol,
        },
        ipv4::Ipv4Header,
    },
};
use ::futures::FutureExt;
use ::libc::{
    EBADF,
    EEXIST,
    ENOTCONN,
};
use ::rand::{
    prelude::SmallRng,
    SeedableRng,
};
use ::runtime::{
    fail::Fail,
    memory::Buffer,
    network::{
        types::{
            IpAddr,
            MacAddress,
        },
        NetworkRuntime,
    },
    scheduler::SchedulerHandle,
    task::SchedulerRuntime,
    QDesc,
};
use ::std::{
    collections::HashMap,
    net::SocketAddr,
};

#[cfg(feature = "profiler")]
use ::runtime::perftools::timer;

//======================================================================================================================
// Constants
//======================================================================================================================

// Maximum size for receive queues (in messages).
const RECV_QUEUE_MAX_SIZE: usize = 1024;

// Maximum size for send queues (in messages).
const SEND_QUEUE_MAX_SIZE: usize = 1024;

//======================================================================================================================
// Structures
//======================================================================================================================

/// UDP Peer
pub struct UdpPeer<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> {
    /// Underlying runtime.
    rt: RT,
    /// Underlying ARP peer.
    arp: ArpPeer<RT>,
    /// Ephemeral ports.
    ephemeral_ports: EphemeralPorts,
    /// Opened sockets.
    sockets: HashMap<QDesc, Option<SocketAddr>>,
    /// Bound sockets.
    bound: HashMap<SocketAddr, SharedQueue<SharedQueueSlot<Buffer>>>,
    /// Queue of unset datagrams. This is shared across fast/slow paths.
    send_queue: SharedQueue<SharedQueueSlot<Buffer>>,
    /// Local link address.
    local_link_addr: MacAddress,
    /// Local IPv4 address.
    local_ip_addr: IpAddr,
    /// Offload checksum to hardware?
    checksum_offload: bool,

    /// The background co-routine sends unset UDP packets.
    /// We annotate it as unused because the compiler believes that it is never called which is not the case.
    #[allow(unused)]
    background: SchedulerHandle,
}

//======================================================================================================================
// Associate Functions
//======================================================================================================================

/// Associate functions for [UdpPeer].
impl<RT: SchedulerRuntime + NetworkRuntime + Clone + 'static> UdpPeer<RT> {
    /// Creates a Udp peer.
    pub fn new(
        rt: RT,
        rng_seed: [u8; 32],
        local_link_addr: MacAddress,
        local_ip_addr: IpAddr,
        offload_checksum: bool,
        arp: ArpPeer<RT>,
    ) -> Self {
        let send_queue: SharedQueue<SharedQueueSlot<Buffer>> =
            SharedQueue::<SharedQueueSlot<Buffer>>::new(SEND_QUEUE_MAX_SIZE);
        let future = Self::background_sender(
            rt.clone(),
            local_ip_addr,
            local_link_addr,
            offload_checksum,
            arp.clone(),
            send_queue.clone(),
        );
        let handle: SchedulerHandle = rt.spawn(FutureOperation::Background::<RT>(future.boxed_local()));
        let mut rng: SmallRng = SmallRng::from_seed(rng_seed);
        let ephemeral_ports: EphemeralPorts = EphemeralPorts::new(&mut rng);
        Self {
            rt: rt.clone(),
            arp,
            ephemeral_ports,
            sockets: HashMap::new(),
            bound: HashMap::new(),
            send_queue,
            local_link_addr,
            local_ip_addr,
            checksum_offload: offload_checksum,
            background: handle,
        }
    }

    /// Asynchronously send unsent datagrams to remote peer.
    async fn background_sender(
        rt: RT,
        local_ip_addr: IpAddr,
        local_link_addr: MacAddress,
        offload_checksum: bool,
        arp: ArpPeer<RT>,
        mut rx: SharedQueue<SharedQueueSlot<Buffer>>,
    ) {
        loop {
            // Grab next unsent datagram.
            match rx.pop().await {
                // Resolve remote address.
                Ok(SharedQueueSlot { local, remote, data }) => match arp.query(remote.ip().clone()).await {
                    // Send datagram.
                    Ok(link_addr) => {
                        Self::do_send(
                            rt.clone(),
                            local_ip_addr,
                            local_link_addr,
                            link_addr,
                            data,
                            &local,
                            &remote,
                            offload_checksum,
                        );
                    },
                    // ARP query failed.
                    Err(e) => warn!("Failed to send UDP datagram: {:?}", e),
                },
                // Pop from shared queue failed.
                Err(e) => warn!("Failed to send UDP datagram: {:?}", e),
            }
        }
    }

    /// Opens a UDP socket.
    pub fn do_socket(&mut self, qd: QDesc) -> Result<(), Fail> {
        #[cfg(feature = "profiler")]
        timer!("udp::socket");

        // Lookup for an existing socket associated to this queue descriptor.
        match self.sockets.contains_key(&qd) {
            // Que descriptor not used.
            false => {
                let socket: Option<SocketAddr> = None;
                self.sockets.insert(qd, socket);
                Ok(())
            },
            // Queue descriptor in use.
            true => return Err(Fail::new(EEXIST, "queue descriptor in use")),
        }
    }

    /// Binds a UDP socket to a local endpoint address.
    pub fn do_bind(&mut self, qd: QDesc, mut addr: SocketAddr) -> Result<(), Fail> {
        #[cfg(feature = "profiler")]
        timer!("udp::bind");

        // Local endpoint address in use.
        if self.bound.contains_key(&addr) {
            return Err(Fail::new(libc::EADDRINUSE, "address in use"));
        }

        // Check if this is an ephemeral port or a wildcard one.
        if EphemeralPorts::is_private(addr.port()) {
            // Allocate ephemeral port from the pool, to leave  ephemeral port allocator in a consistent state.
            self.ephemeral_ports.alloc_port(addr.port())?
        } else if addr.port() == 0 {
            // Allocate ephemeral port.
            // TODO: we should free this when closing.
            let new_port: u16 = self.ephemeral_ports.alloc_any()?;
            addr.set_port(new_port);
        }

        // Register local endpoint address.
        let ret: Result<(), Fail> = match self.sockets.get_mut(&qd) {
            Some(s) if s.is_none() => {
                *s = Some(addr);

                // Bind endpoint and create a receiver-side shared queue.
                let queue: SharedQueue<SharedQueueSlot<Buffer>> =
                    SharedQueue::<SharedQueueSlot<Buffer>>::new(RECV_QUEUE_MAX_SIZE);

                if self.bound.insert(addr, queue).is_some() {
                    Err(Fail::new(libc::EADDRINUSE, "address in use"))
                } else {
                    Ok(())
                }
            },
            _ => Err(Fail::new(libc::EBADF, "invalid queue descriptor")),
        };

        // Handle return value.
        match ret {
            Ok(_) => Ok(()),
            Err(e) => {
                // Rollback ephemeral port allocation.
                if EphemeralPorts::is_private(addr.port()) {
                    self.ephemeral_ports.free(addr.port());
                }
                Err(e)
            },
        }
    }

    /// Closes a UDP socket.
    pub fn do_close(&mut self, qd: QDesc) -> Result<(), Fail> {
        #[cfg(feature = "profiler")]
        timer!("udp::close");

        // Lookup associated endpoint.
        let socket: Option<SocketAddr> = match self.sockets.remove(&qd) {
            Some(s) => s,
            None => return Err(Fail::new(EBADF, "invalid queue descriptor")),
        };

        // Remove endpoint binding.
        match socket {
            Some(local) if self.bound.remove(&local).is_some() => Ok(()),
            _ => return Err(Fail::new(EBADF, "invalid queue descriptor")),
        }
    }

    /// Pushes data to a remote UDP peer.
    pub fn do_pushto(&self, qd: QDesc, data: Buffer, remote: SocketAddr) -> Result<(), Fail> {
        #[cfg(feature = "profiler")]
        timer!("udp::pushto");

        // Lookup associated endpoint.
        let local: SocketAddr = match self.sockets.get(&qd) {
            Some(s) if s.is_some() => s.unwrap(),
            _ => return Err(Fail::new(EBADF, "invalid queue descriptor")),
        };

        // Fast path: try to send the datagram immediately.
        if let Some(link_addr) = self.arp.try_query(remote.ip().clone()) {
            Self::do_send(
                self.rt.clone(),
                self.local_ip_addr,
                self.local_link_addr,
                link_addr,
                data,
                &local,
                &remote,
                self.checksum_offload,
            );
        }
        // Slow path: Defer send operation to the async path.
        else {
            self.send_queue.push(SharedQueueSlot { local, remote, data })?
        }

        Ok(())
    }

    /// Pops data from a socket.
    pub fn do_pop(&self, qd: QDesc) -> UdpPopFuture {
        #[cfg(feature = "profiler")]
        timer!("udp::pop");

        // Lookup associated receiver-side shared queue.
        let recv_queue: SharedQueue<SharedQueueSlot<Buffer>> = match self.sockets.get(&qd) {
            Some(s) if s.is_some() => self.bound.get(&s.unwrap()).unwrap().clone(),
            _ => panic!("invalid queue descriptor"),
        };

        // Issue pop operation.
        UdpPopFuture::new(qd, recv_queue)
    }

    /// Consumes the payload from a buffer.
    pub fn do_receive(&mut self, ipv4_hdr: &Ipv4Header, buf: Buffer) -> Result<(), Fail> {
        #[cfg(feature = "profiler")]
        timer!("udp::receive");

        // Parse datagram.
        let (hdr, data): (UdpHeader, Buffer) = UdpHeader::parse(ipv4_hdr, buf, self.checksum_offload)?;
        debug!("UDP received {:?}", hdr);

        let local: SocketAddr = SocketAddr::new(IpAddr::V4(ipv4_hdr.get_dest_addr()), hdr.dest_port());
        let remote: SocketAddr = SocketAddr::new(IpAddr::V4(ipv4_hdr.get_src_addr()), hdr.src_port());

        // Lookup associated receiver-side shared queue.
        let recv_queue: &mut SharedQueue<SharedQueueSlot<Buffer>> = match self.bound.get_mut(&local) {
            Some(q) => q,
            // TODO: Send ICMPv4 error in this condition.
            None => Err(Fail::new(ENOTCONN, "port not bound"))?,
        };

        // TODO: Drop this packet if local address/port pair is not bound.

        // Push data to the receiver-side shared queue. This will cause the
        // associated pool operation to be ready.
        recv_queue.push(SharedQueueSlot { local, remote, data }).unwrap();

        Ok(())
    }

    /// Sends a UDP datagram.
    fn do_send(
        rt: RT,
        local_ip_addr: IpAddr,
        local_link_addr: MacAddress,
        remote_link_addr: MacAddress,
        buf: Buffer,
        local: &SocketAddr,
        remote: &SocketAddr,
        offload_checksum: bool,
    ) {
        let udp_header: UdpHeader = UdpHeader::new(local.port(), remote.port());
        debug!("UDP send {:?}", udp_header);
        let datagram = UdpDatagram::new(
            Ethernet2Header::new(remote_link_addr, local_link_addr, EtherType2::Ipv4),
            Ipv4Header::new(
                match local_ip_addr {
                    IpAddr::V4(ipv4) => ipv4,
                    IpAddr::V6(_) => todo!(),
                },
                match remote.ip().clone() {
                    IpAddr::V4(ipv4) => ipv4,
                    IpAddr::V6(_) => todo!(),
                },
                IpProtocol::UDP,
            ),
            udp_header,
            buf,
            offload_checksum,
        );
        rt.transmit(datagram);
    }
}
