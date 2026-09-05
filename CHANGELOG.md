# Changelog

All notable changes to the Game Transport Protocol (GTP-rs) project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — 2026-09-05 gate G1: measurement layer activation

Adaptive-routing gate G1 per `docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md` (v1.1);
design and closure under `docs/routing/`. **134 → 149 tests green** (three
consecutive full-gate runs), deployed to both ends at parity `00eb110`.

### Added
- **RE-1 / A-1**: the wire timestamp (`timestamp_micros`, on the wire inside the AAD since 0.1.0 with zero RX consumers — X-4) is now consumed on every authenticated packet: sliding-floor one-way-delay variance + RFC 3550 §6.4.1 jitter (`gtp-recovery::OwdEstimator`, `Copy`/integer-only — zero RX allocation). Floor re-anchors within a 30 s window, bounding 50 ppm crystal-drift error at ≤ 1.5 ms; `u32` wrap-safe by signed interpretation.
- **A-2**: `ControlEvent::OwdSample` (bounded-rate emission via new `GtpConfig::owd_sample_interval`, 100 ms default — the event queue is unbounded and game traffic runs 60–144 Hz) and `PathEventDetected` (+ `PathEventKind`/`PathDirection`, emission from G5); `DetailedMetrics::owd_var`/`jitter` as `Option<Duration>`; `gtp-cli net-client` prints OWD Variance and OWD Jitter lines.
- **D-1** (G2 groundwork): `gtp-sim::SimulatedFabric` — per-direction link profiles, time-scripted impairment deltas (clamped, cursor-idempotent), splitmix64 per-link-per-direction RNG streams, and a determinism event log; same seed ⟹ identical event sequence.
- INV-18 procedural checks in `scripts/verify_remediation.sh` (wire-field consumer regression guards).
- `docs/routing/`: G1 design note, defect registry (opens with RT-1: `loss_ratio()` mixed-unit proxy), E-6 server-authentication design options (PSK vs operator-signed Ed25519 — recommendation recorded, ADR-006 decisions enumerated), G1 closure report.

### Fixed
- **A-6 / New-11**: `GtpConfig::anti_amplification_factor` is live — was set by four profiles (3/3/3/10) and read by nothing while the limiter hardcoded ×3. Wired at all five construction sites; a real wiring gap found and closed: the handshake-driven `new_with_directional_keys` (the production endpoint path) never saw the config. Factor floored at 1; default 3× unchanged.
- **E-1**: stale comment above the TX amplification gate still described the closed New-12 reflection defect as open — corrected to the closed state.
- **E-2**: `RttStats.min_rtt` field crate-privatized (the `u64::MAX` sentinel is no longer readable outside gtp-recovery; consumers use `min_rtt_sample()`).
- Validated path migration now also resets the OWD estimator beside the X-1 RTT reset (pre-positions INV-13).

### Verified live
- 2,000-frame WAN round on the production VPS: OWD Variance **1.541 ms**, OWD Jitter **267 µs** (non-zero, real 60 FPS internet traffic — the gate criterion), 0 loss/retransmissions/PTO/corrupted; server RSS 892 K (peak 1.7 M).

## [Unreleased] — 2026-09-04 comprehensive re-audit & remediation round

Full evidence-based re-verification of all ~100 tracked defects and closure of every
critical/high item: `docs/reaudit/Re-Audit-Report-2026-09.md` (as-found state) and
`docs/reaudit/Closure-Matrix-2026-09.md` (post-remediation outcomes).

### Fixed — merged ready branches
- **N-1**: endpoint no longer misroutes encrypted data datagrams whose ciphertext begins with handshake-frame type bytes; dispatch is gated on the long-header bit.
- **N-2**: the RX loop never awaits application delivery (`try_send` + slow-consumer isolation), eliminating endpoint-wide head-of-line blocking.
- **X-1**: RTT estimator (incl. `min_rtt`) resets on validated path migration.
- **A-5 / New-8**: anti-amplification bytes are counted only after AEAD authentication, and the 3× budget is per-path (probe-scoped), enabling migration under RFC 9000 §9.3.
- **New-12**: PathChallenge reflection amplification closed — responses only to the active path, at most one per inbound datagram, at most two queued.

### Fixed — this round
- **N-3 / X-19**: scheduler DRR keeps a persistent round cursor with once-per-round quantum accrual and a deficit cap; P3/P4 no longer starve under sustained P1 pressure (shares follow the 35:15:5 weights).
- **FR-7**: O(1) per-tier byte/item accounting replaces the per-enqueue scan.
- **N-7**: scheduler tiers and ordered-group reorder buffers cap item counts, bounding zero-payload floods.
- **N-4 / FU-4**: a PTO probe no longer collapses `cwnd`; the window collapses only on persistent congestion (three consecutive probe rounds without an ACK) per RFC 9002 §7.5; the inert `cc.on_loss` call and stale comments are removed.
- **FR-8**: reorder-buffer drains label every delivered message with its own `order_seq` (`OrderedGroupReceiver::on_incoming` returns `(u32, Vec<u8>)` pairs).
- **N-5**: `min_rtt` is `Option<Duration>` on every consumer surface (`NetworkFeedback`, `DetailedMetrics`, `calculate_backpressure`); unsampled minimums render as `n/a` instead of the `u64::MAX` sentinel.
- **FU-5**: ordered-group eviction is LRU (access refreshes recency) instead of FIFO-by-creation; the misleading comment is gone.
- **FR-5**: `handle_incoming_datagram` refuses all input once the connection is `Closed`.
- **WIR-2**: the ACK encoder writes the clamped range count, never emitting a frame its own decoder rejects.
- **WIR-4**: the Close reason is trimmed at a UTF-8 character boundary.
- **SEC-14**: `DirectionalKeys` / `SessionDirectionalKeys` implement redacted `Debug` (`[REDACTED]`).
- **N-6**: README/CLI `dissect` examples are now valid 28-byte long-header packets (verified live), path validation is documented as two-way, the `competitive_fps` β=0.75 preset tuning is documented, and `stress-suite` verdicts are computed from real counters (corrupted-frame counts, session completion, server accepts) instead of fixed strings.

### Changed
- **BREAKING API**: `NetworkFeedback::min_rtt` and `DetailedMetrics::min_rtt` are now `Option<Duration>` (N-5).
- **BREAKING API**: `OrderedGroupReceiver::on_incoming` returns `Vec<(u32, Vec<u8>)>` (FR-8).
- `calculate_backpressure` takes `min_rtt: Option<Duration>` (N-5).
- `scripts/verify_remediation.sh` resolves cargo automatically (pinned toolchain → PATH → newest installed).
- GTP-SEC-01 spec aligned with the implementation: AAD covers the full header, nonce documented as full-CID‖full-PN with the one-key-one-connection-direction invariant (X-2/X-3).

### Added
- `crates/gtp/tests/cross_layer_integration_test.rs`: full-seam recovery (loss → reorder store → PTO → complete in-order delivery with per-message sequences) and saturation-fairness scenarios.
- `docs/reaudit/Live-WAN-Verification-Report-2026-09-04.md`: official live-WAN testing round at version parity `ba2a486` (local + VPS `92.222.80.200:7777`) — five stepped rounds over the public internet (incl. one genuine loss recovered live), server-side telemetry, full stress suite, and the verification/correction record for the follow-up reports.

### Fixed (follow-up round)
- `09f1d8a`'s stress-harness retry loop formatted to satisfy `cargo fmt --check` (was CI-breaking) — `ba2a486`.
- Follow-up documentation corrected in place: fabricated commit hash replaced with the real `09f1d8a390…`, wrong defect descriptions (N-4/N-6/N-7) restored to their actual definitions, CORE-4/REC-8 overclaims qualified, loopback-only stress stages no longer attributed to live-WAN testing, stale dissect hex fixed; verification addenda appended to the affected reports.
- New unit suites pinning every fix above (39 new tests this round; 135 total green, three identical consecutive full-gate runs).

## [0.2.0] - 2026-08-29

### Added
- **Cryptographic Key Confirmation (`client_proof`)**: Wired HMAC-SHA256 key confirmation proof into `HandshakeFinish` (`compute_client_proof` / `verify_client_proof`), guaranteeing both peers derived identical session keys before establishing connection state.
- **Session Key Ratchet Wired into Lifecycle & Control API**: Wired `ConnectionHot::ratchet_session_key()` and exposed `ConnectionControl::ratchet_key()` and `AsyncGtpConnection::ratchet_key()` for runtime forward-secrecy key rotation.
- **RFC 8312 CUBIC TCP-Friendly Region & Fast Convergence**: Implemented $W_{tcp}(t)$ window estimation to ensure fair coexistence with TCP/QUIC, and Fast Convergence capacity release upon packet loss.
- **Active MITM Attack E2E Verification**: Added test `test_active_mitm_key_tamper_rejected` proving immediate rejection of key substitution attacks.
- **Criterion Benchmark Suites**: Added comprehensive benchmark targets in `crates/gtp-crypto/benches/crypto_bench.rs` and `crates/gtp-wire/benches/wire_bench.rs`.
- **Coverage-Guided Fuzzing Infrastructure**: Added `fuzz/` targets for MTU-sized frame decoding and handshake frame validation.
- **Dynamic Server Connection Intake (`GtpEndpoint::accept`)**: Implemented dynamic async accept loop yielding verified `AsyncGtpConnection` handles upon valid client handshakes.
- **Full-Duplex Live X25519 Diffie-Hellman Handshake**: Wired automated `ClientHello`, `ServerHello`, and `HandshakeFinish` state machine into `GtpEndpoint::connect` and `start_rx_loop` with dynamic shared secret computation and zeroization.
- **Handshake Loss Recovery**: Automated 400ms `ClientHello` retransmission with server-side ephemeral state preservation for duplicate packet idempotency.
- **Strict Stateless Cookie Verification**: Initialized `StatelessTokenManager` in `GtpEndpoint` with constant-time `subtle::ConstantTimeEq` validation strictly enforced in `HandshakeFinish`.
- **ConnectionId-Based Datagram Routing**: Decoupled packet demultiplexing from socket IP addresses to support seamless NAT rebinding and multi-session concurrency.
- **Active Anti-Amplification & Per-IP Rate Limiter**: Enforced RFC 9000 3× bytes boundary for unauthenticated peers with loopback exemption in `GtpEndpoint`.
- **Dynamic NAT Rebind Cryptographic Verification**: Integrated live `PathValidator::start_challenge` and `validate_response` in `gtp-cli stress-suite`.
- **Production ChaCha20-Poly1305 AEAD**: Standard RFC 8439 ChaCha20-Poly1305 authenticated encryption with HKDF-SHA256 session key derivation.
- **Decode-Path Hardening**: Bounded parsing on untrusted wire bytes returning structured `TransportError` without panics.
- **Security Audit in CI**: Added `cargo-audit` via `rustsec/audit-check` to `.github/workflows/ci.yml`.

### Deprecated
- `ConnectionHot::new` and `ConnectionHot::new_with_master_secret` marked as deprecated; live network code now requires dynamic handshake derivation.

### Changed
- **Elimination of Silent Static Fallback**: `GtpEndpoint::connect` now strictly returns `Result<AsyncGtpConnection, TransportError>`, returning typed errors on failure rather than falling back to static keys.
- **BREAKING WIRE CHANGE**: Upgraded handshake wire frames to `ClientHello` (32-byte public key + 32-byte nonce) and `ServerHello` (32-byte public key + 32-byte nonce + 32-byte stateless cookie).
- `PacketProtector::open` signature now accepts `ciphertext_len: usize` for explicit boundary verification.
- `DetailedMetrics` now exposes `total_corrupted_packets` counter.

---

## [0.1.0] - 2026-08-28

### Added
- Initial release of the Game Transport Protocol (GTP/1.1) Rust workspace.
- 13 modular crates: `gtp`, `gtp-types`, `gtp-wire`, `gtp-recovery`, `gtp-cc`, `gtp-scheduler`, `gtp-path`, `gtp-crypto`, `gtp-core`, `gtp-io`, `gtp-runtime-tokio`, `gtp-sim`, `gtp-cli`.
- 4 delivery semantics: `Unreliable`, `UnreliableSequenced`, `ReliableUnordered`, `ReliableOrdered`.
- RFC 9002 loss recovery with adaptive ACK ranges (up to 32 blocks).
- 5-tier DRR game scheduler with state table supersession and deadline pruning.
- CUBIC congestion controller and token-bucket pacing engine.
- Dedicated Control API (`ConnectionControl`, `GtpConfig`, `DetailedMetrics`).
- Deterministic virtual-time network simulation testbed (`gtp-sim`).
- Full AGPL-3.0 licensing.
