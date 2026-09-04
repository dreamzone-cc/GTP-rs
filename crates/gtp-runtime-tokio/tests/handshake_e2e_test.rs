use gtp_runtime_tokio::GtpEndpoint;
use gtp_types::{ConnectionId, PriorityTier};

#[tokio::test]
async fn test_live_udp_x25519_dynamic_server_accept_and_gameplay() {
    let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("Server bind failed");
    let server_addr = server_ep.local_addr().unwrap();

    let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("Client bind failed");

    let cid = ConnectionId(0xCAFE_BABE_9988_7766);

    // 1. Client connects to server (server has NOT pre-registered anything!)
    let client_task = tokio::spawn(async move {
        client_ep
            .connect(cid, server_addr, true)
            .await
            .expect("Client handshake failed")
    });

    // 2. Server dynamically accepts the new client
    let mut server_conn =
        tokio::time::timeout(std::time::Duration::from_secs(2), server_ep.accept())
            .await
            .expect("Server accept timed out")
            .expect("Accept channel closed");

    assert_eq!(server_conn.cid, cid);

    let client_conn = client_task.await.expect("Client task panicked");

    // 3. Client streams gameplay packet over AEAD authenticated transport
    let test_payload = b"championship_final_match_input_event_42";
    let msg_id = client_conn
        .send_unreliable(test_payload.to_vec(), PriorityTier::P1Input)
        .await
        .expect("Send failed");

    assert!(msg_id.as_u64() > 0);

    // 4. Server receives, decrypts, and verifies message
    let received = tokio::time::timeout(std::time::Duration::from_millis(1500), server_conn.recv())
        .await
        .expect("Receive timed out")
        .expect("Channel closed");

    assert_eq!(received.payload, test_payload);
}

#[tokio::test]
async fn test_live_udp_x25519_multi_client_dynamic_accept() {
    let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("Server bind failed");
    let server_addr = server_ep.local_addr().unwrap();

    let num_clients = 10;
    let mut client_handles = Vec::new();

    for i in 0..num_clients {
        let cid = ConnectionId(0xA000_0000_0000_0000 + i);
        let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .expect("Client bind failed");

        let handle = tokio::spawn(async move {
            let conn = client_ep
                .connect(cid, server_addr, true)
                .await
                .expect("Connect failed");
            let payload = format!("player_packet_cid_{:X}", cid.as_u64()).into_bytes();
            // Use a RELIABLE send: this test asserts that each client's packet is
            // delivered, which is a deterministic guarantee only for reliable
            // classes. An unreliable packet is best-effort and may legitimately be
            // dropped under the socket-buffer pressure of many concurrent clients,
            // which made this test flaky for reasons unrelated to what it verifies.
            conn.send_reliable_unordered(payload.clone(), PriorityTier::P3ReliableGameplay)
                .await
                .expect("Send failed");
            (cid, payload)
        });
        client_handles.push(handle);
    }

    let mut accepted_cids = std::collections::HashSet::new();
    for _ in 0..num_clients {
        let mut server_conn =
            tokio::time::timeout(std::time::Duration::from_secs(3), server_ep.accept())
                .await
                .expect("Accept timed out")
                .expect("Accept channel closed");

        let received =
            tokio::time::timeout(std::time::Duration::from_millis(1500), server_conn.recv())
                .await
                .expect("Recv timed out")
                .expect("Channel closed");

        let expected_payload =
            format!("player_packet_cid_{:X}", server_conn.cid.as_u64()).into_bytes();
        assert_eq!(received.payload, expected_payload);
        accepted_cids.insert(server_conn.cid);
    }

    assert_eq!(accepted_cids.len(), num_clients as usize);

    for handle in client_handles {
        let _ = handle.await.unwrap();
    }
}
