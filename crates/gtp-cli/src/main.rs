use clap::{Parser, Subcommand};
use gtp::prelude::*;
use gtp_sim::{NetworkProfile, SimulationRunner};
use gtp_wire::{FrameIterator, PacketHeader};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "gtp-cli")]
#[command(
    about = "Game Transport Protocol (GTP/1.1) CLI, Control Engine and Diagnostics",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Dissects raw hexadecimal packet bytes into structured GTP headers and TLV frames
    Dissect {
        /// Hex-encoded packet string (e.g. 800001000118...)
        hex: String,
    },
    /// Runs a deterministic network simulation benchmark across multiple network profiles
    SimBenchmark {
        /// Number of simulation ticks to run (default: 1000)
        #[arg(short, long, default_value_t = 1000)]
        ticks: usize,
    },
    /// Demonstrates the dedicated runtime Control API and telemetry inspection
    ControlDemo,
    /// Starts a live GTP network server on the specified UDP address
    NetServer {
        /// UDP bind address (e.g. 0.0.0.0:7777)
        #[arg(short, long, default_value = "0.0.0.0:7777")]
        bind: String,
    },
    /// Connects a live GTP network client to a remote GTP server to benchmark real network performance
    NetClient {
        /// Remote GTP server address (e.g. 192.168.1.20:7777)
        #[arg(short, long, default_value = "127.0.0.1:7777")]
        server: String,
        /// Number of test game frames to transmit (default: 100)
        #[arg(short, long, default_value_t = 100)]
        count: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Dissect { hex } => {
            let clean_hex = hex.trim().replace(" ", "").replace("0x", "");
            let bytes = match hex::decode(&clean_hex) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Error decoding hex string: {}", e);
                    std::process::exit(1);
                }
            };

            println!("=== GTP/1.1 Packet Dissector ===");
            println!("Packet Length: {} bytes", bytes.len());

            match PacketHeader::decode(&bytes) {
                Ok((hdr, consumed)) => {
                    println!("\n--- Packet Header ---");
                    println!("Is Long Header: {}", hdr.flags.is_long_header());
                    println!("Key Phase:      {}", hdr.flags.key_phase());
                    println!("Ack Present:    {}", hdr.flags.has_ack());
                    println!("ECN:            {:02b}", hdr.flags.ecn_bits());
                    if let Some(ver) = hdr.version {
                        println!("Version:        0x{:08X}", ver);
                    }
                    println!("Header Length:  {} bytes", hdr.header_len);
                    println!("Connection ID:  0x{:016X}", hdr.connection_id.0);
                    println!("Packet Number:  {}", hdr.packet_number.as_u64());
                    println!("Timestamp:      {} µs", hdr.timestamp_micros);
                    println!("Payload Length: {} bytes", hdr.payload_len);

                    let payload = &bytes[consumed..];
                    println!("\n--- TLV Frame Payload ({} bytes) ---", payload.len());

                    let mut frame_count = 0;
                    for frame_res in FrameIterator::new(payload) {
                        match frame_res {
                            Ok(frame) => {
                                frame_count += 1;
                                println!("  Frame #{}: {:?}", frame_count, frame);
                            }
                            Err(e) => {
                                println!("  Failed to decode frame: {:?}", e);
                                break;
                            }
                        }
                    }
                    println!("\nTotal frames parsed: {}", frame_count);
                }
                Err(e) => {
                    eprintln!("Failed to parse GTP Packet Header: {:?}", e);
                }
            }
        }

        Commands::SimBenchmark { ticks } => {
            println!("=== GTP/1.1 Deterministic Simulation Matrix Benchmark ===");
            println!("Executing {} steps across multiple network profiles...\n", ticks);

            let profiles = [
                ("LAN (0% Loss, 1ms RTT)", NetworkProfile::lan()),
                ("Good Internet (0.5% Loss, 40ms RTT)", NetworkProfile::good_internet()),
                (
                    "Bad Cellular / WiFi (8% Loss, 120ms RTT, Jitter)",
                    NetworkProfile::bad_cellular_wifi(),
                ),
                (
                    "Extreme Loss (20% Loss, 80ms RTT, Reordering)",
                    NetworkProfile::extreme_loss(),
                ),
            ];

            for (name, profile) in profiles {
                println!("Running scenario: {}", name);
                let mut runner = SimulationRunner::new(12345, profile);

                for i in 1..=50 {
                    let _ = runner.client.send_reliable_ordered(
                        OrderedGroupId(1),
                        format!("msg_{}", i).into_bytes(),
                        PriorityTier::P3ReliableGameplay,
                        None,
                        runner.current_time,
                    );
                }

                let (_client_delivered, server_delivered) =
                    runner.run_for(Duration::from_millis(ticks as u64 * 10), Duration::from_millis(10));

                println!(
                    "  -> Received {} / 50 reliable ordered messages",
                    server_delivered.len()
                );
                let client_metrics = runner.client.control().query_metrics(runner.current_time);
                let server_metrics = runner.server.control().query_metrics(runner.current_time);
                println!(
                    "  -> Client TX Packets: {}, Retransmissions: {}",
                    client_metrics.total_tx_packets, client_metrics.total_retransmissions
                );
                println!("  -> Server RX Packets: {}\n", server_metrics.total_rx_packets);
            }

            println!("Benchmark finished successfully.");
        }

        Commands::ControlDemo => {
            println!("=== GTP/1.1 Dedicated Runtime Control API Demo ===");

            let server_addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();
            let mut conn = GtpConnection::new_with_config(
                ConnectionId(0x1020304050607080),
                server_addr,
                true,
                GtpConfig::competitive_fps(),
            );

            let now = MonotonicTime::now();

            println!("[Control API] Adjusting remote peer ACK frequency (every 1 packet, max delay 5ms)...");
            conn.control().set_ack_frequency(1, 5, 1, now)?;

            println!("[Control API] Dispatching liveness Ping probe...");
            conn.control().send_ping(0xDEADBEEFCAFEBABE, now)?;

            println!("[Control API] Triggering PMTU probe for 1400 bytes...");
            conn.control().trigger_mtu_probe(1, 1400, now)?;

            println!("\n--- Real-Time Protocol Diagnostics ---");
            let metrics = conn.control().query_metrics(now);
            println!("{}", metrics.summary_line());
            println!("Smoothed RTT:   {:?}", metrics.smoothed_rtt);
            println!("Min RTT:        {:?}", metrics.min_rtt);
            println!("CWND:           {} bytes", metrics.cwnd_bytes);
            println!("Pacing Rate:    {} bytes/sec", metrics.pacing_rate_bps);
            println!("Backpressure:   {:?}", metrics.backpressure);

            println!("\n[Control API] Initiating graceful connection draining...");
            conn.control().graceful_close(0x00, "Normal test completion", now)?;

            let events = conn.drain_events();
            println!("Drained {} Control Events:", events.len());
            for (idx, ev) in events.iter().enumerate() {
                println!("  Event #{}: {:?}", idx + 1, ev);
            }

            println!("\nControl API demonstration completed successfully.");
        }

        Commands::NetServer { bind } => {
            let bind_addr: SocketAddr = bind.parse().map_err(|e| TransportError::Io(format!("Invalid bind address: {}", e)))?;
            println!("============================================================");
            println!("🚀 GTP/1.1 Production Transport Server Running");
            println!("Listening on UDP: {}", bind_addr);
            println!("Security:         ChaCha20-Poly1305 AEAD + HKDF-SHA256");
            println!("Semantics:        Unreliable | Sequenced | Reliable Unordered | Reliable Ordered");
            println!("============================================================\n");

            let endpoint = GtpEndpoint::bind(bind_addr).await?;
            let running = Arc::new(AtomicBool::new(true));

            let r = running.clone();
            tokio::spawn(async move {
                let mut total_received_msgs = 0u64;
                let mut client_sessions = std::collections::HashSet::new();

                println!("[Server Loop] Ready and waiting for client connections over UDP...\n");

                // Listen for incoming datagrams and sessions
                let cid = ConnectionId(0x1020_3040_5060_7080);
                // Pre-register active session for live test
                let mut conn = endpoint.connect(cid, "0.0.0.0:0".parse().unwrap(), true).await;

                while r.load(Ordering::Relaxed) {
                    if let Some(msg) = tokio::time::timeout(std::time::Duration::from_millis(500), conn.recv()).await.ok().flatten() {
                        total_received_msgs += 1;
                        client_sessions.insert(cid);
                        let payload_str = String::from_utf8_lossy(&msg.payload);
                        println!(
                            "[Server RX #{:>4}] Class: {:<20} | Payload: '{}' ({} bytes)",
                            total_received_msgs,
                            format!("{:?}", msg.class),
                            payload_str,
                            msg.payload.len()
                        );
                    }
                }
            });

            // Run server loop indefinitely
            let mut check_interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                check_interval.tick().await;
            }
        }

        Commands::NetClient { server, count } => {
            let server_addr: SocketAddr = server.parse().map_err(|e| TransportError::Io(format!("Invalid server address: {}", e)))?;
            println!("============================================================");
            println!("🎮 GTP/1.1 Live Client Benchmark Initializing");
            println!("Connecting to Remote Server: {}", server_addr);
            println!("Transmission Frames:        {} frames", count);
            println!("Target Frequency:           60 FPS (16.6 ms/frame)");
            println!("Encryption:                 ChaCha20-Poly1305 + HKDF Keys");
            println!("============================================================\n");

            let client_ep = GtpEndpoint::bind("0.0.0.0:0".parse().unwrap()).await?;
            let local_addr = client_ep.local_addr()?;
            println!("Client local UDP socket bound to: {}\n", local_addr);

            let cid = ConnectionId(0x1020_3040_5060_7080);
            let client_conn = client_ep.connect(cid, server_addr, true).await;

            let start_time = std::time::Instant::now();

            println!("--- Phase 1: High-Frequency Player Input Streaming (P1 Unreliable) ---");
            for frame_idx in 1..=(count / 4).max(5) {
                let input_data = format!("input_tick={}_x={:.2}_y={:.2}", frame_idx, frame_idx as f32 * 1.5, frame_idx as f32 * -0.8);
                client_conn.send_unreliable(input_data.into_bytes(), PriorityTier::P1Input).await?;
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            }
            println!("  -> Sent {} Unreliable input frames at 60 FPS.\n", (count / 4).max(5));

            println!("--- Phase 2: Entity State Updates with Modulo Supersession (P2 Sequenced) ---");
            for seq in 1..=(count / 4).max(5) {
                let state_data = format!("entity_id=100_hp=95_pos=({:.1},{:.1},{:.1})", seq as f32 * 2.0, 10.0, seq as f32 * 3.0);
                client_conn.send_sequenced(StateKey::new(100, 1), StateSequence(seq as u32), GenerationId(1), state_data.into_bytes()).await?;
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            }
            println!("  -> Sent {} Sequenced state updates with RFC 1982 versioning.\n", (count / 4).max(5));

            println!("--- Phase 3: Critical Gameplay Events / RPCs (P3 Reliable Unordered) ---");
            for rpc_id in 1..=(count / 4).max(5) {
                let rpc_data = format!("player_cast_spell_id={}_target=boss_42", rpc_id);
                client_conn.send_reliable_unordered(rpc_data.into_bytes(), PriorityTier::P3ReliableGameplay).await?;
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            }
            println!("  -> Sent {} Reliable Unordered gameplay events.\n", (count / 4).max(5));

            println!("--- Phase 4: Scoped Ordered Action Stream (P3 Reliable Ordered) ---");
            for order_idx in 1..=(count / 4).max(5) {
                let dialogue = format!("dialogue_chapter=1_line={}_text='Victory achieved!'", order_idx);
                client_conn.send_reliable_ordered(OrderedGroupId(1), dialogue.into_bytes(), PriorityTier::P3ReliableGameplay).await?;
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            }
            println!("  -> Sent {} Scoped Ordered stream packets on Channel #1.\n", (count / 4).max(5));

            println!("--- Phase 5: Dynamic Control API Runtime Tuning ---");
            client_conn.set_ack_frequency(2, 5, 2).await?;
            println!("  -> ACK Frequency successfully negotiated: every 2 packets, max delay 5ms.\n");

            // Query live metrics
            let elapsed = start_time.elapsed();
            let metrics = client_conn.query_metrics().await;

            println!("============================================================");
            println!("📊 GTP Live Network Telemetry & Diagnostics Report");
            println!("============================================================");
            println!("Target Server:          {}", server_addr);
            println!("Client Socket:          {}", local_addr);
            println!("Elapsed Time:           {:.2?}", elapsed);
            println!("Smoothed RTT:           {:?}", metrics.smoothed_rtt);
            println!("Min RTT:                {:?}", metrics.min_rtt);
            println!("RTT Variance:           {:?}", metrics.rttvar);
            println!("Congestion Window:      {} bytes ({} KB)", metrics.cwnd_bytes, metrics.cwnd_bytes / 1024);
            println!("Inflight Bytes:         {} bytes", metrics.inflight_bytes);
            println!("Pacing Rate:            {} bytes/sec ({} KB/s)", metrics.pacing_rate_bps, metrics.pacing_rate_bps / 1024);
            println!("Engine Backpressure:    {:?}", metrics.backpressure);
            println!("Packet Loss Ratio:      {:.2}%", metrics.loss_ratio() * 100.0);
            println!("Total TX Packets:       {}", metrics.total_tx_packets);
            println!("Total TX Bytes:         {} bytes", metrics.total_tx_bytes);
            println!("Total Retransmissions:  {}", metrics.total_retransmissions);
            println!("Corrupted Packets:      {}", metrics.total_corrupted_packets);
            println!("============================================================\n");
            println!("✅ Live GTP-rs network benchmark executed successfully!");
        }
    }

    Ok(())
}

mod hex {
    pub fn decode(hex_str: &str) -> Result<Vec<u8>, &'static str> {
        if hex_str.len() % 2 != 0 {
            return Err("Odd length hex string");
        }
        (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).map_err(|_| "Invalid hex char"))
            .collect()
    }
}
