use std::convert::TryFrom;
use std::task::Context;
use std::time::Instant;
use futures::task::noop_waker_ref;
use runtime::queue::IoQueueDescriptor;
use crate::protocols::ip;
use crate::protocols::ipv4::Ipv4Endpoint;
use crate::protocols::tcp::tests::setup::connection_setup;
use crate::test_helpers;

#[test]
fn test_migration1() {
    let mut ctx = Context::from_waker(noop_waker_ref());
    let mut now = Instant::now();

    // Connection parameters
    let listen_port: ip::Port = ip::Port::try_from(80).unwrap();
    let listen_addr: Ipv4Endpoint = Ipv4Endpoint::new(test_helpers::BOB_IPV4, listen_port);

    // Setup peers.
    let mut server = test_helpers::new_bob2(now);
    let mut client = test_helpers::new_alice2(now);

    let (server_fd, client_fd): (IoQueueDescriptor, IoQueueDescriptor) = connection_setup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        listen_port,
        listen_addr,
    );
}