use gtp::prelude::*;
use std::net::SocketAddr;

#[test]
fn test_gtp_library_end_to_end_integration() {
    let client_addr: SocketAddr = "127.0.0.1:9001".parse().unwrap();
    let server_addr: SocketAddr = "127.0.0.1:9002".parse().unwrap();
    let cid = ConnectionId(0x1234_5678_9ABC_DEF0);

    // 1. Instantiate client and server using the unified `gtp` library
    let mut client = GtpConnection::new_with_config(
        cid,
        server_addr,
        true, // AEAD encryption
        GtpConfig::competitive_fps(),
    );

    let mut server =
        GtpConnection::new_with_config(cid, client_addr, true, GtpConfig::competitive_fps());

    let now = MonotonicTime::from_micros(1_000_000);

    // 2. Client sends 4 distinct message semantics through the public library API
    let msg_unreliable = client
        .send_unreliable(
            b"client_input_vector".to_vec(),
            PriorityTier::P1Input,
            None,
            now,
        )
        .expect("Failed to send unreliable input");

    let msg_sequenced = client
        .send_sequenced(
            StateKey::new(100, 1),
            StateSequence(1),
            GenerationId(1),
            None,
            b"entity_position_update".to_vec(),
            now,
        )
        .expect("Failed to send sequenced state");

    let msg_reliable_unordered = client
        .send_reliable_unordered(
            b"player_hit_damage_event".to_vec(),
            PriorityTier::P3ReliableGameplay,
            None,
            now,
        )
        .expect("Failed to send reliable unordered event");

    let msg_reliable_ordered = client
        .send_reliable_ordered(
            OrderedGroupId(1),
            b"chat_dialogue_sequence_1".to_vec(),
            PriorityTier::P3ReliableGameplay,
            None,
            now,
        )
        .expect("Failed to send reliable ordered stream");

    assert!(msg_unreliable.as_u64() > 0);
    assert!(msg_sequenced.as_u64() > 0);
    assert!(msg_reliable_unordered.as_u64() > 0);
    assert!(msg_reliable_ordered.as_u64() > 0);

    // 3. Client serializes datagram into wire buffer
    let mut wire_buffer = [0u8; 1500];
    let (dest, datagram_len) = client
        .produce_outgoing_datagram(now, &mut wire_buffer)
        .expect("Produce outgoing failed")
        .expect("Expected datagram to be produced");

    assert_eq!(dest, server_addr);
    assert!(datagram_len > 24);

    // 4. Server receives, decrypts, and dispatches frames
    let mut recv_buffer = wire_buffer;
    let delivered = server
        .handle_incoming_datagram(
            client_addr,
            &mut recv_buffer[..datagram_len],
            now + Duration::from_millis(5),
        )
        .expect("Server RX failed");

    // Verify delivered messages
    assert_eq!(delivered.len(), 4);
    assert_eq!(delivered[0].payload, b"client_input_vector");
    assert_eq!(delivered[1].payload, b"entity_position_update");
    assert_eq!(delivered[2].payload, b"player_hit_damage_event");
    assert_eq!(delivered[3].payload, b"chat_dialogue_sequence_1");

    // 5. Test Library Control API
    let metrics = client.control().query_metrics(now);
    assert_eq!(metrics.total_tx_packets, 1);
    assert_eq!(server.control().query_metrics(now).total_rx_packets, 1);
}
