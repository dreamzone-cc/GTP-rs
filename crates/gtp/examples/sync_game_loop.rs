//! Example: Synchronous 60 FPS Game Loop using the GTP Rust Library SDK

use gtp::prelude::*;
use gtp_core::state::OFFLINE_SIM_MASTER_SECRET;
use std::net::SocketAddr;

fn main() -> Result<()> {
    println!("=== GTP/1.1 Rust Library Example: Synchronous Game Loop ===");

    let server_addr: SocketAddr = "127.0.0.1:7777".parse().unwrap();
    let cid = ConnectionId(0xDEAD_BEEF_0000_0001);

    // 1. Initialize connection using the high-level library API
    let mut conn = GtpConnection::new_with_role(
        cid,
        server_addr,
        true, // AEAD Protected
        true, // client role
        OFFLINE_SIM_MASTER_SECRET,
        GtpConfig::competitive_fps(),
    );

    let mut current_time = MonotonicTime::now();

    // 2. Simulate 5 game ticks at 60 FPS (16ms per frame)
    for tick in 1..=5 {
        current_time += Duration::from_millis(16);

        // Send Player Input (P1 Tier)
        let input_payload = format!("player_move_tick_{}", tick).into_bytes();
        conn.send_unreliable(input_payload, PriorityTier::P1Input, None, current_time)?;

        // Send Entity State (P2 Tier - Supersedable)
        let state_payload = format!("entity_pos_tick_{}", tick).into_bytes();
        conn.send_sequenced(
            StateKey::new(100, 1),
            StateSequence(tick),
            GenerationId(1),
            None,
            state_payload,
            current_time,
        )?;

        // Produce outgoing datagrams for network transmission
        let mut out_buffer = [0u8; 1500];
        while let Some((dest, len)) =
            conn.produce_outgoing_datagram(current_time, &mut out_buffer)?
        {
            println!(
                "Tick #{}: Transmitted {} bytes datagram to {}",
                tick, len, dest
            );
        }
    }

    // 3. Inspect telemetry metrics using the Control API
    let metrics = conn.control().query_metrics(current_time);
    println!("\n--- GTP Diagnostics ---");
    println!("Total Packets Transmitted: {}", metrics.total_tx_packets);
    println!(
        "Total Bytes Transmitted:   {} bytes",
        metrics.total_tx_bytes
    );
    println!(
        "Pacing Rate:               {} bytes/sec",
        metrics.pacing_rate_bps
    );
    println!("Engine Backpressure:       {:?}", metrics.backpressure);

    println!("\nGTP library game loop example executed successfully.");
    Ok(())
}
