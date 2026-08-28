# GTP/1 — Architecture Decision & Implementation Boundary Paper v1.0
## Technical Paper Defining What is Built Inside the Protocol, What is Reused, and What Serves as Design Reference

**Status:** Technical Architecture / Architecture Decision Record  
**Version:** 1.0  
**Date:** August 28, 2026  
**Project:** Game Transport Protocol (GTP/1)  
**Target Language:** Rust  
**Reference Platform:** Linux / Internet / Data Center / LAN  
**Primary Specification Reference:** GTP/1.1 Comprehensive Technical Specification  

---

## 1. Purpose of This Paper

This paper is an independent Architecture Decision Record (ADR) derived from the comprehensive GTP/1.1 technical specification. Its primary objective is to settle a fundamental question prior to implementation:

> Is GTP a brand-new transport protocol designed from first principles, or is it merely an aggregation of KCP, QUIC, Homa components, and pre-existing libraries?

The formal decision established herein is:

> **GTP is a novel, specialized game transport protocol regarding its wire semantics, message model, connection state machine, recovery mechanisms, scheduling, pacing, path management, and application API. However, it leverages battle-tested, off-the-shelf implementation components when those components constitute low-level infrastructure or cryptographic primitives and do not define the identity or wire semantics of GTP.**

Consequently, GTP is neither defined as a fork of KCP or QUIC, nor as a thin wrapper around existing QUIC libraries.

---

## 2. Fundamental Architectural Decision

The original specification describes GTP as a specialized transport layer for games operating over UDP, emphasizing that it does not clone any single existing protocol in full. Instead, it draws inspiration from:
- **KCP**: Selectivity, tunability, low overhead, and delivery telemetry.
- **QUIC**: Connection IDs, monotonic packet numbering, ACK ranges, RFC 9002 RTT/loss recovery, path validation, anti-amplification, and AEAD security boundaries.
- **QUIC DATAGRAM**: Unreliable congestion-controlled datagrams.
- **Homa**: Message-oriented scheduling, urgency/deadline awareness, and receiver-aware transport concepts.
- **Linux UDP & OS Primitives**: GSO/GRO, batching, socket2, and io_uring.
- **Rust Idioms**: Safe memory ownership, zero-copy views, and thread-local state partitioning.

This establishes three distinct, non-overlapping architectural tiers:

```text
A. GTP Protocol Definition
   What is GTP on the wire and in its state machines?

B. GTP Implementation Infrastructure
   How do we implement GTP with high efficiency in Rust/Linux?

C. External Design References
   What architectural lessons and proven algorithms do we learn from QUIC, KCP, Homa, etc.?
```

---

## 3. The Golden Rules of the Project

### 3.1 The Protocol Belongs to GTP
Everything that defines the normative identity, wire representation, or network behavior of GTP MUST be defined natively within GTP, rather than borrowing wire semantics from another protocol.

This includes:
- Packet format and wire layout.
- Frame model and TLV encoding.
- Message semantics (4 core delivery semantics).
- Message, packet, fragment, and transmission identities.
- Ordering, sequence, and generation semantics.
- Reliability guarantees and selective recovery policies.
- Freshness, deadline, and supersession pruning rules.
- ACK format, range encoding, and adaptive frequency rules.
- Loss declaration, RTT estimators, and probe timeout (PTO) triggers.
- Game scheduler semantics, DRR deficits, and priority tiers.
- Pacing engine contract and token-bucket budget.
- Congestion controller interface and backpressure signaling.
- Path state machine, migration, and NAT rebinding logic.
- Connection ID (CID) routing and lifecycle state transitions.
- Handshake state machine, stateless tokens, and anti-amplification defenses.
- Anti-replay sliding window and AEAD security boundary.
- High-level semantic Game Application API.

### 3.2 Libraries Belong to the Implementation Layer
Mature, low-level primitives should not be reinvented simply because the protocol is new.

The implementation may reuse:
- Linux UDP system interfaces.
- `socket2` for portable socket configuration.
- `io_uring` for asynchronous zero-copy batching where available.
- Runtime adapters (e.g., Tokio, custom event loops).
- Audited cryptographic primitive backends (e.g., AES-GCM, ChaCha20-Poly1305).
- Low-level byte-view utilities (e.g., `bytes`, `zerocopy`).

However, these components must NEVER dictate or alter GTP's normative wire protocol.

### 3.3 Reference Protocols are Not Dependencies
QUIC, KCP, Homa, `s2n-quic`, and `quinn` serve as design references to study proven patterns and validate algorithmic decisions, NOT as mandatory dependencies or underlying protocol engines for GTP.

---

## 4. Tier Definitions

### 4.1 GTP-Native
A component is **GTP-Native** when it defines the semantics, state machine, wire representation, scheduling, or recovery behavior of the protocol.

**Decision Rule:**
> If changing the component alters the meaning of a packet/message or how peers interact on the wire, it is part of GTP and MUST be owned by the GTP codebase.

### 4.2 Reused Implementation Component
An implementation component provides an execution mechanism or low-level OS primitive without dictating wire semantics.

*Examples:* `socket2`, `io_uring`, Linux UDP APIs, hardware GSO/GRO, audited crypto backends, Tokio runtime adapters.

**Decision Rule:**
> An implementation component can be swapped or disabled without changing GTP wire format or delivery semantics.

### 4.3 Design Reference
An external system or paper studied for design concepts and algorithmic models, without importing its code as a runtime protocol engine.

*Examples:* RFC 9000 (QUIC), RFC 9002 (QUIC Loss Recovery), KCP, Homa, BBR algorithms, `s2n-quic`, `quinn`.

---

## 5. Architecture Decision Matrix

| Component | Classification | Architectural Decision | Code Ownership | Mandatory Dependency? |
| :--- | :--- | :--- | :--- | :--- |
| **GTP Wire Format** | GTP-Native | Custom binary format | GTP | No |
| **Common Header** | GTP-Native | Custom binary format | GTP | No |
| **Long Header (28B)** | GTP-Native | Custom binary format | GTP | No |
| **Short Header (24B)**| GTP-Native | Custom binary format | GTP | No |
| **Connection ID (CID)**| GTP-Native | Independent 64-bit identifier | GTP | No |
| **Packet Number (PN)** | GTP-Native | Monotonically increasing 64-bit | GTP | No |
| **Message ID** | GTP-Native | Logical application entity ID | GTP | No |
| **Fragment ID** | GTP-Native | Fragmentation index | GTP | No |
| **Transmission ID** | GTP-Native | Retransmission counter | GTP | No |
| **State Sequence** | GTP-Native | RFC 1982 modulo sequence | GTP | No |
| **Generation ID** | GTP-Native | State epoch identifier | GTP | No |
| **UNRELIABLE** | GTP-Native | Core delivery semantic | GTP | No |
| **UNRELIABLE_SEQUENCED** | GTP-Native | Core delivery semantic | GTP | No |
| **RELIABLE_UNORDERED** | GTP-Native | Core delivery semantic | GTP | No |
| **RELIABLE_ORDERED** | GTP-Native | Core delivery semantic | GTP | No |
| **ACK Format & Ranges** | GTP-Native | Bounded 32-range codec | GTP | No |
| **Adaptive ACK Frequency** | GTP-Native | Dynamic rate adjustment frame | GTP | No |
| **RTT Estimation (RFC 9002)**| GTP-Native | Smoothed RTT, RTTVAR, PTO | GTP | No |
| **Loss Detection** | GTP-Native | $k=3$ packet, $9/8$ time threshold | GTP | No |
| **Selective Recovery** | GTP-Native | Message/fragment granularity | GTP | No |
| **Retransmission Cancellation** | GTP-Native | Generation & deadline aware | GTP | No |
| **Deadline Pruning Engine** | GTP-Native | Drop expired payloads | GTP | No |
| **State Supersession** | GTP-Native | Automatic eviction via StateTable | GTP | No |
| **Game Priority Scheduler** | GTP-Native | 5 DRR Tiers (P0..P4) | GTP | No |
| **Token-Bucket Pacing** | GTP-Native | Sub-ms burst & rate control | GTP | No |
| **Congestion Controller Trait** | GTP-Native | Pluggable interface | GTP | No |
| **CUBIC Baseline** | Algorithm | Implemented as baseline CC | GTP | No |
| **ECN Processing** | GTP-Native | Integrated into CC feedback | GTP | No |
| **Backpressure Feedback** | GTP-Native | 4-Tier game engine signal | GTP | No |
| **Path State Machine** | GTP-Native | Full connection lifecycle | GTP | No |
| **PATH_CHALLENGE / RESPONSE** | GTP-Native | 3-way NAT validation frames | GTP | No |
| **Anti-Amplification** | GTP-Native | 3x incoming datagram budget | GTP | No |
| **Handshake State Machine** | GTP-Native | 3-way with stateless cookies | GTP | No |
| **Packet Protector Trait** | GTP-Native abstraction | Pluggable security boundary | GTP | No |
| **AEAD Primitive** | Reused Implementation | Audited cryptographic library | External backend | Yes (implementation level) |
| **128-bit Replay Window** | GTP-Native | Sliding bitmap window | GTP | No |
| **Packet Codecs & Builders** | GTP-Native | Zero-copy serialization | GTP | No |
| **Connection Table** | GTP-Native | Sharded worker-local state | GTP | No |
| **Portable UDP** | Backend | Standard OS sockets | OS / socket2 | No |
| **Linux UDP & GSO/GRO** | Backend | Platform optimizations | Linux | Yes (reference platform) |
| **io_uring** | Reused Mechanism | Async zero-copy backend | Linux / library | Optional |
| **Tokio Adapter** | Runtime | Async integration wrapper | External + adapter | No |
| **Monoio Adapter** | Runtime | Thread-per-core integration | External + adapter | No |
| **QUIC / RFC 9000** | Design Reference | Source of concepts | External | No |
| **KCP** | Design Reference | Reliability lessons | External | No |
| **Homa** | Design Reference | Scheduling ideas | External | No |

---

## 6. Implementation Scope: What is Built Inside GTP

### 6.1 Wire Layer
GTP fully owns:
- Header layout (Version, Flags, Header Length, Connection ID, Packet Number, Timestamp, Payload Length).
- ACK section encoding (largest acknowledged, ACK delay, range count, gap-length pairs, ECN marks).
- All 14 TLV Frame structures.
- Authenticated payload framing and padding boundaries.

*Rationale:* Wire format dictates network interoperability across all current and future GTP implementations and must never depend on third-party library structs.

### 6.2 Identity Model
Clear separation is maintained between:
- `PacketNumber`: Wire datagram sequence for congestion control, pacing, and ACK tracking.
- `MessageId`: Application-level message identifier.
- `FragmentId`: Sub-message index for segmentation.
- `TransmissionId`: Retransmission attempt counter.
- `StateSequence`: Entity state version sequence.
- `GenerationId`: State epoch identifier.

*Rationale:* This separation enables selective retransmission of individual frames/messages rather than whole packets.

### 6.3 Message Semantics
GTP natively implements the four distinct delivery classes:
1. `UNRELIABLE`
2. `UNRELIABLE_SEQUENCED`
3. `RELIABLE_UNORDERED`
4. `RELIABLE_ORDERED`

### 6.4 Freshness and Deadline Engine
The transport layer inherently understands:
- `created_at`, `remaining_lifetime`, `deadline`, `priority`, `state_generation`, and `supersession`.
- Evaluates `DROP`, `SEND`, or `RETX` decisions directly in the scheduler based on freshness.

### 6.5 Priority Scheduler
A custom game-aware scheduler combining:
- 5 priority tiers (P0 Control, P1 Input, P2 World State, P3 Reliable Gameplay, P4 Bulk Cosmetic).
- Deficit Round Robin (DRR) fairness with strict P0 control reservation.
- Automatic eviction of superseded state frames via `StateTable`.
- Deadline-based expiration pruning.

### 6.6 Loss Recovery Subsystem
GTP-native recovery featuring:
- RFC 9002 compliant smoothed RTT and RTTVAR calculation.
- Packet threshold ($k=3$) and time threshold ($\frac{9}{8}\max$) loss declaration.
- Probe Timeout (PTO) triggering.
- Independent logical message retransmission and cancellation.

### 6.7 Path Management & DoS Protection
- Path validation via `PATH_CHALLENGE` and `PATH_RESPONSE`.
- NAT rebinding and address migration.
- 3x anti-amplification budget enforcement for unvalidated peers.
- Stateless cookie tokens for handshake flood mitigation.

### 6.8 Semantic Application API
High-level, semantic-oriented API:
```rust
send_unreliable(payload, priority, deadline, now);
send_sequenced(state_key, sequence, generation, deadline, payload, now);
send_reliable_unordered(payload, priority, deadline, now);
send_reliable_ordered(group_id, payload, priority, deadline, now);
```

---

## 7. Reused Components & Infrastructure

### 7.1 Cryptography
Cryptographic primitives are accessed through clean abstractions (`PacketProtector` trait). Audited, hardware-accelerated libraries provide AES-GCM and ChaCha20-Poly1305 implementations without hardcoding crypto code directly into GTP.

### 7.2 `socket2`
Used for portable, non-blocking UDP socket initialization, OS buffer configuration (2MB send/recv buffers), and IP-level flags (DF bit).

### 7.3 `io_uring` & Linux GSO/GRO
Considered optional transport I/O optimizations.
- **Rule:** Enabling or disabling GSO/GRO/io_uring MUST NOT alter GTP delivery semantics, reliability, or wire format.

### 7.4 Runtime Adapters
`gtp-core` remains runtime-agnostic. Adapters (`gtp-runtime-tokio`, etc.) integrate GTP into specific async runtimes without coupling the core engine.

---

## 8. Architectural Independence Tests

GTP architecture is validated against the following modularity tests:

1. **Backend Interchangeability**: The same `gtp-core` operates seamlessly over standard UDP sockets, Tokio channels, or simulated network testbeds.
2. **Crypto Interchangeability**: The crypto backend can be swapped (or disabled in simulations via `PlaintextProtector`) without altering wire semantics.
3. **Hardware Acceleration Agility**: Disabling GSO/GRO has zero effect on reliability, ordering, or supersession.
4. **Congestion Controller Agility**: Any CC implementing `CongestionController` (CUBIC, BBR-like, custom) can be swapped via dynamic or static dispatch.
5. **Zero Hidden Protocol Dependencies**: If QUIC or KCP libraries are removed from dependencies, GTP compiles and runs completely independently.

---

## 9. Architectural Ownership Summary

> **Any code that determines what a packet/message means belongs to GTP. Any code that determines how raw bytes reach the kernel/NIC belongs to an external backend.**

| Functionality | Component Ownership |
| :--- | :--- |
| ACK range encoding & semantics | **GTP** |
| Socket send syscall | **Backend** |
| Reliable unordered delivery logic | **GTP** |
| UDP socket binding & buffer sizing | **Backend** |
| Deadline & state supersession | **GTP** |
| System timer / epoll / timerfd | **Backend** |
| Connection ID routing semantics | **GTP** |
| AEAD nonce derivation formula | **GTP** |
| AES-GCM instruction execution | **Crypto Library** |

---

## 10. Conclusion

**GTP is neither a wrapper nor a fork.** It is a dedicated, first-class game transport protocol that owns its architecture, wire protocol, and game-aware semantics while reusing proven low-level infrastructure.

> **Build the protocol. Reuse the infrastructure. Study proven protocols. Do not inherit their architecture unless the requirement explicitly demands it.**
