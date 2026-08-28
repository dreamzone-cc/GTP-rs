# Game Transport Protocol (GTP/1.1): API Architecture & Continuous Evolution Guidelines

## 1. Executive Summary & Vision

The **Game Transport Protocol (GTP/1.1)** is a high-performance, connection-oriented UDP transport layer designed specifically for modern multiplayer game engines, simulations, and real-time interactive systems.

This document establishes the formal **API Architecture**, **Public Interface Reference**, and **Continuous Evolution Guidelines** to govern the protocol's development as new features, sub-protocols, and extensions are introduced.

---

## 2. API Design Principles & Architecture

```
+------------------------------------------------------------------------------------+
|                                Game Engine Application                             |
|       (Player Input, Entity Replication, Critical RPCs, Matchmaking, Voice)        |
+-----------------------------------------+------------------------------------------+
                                          |
                        GTP Public Semantic & Control API
                                          |
          +-------------------------------+-------------------------------+
          |                                                               |
+---------v-------------------+                         +-----------------v----------+
|      Data Plane API         |                         |      Control Plane API     |
| - send_unreliable()         |                         | - set_ack_frequency()      |
| - send_sequenced()          |                         | - trigger_path_challenge() |
| - send_reliable_unordered() |                         | - trigger_mtu_probe()      |
| - send_reliable_ordered()   |                         | - query_metrics()          |
| - send_state()              |                         | - graceful_close()         |
+-----------------------------+                         +----------------------------+
          |                                                               |
+---------v---------------------------------------------------------------v----------+
|                                    GtpConnection                                   |
|  +-------------------------------------+  +-------------------------------------+  |
|  |           Hot State (Cache-line)    |  |          Cold State (Telemetry)     |  |
|  | - ReplayWindow (128-bit)            |  | - Total Rx/Tx Packet Counters       |  |
|  | - GameScheduler (5 DRR Tiers)       |  | - Spurious Loss Counters            |  |
|  | - LossDetector & RttStats (RFC 9002)|  | - Retransmission Accounting         |  |
|  | - CubicCongestionController         |  | - Event Queue (ControlEvent)        |  |
|  | - PacingEngine (Token-Bucket)       |  |                                     |  |
|  | - GtpAeadProtector (AES/ChaCha20)   |  |                                     |  |
|  +-------------------------------------+  +-------------------------------------+  |
+-----------------------------------------+------------------------------------------+
                                          |
+-----------------------------------------v------------------------------------------+
|                       I/O Boundary (PacketIo & Socket2)                            |
|             Zero-Copy Framing, Batch Send/Recv, Non-Blocking Sockets               |
+------------------------------------------------------------------------------------+
```

### Core Architecture Axioms:
1. **Strict Separation of Data Plane & Control Plane**:
   - The *Data Plane* operates with zero heap allocations during steady-state frame dispatch.
   - The *Control Plane* manages lifecycle, parameter adjustments, MTU probing, NAT migration, and deep diagnostics.
2. **Hot / Cold Cache-Line Partitioning**:
   - High-frequency per-packet variables (`ConnectionHot`) reside contiguously in memory to eliminate cache misses.
   - Infrequent administrative and telemetry structures (`ConnectionCold`) are updated outside inner loops.
3. **Four Semantic Guarantees**:
   - **Unreliable**: Fire-and-forget; lowest latency.
   - **Unreliable Sequenced**: Modulo-safe generation and sequence supersession via `StateTable`.
   - **Reliable Unordered**: Guaranteed delivery without head-of-line blocking across unrelated packets.
   - **Reliable Ordered**: Scoped stream channels (`OrderedGroupId`) preserving strict sequence without cross-stream stalls.
4. **Adaptive Game-Aware Flow Control**:
   - CUBIC congestion avoidance coupled with high-resolution token-bucket pacing.
   - 4-Tier backpressure feedback (`Low`, `Medium`, `High`, `Critical`) informing game simulation LOD.

---

## 3. Comprehensive Control API Reference

### 3.1 `GtpConfig` & Configuration Presets
Defined in `gtp_core::control::config`:

| Preset Method | Target Network & Game Genre | Default Characteristics |
| :--- | :--- | :--- |
| `GtpConfig::competitive_fps()` | 128-tick FPS, Battle Royale, Arena Shooters | ACK freq: 1 pkt, max ACK delay: 5ms, Initial CWND: 20 pkts, Aggressive state supersession. |
| `GtpConfig::mmo_world()` | MMORPG, Persistent Open-Worlds | ACK freq: 2 pkts, max ACK delay: 25ms, Initial CWND: 10 pkts, High throughput ordered streams. |
| `GtpConfig::mobile_wireless()` | Mobile Games, 4G/5G/WiFi with Jitter | ACK freq: 1 pkt, max ACK delay: 15ms, Reorder threshold: 3 pkts, Resilient loss backoff. |
| `GtpConfig::lan_cluster()` | Dedicated Server Clusters & LAN | ACK freq: 8 pkts, max ACK delay: 2ms, Initial CWND: 50 pkts, MTU: 1450, Max pacing burst: 100 KB. |

### 3.2 `ConnectionControl` Runtime Interface
Defined in `gtp_core::control::handle`:

```rust
impl<'a> ConnectionControl<'a> {
    /// Dynamically adjust the remote peer's ACK frequency
    pub fn set_ack_frequency(&mut self, ack_frequency_packets: u8, max_ack_delay_ms: u16, reorder_threshold: u8, now: MonotonicTime) -> Result<()>;

    /// Initiate 3-way path validation challenge for NAT migration
    pub fn trigger_path_challenge(&mut self, new_addr: SocketAddr, nonce: [u8; 8], now: MonotonicTime) -> Result<()>;

    /// Dispatch Path MTU Discovery probe
    pub fn trigger_mtu_probe(&mut self, probe_id: u32, target_size: usize, now: MonotonicTime) -> Result<()>;

    /// Send liveness ping frame
    pub fn send_ping(&mut self, nonce: u64, now: MonotonicTime) -> Result<()>;

    /// Graceful closing handshake
    pub fn graceful_close(&mut self, error_code: u16, reason: &'static str, now: MonotonicTime) -> Result<()>;

    /// Force termination
    pub fn force_close(&mut self, error_code: u16) -> Result<()>;

    /// Snapshot real-time protocol telemetry
    pub fn query_metrics(&self, now: MonotonicTime) -> DetailedMetrics;

    /// Drain queued telemetry events for game engine hooks
    pub fn drain_events(&mut self) -> Vec<ControlEvent>;
}
```

### 3.3 `ControlEvent` Event System
Defined in `gtp_core::control::events`:

```rust
pub enum ControlEvent {
    StateChanged { old_state: ConnectionState, new_state: ConnectionState },
    BackpressureChanged { old_level: BackpressureLevel, new_level: BackpressureLevel, effective_queue_bytes: usize },
    PathMigrated { old_addr: SocketAddr, new_addr: SocketAddr },
    PacketLossDetected { lost_count: usize, lost_bytes: usize },
    RetransmissionTriggered { message_id: MessageId, fragment_id: FragmentId },
    PtoTriggered { pto_count: u32, inflight_bytes: u64 },
    MtuUpdated { new_mtu: usize },
    KeyPhaseRotated { new_phase: bool },
    EcnExperienced { ce_count: u32 },
    CustomExtensionEvent { extension_id: u16, payload: Vec<u8> },
}
```

---

## 4. Guidelines for Continuous Evolution & Future Extensions

When extending the GTP protocol with new features (e.g. forward error correction / FEC, multipath, voice transport channels, compression frames), developers **MUST** adhere to the following architectural rules:

### Rule 1: Backward & Forward Compatibility via TLV Framing
- New control and data features MUST be assigned unique Frame Type IDs (`0x0F` to `0xFF`).
- Frame decoders must safely ignore unrecognized extension frame types without failing the entire datagram.

### Rule 2: Non-Breaking API Additions via Fluent Builders
- Never break existing `GtpConfig` constructors. New configuration options must:
  1. Have sensible default values in existing preset constructors.
  2. Provide fluent builder methods on `GtpConfigBuilder`.

### Rule 3: Extending the Event System (`ControlEvent`)
- When introducing new runtime signals (e.g., FEC packet recovery, bandwidth probe completion), add new enum variants to `ControlEvent` without mutating existing variants.
- The `CustomExtensionEvent { extension_id, payload }` variant provides an immediate bridge for experimental prototype events before formal standardization.

### Rule 4: Zero Allocation Constraint in Steady-State Data Paths
- New message types or framing codecs MUST NOT perform heap allocations (`Box`, `Vec`, `String`) in the per-packet RX/TX hot loops.
- Use static arrays, slice references (`&[u8]`), and in-place buffer manipulations.

---

## 5. Game Engine Integration Blueprints

### Blueprint 1: 60 FPS Synchronous Game Loop (Rust / Bevy / Custom Engine)

```rust
use gtp_core::{GtpConnection, GtpConfig, PriorityTier, OrderedGroupId};
use gtp_types::{MonotonicTime, Duration, StateKey, StateSequence, GenerationId};
use std::net::SocketAddr;

struct GameNetworkClient {
    connection: GtpConnection,
    socket: std::net::UdpSocket,
}

impl GameNetworkClient {
    pub fn update(&mut self, now: MonotonicTime) {
        // 1. Receive incoming packets
        let mut in_buf = [0u8; 2048];
        while let Ok((bytes, src)) = self.socket.recv_from(&mut in_buf) {
            if let Ok(messages) = self.connection.handle_incoming_datagram(src, &mut in_buf[..bytes], now) {
                for msg in messages {
                    self.dispatch_game_message(msg);
                }
            }
        }

        // 2. Sample Backpressure & Adapt Dynamic LOD
        let feedback = self.connection.feedback(now);
        if feedback.backpressure >= gtp_cc::BackpressureLevel::High {
            // Drop cosmetic particles and throttle secondary entity snapshot rate
        }

        // 3. Send Player Input (P1 Tier)
        let input_bytes = b"player_move_forward".to_vec();
        let _ = self.connection.send_unreliable(input_bytes, PriorityTier::P1Input, None, now);

        // 4. Send Player State (P2 Tier - Supersedable)
        let _ = self.connection.send_sequenced(
            StateKey::new(1, 0),
            StateSequence(42),
            GenerationId(1),
            None,
            b"position_x_y_z".to_vec(),
            now,
        );

        // 5. Produce and transmit outgoing packets
        let mut out_buf = [0u8; 1500];
        while let Ok(Some((dest, len))) = self.connection.produce_outgoing_datagram(now, &mut out_buf) {
            let _ = self.socket.send_to(&out_buf[..len], dest);
        }

        // 6. Handle Control Events
        for event in self.connection.drain_events() {
            println!("Engine Network Event: {:?}", event);
        }
    }
}
```

### Blueprint 2: High-Density Async Server (Tokio / 10,000+ Concurrent Players)

```rust
use gtp_runtime_tokio::GtpEndpoint;
use gtp_types::{ConnectionId, PriorityTier};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = GtpEndpoint::bind("0.0.0.0:7777".parse()?).await?;
    println!("GTP Server listening on port 7777");

    // Accept and manage connections via AsyncGtpConnection handles
    let client_addr: SocketAddr = "192.168.1.50:5000".parse()?;
    let mut conn = endpoint.connect(ConnectionId(0x1122334455667788), client_addr, true).await;

    tokio::spawn(async move {
        while let Some(msg) = conn.recv().await {
            // Process incoming gameplay message concurrently
            let _ = conn.send_unreliable(b"server_ack".to_vec(), PriorityTier::P1Input).await;
        }
    });

    Ok(())
}
```
