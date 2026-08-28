# GTP-RUST-01: Rust Architecture, Memory & Ownership Design

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. Hot/Cold State Separation
```rust
pub struct ConnectionHot {
    pub connection_id: ConnectionId,
    pub next_packet_number: u64,
    pub largest_acked: u64,
    pub rtt_stats: RttStats,
    pub cwnd: u64,
    pub inflight: u64,
    pub next_send_time: MonotonicTime,
    pub active_path: SocketAddr,
}

pub struct ConnectionCold {
    pub total_rx_packets: u64,
    pub total_tx_packets: u64,
    pub total_stale_drops: u64,
    pub total_deadline_misses: u64,
    pub migration_history: Vec<PathRecord>,
}
```

## 2. Zero-Allocation Strategy on Hot Path
- Pre-allocated slab/ring buffers for incoming datagrams and outgoing packet construction.
- Reusable `PacketPool` and `MessagePool` per thread/worker to eliminate steady-state heap allocations.
- Safe Rust by default; `unsafe` is strictly isolated to OS socket syscall wrappers with documented invariants.
