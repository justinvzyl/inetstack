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
use ::runtime::{memory::Bytes, network::types::Port16, QDesc, QToken};
use ::std::{
    convert::TryFrom,
    thread::{self, JoinHandle},
};

//==============================================================================
// Connect
//==============================================================================

/// Tests if a connection can be successfully established and closed to a remote
/// endpoint.
#[test]
fn udp_connect_remote() {
    let (tx, rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let mut libos: Catnip<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    let port: Port16 = Port16::try_from(PORT_BASE).unwrap();
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

    // Open and close a connection.
    let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_DGRAM, 0).unwrap();
    libos.bind(sockfd, local).unwrap();
    libos.close(sockfd).unwrap();
}

/// Tests if a connection can be successfully established in loopback mode.
#[test]
fn udp_connect_loopback() {
    let (tx, rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let mut libos: Catnip<DummyRuntime> = DummyLibOS::new(ALICE_MAC, ALICE_IPV4, tx, rx, arp());

    let port: Port16 = Port16::try_from(PORT_BASE).unwrap();
    let local: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, port);

    // Open and close a connection.
    let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_DGRAM, 0).unwrap();
    libos.bind(sockfd, local).unwrap();
    libos.close(sockfd).unwrap();
}

//==============================================================================
// Push
//==============================================================================

/// Tests if data can be successfully pushed/popped form a local endpoint to
/// itself.
#[test]
fn udp_push_remote() {
    let (alice_tx, alice_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();

    let bob_port: Port16 = Port16::try_from(PORT_BASE).unwrap();
    let bob_addr: Ipv4Endpoint = Ipv4Endpoint::new(BOB_IPV4, bob_port);
    let alice_port: Port16 = Port16::try_from(PORT_BASE).unwrap();
    let alice_addr: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, alice_port);

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_DGRAM, 0).unwrap();
        libos.bind(sockfd, alice_addr).unwrap();

        // Cook some data.
        let bytes: Bytes = DummyLibOS::cook_data(32);

        // Push data.
        let qt: QToken = libos.pushto2(sockfd, &bytes, bob_addr).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() failed"),
        }

        // Pop data.
        let qt: QToken = libos.pop(sockfd).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Pop(_, _) => (),
            _ => panic!("pop() failed"),
        }

        // Close connection.
        libos.close(sockfd).unwrap();
    });

    let bob: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(BOB_MAC, BOB_IPV4, bob_tx, alice_rx, arp());

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_DGRAM, 0).unwrap();
        libos.bind(sockfd, bob_addr).unwrap();

        // Pop data.
        let qt: QToken = libos.pop(sockfd).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        let bytes: Bytes = match qr {
            OperationResult::Pop(_, bytes) => bytes,
            _ => panic!("pop() failed"),
        };

        // Push data.
        let qt: QToken = libos.pushto2(sockfd, &bytes, alice_addr).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() failed"),
        }

        // Close connection.
        libos.close(sockfd).unwrap();
    });

    alice.join().unwrap();
    bob.join().unwrap();
}

/// Tests if data can be successfully pushed/popped in loopback mode.
#[test]
fn udp_loopback() {
    let (alice_tx, alice_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();
    let (bob_tx, bob_rx): (Sender<Bytes>, Receiver<Bytes>) = crossbeam_channel::unbounded();

    let bob_port: Port16 = Port16::try_from(PORT_BASE).unwrap();
    let bob_addr: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, bob_port);
    let alice_port: Port16 = Port16::try_from(PORT_BASE).unwrap();
    let alice_addr: Ipv4Endpoint = Ipv4Endpoint::new(ALICE_IPV4, alice_port);

    let alice: JoinHandle<()> = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(ALICE_MAC, ALICE_IPV4, alice_tx, bob_rx, arp());

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_DGRAM, 0).unwrap();
        libos.bind(sockfd, alice_addr).unwrap();

        // Cook some data.
        let bytes: Bytes = DummyLibOS::cook_data(32);

        // Push data.
        let qt: QToken = libos.pushto2(sockfd, &bytes, bob_addr).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() failed"),
        }

        // Pop data.
        let qt: QToken = libos.pop(sockfd).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Pop(_, _) => (),
            _ => panic!("pop() failed"),
        }

        // Close connection.
        libos.close(sockfd).unwrap();
    });

    let bob = thread::spawn(move || {
        let mut libos: Catnip<DummyRuntime> =
            DummyLibOS::new(ALICE_MAC, ALICE_IPV4, bob_tx, alice_rx, arp());

        // Open connection.
        let sockfd: QDesc = libos.socket(libc::AF_INET, libc::SOCK_DGRAM, 0).unwrap();
        libos.bind(sockfd, bob_addr).unwrap();

        // Pop data.
        let qt: QToken = libos.pop(sockfd).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        let bytes: Bytes = match qr {
            OperationResult::Pop(_, bytes) => bytes,
            _ => panic!("pop() failed"),
        };

        // Push data.
        let qt: QToken = libos.pushto2(sockfd, &bytes, alice_addr).unwrap();
        let (_, qr): (QDesc, OperationResult<Bytes>) = libos.wait2(qt);
        match qr {
            OperationResult::Push => (),
            _ => panic!("push() failed"),
        }

        // Close connection.
        libos.close(sockfd).unwrap();
    });

    alice.join().unwrap();
    bob.join().unwrap();
}
