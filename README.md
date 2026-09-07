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
  - CUBIC congestion controller ($W_{cubic}(t) = C(t-K)^3 + W_{max}$, $\beta=0.75$ under the default `competitive_fps` preset; RFC 8312's recommended $0.7$ remains configurable).
  - High-precision Token-Bucket Pacing Engine to smooth bursty game packet streams.
  - 4-Tier Engine Backpressure feedback (`Low`, `Medium`, `High`, `Critical`) for dynamic Level-of-Detail (LOD).
- **Security & DoS Defenses**:
  - Authenticated Encryption with Associated Data (AEAD) with 96-bit nonce derivation from CID and PacketNumber.
  - 128-bit Sliding Replay Window to defeat duplicate/replayed packet attacks.
  - 3x Anti-Amplification limit for unvalidated peer addresses.
  - Stateless Cookie Tokens for stateless handshake and DoS mitigation.
  - Two-way Path Challenge/Response for seamless NAT rebinding and path migration.
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
| **`gtp`** | `crates/gtp` | **Primary Unified Rust SDK & Transport Library facade for game engine integration.** |
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
| **`gtp-cli`** | `crates/gtp-cli` | Command-line dissector, benchmark runner, and Control API demo executable (`gtp-cli`). |

---

## Quick Start

### 1. Build and Run Workspace Tests
```bash
# Compile and test all 13 crates & integration suites
cargo test --workspace
```

### 2. Run Synchronous 60 FPS Game Loop Example
```bash
cargo run -p gtp --example sync_game_loop
```

### 3. Run Asynchronous Tokio Server Example
```bash
cargo run -p gtp --example async_tokio_server
```

### 4. Run Deterministic Simulation Benchmark
```bash
cargo run -p gtp-cli -- sim-benchmark --ticks 500
```

### 5. Run Dedicated Control API Demonstration
```bash
cargo run -p gtp-cli -- control-demo
```

### 6. Dissect Raw GTP Hex Packets
```bash
cargo run -p gtp-cli -- dissect 80000100011C11223344556677880000000000000001000F4240000905DEADBEEFCAFEBABE
```

---

## Documentation Index

- **Primary Engineering Reference**:
  - [`docs/ENGINEERING-REFERENCE.md`](docs/ENGINEERING-REFERENCE.md): consolidated remediation history (fix-by-fix with commits), current API surface changes, environment quirks, test-tier guide, quality-gate workflow, VPS runbook, deferred register, and the engineering principles established by the audit rounds. **Start here for development and onboarding.**
- **Specifications**:
  - [`GTP_1_1_Comprehensive_Technical_Specification.md`](GTP_1_1_Comprehensive_Technical_Specification.md): Full technical specification (516 sections).
  - [`GTP-rs-Technical-Specification.md`](GTP-rs-Technical-Specification.md): Developer-team requirements & traceability spec (Arabic).
  - [`GTP_Architecture_Decision_Paper_v1.0.md`](GTP_Architecture_Decision_Paper_v1.0.md): Architecture decision record.
  - Sub-specifications: `docs/specs/GTP-ARCH-01.md` through `docs/specs/GTP-TEST-01.md`.
- **Reports & Operational Verification**:
  - [`docs/reaudit/GTP-rs_Live_WAN_Testing_and_Verification_Report_AR.md`](docs/reaudit/GTP-rs_Live_WAN_Testing_and_Verification_Report_AR.md): **Official Comprehensive Live WAN Testing & Verification Report (Arabic)** — Empirical trans-continental testing to production VPS (`92.222.80.200`), 0.00% loss, 160 MB/s pacing, flat RSS.
  - [`docs/reaudit/GTP-rs_Final_Audit_Conclusions_and_Findings_AR.md`](docs/reaudit/GTP-rs_Final_Audit_Conclusions_and_Findings_AR.md): **Final Comprehensive Audit Conclusions, Findings & Engineering Insights (Arabic)** — Master compendium of all 3 inspection cycles, core architectural conclusions, and defect closure.
  - [`docs/reaudit/GTP-rs_Current_System_State_and_Verification_Report_AR.md`](docs/reaudit/GTP-rs_Current_System_State_and_Verification_Report_AR.md): **Current System State & Laboratory Re-Audit Report (Arabic)** — Evidence-based 134-test re-audit verifying N-1, N-2, New-8, New-12.
  - [`docs/GTP_COMPREHENSIVE_OPERATION_AND_IMPLEMENTATION_REPORT.md`](docs/GTP_COMPREHENSIVE_OPERATION_AND_IMPLEMENTATION_REPORT.md): Complete engineering report covering dynamic server accept, security audits, and multi-machine benchmarks.
  - [`docs/GTP_COMPREHENSIVE_STRESS_AND_STABILITY_TEST_REPORT.md`](docs/GTP_COMPREHENSIVE_STRESS_AND_STABILITY_TEST_REPORT.md): Comprehensive 6-stage stress, endurance, and stability testing report.
  - [`docs/GTP_COMPREHENSIVE_COMPLIANCE_AND_FEATURE_AUDIT_REPORT.md`](docs/GTP_COMPREHENSIVE_COMPLIANCE_AND_FEATURE_AUDIT_REPORT.md): Compliance audit report mapping all specification requirements.
- **API & Architecture Guides**:
  - [`docs/GTP_API_ARCHITECTURE_AND_DEVELOPMENT_GUIDELINES.md`](docs/GTP_API_ARCHITECTURE_AND_DEVELOPMENT_GUIDELINES.md): Architecture guide and continuous protocol evolution rules.
  - [`docs/GTP_CONTROL_API_REFERENCE_MANUAL.md`](docs/GTP_CONTROL_API_REFERENCE_MANUAL.md): Complete function-by-function reference manual.
- **Security & Remediation**:
  - [`GTP-rs-Architecture-Protocol-Audit-Paper-v1.0.md`](GTP-rs-Architecture-Protocol-Audit-Paper-v1.0.md): Comprehensive architecture/protocol audit (~70 findings with severity ratings and exact locations).
  - [`GTP-rs-Comprehensive-Audit-and-Remediation-Plan.md`](GTP-rs-Comprehensive-Audit-and-Remediation-Plan.md): Adopted master re-verification & remediation plan of the 2026-09-04 round (bilingual EN/AR).
  - [`docs/reaudit/Re-Audit-Report-2026-09.md`](docs/reaudit/Re-Audit-Report-2026-09.md): Evidence-based re-verification of all ~100 tracked defects (as-found state).
  - [`docs/reaudit/Closure-Matrix-2026-09.md`](docs/reaudit/Closure-Matrix-2026-09.md): Final closure matrix with measured outcomes, deferred register, and reproducibility evidence.
  - [`docs/reaudit/Live-WAN-Verification-Report-2026-09-04.md`](docs/reaudit/Live-WAN-Verification-Report-2026-09-04.md): Official live-WAN testing report
  - Adaptive routing: [`docs/routing/MEASUREMENT-AND-SELECTION-REFERENCE.md`](docs/routing/MEASUREMENT-AND-SELECTION-REFERENCE.md) — **the primary reference for the measurement mechanisms** (wire timestamp → OwdEstimator → telemetry → bidirectional reports → scoring → selection), the extension recipe for adding new measurement patterns, and the accomplishment record. Start here for any telemetry/routing work.
  - [`docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md`](docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md) (plan of record) and [`docs/routing/`](docs/routing/) — G1/G2 design notes, closure reports, defect registry, and the E-6 server-authentication design. `gtp-cli route-probe` prints the live bidirectional device↔node measurement table and the shadow route verdict. — version parity audit, five stepped rounds over the public internet, stress suite, server telemetry, and the corrections applied to the follow-up reports.
  - [`GTP-rs-Cross-Audit-Reconciliation.md`](GTP-rs-Cross-Audit-Reconciliation.md): Independent cross-audit reconciliation with the unified critical list.
  - [`GTP-rs-Remediation-Execution-Plan-v1.0.md`](GTP-rs-Remediation-Execution-Plan-v1.0.md): Phased remediation execution plan derived from the audit.
  - [`GTP-rs-Remediation-Tracker.md`](GTP-rs-Remediation-Tracker.md): Live execution tracker with per-item regression-test evidence.
  - Verification gate: `bash scripts/verify_remediation.sh` (tests + clippy + procedural checks).
- **Process**:
  - [`GTP-rs_SESSION_CONTEXT_GUIDE.md`](GTP-rs_SESSION_CONTEXT_GUIDE.md): Permanent session-continuity and context-restoration guide.
- **Roadmap & Integration (Adaptive Routing / Gaming VPN)**:
  - [`docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md`](docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md): **Plan of record for the adaptive-routing work** — reconciles the technical paper against the code as built, records the corrections the paper needs, and schedules every capability into tracks A–E with gates G1–G7. **Start here before writing routing-engine code.**
  - [`GTPrs_Integrated_CrossLayer_Design_and_Audit__AR.md`](GTPrs_Integrated_CrossLayer_Design_and_Audit__AR.md): Cross-layer integration audit, system invariants, and the adaptive-routing engine specification (GTP-rs-ICD-01, Arabic).
  - [`GTP++.md`](GTP++.md): Extended technical paper on server-side adaptive routing for a gaming VPN over GTP-rs (Arabic).
  - [`GTP_Adaptive_Routing_Technical_Paper.md`](GTP_Adaptive_Routing_Technical_Paper.md): Companion copy of the adaptive-routing paper (Arabic).
  - [`Technical_Paper_Gaming_VPN_Adaptive_Routing.md`](Technical_Paper_Gaming_VPN_Adaptive_Routing.md): Gaming VPN adaptive & server-side route selection (English/Arabic mixed).
- **Roadmap & Integration**:
  - [`GTP-rs-technical-paper.md`](GTP-rs-technical-paper.md): Remaining work, enhancements, and additional testing.
  - [`GTP-rs-remediation-plan.md`](GTP-rs-remediation-plan.md): Phased fix-and-development plan across all crates.
  - [`GTP-rs-remaining-fixes.md`](GTP-rs-remaining-fixes.md): Post-705f985 fixes and updates.
  - [`GTP-rs-wiring-fixes.md`](GTP-rs-wiring-fixes.md): Handshake and component wiring fixes.
  - [`zgalaxy_rs_gtp_direct_relay_technical_spec_v3.md`](zgalaxy_rs_gtp_direct_relay_technical_spec_v3.md): zgalaxy-rs integration spec (direct-first with relay fallback).

---

## License

Licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0)**. See [LICENSE](LICENSE) for details.

