use gtp_runtime_tokio::GtpEndpoint;
use gtp_types::{ConnectionId, PriorityTier};

#[tokio::test]
async fn test_live_udp_x25519_handshake_and_gameplay_streaming() {
    let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("Server bind failed");
    let server_addr = server_ep.local_addr().unwrap();

    let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("Client bind failed");
    let client_addr = client_ep.local_addr().unwrap();

    let cid = ConnectionId(0xCAFE_BABE_9988_7766);

    // 1. Server establishes connection handler
    let mut server_conn = server_ep.connect(cid, client_addr, true).await;

    // 2. Client initiates connect (dispatches ClientHello and receives ServerHello)
    let client_conn = client_ep.connect(cid, server_addr, true).await;

    // 3. Client streams gameplay packet over AEAD authenticated transport
    let test_payload = b"championship_final_match_input_event_42";
    let msg_id = client_conn
        .send_unreliable(test_payload.to_vec(), PriorityTier::P1Input)
        .await
        .expect("Send failed");

    assert!(msg_id.as_u64() > 0);

    // 4. Server receives, decrypts, and verifies message
    let received = tokio::time::timeout(std::time::Duration::from_millis(500), server_conn.recv())
        .await
        .expect("Receive timed out")
        .expect("Channel closed");

    assert_eq!(received.payload, test_payload);
}
