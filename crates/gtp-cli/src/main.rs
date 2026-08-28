use clap::{Parser, Subcommand};
use gtp_core::{GtpConfig, GtpConnection};
use gtp_sim::{NetworkProfile, SimulationRunner};
use gtp_types::{ConnectionId, Duration, MonotonicTime, OrderedGroupId, PriorityTier};
use gtp_wire::{FrameIterator, PacketHeader};
use std::net::SocketAddr;

#[derive(Parser, Debug)]
#[command(name = "gtp")]
#[command(about = "Game Transport Protocol (GTP/1.1) CLI, Control Engine and Diagnostics", long_about = None)]
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
}

fn main() {
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
                    println!("Connection ID:  {:?}", hdr.connection_id);
                    println!("Packet Number:  {:?}", hdr.packet_number);
                    println!("Timestamp:      {} us", hdr.timestamp_micros);
                    println!("Payload Length: {} bytes", hdr.payload_len);

                    println!("\n--- Payload Frames ---");
                    let payload = &bytes[consumed..];
                    let mut count = 0;
                    for frame_res in FrameIterator::new(payload) {
                        match frame_res {
                            Ok(frame) => {
                                count += 1;
                                println!("Frame #{}: {:?}", count, frame);
                            }
                            Err(e) => {
                                println!("Frame parse error / AEAD encrypted: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to parse GTP header: {}", e);
                }
            }
        }

        Commands::SimBenchmark { ticks } => {
            println!("=== GTP/1.1 Deterministic Simulation Matrix Benchmark ===");
            println!("Executing {} steps across multiple network profiles...\n", ticks);

            let profiles = [
                ("LAN (0% Loss, 1ms RTT)", NetworkProfile::lan()),
                ("Good Internet (0.5% Loss, 40ms RTT)", NetworkProfile::good_internet()),
                ("Bad Cellular / WiFi (8% Loss, 120ms RTT, Jitter)", NetworkProfile::bad_cellular_wifi()),
                ("Extreme Loss (20% Loss, 80ms RTT, Reordering)", NetworkProfile::extreme_loss()),
            ];

            for (name, profile) in profiles {
                println!("Running scenario: {}", name);
                let mut runner = SimulationRunner::new(42, profile);

                // Enqueue 50 ordered reliable game events
                for i in 0..50 {
                    let payload = format!("benchmark_event_{}", i).into_bytes();
                    let _ = runner.client.send_reliable_ordered(
                        OrderedGroupId(1),
                        payload,
                        PriorityTier::P3ReliableGameplay,
                        None,
                        runner.current_time,
                    );
                }

                let (_c, s) = runner.run_for(Duration::from_millis(ticks as u64), Duration::from_millis(1));
                println!("  -> Received {} / 50 reliable ordered messages", s.len());
                println!("  -> Client TX Packets: {}, Retransmissions: {}",
                    runner.client.cold.total_tx_packets,
                    runner.client.cold.total_retransmissions
                );
                println!("  -> Server RX Packets: {}\n", runner.server.cold.total_rx_packets);
            }

            println!("Benchmark finished successfully.");
        }

        Commands::ControlDemo => {
            println!("=== GTP/1.1 Dedicated Runtime Control API Demo ===");
            let server_addr: SocketAddr = "127.0.0.1:7777".parse().unwrap();
            let mut conn = GtpConnection::new_with_config(
                ConnectionId(0xCAFE_BABE_0000_0001),
                server_addr,
                true,
                GtpConfig::competitive_fps(),
            );

            let now = MonotonicTime::from_micros(1_000_000);

            // 1. Send initial messages
            let _ = conn.send_unreliable(b"player_input_vector".to_vec(), PriorityTier::P1Input, None, now);
            let _ = conn.send_reliable_ordered(OrderedGroupId(1), b"inventory_equip_weapon".to_vec(), PriorityTier::P3ReliableGameplay, None, now);

            // 2. Dynamically adjust ACK frequency at runtime
            println!("[Control API] Adjusting remote peer ACK frequency (every 1 packet, max delay 5ms)...");
            conn.control().set_ack_frequency(1, 5, 1, now).unwrap();

            // 3. Send Ping keepalive
            println!("[Control API] Dispatching liveness Ping probe...");
            conn.control().send_ping(0x12345678, now).unwrap();

            // 4. Trigger MTU probing
            println!("[Control API] Triggering PMTU probe for 1400 bytes...");
            conn.control().trigger_mtu_probe(1, 1400, now).unwrap();

            // 5. Query detailed telemetry metrics
            let metrics = conn.control().query_metrics(now);
            println!("\n--- Real-Time Protocol Diagnostics ---");
            println!("{}", metrics.summary_line());
            println!("Smoothed RTT:   {:?}", metrics.smoothed_rtt);
            println!("Min RTT:        {:?}", metrics.min_rtt);
            println!("CWND:           {} bytes", metrics.cwnd_bytes);
            println!("Pacing Rate:    {} bytes/sec", metrics.pacing_rate_bps);
            println!("Backpressure:   {:?}", metrics.backpressure);

            // 6. Gracefully close connection and inspect emitted events
            println!("\n[Control API] Initiating graceful connection draining...");
            conn.control().graceful_close(0x0000, "Normal game exit", now).unwrap();

            let events = conn.drain_events();
            println!("Drained {} Control Events:", events.len());
            for (idx, event) in events.iter().enumerate() {
                println!("  Event #{}: {:?}", idx + 1, event);
            }

            println!("\nControl API demonstration completed successfully.");
        }
    }
}

mod hex {
    pub fn decode(hex_str: &str) -> Result<Vec<u8>, &'static str> {
        if !hex_str.len().is_multiple_of(2) {
            return Err("Odd length hex string");
        }
        (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).map_err(|_| "Invalid hex char"))
            .collect()
    }
}
