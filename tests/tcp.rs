// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![feature(new_uninit)]

mod common;

//==============================================================================
// Imports
//==============================================================================

use crate::common::{
    arp, libos::*, runtime::DummyRuntime, ALICE_IPV4, ALICE_MAC, BOB_IPV4, BOB_MAC, PORT_BASE,
};
use ::catnip::{operations::OperationResult, protocols::ipv4::Ipv4Endpoint, Catnip};
use ::crossbeam_channel::{self, Receiver, Sender};
use ::runtime::{fail::Fail, memory::Bytes, network::types::Port16, QDesc, QToken};
use ::std::{
    convert::TryFrom,
    net::Ipv4Addr,
    thread::{self, JoinHandle},
};

//==============================================================================
// Open/Close Passive Socket
//==============================================================================

/// Tests if a passive socket may be successfully opened and closed.
fn do_tcp_connection_setup(libos: &mut Catnip<DummyRuntime>, port: u16) {
    let port: Port16 = Port16::try_from(port).unwrap();
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

    // Open and close a connection.
    let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
    libos.bind(sockfd, local).unwrap();
    libos.listen(sockfd, 8).unwrap();
    match libos.close(sockfd) {
        Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
        Err(_) => (),
    };
}

#[test]
fn catnip_tcp_connection_setup() {
    let (tx, rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let mut libos: Catnip<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    do_tcp_connection_setup(&mut libos, PORT_BASE);
}

//==============================================================================
// Establish Connection
//==============================================================================

/// Tests if data can be successfully established.
fn do_tcp_establish_connection(port: u16) {
    let (alice_tx, alice_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        let port: Port16 = Port16::try_from(port).unwrap();
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
        libos.bind(sockfd, local).unwrap();
        libos.listen(sockfd, 8).unwrap();
        let qt: QToken = libos.accept(sockfd).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => panic!("close() on passive socket has failed"),
        };
        match libos.close(sockfd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = Port16::try_from(port).unwrap();
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
        let qt: QToken = libos.connect(sockfd, remote).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Close connection.
        match libos.close(sockfd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

#[test]
fn catnip_tcp_establish_connection() {
    do_tcp_establish_connection(PORT_BASE + 1)
}

//==============================================================================
// Push
//==============================================================================

/// Tests if data can be pushed.
fn do_tcp_push_remote(port: u16) {
    let (alice_tx, alice_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        let port: Port16 = Port16::try_from(port).unwrap();
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
        libos.bind(sockfd, local).unwrap();
        libos.listen(sockfd, 8).unwrap();
        let qt: QToken = libos.accept(sockfd).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Pop data.
        let qt: QToken = libos.pop(qd).unwrap();
        let (qd, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Pop(_, _) => (),
            _ => panic!("pop() has has failed {:?}", qr),
        }

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => panic!("close() on passive socket has failed"),
        };
        match libos.close(sockfd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = Port16::try_from(port).unwrap();
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
        let qt: QToken = libos.connect(sockfd, remote).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Cook some data.
        let bytes: Bytes = DummyLibOS::cook_data(32);

        // Push data.
        let qt: QToken = libos.push2(sockfd, &bytes).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() has failed"),
        }

        // Close connection.
        match libos.close(sockfd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

#[test]
fn catnip_tcp_push_remote() {
    do_tcp_push_remote(PORT_BASE + 2)
}

//==============================================================================
// Bad Socket
//==============================================================================

/// Tests for bad socket creation.
fn do_tcp_bad_socket() {
    let (tx, rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let mut libos: Catnip<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    let domains: Vec<libc::c_int> = vec![
        libc::AF_ALG,
        libc::AF_APPLETALK,
        libc::AF_ASH,
        libc::AF_ATMPVC,
        libc::AF_ATMSVC,
        libc::AF_AX25,
        libc::AF_BLUETOOTH,
        libc::AF_BRIDGE,
        libc::AF_CAIF,
        libc::AF_CAN,
        libc::AF_DECnet,
        libc::AF_ECONET,
        libc::AF_IB,
        libc::AF_IEEE802154,
        // libc::AF_INET,
        libc::AF_INET6,
        libc::AF_IPX,
        libc::AF_IRDA,
        libc::AF_ISDN,
        libc::AF_IUCV,
        libc::AF_KEY,
        libc::AF_LLC,
        libc::AF_LOCAL,
        libc::AF_MPLS,
        libc::AF_NETBEUI,
        libc::AF_NETLINK,
        libc::AF_NETROM,
        libc::AF_NFC,
        libc::AF_PACKET,
        libc::AF_PHONET,
        libc::AF_PPPOX,
        libc::AF_RDS,
        libc::AF_ROSE,
        libc::AF_ROUTE,
        libc::AF_RXRPC,
        libc::AF_SECURITY,
        libc::AF_SNA,
        libc::AF_TIPC,
        libc::AF_UNIX,
        libc::AF_UNSPEC,
        libc::AF_VSOCK,
        libc::AF_WANPIPE,
        libc::AF_X25,
        libc::AF_XDP,
    ];

    let scoket_types: Vec<libc::c_int> = vec![
        libc::SOCK_DCCP,
        // libc::SOCK_DGRAM,
        libc::SOCK_PACKET,
        libc::SOCK_RAW,
        libc::SOCK_RDM,
        libc::SOCK_SEQPACKET,
        // libc::SOCK_STREAM,
    ];

    // Invalid domain.
    for d in domains {
        let sockfd: Result<QDesc, Fail> = libos.socket(d, libc::SOCK_STREAM, 0);
        let e: Fail = sockfd.unwrap_err();
        assert_eq!(e.errno, libc::ENOTSUP);
    }

    // Invalid socket tpe.
    for t in scoket_types {
        let sockfd: Result<QDesc, Fail> = libos.socket(libc::AF_INET, t, 0);
        let e: Fail = sockfd.unwrap_err();
        assert_eq!(e.errno, libc::ENOTSUP);
    }
}

#[test]
fn catnip_tcp_bad_socket() {
    do_tcp_bad_socket()
}

//==============================================================================
// Bad Bind
//==============================================================================

/// Test bad calls for `bind()`.
fn do_tcp_bad_bind(port: u16) {
    let (tx, rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let mut libos: Catnip<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    // Invalid file descriptor.
    let port: Port16 = Port16::try_from(port).unwrap();
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);
    let e: Fail = libos.bind(QDesc::from(0), local).unwrap_err();
    assert_eq!(e.errno, libc::EBADF);
}

#[test]
fn catnip_tcp_bad_bind() {
    do_tcp_bad_bind(PORT_BASE + 3);
}

//==============================================================================
// Bad Listen
//==============================================================================

/// Tests bad calls for `listen()`.
fn do_tcp_bad_listen(port: u16) {
    let (tx, rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let mut libos: Catnip<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    let port: Port16 = Port16::try_from(port).unwrap();
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

    // Invalid file descriptor.
    let e: Fail = libos.listen(QDesc::from(0), 8).unwrap_err();
    assert_eq!(e.errno, libc::EBADF);

    // Invalid backlog length
    let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
    libos.bind(sockfd, local).unwrap();
    let e: Fail = libos.listen(sockfd, 0).unwrap_err();
    assert_eq!(e.errno, libc::EINVAL);
    libos.close(sockfd).unwrap_err();
}

#[test]
fn catnip_tcp_bad_listen() {
    do_tcp_bad_listen(PORT_BASE + 4);
}

//==============================================================================
// Bad Accept
//==============================================================================

/// Tests bad calls for `accept()`.
fn do_tcp_bad_accept() {
    let (tx, rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let mut libos: Catnip<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    // Invalid file descriptor.
    let e: Fail = libos.accept(QDesc::from(0)).unwrap_err();
    assert_eq!(e.errno, libc::EBADF);
}

#[test]
fn catnip_tcp_bad_accept() {
    do_tcp_bad_accept();
}

//==============================================================================
// Bad Accept
//==============================================================================

/// Tests if data can be successfully established.
fn do_tcp_bad_connect(port: u16) {
    let (alice_tx, alice_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());
        let port: Port16 = Port16::try_from(port).unwrap();
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
        libos.bind(sockfd, local).unwrap();
        libos.listen(sockfd, 8).unwrap();
        let qt: QToken = libos.accept(sockfd).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => {
                panic!("close() on passive socket has failed")
            }
        };
        match libos.close(sockfd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = Port16::try_from(port).unwrap();
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Bad file descriptor.
        let e: Fail = libos.connect(QDesc::from(0), remote).unwrap_err();
        assert_eq!(e.errno, libc::EBADF);

        // Bad endpoint.
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(Ipv4Addr::new(0, 0, 0, 0), port);
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
        let qt: QToken = libos.connect(sockfd, remote).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Connect => panic!("connect() should have failed"),
            _ => (),
        }

        // Close connection.
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0).unwrap();
        let qt: QToken = libos.connect(sockfd, remote).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

#[test]
fn catnip_tcp_bad_connect() {
    do_tcp_bad_connect(PORT_BASE + 5)
}
