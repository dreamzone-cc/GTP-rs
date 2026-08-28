# GTP-rs: Game Transport Protocol (GTP/1.1) in Rust

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/compliance-100%25-brightgreen.svg)](docs/GTP_COMPREHENSIVE_COMPLIANCE_AND_FEATURE_AUDIT_REPORT.md)

**GTP-rs** is a production-grade, modular, high-performance implementation of the **Game Transport Protocol (GTP/1.1)** written in pure **Rust**. Designed specifically for multiplayer game engines, interactive simulations, and real-time networked systems, GTP-rs combines ultra-low latency fire-and-forget messaging, generation-aware state supersession, and scoped out-of-order reliable streams over UDP.

---

## Key Features

- **4 Core Delivery Semantics**:
  - `Unreliable`: Fire-and-forget for high-frequency gameplay data (inputs, aim vectors).
  - `UnreliableSequenced`: RFC 1982 modulo-safe state updates with automatic predecessor supersession.
  - `ReliableUnordered`: Guaranteed delivery without head-of-line blocking across unrelated packets.
  - `ReliableOrdered`: Scoped stream channels (`OrderedGroupId`) preserving strict sequence without cross-stream stalls.
- **5-Tier Priority Scheduling**:
  - `P0Control` (Strict bandwidth reservation, never starved), `P1Input`, `P2WorldState`, `P3ReliableGameplay`, `P4BulkCosmetic` with Deficit Round Robin (DRR) scheduling and deadline pruning.
- **Advanced Loss Recovery & RTT Estimation (RFC 9002)**:
  - Smoothed RTT, RTTVAR, and min RTT tracking with ACK delay adjustment.
  - Loss detection via packet threshold ($k=3$), time threshold ($\frac{9}{8}$ factor), and Probe Timeout (PTO).
  - Adaptive ACK frequency and gap-compressed ACK ranges bounded to 32 intervals.
- **Congestion Control & Pacing**:
  - CUBIC congestion controller ($W_{cubic}(t) = C(t-K)^3 + W_{max}$, $\beta=0.7$).
  - High-precision Token-Bucket Pacing Engine to smooth bursty game packet streams.
  - 4-Tier Engine Backpressure feedback (`Low`, `Medium`, `High`, `Critical`) for dynamic Level-of-Detail (LOD).
- **Security & DoS Defenses**:
  - Authenticated Encryption with Associated Data (AEAD) with 96-bit nonce derivation from CID and PacketNumber.
  - 128-bit Sliding Replay Window to defeat duplicate/replayed packet attacks.
  - 3x Anti-Amplification limit for unvalidated peer addresses.
  - Stateless Cookie Tokens for stateless handshake and DoS mitigation.
  - 3-Way Path Challenge/Response for seamless NAT rebinding and path migration.
- **Dedicated Runtime Control API & Async Tokio Integration**:
  - Ergonomic runtime control handle (`ConnectionControl`) for dynamic ACK frequency adjustments, PMTU discovery probing, path validation, and graceful draining.
  - Live diagnostic metrics snapshot (`DetailedMetrics`) and typed notification events (`ControlEvent`).
  - Asynchronous endpoint routing via `gtp-runtime-tokio`.
- **Deterministic Simulation Matrix & CLI Suite (`gtp-sim` & `gtp-cli`)**:
  - 100% reproducible virtual time simulation testing under extreme packet loss (20%), jitter, reordering, and bandwidth bottlenecks.
  - Packet Dissector CLI (`gtp dissect <HEX>`), live simulation benchmark (`gtp sim-benchmark`), and Control API demonstration (`gtp control-demo`).

---

## Workspace Crates Architecture

| Crate | Directory | Purpose |
| :--- | :--- | :--- |
| **`gtp-types`** | `crates/gtp-types` | Fundamental types, identifiers, monotonic time, RFC 1982 modulo sequence, errors. |
| **`gtp-wire`** | `crates/gtp-wire` | Zero-copy binary framing, Long/Short headers, 14 TLV frames, VarInt, PacketBuilder. |
| **`gtp-recovery`** | `crates/gtp-recovery` | ACK tracking, RTT estimation, packet & time loss detector, PTO sweeps. |
| **`gtp-cc`** | `crates/gtp-cc` | CongestionController trait, CUBIC baseline, token-bucket pacing, backpressure. |
| **`gtp-scheduler`** | `crates/gtp-scheduler`| 5-tier DRR scheduler, StateTable supersession, scoped ordered stream buffers. |
| **`gtp-path`** | `crates/gtp-path` | State machine, anti-amplification 3x limiter, stateless tokens, NAT path validator. |
| **`gtp-crypto`** | `crates/gtp-crypto` | PacketProtector trait, AEAD with 96-bit nonce, 128-bit replay window. |
| **`gtp-core`** | `crates/gtp-core` | High-level GtpConnection engine, hot/cold memory separation, RX/TX pipelines, Control API. |
| **`gtp-io`** | `crates/gtp-io` | Low-level socket2 UDP socket abstraction with batching (PacketIo). |
| **`gtp-runtime-tokio`** | `crates/gtp-runtime-tokio` | Async Tokio endpoint, background worker tasks, AsyncGtpConnection. |
| **`gtp-sim`** | `crates/gtp-sim` | Deterministic stepping simulation testbed with configurable network impairments. |
| **`gtp-cli`** | `crates/gtp-cli` | Command-line dissector, benchmark runner, and Control API demo executable (`gtp`). |

---

## Quick Start

### 1. Build and Run Workspace Tests
```bash
# Compile and test all 12 crates
cargo test --workspace
```

### 2. Run Deterministic Simulation Benchmark
```bash
# Execute benchmark across LAN, Good Internet, Bad Cellular, and Extreme 20% Loss scenarios
cargo run -p gtp-cli -- sim-benchmark --ticks 500
```

### 3. Run Dedicated Control API Demonstration
```bash
# Demonstrates runtime ACK frequency adjustment, MTU probing, ping keepalive, and metrics
cargo run -p gtp-cli -- control-demo
```

### 4. Dissect Raw GTP Hex Packets
```bash
cargo run -p gtp-cli -- dissect 80000100011811223344556677880000000000000001000F4240000E05DEADBEEFCAFEBABE
```

---

## Documentation Index

- **Specifications**:
  - [`GTP_1_1_Comprehensive_Technical_Specification.md`](GTP_1_1_Comprehensive_Technical_Specification.md): Full technical specification (516 sections).
  - [`GTP_Architecture_Decision_Paper_v1.0.md`](GTP_Architecture_Decision_Paper_v1.0.md): Architecture decision record.
  - Sub-specifications: `docs/specs/GTP-ARCH-01.md` through `docs/specs/GTP-TEST-01.md`.
- **API & Architecture Guides**:
  - [`docs/GTP_API_ARCHITECTURE_AND_DEVELOPMENT_GUIDELINES.md`](docs/GTP_API_ARCHITECTURE_AND_DEVELOPMENT_GUIDELINES.md): Architecture guide and continuous protocol evolution rules.
  - [`docs/GTP_CONTROL_API_REFERENCE_MANUAL.md`](docs/GTP_CONTROL_API_REFERENCE_MANUAL.md): Complete function-by-function reference manual.
  - [`docs/GTP_COMPREHENSIVE_COMPLIANCE_AND_FEATURE_AUDIT_REPORT.md`](docs/GTP_COMPREHENSIVE_COMPLIANCE_AND_FEATURE_AUDIT_REPORT.md): Compliance audit report mapping all specification requirements.

---

## License

Licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0)**. See [LICENSE](LICENSE) for details.

