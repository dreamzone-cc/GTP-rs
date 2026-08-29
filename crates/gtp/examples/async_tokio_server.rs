//! Example: Asynchronous Game Server using the GTP Rust Library with Tokio

use gtp::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== GTP/1.1 Rust Library Example: Async Tokio Endpoint ===");

    let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap()).await?;
    let server_addr = server_ep.local_addr()?;
    println!("Server bound to: {}", server_addr);

    let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap()).await?;
    let client_addr = client_ep.local_addr()?;
    println!("Client bound to: {}", client_addr);

    let cid = ConnectionId(0xCAFE_BABE_1122_3344);

    // 1. Client connects via automated X25519 ephemeral handshake
    let client_task = tokio::spawn(async move { client_ep.connect(cid, server_addr, true).await });

    // 2. Server dynamically accepts incoming client
    let mut server_conn = server_ep.accept().await.expect("Failed to accept client");
    let client_conn = client_task.await.unwrap()?;

    // 3. Client sends gameplay message asynchronously
    println!("Client sending reliable gameplay event...");
    client_conn
        .send_reliable_ordered(
            OrderedGroupId(1),
            b"player_purchased_item_id_42".to_vec(),
            PriorityTier::P3ReliableGameplay,
        )
        .await?;

    // 3. Server receives message asynchronously
    if let Some(msg) =
        tokio::time::timeout(std::time::Duration::from_millis(500), server_conn.recv())
            .await
            .ok()
            .flatten()
    {
        println!(
            "Server received payload: {}",
            String::from_utf8_lossy(&msg.payload)
        );
        assert_eq!(msg.payload, b"player_purchased_item_id_42");
    }

    // 4. Query live metrics via async Control API
    let metrics = client_conn.query_metrics().await;
    println!("Client Metrics Smoothed RTT: {:?}", metrics.smoothed_rtt);

    println!("\nGTP async Tokio server example executed successfully.");
    Ok(())
}
