# Changelog

All notable changes to the Game Transport Protocol (GTP-rs) project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-08-29

### Added
- **Ephemeral X25519 Diffie-Hellman Key Exchange**: Integrated `x25519-dalek` in `gtp-crypto::handshake` with random client/server nonces and zeroization via `zeroize`.
- **Active Anti-Amplification Enforcement**: Strictly enforces RFC 9000 3× bytes limit on unauthenticated peer responses until address ownership is verified via authentic AEAD packets or handshake tokens.
- **Session Key Ratchet & Rotation**: Implemented `ratchet_key` in `gtp-crypto::handshake` for long-lived session key forward secrecy.
- **Constant-Time Verification**: Applied `subtle::ConstantTimeEq` to stateless tokens and cookies in `gtp-path` to mitigate timing side-channels.
- **Expanded 6-Stage Stress Suite**: Added high-concurrency multi-session stress (200 parallel clients) and live NAT rebinding/path migration stages to `gtp-cli stress-suite`.
- **Production ChaCha20-Poly1305 AEAD**: Replaced initial XOR scaffold with standard RFC 8439 ChaCha20-Poly1305 authenticated encryption in `gtp-crypto`.
- **HKDF-SHA256 Session Key Derivation**: Implemented `kdf.rs` to derive unique, isolated 256-bit encryption keys and 96-bit base IVs per `ConnectionId`.
- **Static Dispatch `Protector` Enum**: Replaced `Box<dyn PacketProtector>` in `ConnectionHot` with zero-allocation `Protector` enum for static dispatch.
- **Decode-Path Hardening**: Replaced all unchecked `unwrap()` calls on incoming untrusted wire bytes in `gtp-wire` with bounds-checked parsing returning `TransportError::TruncatedFrame` or `TransportError::MalformedFrame`.
- **Fast Hot-Path Integer Hashing**: Integrated `rustc-hash::FxHashMap` in `gtp-core` for internal channel state tables.
- **Security Audit in CI**: Added `cargo-audit` via `rustsec/audit-check` to `.github/workflows/ci.yml`.

### Changed
- **BREAKING WIRE CHANGE**: Handshake frame types upgraded to `ClientHello` and `ServerHello` carrying 32-byte X25519 public keys and 32-byte nonces.
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
