# Changelog

All notable changes to the Game Transport Protocol (GTP-rs) project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
