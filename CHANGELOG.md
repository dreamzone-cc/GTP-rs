# Changelog

All notable changes to the Game Transport Protocol (GTP-rs) project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-08-29

### Added
- **Full-Duplex Live X25519 Diffie-Hellman Handshake**: Wired automated `ClientHello`, `ServerHello`, and `HandshakeFinish` state machine into `GtpEndpoint::connect` and `start_rx_loop` with dynamic shared secret computation and zeroization.
- **Active Anti-Amplification & Per-IP Rate Limiter**: Enforced RFC 9000 3× bytes boundary for unauthenticated peers with 20 hellos/sec rate limiting in `GtpEndpoint`.
- **Integrated Stateless Cookie Verification**: Initialized `StatelessTokenManager` in `GtpEndpoint` with constant-time `subtle::ConstantTimeEq` validation.
- **Dynamic NAT Rebind Cryptographic Verification**: Integrated live `PathValidator::start_challenge` and `validate_response` in `gtp-cli stress-suite`.
- **Production ChaCha20-Poly1305 AEAD**: Standard RFC 8439 ChaCha20-Poly1305 authenticated encryption with HKDF-SHA256 session key derivation.
- **Session Key Ratchet**: Implemented `ratchet_key` in `gtp-crypto::handshake` for forward-secure long-lived connections.
- **Static Dispatch `Protector` Enum**: Zero-allocation static dispatch enum for packet encryption/decryption.
- **Decode-Path Hardening**: Bounded parsing on untrusted wire bytes returning structured `TransportError` without panics.
- **Security Audit in CI**: Added `cargo-audit` via `rustsec/audit-check` to `.github/workflows/ci.yml`.

### Deprecated
- `ConnectionHot::new` and `ConnectionHot::new_with_master_secret` marked as deprecated; live network code now requires dynamic handshake derivation.

### Changed
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
