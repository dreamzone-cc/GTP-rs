# GTP/1.1 Control API Reference Manual

This document provides exhaustive documentation for all control and configuration interfaces of the **Game Transport Protocol (GTP/1.1)**.

---

## 1. Module Index

The Control API is located under `gtp_core::control` and re-exported from `gtp_core`:
- `gtp_core::control::config::GtpConfig`: Static configuration profiles.
- `gtp_core::control::config::GtpConfigBuilder`: Fluent builder for customized parameters.
- `gtp_core::control::handle::ConnectionControl`: Dynamic runtime management handle.
- `gtp_core::control::metrics::DetailedMetrics`: Snapshot of all protocol diagnostics.
- `gtp_core::control::events::ControlEvent`: Asynchronous notification events.

---

## 2. Configuration API (`GtpConfig` & `GtpConfigBuilder`)

### Structure Definition

```rust
pub struct GtpConfig {
    pub initial_mtu: usize,
    pub min_mtu: usize,
    pub max_mtu: usize,
    pub mtu_probe_interval: Duration,

    pub ack_frequency_packets: u8,
    pub max_ack_delay: Duration,
    pub ack_reorder_threshold: u8,
    pub immediate_ack_on_gap: bool,

    pub initial_cwnd_packets: u64,
    pub min_cwnd_packets: u64,
    pub smss: u64,
    pub cubic_beta: f64,
    pub cubic_c: f64,
    pub pacing_gain: f64,
    pub max_pacing_burst_bytes: u64,

    pub max_queue_bytes_per_tier: [usize; 5],
    pub tier_weights: [u32; 5],
    pub auto_state_supersession: bool,
    pub enable_deadline_pruning: bool,

    pub idle_timeout: Duration,
    pub keepalive_ping_interval: Duration,
    pub pto_max_duration: Duration,

    pub anti_amplification_factor: u64,
    pub stateless_token_lifetime: Duration,
    pub replay_window_size: usize,
    pub key_rotation_interval_packets: u64,
}
```

### Preset Constructors

#### `GtpConfig::competitive_fps() -> GtpConfig`
Optimized for ultra-low latency, sub-frame input responsiveness, and high tick rates (60–128 Hz).
- ACK frequency: every 1 packet (immediate feedback).
- Max ACK delay: 5 ms.
- Initial CWND: 20 packets.
- Aggressive state supersession enabled.

#### `GtpConfig::mmo_world() -> GtpConfig`
Optimized for massive concurrency, large world state replication, and high throughput.
- ACK frequency: every 2 packets.
- Max ACK delay: 25 ms.
- Large ordered stream queue capacities.

#### `GtpConfig::mobile_wireless() -> GtpConfig`
Optimized for cellular (4G/5G) and erratic Wi-Fi connections with high jitter and random loss.
- Reorder threshold: 3 packets.
- Resilient loss recovery and conservative backoff.

#### `GtpConfig::lan_cluster() -> GtpConfig`
Optimized for inter-server communication within data centers and LAN tournament play.
- MTU: 1450 bytes.
- Pacing burst limit: 100 KB.
- Initial CWND: 50 packets.

---

## 3. Runtime Control Interface (`ConnectionControl`)

Acquired via `conn.control()` on any `GtpConnection` instance.

### Method Reference

#### `set_ack_frequency(&mut self, ack_frequency_packets: u8, max_ack_delay_ms: u16, reorder_threshold: u8, now: MonotonicTime) -> Result<()>`
Instructs the remote peer to update its ACK emission rate using a wire `ACK_FREQUENCY` frame (Type `0x0A`).
- **Parameters**:
  - `ack_frequency_packets`: Number of unacknowledged packets to accumulate before triggering an immediate ACK.
  - `max_ack_delay_ms`: Maximum duration to delay sending an ACK if packet count is not reached.
  - `reorder_threshold`: Packet reordering gap required to force an out-of-order immediate ACK.
- **Returns**: `Ok(())` on success, or `TransportError::BufferOverflow` if P0 queue is full.

#### `trigger_path_challenge(&mut self, new_addr: SocketAddr, nonce: [u8; 8], now: MonotonicTime) -> Result<()>`
Initiates a 3-way path validation handshake on a newly detected or roaming network interface.
- **Parameters**:
  - `new_addr`: The candidate remote socket address.
  - `nonce`: Cryptographically random 8-byte challenge data.
- **Behavior**: Enqueues `PATH_CHALLENGE` frame. When `PATH_RESPONSE` is received with matching nonce, active path is migrated automatically.

#### `trigger_mtu_probe(&mut self, probe_id: u32, target_size: usize, now: MonotonicTime) -> Result<()>`
Sends an oversized `MTU_PROBE` frame padded to `target_size` bytes to test Path MTU expansion without risking data loss.

#### `send_ping(&mut self, nonce: u64, now: MonotonicTime) -> Result<()>`
Sends a low-overhead `PING` frame to verify path liveness and solicit an immediate ACK.

#### `graceful_close(&mut self, error_code: u16, reason: &'static str, now: MonotonicTime) -> Result<()>`
Enqueues a `CLOSE` frame (Type `0x09`) containing an application-defined error code and UTF-8 reason string, then transitions connection state to `Draining`.

#### `force_close(&mut self, error_code: u16) -> Result<()>`
Immediately transitions connection state to `Closed` without sending wire frames or waiting for outstanding packets.

#### `query_metrics(&self, now: MonotonicTime) -> DetailedMetrics`
Takes a lock-free snapshot of all internal telemetry counters, RTT statistics, queue depths, and congestion metrics.

#### `drain_events(&mut self) -> Vec<ControlEvent>`
Drains and clears all queued asynchronous events generated by the protocol engine.

---

## 4. Telemetry Metrics Reference (`DetailedMetrics`)

| Field | Type | Description |
| :--- | :--- | :--- |
| `latest_rtt` | `Duration` | Most recent single-trip RTT sample adjusted for ACK delay. |
| `smoothed_rtt` | `Duration` | Exponentially weighted moving average of RTT (RFC 9002). |
| `rttvar` | `Duration` | RTT variance estimate used for timeout calculations. |
| `min_rtt` | `Duration` | Minimum RTT observed across connection lifetime. |
| `pto_duration` | `Duration` | Calculated Probe Timeout (`smoothed_rtt + 4*rttvar + max_ack_delay`). |
| `cwnd_bytes` | `u64` | Current congestion window size in bytes. |
| `inflight_bytes` | `u64` | Unacknowledged data bytes currently in flight. |
| `pacing_rate_bps` | `u64` | Current transmission pacing rate in bytes per second. |
| `backpressure` | `BackpressureLevel` | Engine backpressure severity (`Low`, `Medium`, `High`, `Critical`). |
| `effective_queue_bytes` | `usize` | Total non-expired payload bytes queued across all priority tiers. |
| `total_tx_packets` | `u64` | Total packets transmitted. |
| `total_rx_packets` | `u64` | Total packets received. |
| `total_retransmissions`| `u64` | Number of individual reliable fragment retransmissions. |
| `pto_count` | `u32` | Number of consecutive Probe Timeouts triggered without progress. |

---

## 5. Asynchronous Tokio API (`AsyncGtpConnection`)

All synchronous Control API methods have async equivalents:

```rust
// Asynchronously set ACK frequency
conn.set_ack_frequency(1, 10, 1).await?;

// Query real-time metrics
let metrics = conn.query_metrics().await;
println!("Current RTT: {:?}", metrics.smoothed_rtt);

// Drain events
let events = conn.drain_events().await;
for event in events {
    match event {
        ControlEvent::BackpressureChanged { new_level, .. } => {
            println!("Network load changed to: {:?}", new_level);
        }
        ControlEvent::PathMigrated { new_addr, .. } => {
            println!("Client migrated to: {}", new_addr);
        }
        _ => {}
    }
}
```
