# A Comprehensive Plan for Fixing and Developing the GTP-rs Project

**Scope:** all thirteen workspace crates
**Reference:** direct inspection of the source code (not the marketing documentation)
**Governing principle:** every change must preserve — or improve — performance, efficiency, and stability, and must respect the existing dependency graph between the crates, along with Rust's philosophy of error handling, allocation, and concurrency.

---

## 0. The current dependency map (must be respected in execution order)

```
gtp-types  (no_std-capable, depends on nothing)
   └─▶ gtp-wire        (packet encode/decode)
          ├─▶ gtp-recovery    (RTT / packet loss)
          ├─▶ gtp-scheduler   (DRR / ordering)
          └─▶ gtp-path        (path state / DoS)
gtp-crypto (standalone, does not depend on gtp-wire)
   └─▶ (consumed by) gtp-core
gtp-cc     (depends on gtp-recovery + gtp-types)
   └─▶ (consumed by) gtp-core

gtp-core  = gtp-types + gtp-wire + gtp-recovery + gtp-cc + gtp-scheduler + gtp-path + gtp-crypto
   ├─▶ gtp-io
   ├─▶ gtp-runtime-tokio
   └─▶ gtp-sim
          └─▶ gtp-cli
gtp (the unified SDK) = gathers all of the above behind one interface
```

**Practical impact on the plan:** any fix in `gtp-types` or `gtp-wire` forces retesting everything above them (11 crates). Therefore the lower layers must be fixed **first and in full isolation**, with their public APIs frozen before moving to the higher layers — this prevents repeated "rework waves".

---

## Phase 0 — the baseline before any change

**Goal:** own a trustworthy reference point for comparison, so we can prove the fixes did not degrade performance or stability.

| Task | Details | Affected crates |
|---|---|---|
| Pin the toolchain | Add `rust-toolchain.toml` with a fixed Rust version (the same as the compliance report: 1.98.0) | workspace root |
| Add basic CI | `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check` | workspace root |
| Reference benchmarks | Add `criterion` as a dev-dependency in `gtp-cc`, `gtp-wire`, `gtp-crypto`, recording current numbers (encode/decode throughput, seal/open time, CUBIC convergence) | gtp-cc, gtp-wire, gtp-crypto |
| Document current error behavior | A full inventory of every `unwrap`/`expect`/`panic!` classified: (a) inside `#[cfg(test)]` — do not touch, (b) on a path handling untrusted network input — highest priority, (c) on an internal path with type-guaranteed valid inputs — low priority | gtp-wire, gtp-core, gtp-recovery, gtp-scheduler |

**Acceptance criteria:** green CI on the current code as-is (before any fix), with benchmark numbers recorded as the reference. **Do not start Phase 1 before this phase completes.**

---

## Phase 1 — fixing the cryptography layer (security-critical, architecturally isolated)

### 1.1 Key derivation — done first because it is the foundation AEAD relies on

**The discovered problem:** a fixed key `[0x3C; 32]` and a fixed IV `[0x7E; 12]` are written directly in `ConnectionHot::new`, shared across all connections.

**The architectural solution:**
- Add a new `HandshakeSecret` type in `gtp-crypto` (a new module `gtp-crypto/src/kdf.rs`) using HKDF (via the `hkdf` + `sha2` crates, both pure-Rust and `no_std`-compatible when needed).
- Every connection derives a separate `[u8; 32]` key and `[u8; 12]` IV from a shared secret produced by a key exchange (X25519 via the `x25519-dalek` crate is preferred) + the `ConnectionId` as the HKDF "info" to guarantee no key repetition between connections.
- **Timing:** this derivation happens **once at connection creation** (the handshake), not per packet — **zero impact on actual gameplay latency**.

**Architectural compatibility:** it does not change the `PacketProtector` trait signature at all. `ConnectionHot::new` changes only in how the `GtpAeadProtector` is constructed (it now receives a derived key instead of the hardcoded constant).

### 1.2 Replacing the AEAD algorithm

**The replacement:** `GtpAeadProtector` (XOR + an FNV-like hash) → a real implementation via the `chacha20poly1305` crate (pure Rust, needs no AES-NI, predictable performance across all player hardware).

**Architectural compatibility — a critical point:** as noted in the previous analysis, `protector: Box<dyn PacketProtector>` is the project's only exception to static dispatch. In the rewrite, it is replaced with:

```rust
pub enum Protector {
    Aead(GtpAeadProtector),
    Plaintext(PlaintextProtector),
}
impl PacketProtector for Protector {
    fn seal(&self, ...) -> Result<usize> {
        match self {
            Protector::Aead(p) => p.seal(...),
            Protector::Plaintext(p) => p.seal(...),
        }
    }
    // ... open, tag_len in the same pattern
}
```
This removes the heap allocation (`Box`) and returns the project to its internal consistency with static dispatch, while preserving the same functional flexibility (toggling encryption at runtime via `secure: bool`).

**The required changes in detail:**

| File | The change |
|---|---|
| `gtp-crypto/Cargo.toml` | Add `chacha20poly1305`, `hkdf`, `sha2`, `x25519-dalek` |
| `gtp-crypto/src/aead.rs` | Fully rewrite `seal`/`open`/`compute_tag` to call `ChaCha20Poly1305::encrypt/decrypt` instead of the manual loops |
| `gtp-crypto/src/kdf.rs` (new) | HKDF logic + the X25519 exchange |
| `gtp-crypto/src/lib.rs` | Add `pub mod kdf;` and export the new types |
| `gtp-core/src/state.rs` | Replace `Box<dyn PacketProtector>` with the `enum Protector`, and wire `ConnectionHot::new` to the new handshake path instead of the fixed key |

**What does not change (to guarantee stability):** the `PacketProtector` trait signature, the `AEAD_TAG_LEN` size (stays 16 bytes — compatible with the current packet format), and the `seal`/`open` interface used throughout the rest of `gtp-core`.

### 1.3 The mandatory regression tests for this phase
- A full seal/open round-trip test with actually derived keys (not test constants as currently).
- A test that the keys of two different connections (the same two parties, two separate sessions) **actually differ** — guaranteeing no nonce/key repetition across connections.
- A tag/AAD tamper-break test — partially present today; keep and extend.
- Comparative benchmarks: `seal`/`open` time before/after — **acceptance criterion: the difference must stay within an acceptable margin for small packets (<200 bytes), expected to be only a few hundred nanoseconds**.

---

## Phase 2 — hardening the decode path

**The reason:** `gtp-wire/src/frame.rs` contains 39 uses of `unwrap()`/`expect()` on bytes arriving directly from the network (an untrusted source) — a real DoS hazard.

### 2.1 The fix strategy (without changing the file layout)
- Add a new error type or extend the existing `TransportError` in `gtp-types` with a `TruncatedFrame { needed: usize, available: usize }` variant.
- Replace every pattern:
  ```rust
  buf[offset..offset + N].try_into().unwrap()
  ```
  with:
  ```rust
  buf.get(offset..offset + N)
      .ok_or(TransportError::TruncatedFrame { needed: N, available: buf.len().saturating_sub(offset) })?
      .try_into()
      .expect("slice length checked above") // this expect is now safe because the length was verified
  ```
- Change the decode function signatures in `frame.rs` from `-> FrameType` to `-> Result<FrameType, TransportError>` (if not already so) and propagate the error via `?` up to the consuming layer in `gtp-core`.

### 2.2 Handling the consuming layer (gtp-core)
- In `connection.rs`, upon receiving a packet producing `TransportError::TruncatedFrame` or similar: **silently drop the packet and record it in the metrics**, not terminate the connection or panic. This is the correct behavior for a network protocol over UDP: one corrupt packet must not kill the session.
- No other `gtp-core` logic needs changing — only the error-receiving point from `gtp-wire`.

### 2.3 Why this does not affect performance
- On the healthy path (more than 99.9% of packets in normal operation), the difference between `unwrap()` and `ok_or(...)?` is one additional length check (a single branch the CPU predicts correctly almost always) — a cost below one nanosecond per field, negligible against the full packet processing time (encryption + scheduling + socket send).
- **No additional heap allocation is used**: `TransportError` is designed as a simple enum with `Copy` fields, so its propagation through `Result` costs essentially nothing.

### 2.4 The crates affected by this phase (in the mandatory order imposed by the dependency graph)
1. `gtp-types` (extending `TransportError`) — must compile and pass its tests first before anything else.
2. `gtp-wire` (frame.rs, header.rs, codec.rs, varint.rs) — the largest workload of this phase.
3. `gtp-recovery`, `gtp-scheduler` — also contain unwraps (4 each) but are less critical since they operate on data already validated by `gtp-wire`; handle them with the same pattern at a lower priority.
4. `gtp-core` — the final consumption point of all the new errors.

**Acceptance criteria:** run fuzzing (see Phase 3) for at least one hour on `gtp-wire::frame::decode` with zero panics — only structured `Result::Err`.

---

## Phase 3 — testing and verification infrastructure

| Task | Details | Why it does not affect production |
|---|---|---|
| `cargo-fuzz` on `gtp-wire` | A dedicated fuzz target (`fuzz_targets/decode_frame.rs`) feeding random bytes to the decode function | Fuzzing code lives entirely separately in `fuzz/`, never compiled into the normal `cargo build --release` |
| `cargo-fuzz` on `gtp-crypto` | Verify no panics when opening corrupted payloads after the new encryption | The same isolation as above |
| Network edge-case tests | Truncated packets, packet-number wraparound (RFC 1982), extreme reordering, long consecutive loss, duplication exactly at the replay-window boundary | Added inside the existing `#[cfg(test)]`, zero impact on the release binary |
| Full integration tests via `gtp-sim` | Use the existing deterministic simulator for a full connection under realistic network conditions (20% loss, jitter, limited bandwidth), verifying: no panics, CUBIC convergence, no anti-amplification violations | Uses the pre-existing `gtp-sim` infrastructure — nothing new to build |
| `cargo audit` in CI | Automatic checking of known vulnerabilities in the new dependencies (chacha20poly1305, hkdf, etc.) on every push | Build-time only |

**Execution order:** this phase runs in parallel with the end of Phase 2 (fuzzing needs the new decode-path code to exist first to be useful).

---

## Phase 4 — performance enhancements and ideal Rust practices (optional, after Phases 1-3 stabilize)

This phase **does not address critical defects**; it elevates the project from "works correctly" to "exploits Rust with maximum efficiency".

### 4.1 The release profile
Add to the root `Cargo.toml`:
```toml
[profile.release]
lto = "fat"
codegen-units = 1
opt-level = 3
panic = "abort"   # only after Phase 2 fully completes and fuzzing verifies it
strip = "symbols" # reduces the final binary size for distribution to game clients
```
**Condition:** `panic = "abort"` is conditional on first removing every unwrap on untrusted-input paths (Phase 2); otherwise it increases risk instead of reducing it.

### 4.2 Replacing the hasher on the hot path
In `gtp-core/src/state.rs`, replace `std::collections::HashMap` with `rustc-hash::FxHashMap` for the `ordered_groups` and `next_order_seqs` fields — the keys are internal (`u16`) and not fed raw from the network, so no hash-flooding risk justifies the slower SipHash.

### 4.3 Enriching the CUBIC algorithm (optional)
- Add the "TCP-friendly region" and "fast convergence" (RFC 8312) in `gtp-cc/src/cubic.rs` as an extra logical branch inside `update_w_cubic` — it does not change the function's external structure or the trait.
- Improve the `pacing_rate` computation to depend on RTT variance (`RTTVAR`) instead of the fixed 1.2× factor.

### 4.4 Permanent Criterion benchmarks as part of CI (optional but recommended)
Periodic runs (e.g., weekly, or on any PR touching `gtp-cc`/`gtp-wire`/`gtp-crypto`) with results archived to automatically detect any future performance regression.

---

## Phase 5 — project maturity and long-term scalability

This phase serves the "maintainability and scalability" you specifically requested and touches no operational code at all:

| Task | The goal |
|---|---|
| A `CHANGELOG.md` in Keep a Changelog format | Track every behavioral change since 0.1.0 |
| Actual Semantic Versioning adoption | Bump to 0.2.0 after Phase 1 (a backward-incompatible change in the encryption format), documenting the "breaking change" |
| Future `PacketProtector` extension | Design the enum in 1.2 so a third variant (e.g., AES-128-GCM for AES-NI-equipped servers) is easy to add without breaking the public interface — the same extensible-enum pattern: add a variant only |
| Architecture Decision Records (ADR) for every major change | Update `GTP_Architecture_Decision_Paper` with Phase 1 decisions (why ChaCha20-Poly1305 rather than AES-GCM by default, why enum rather than dyn) |
| Rewrite the self-compliance report | Correct the items that were inaccurately "100% Pass" (specifically the cryptography item) after the actual fix completes, tying them to real fuzzing/benchmark results instead of self-assessment |

---

## Summary table: priority × impact × risk

| Phase | Priority | Performance impact | Stability impact | Implementation risk | Depends on |
|---|---|---|---|---|---|
| 0. Baseline | Mandatory first | Zero | Protects against later regression | Low | Nothing |
| 1. Crypto + key derivation | **Critical** | Near zero (a one-time handshake) | Removes a serious security hole | Medium (a backward-incompatible change) | Phase 0 |
| 2. Decode hardening | **Critical** | Near zero | Prevents DoS via panic | Low-medium (39 edit sites) | Phase 0, independent of 1 (can parallelize) |
| 3. Fuzzing/tests | High | Zero (build-time) | Exposes hidden defects | Low | Full value after 1+2 |
| 4. Performance enhancements | Medium (optional) | Tangible improvement (LTO especially) | Neutral to positive | Low | After 1, 2, 3 |
| 5. Maturity and docs | Low (no rush) | Zero | Improves long-term maintenance | Zero | Continuous, in parallel |

---

## The integration principle between phases (to guarantee architectural coherence)

1. **Phases 1 and 2 are technically independent** (they touch different crates: `gtp-crypto` versus `gtp-wire`) and can be executed in parallel by two different developers without conflict, since `gtp-core` is the only shared merge point and its changes in each phase are isolated (replacing the `protector` type versus handling decode errors).
2. **No phase merges into the main branch before passing the full CI** (Phase 0) — this guarantees each subsequent phase builds on an actually tested foundation.
3. **Preserving the public trait signatures** (`PacketProtector`, `CongestionController`) throughout all phases means no externally consuming code (via the unified `gtp` crate) needs changes except at the connection-creation point (due to the new handshake in Phase 1) — the only change expected to break compatibility with the current version, clearly documented in the `CHANGELOG` as a breaking change for the 0.2.0 release.
4. **The recommended actual execution order:**
   Phase 0 → (Phase 1 and Phase 2 in parallel) → Phase 3 (built on 1+2) → Phase 4 → Phase 5 (continuous throughout).

This order guarantees that the two most dangerous gaps (the fake encryption and the exploitable panic) are addressed first via the fastest possible path, while fully preserving the inter-crate dependency structure, and without any unnecessary refactoring of the parts the inspections already proved are correctly designed (CUBIC, the replay window, the general trait design).
