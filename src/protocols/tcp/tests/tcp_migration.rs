use std::convert::TryFrom;
use std::task::Context;
use std::time::Instant;
use futures::task::noop_waker_ref;
use runtime::fail::Fail;
use runtime::memory::Bytes;
use runtime::network::NetworkRuntime;
use runtime::queue::IoQueueDescriptor;
use crate::protocols::ip;
use crate::protocols::ipv4::Ipv4Endpoint;
use crate::protocols::tcp::SeqNumber;
use crate::protocols::tcp::tests::setup::{advance_clock, connection_setup};
use crate::protocols::tcp::tests::established::{connection_hangup, cook_buffer, recv_data, send_data, send_recv};
use crate::protocols::tcp::tests::extract_headers;
use crate::protocols::tcp::tests::established::send_recv_round;
use crate::test_helpers;

/// Tests a established connection being migrated out of the server. If the client continues
/// to send a message the server should send a RST, since the connection no longer exists.
#[test]
fn migrated_out_rst() {
    let mut ctx = Context::from_waker(noop_waker_ref());
    let mut now = Instant::now();

    // Connection parameters
    let listen_port: ip::Port = ip::Port::try_from(80).unwrap();
    let listen_addr: Ipv4Endpoint = Ipv4Endpoint::new(test_helpers::BOB_IPV4, listen_port);

    // Setup peers.
    let mut server = test_helpers::new_bob2(now);
    let mut client = test_helpers::new_alice2(now);

    let window_scale: u8 = client.rt().tcp_options().get_window_scale();
    let max_window_size: u32 = (client.rt().tcp_options().get_receive_window_size() as u32)
        .checked_shl(window_scale as u32)
        .unwrap();

    let (server_fd, client_fd): (IoQueueDescriptor, IoQueueDescriptor) = connection_setup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        listen_port,
        listen_addr,
    );

    let _state = server.tcp_migrate_out_connection(server_fd).unwrap();

    let bufsize: u32 = 64;
    let buf: Bytes = cook_buffer(bufsize as usize, None);
    let seq_no: SeqNumber = SeqNumber::from(1);

    // Client sends data but connection has been migrated out.
    let (bytes, _): (Bytes, usize) = send_data(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        client_fd,
        max_window_size as u16,
        seq_no,
        None,
        buf.clone(),
    );

    // Server receives this data.
    let e = recv_data(&mut ctx,
              &mut server,
              &mut client,
              server_fd,
             bytes);

    // Server should complain this connection no longer exists.
    assert_eq!(e.unwrap_err(), Fail::ConnectionMigratedOut);

    // Check the outgoing queue of server for RST segment.
    let bytes = server.rt().pop_frame();
    let (_, _, tcp_header, _) = extract_headers(bytes);
    assert_eq!(tcp_header.rst, true);
}

/// We set up a connection. Migrate out the state, and migrate that state back in.
#[test]
fn migration_in_out_compare_state() {
    let mut ctx = Context::from_waker(noop_waker_ref());
    let mut now = Instant::now();

    // Connection parameters
    let listen_port: ip::Port = ip::Port::try_from(80).unwrap();
    let listen_addr: Ipv4Endpoint = Ipv4Endpoint::new(test_helpers::BOB_IPV4, listen_port);

    // Setup peers.
    let mut server = test_helpers::new_bob2(now);
    let mut client = test_helpers::new_alice2(now);

    let (server_fd, _client_fd): (IoQueueDescriptor, IoQueueDescriptor) = connection_setup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        listen_port,
        listen_addr,
    );

    // Migrate connection out and back in, comparing the states both times!
    dbg!(server_fd);
    let state = server.tcp_migrate_out_connection(server_fd).unwrap();
    let new_fd = server.tcp_migrate_in_connection(state.clone()).unwrap();
    let state_second_time = server.tcp_migrate_out_connection(new_fd).unwrap();

    assert_eq!(state, state_second_time);
}

#[test]
fn migration_in_out_send_data_compare_state() {
    let mut ctx = Context::from_waker(noop_waker_ref());
    let mut now = Instant::now();

    // Connection parameters
    let listen_port: ip::Port = ip::Port::try_from(80).unwrap();
    let listen_addr: Ipv4Endpoint = Ipv4Endpoint::new(test_helpers::BOB_IPV4, listen_port);

    // Setup peers.
    let mut server = test_helpers::new_bob2(now);
    let mut client = test_helpers::new_alice2(now);

    let window_scale: u8 = client.rt().tcp_options().get_window_scale();
    let max_window_size: u32 = (client.rt().tcp_options().get_receive_window_size() as u32)
        .checked_shl(window_scale as u32)
        .unwrap();

    let (server_fd, client_fd): (IoQueueDescriptor, IoQueueDescriptor) = connection_setup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        listen_port,
        listen_addr,
    );

    // Grab a copy of the server_s TCP state.
    let state = server.tcp_get_state(server_fd).unwrap();

    // Server sends message.
    let bufsize: u32 = 64;
    let buf: Bytes = cook_buffer(bufsize as usize, None);
    let seq_no: SeqNumber = SeqNumber::from(1);
    let (bytes, _): (Bytes, usize) = send_data(
        &mut ctx,
        &mut now,
        &mut client,
        &mut server,
        server_fd,
        max_window_size as u16,
        seq_no,
        None,
        buf.clone(),
    );

    // Get a new copy of the state. It should not be equal anymore.
    let state_second_time = server.tcp_get_state(server_fd).unwrap();
    // Unacked queue should be filled.
    assert_ne!(state.unacked_queue, state_second_time.unacked_queue);
}

#[test]
pub fn test_send_recv_round_loop() {
    let mut ctx = Context::from_waker(noop_waker_ref());
    let mut now = Instant::now();

    // Connection parameters
    let listen_port: ip::Port = ip::Port::try_from(80).unwrap();
    let listen_addr: Ipv4Endpoint = Ipv4Endpoint::new(test_helpers::BOB_IPV4, listen_port);

    // Setup peers.
    let mut server = test_helpers::new_bob2(now);
    let mut client = test_helpers::new_alice2(now);
    let window_scale: u8 = client.rt().tcp_options().get_window_scale();
    let max_window_size: u32 = (client.rt().tcp_options().get_receive_window_size() as u32)
        .checked_shl(window_scale as u32)
        .unwrap();

    let (server_fd, client_fd): (IoQueueDescriptor, IoQueueDescriptor) = connection_setup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        listen_port,
        listen_addr,
    );

    let state = server.tcp_get_state(server_fd).unwrap();

    let bufsize: u32 = 64;
    let buf: Bytes = cook_buffer(bufsize as usize, None);
    dbg!(max_window_size);
    for i in 0..((max_window_size + 1) / bufsize) {
        send_recv_round(
            &mut ctx,
            &mut now,
            &mut server,
            &mut client,
            server_fd,
            client_fd,
            max_window_size as u16,
            SeqNumber::from(1 + i * bufsize),
            buf.clone(),
        );
    }

    // Up to here, the server hasn't heard back from client. So server holds onto some unacked queue
    // messages.

    let state_second_time = server.tcp_get_state(server_fd).unwrap();
    assert_ne!(state.unacked_queue, state_second_time.unacked_queue);
    assert_ne!(state.rcv_nxt, state_second_time.rcv_nxt);
    assert_ne!(state.snd_una, state_second_time.snd_una);
    assert_ne!(state.base_seq_no, state_second_time.base_seq_no);
    assert_ne!(state.recv_seq_no, state_second_time.recv_seq_no);
}

#[test]
#[should_panic(expected = "bad peer state Established")]
pub fn test_send_recv_round_loop_currently_panicking() {
    let mut ctx = Context::from_waker(noop_waker_ref());
    let mut now = Instant::now();

    // Connection parameters
    let listen_port: ip::Port = ip::Port::try_from(80).unwrap();
    let listen_addr: Ipv4Endpoint = Ipv4Endpoint::new(test_helpers::BOB_IPV4, listen_port);

    // Setup peers.
    let mut server = test_helpers::new_bob2(now);
    let mut client = test_helpers::new_alice2(now);
    let window_scale: u8 = client.rt().tcp_options().get_window_scale();
    let max_window_size: u32 = (client.rt().tcp_options().get_receive_window_size() as u32)
        .checked_shl(window_scale as u32)
        .unwrap();

    let (server_fd, client_fd): (IoQueueDescriptor, IoQueueDescriptor) = connection_setup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        listen_port,
        listen_addr,
    );

    let bufsize: u32 = 64;
    let buf: Bytes = cook_buffer(bufsize as usize, None);
    dbg!(max_window_size);
    for i in 0..((max_window_size + 1) / bufsize) {
        send_recv_round(
            &mut ctx,
            &mut now,
            &mut server,
            &mut client,
            server_fd,
            client_fd,
            max_window_size as u16,
            SeqNumber::from(1 + i * bufsize),
            buf.clone(),
        );
    }

    connection_hangup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        server_fd,
        client_fd,
    );
}

#[test]
pub fn migrate_conection() {
    let mut ctx = Context::from_waker(noop_waker_ref());
    let mut now = Instant::now();

    // Connection parameters
    let listen_port: ip::Port = ip::Port::try_from(80).unwrap();
    let listen_addr: Ipv4Endpoint = Ipv4Endpoint::new(test_helpers::BOB_IPV4, listen_port);

    // Setup peers.
    let mut server = test_helpers::new_bob2(now);
    let mut client = test_helpers::new_alice2(now);
    let mut server2 = test_helpers::new_bob2(now);

    let window_scale: u8 = client.rt().tcp_options().get_window_scale();
    let max_window_size: u32 = (client.rt().tcp_options().get_receive_window_size() as u32)
        .checked_shl(window_scale as u32)
        .unwrap();

    let (server_fd, client_fd): (IoQueueDescriptor, IoQueueDescriptor) = connection_setup(
        &mut ctx,
        &mut now,
        &mut server,
        &mut client,
        listen_port,
        listen_addr,
    );

    let bufsize: u32 = 64;
    let buf: Bytes = cook_buffer(bufsize as usize, None);
    dbg!(max_window_size);

    for i in 0..((max_window_size + 1) / bufsize) {
        send_recv_round(
            &mut ctx,
            &mut now,
            &mut server,
            &mut client,
            server_fd,
            client_fd,
            max_window_size as u16,
            SeqNumber::from(1 + i * bufsize), buf.clone(),
        );
    }

    let state = server.tcp_migrate_out_connection(server_fd).unwrap();
    let server2_fd = server2.tcp_migrate_in_connection(state.clone()).unwrap();
    // Drop to ensure we don't accidentally use them again.
    drop(server);
    drop(server_fd);

    for i in 0..((max_window_size + 1) / bufsize) {
        send_recv_round(
            &mut ctx,
            &mut now,
            &mut server2,
            &mut client,
            server2_fd,
            client_fd,
            max_window_size as u16,
            // + 1 for ACKing the SYN, and + 1 because it is the _next_ seq_no we expect?
            SeqNumber::from((1 + max_window_size + 1) + i * bufsize),
            buf.clone(),
        );
    }

}