// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![feature(new_uninit)]

mod common;

//==============================================================================
// Imports
//==============================================================================

use crate::common::{
    arp,
    libos::*,
    runtime::DummyRuntime,
    ALICE_IPV4,
    ALICE_MAC,
    BOB_IPV4,
    BOB_MAC,
    PORT_BASE,
};
use ::crossbeam_channel::{
    self,
    Receiver,
    Sender,
};
use ::inetstack::{
    operations::OperationResult,
    protocols::ipv4::Ipv4Endpoint,
    InetStack,
};
use ::runtime::{
    memory::{
        Buffer,
        DataBuffer,
    },
    network::types::Port16,
    QDesc,
    QToken,
};
use ::std::{
    convert::TryFrom,
    net::Ipv4Addr,
    thread::{
        self,
        JoinHandle,
    },
};

//==============================================================================
// Open/Close Passive Socket
//==============================================================================

/// Tests if a passive socket may be successfully opened and closed.
#[test]
fn tcp_connection_setup() {
    let (tx, rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    let port: Port16 = match Port16::try_from(PORT_BASE) {
        Ok(port) => port,
        Err(e) => panic!("conversion failed: {:?}", e),
    };
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

    // Open and close a connection.
    let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
        Ok(sockqd) => sockqd,
        Err(e) => panic!("failed to create socket: {:?}", e),
    };
    match libos.bind(sockqd, local) {
        Ok(_) => (),
        Err(e) => panic!("bind() failed: {:?}", e),
    };
    match libos.listen(sockqd, 8) {
        Ok(_) => (),
        Err(e) => panic!("listen() failed: {:?}", e),
    };
    match libos.close(sockqd) {
        Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
        Err(_) => (),
    };
}

//==============================================================================
// Establish Connection
//==============================================================================

/// Tests if data can be successfully established.
#[test]
fn tcp_establish_connection() {
    let (alice_tx, alice_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        match libos.bind(sockqd, local) {
            Ok(_) => (),
            Err(e) => panic!("bind() failed: {:?}", e),
        };
        match libos.listen(sockqd, 8) {
            Ok(_) => (),
            Err(e) => panic!("listen() failed: {:?}", e),
        };
        let qt: QToken = match libos.accept(sockqd) {
            Ok(qt) => qt,
            Err(e) => panic!("accept() failed: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };

        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => panic!("close() on passive socket has failed"),
        };
        match libos.close(sockqd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        let qt: QToken = match libos.connect(sockqd, remote) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to establish connection: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Close connection.
        match libos.close(sockqd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

//==============================================================================
// Push
//==============================================================================

/// Tests if data can be pushed.
#[test]
fn tcp_push_remote() {
    let (alice_tx, alice_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        match libos.bind(sockqd, local) {
            Ok(_) => (),
            Err(e) => panic!("bind() failed: {:?}", e),
        };
        match libos.listen(sockqd, 8) {
            Ok(_) => (),
            Err(e) => panic!("listen() failed: {:?}", e),
        };
        let qt: QToken = match libos.accept(sockqd) {
            Ok(qt) => qt,
            Err(e) => panic!("accept() failed: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Pop data.
        let qt: QToken = match libos.pop(qd) {
            Ok(qt) => qt,
            Err(e) => panic!("pop() failed: {:?}", e),
        };
        let (qd, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Pop(_, _) => (),
            _ => panic!("pop() has has failed {:?}", qr),
        }

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => panic!("close() on passive socket has failed"),
        };
        match libos.close(sockqd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        let qt: QToken = match libos.connect(sockqd, remote) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to establish connection: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Cook some data.
        let bytes: Box<dyn Buffer> = DummyLibOS::cook_data(32);

        // Push data.
        let qt: QToken = match libos.push2(sockqd, &bytes) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to push: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() has failed"),
        }

        // Close connection.
        match libos.close(sockqd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

//==============================================================================
// Bad Socket
//==============================================================================

/// Tests for bad socket creation.
#[test]
fn tcp_bad_socket() {
    let (tx, rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

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
        match libos.socket(d, libc::SOCK_STREAM, 0) {
            Err(e) if e.errno == libc::ENOTSUP => (),
            _ => panic!("invalid call to socket() should fail with ENOTSUP"),
        };
    }

    // Invalid socket tpe.
    for t in scoket_types {
        match libos.socket(libc::AF_INET, t, 0) {
            Err(e) if e.errno == libc::ENOTSUP => (),
            _ => panic!("invalid call to socket() should fail with ENOTSUP"),
        };
    }
}

//==============================================================================
// Bad Bind
//==============================================================================

/// Test bad calls for `bind()`.
#[test]
fn tcp_bad_bind() {
    let (tx, rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    // Invalid queue descriptor.
    let port: Port16 = match Port16::try_from(PORT_BASE) {
        Ok(port) => port,
        Err(e) => panic!("conversion failed: {:?}", e),
    };
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);
    match libos.bind(QDesc::from(0), local) {
        Err(e) if e.errno == libc::EBADF => (),
        _ => panic!("invalid call to bind() should failed with EBADF"),
    };
}

//==============================================================================
// Bad Listen
//==============================================================================

/// Tests bad calls for `listen()`.
#[test]
fn tcp_bad_listen() {
    let (tx, rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    let port: Port16 = match Port16::try_from(PORT_BASE) {
        Ok(port) => port,
        Err(e) => panic!("conversion failed: {:?}", e),
    };
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

    // Invalid queue descriptor.
    match libos.listen(QDesc::from(0), 8) {
        Err(e) if e.errno == libc::EBADF => (),
        _ => panic!("invalid call to listen() should failed with EBADF"),
    };

    // Invalid backlog length
    let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
        Ok(sockqd) => sockqd,
        Err(e) => panic!("failed to create socket: {:?}", e),
    };
    match libos.bind(sockqd, local) {
        Ok(_) => (),
        Err(e) => panic!("bind() failed: {:?}", e),
    };
    match libos.listen(sockqd, 0) {
        Err(e) if e.errno == libc::EINVAL => (),
        _ => panic!("invalid call to listen() should failed with EINVAL"),
    };

    // Close socket.
    match libos.close(sockqd) {
        Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
        Err(_) => (),
    };
}

//==============================================================================
// Bad Accept
//==============================================================================

/// Tests bad calls for `accept()`.
#[test]
fn tcp_bad_accept() {
    let (tx, rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    // Invalid queue descriptor.
    match libos.accept(QDesc::from(0)) {
        Err(e) if e.errno == libc::EBADF => (),
        _ => panic!("invalid call to accept() should failed with EBADF"),
    };
}

//==============================================================================
// Bad Accept
//==============================================================================

/// Tests if data can be successfully established.
#[test]
fn tcp_bad_connect() {
    let (alice_tx, alice_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());
        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        match libos.bind(sockqd, local) {
            Ok(_) => (),
            Err(e) => panic!("bind() failed: {:?}", e),
        };
        match libos.listen(sockqd, 8) {
            Ok(_) => (),
            Err(e) => panic!("listen() failed: {:?}", e),
        };
        let qt: QToken = match libos.accept(sockqd) {
            Ok(qt) => qt,
            Err(e) => panic!("accept() failed: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => {
                panic!("close() on passive socket has failed")
            },
        };
        match libos.close(sockqd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Bad queue descriptor.
        match libos.connect(QDesc::from(0), remote) {
            Err(e) if e.errno == libc::EBADF => (),
            _ => panic!("invalid call to connect() should failed with EBADF"),
        };

        // Bad endpoint.
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(Ipv4Addr::new(0, 0, 0, 0), port);
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        let qt: QToken = match libos.connect(sockqd, remote) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to establish connection: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Connect => panic!("connect() should have failed"),
            _ => (),
        }

        // Close connection.
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        let qt: QToken = match libos.connect(sockqd, remote) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to establish connection: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Close connection.
        match libos.close(sockqd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

//==============================================================================
// Bad Close
//==============================================================================

/// Tests if bad calls t `close()`.
#[test]
fn tcp_bad_close() {
    let (alice_tx, alice_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        match libos.bind(sockqd, local) {
            Ok(_) => (),
            Err(e) => panic!("bind() failed: {:?}", e),
        };
        match libos.listen(sockqd, 8) {
            Ok(_) => (),
            Err(e) => panic!("listen() failed: {:?}", e),
        };
        let qt: QToken = match libos.accept(sockqd) {
            Ok(qt) => qt,
            Err(e) => panic!("accept() failed: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Close bad queue descriptor.
        let bad_qd: QDesc = 2.into();
        match libos.close(bad_qd) {
            Ok(_) => panic!("close() invalid file descriptir should fail"),
            Err(_) => (),
        };

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => panic!("close() on passive socket has failed"),
        };
        match libos.close(sockqd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };

        // Double close queue descriptor.
        match libos.close(qd) {
            Ok(_) => panic!("double close() should fail"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        let qt: QToken = match libos.connect(sockqd, remote) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to establish connection: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Close bad queue descriptor.
        let bad_qd: QDesc = 2.into();
        match libos.close(bad_qd) {
            Ok(_) => panic!("close() invalid queue descriptor should fail"),
            Err(_) => (),
        };

        // Close connection.
        match libos.close(sockqd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };

        // Double close queue descriptor.
        match libos.close(sockqd) {
            Ok(_) => panic!("double close() should fail"),
            Err(_) => (),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

//==============================================================================
// Bad Push
//==============================================================================

/// Tests bad calls to `push()`.
#[test]
fn tcp_bad_push() {
    let (alice_tx, alice_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        match libos.bind(sockqd, local) {
            Ok(_) => (),
            Err(e) => panic!("bind() failed: {:?}", e),
        };
        match libos.listen(sockqd, 8) {
            Ok(_) => (),
            Err(e) => panic!("listen() failed: {:?}", e),
        };
        let qt: QToken = match libos.accept(sockqd) {
            Ok(qt) => qt,
            Err(e) => panic!("accept() failed: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Pop data.
        let qt: QToken = match libos.pop(qd) {
            Ok(qt) => qt,
            Err(e) => panic!("pop() failed: {:?}", e),
        };
        let (qd, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Pop(_, _) => (),
            _ => panic!("pop() has has failed {:?}", qr),
        }

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => panic!("close() on passive socket has failed"),
        };
        match libos.close(sockqd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        let qt: QToken = match libos.connect(sockqd, remote) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to establish connection: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Cook some data.
        let bytes: Box<dyn Buffer> = DummyLibOS::cook_data(32);

        // Push to bad socket.
        match libos.push2(2.into(), &bytes) {
            Ok(_) => panic!("push2() to bad socket should fail."),
            Err(_) => (),
        };

        // Push bad data to socket.
        let zero_bytes: [u8; 0] = [];
        match libos.push2(sockqd, &DataBuffer::from_slice(&zero_bytes)) {
            Ok(_) => panic!("push2() zero-length slice should fail."),
            Err(_) => (),
        };

        // Push data.
        let qt: QToken = match libos.push2(sockqd, &bytes) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to push: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() has failed"),
        }

        // Close connection.
        match libos.close(sockqd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

//==============================================================================
// Bad Pop
//==============================================================================

/// Tests bad calls to `pop()`.
#[test]
fn tcp_bad_pop() {
    let (alice_tx, alice_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<DataBuffer>, Receiver<DataBuffer>) = crossbeam_channel::unbounded();

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        match libos.bind(sockqd, local) {
            Ok(_) => (),
            Err(e) => panic!("bind() failed: {:?}", e),
        };
        match libos.listen(sockqd, 8) {
            Ok(_) => (),
            Err(e) => panic!("listen() failed: {:?}", e),
        };
        let qt: QToken = match libos.accept(sockqd) {
            Ok(qt) => qt,
            Err(e) => panic!("accept() failed: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        let qd: QDesc = match qr {
            OperationResult::Accept(qd) => qd,
            _ => panic!("accept() has failed"),
        };

        // Pop from bad socket.
        match libos.pop(2.into()) {
            Ok(_) => panic!("pop() form bad socket should fail."),
            Err(_) => (),
        };

        // Pop data.
        let qt: QToken = match libos.pop(qd) {
            Ok(qt) => qt,
            Err(e) => panic!("pop() failed: {:?}", e),
        };
        let (qd, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Pop(_, _) => (),
            _ => panic!("pop() has has failed {:?}", qr),
        }

        // Close connection.
        match libos.close(qd) {
            Ok(_) => (),
            Err(_) => panic!("close() on passive socket has failed"),
        };
        match libos.close(sockqd) {
            Ok(_) => panic!("close() on listening socket should have failed (this is a known bug)"),
            Err(_) => (),
        };
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: InetStack<DummyRuntime> = DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        let port: Port16 = match Port16::try_from(PORT_BASE) {
            Ok(port) => port,
            Err(e) => panic!("conversion failed: {:?}", e),
        };
        let remote: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

        // Open connection.
        let sockqd: QDesc = match libos.socket(libc::AF_INET, libc::SOCK_STREAM, 0) {
            Ok(sockqd) => sockqd,
            Err(e) => panic!("failed to create socket: {:?}", e),
        };
        let qt: QToken = match libos.connect(sockqd, remote) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to establish connection: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Connect => (),
            _ => panic!("connect() has failed"),
        }

        // Cook some data.
        let bytes: Box<dyn Buffer> = DummyLibOS::cook_data(32);

        // Push data.
        let qt: QToken = match libos.push2(sockqd, &bytes) {
            Ok(qt) => qt,
            Err(e) => panic!("failed to push: {:?}", e),
        };
        let (_, qr): (QDesc, OperationResult) = match libos.wait2(qt) {
            Ok((qd, qr)) => (qd, qr),
            Err(e) => panic!("operation failed: {:?}", e.cause),
        };
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() has failed"),
        }

        // Close connection.
        match libos.close(sockqd) {
            Ok(_) => (),
            Err(_) => panic!("close() on active socket has failed"),
        };
    });

    alice.join().unwrap();
    bob.join().unwrap();
}
