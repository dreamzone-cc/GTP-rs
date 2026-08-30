# Developer Technical Paper: The Remaining Fixes and Updates — GTP-rs (after commit 705f985)

**Opening note:** the fundamental security gap we have been tracking across several previous reviews (the fixed master secret, then the unwired handshake) **is genuinely closed this time** — I verified the code line by line: `connections.insert()` now happens only after the stateless-cookie verification succeeds, `connect()` returns an explicit `Result` instead of silently falling back to a fixed secret, and routing keys off `ConnectionId` rather than the network address. This work is good and does not need reopening.

What remains now is a set of **medium-to-low priority improvements** — there is currently no open critical vulnerability in the core connection path. Each item is independent, in the format: **Current state / Why it deserves fixing / Proposed implementation / Performance & stability impact**.

---

## 1. Enabling `ratchet_key` (still isolated code)

**Current state:** `CHANGELOG.md` lists it under "Added" as if it were an active feature, but `grep -rn "ratchet_key"` shows it is **not invoked anywhere in `gtp-core` or `gtp-runtime-tokio`** — only re-exported from `gtp-crypto`/`gtp`. The function itself is correct and unit-tested.

**Why it deserves fixing:** without enabling it, any session running past a few hours continues on the same connection-start key forever — no immediate danger, but it reduces forward secrecy for long streaming sessions or very long games (MMOs, for example).

**Proposed implementation:**
1. Add a `packets_since_rotation: u32` field to `ConnectionHot` (or use the current `PacketNumber` directly via `& ROTATION_MASK`).
2. Add a single "key phase" bit in the `PacketHeader` (`gtp-wire/src/header.rs`) — when the threshold is exceeded (proposed: 2^24 packets or one hour, whichever comes first), flip the bit and call `ratchet_key(current_key, cid)` to build a new `Protector::Aead`.
3. On the receiving side: upon seeing a bit different from the locally stored one, compute `ratchet_key` locally as well **before** attempting `open()` — if the first attempt fails, automatically try the rotated key (a grace window for late packets under the old key).

**Impact:** one HKDF operation every few hours/millions of packets — entirely negligible against the session volume. Zero impact on normal latency.

---

## 2. The `client_proof` in `HandshakeFinish` is not enabled — there is no actual key-confirmation possession proof

**A new discovery not previously mentioned.** I verified `endpoint.rs:172`:
```rust
let fin_frame = Frame::HandshakeFinish {
    cookie_echo: stateless_cookie,
    client_proof: [0u8; 32],   // ⚠️ always zero, never computed, never verified
};
```
The field exists in the wire format (`gtp-wire/src/frame.rs`) but is not actually used — meaning the only "proof of possession" authentication today is the cookie (which proves IP address ownership), and there is no proof that both parties actually derived the **same** session key before real encrypted data transmission begins.

**Why it deserves fixing:** not a directly exploitable vulnerability (without the private key an attacker cannot compute the same secret anyway), but it means any simple mismatch (a wrong protocol-version handshake, a bug in nonce ordering, etc.) is discovered only when the first real data packet fails to decrypt — instead of being caught explicitly during the handshake itself with a clear error message.

**Proposed implementation:**
```rust
// The client computes:
let client_proof = hmac_sha256(derived_key, b"gtp-handshake-finish" || client_public_key || server_public_key);
```
and the server verifies the same value after deriving its own key — using the `hmac` crate (lightweight, pure Rust). This happens once inside the handshake, **zero impact on the subsequent data path**.

**Priority:** medium (a diagnostic improvement / defense in depth, not the closure of an exploitable hole).

---

## 3. No real (coverage-guided) `cargo-fuzz` yet

**Current state:** the existing test (`test_fuzz_mutated_buffers_resilience`) is a simple LCG loop with buffer sizes capped at 64 bytes. There is no `fuzz/` directory in the project.

**Proposed implementation:**
```bash
cargo install cargo-fuzz && cargo fuzz init
```
At least two targets:
- `fuzz_targets/decode_frame.rs` → feeds `Frame::decode` sizes up to the full MTU (1200+ bytes), not just 64.
- `fuzz_targets/handshake_frames.rs` → feeds specifically `ClientHello`/`ServerHello`/`HandshakeFinish`, since that is the newest, most security-sensitive path right now.

Run as a separate CI task (time-boxed, e.g., 15 minutes per push to the main branch, or nightly); **not included in `cargo build --release`**, hence no production impact.

---

## 4. No real standard benchmarks (`criterion`)

**Current state:** all the figures published in internal performance reports (throughput, seal/open time) are manual measurements via `Instant::now()` inside the `gtp-cli StressSuite` — useful, but not standard benchmarks comparable over time or capable of automatic regression detection.

**Proposed implementation:** add `criterion` as a dev-dependency in `gtp-cc`, `gtp-wire`, `gtp-crypto` with measurement targets for: `Frame::encode`/`decode`, `GtpAeadProtector::seal`/`open`, CUBIC convergence across simulation iterations. Periodic CI runs (weekly, or on PRs touching these crates) with historical archiving of results to automatically detect any performance regression.

**Impact:** zero on production — `cargo bench` is a completely separate target.

---

## 5. The "TCP-Friendly" region is missing from CUBIC (still pending from previous reviews)

**Current state:** `gtp-cc/src/cubic.rs` applies the basic CUBIC equation correctly (β=0.7), but without the "TCP-friendly region" or "fast convergence" from RFC 8312.

**Why it deserves fixing:** without this region, GTP flows can be unfairly aggressive toward other TCP/QUIC flows sharing the same network bottleneck (e.g., a player downloading a game update over HTTP alongside an active GTP session).

**Proposed implementation:** an extra logical branch inside `update_w_cubic` that computes `W_tcp` (the approximate equivalent TCP window) and uses the larger of `W_cubic` and `W_tcp` under competition — it changes neither the function's external signature nor adds meaningful computational complexity (just a few additional arithmetic operations).

**Priority:** low (a fairness improvement, not a functional defect).

---

## 6. GSO/GRO batching absent (an optional performance improvement for high-load servers)

**Current state:** `gtp-io/src/udp.rs` uses `socket2` for basic configuration only, without `UDP_SEGMENT`/`UDP_GRO`.

**Proposed implementation:** when sending large packet batches to multiple connections in the same tick (e.g., broadcasting several entity states for a server with thousands of players), use batched sending through a single syscall instead of one syscall per packet — it reduces system overhead only under high load. There is no meaningful benefit for small/medium-load sessions, so this is a **deferred improvement** until a real need is shown by real-load benchmarks (item 4).

**Priority:** low, conditional on future benchmark results showing that syscall overhead is actually a bottleneck.

---

## 7. An end-to-end test for an active (not just passive) MITM scenario

**Current state:** the existing security tests (`crypto_security_test.rs`) cover the passive observer scenario well (it cannot decrypt without the private key). But there is no dedicated test for the **active attacker** scenario that intercepts a `ClientHello` and substitutes its own public key (the classic MITM on unauthenticated Diffie-Hellman).

**Why it deserves fixing:** X25519 alone (without signatures/identity authentication) is theoretically always vulnerable to this attack — the risk here is bounded because source-IP spoofing also requires later passing the cookie check in `HandshakeFinish` (a genuine additional difficulty), but it deserves documentation as a known design limit via an explicit test proving when the system fails and when it holds under this attack type, instead of leaving it undocumented.

**Proposed implementation:** an integration test that inserts a fake " intermediary server" between the client and the real server, intercepting `ClientHello`/`ServerHello` and substituting keys, verifying: (a) if it cannot pass `HandshakeFinish` with a correct cookie matching the real client's address, the connection fails. (b) Documenting the security assumption clearly in code comments: "the GTP handshake assumes a partially IP-spoofing-resistant environment via the cookie, not full identity authentication as in mTLS."

---

## 8. Correcting the accuracy of `CHANGELOG.md` regarding `ratchet_key`

**Current state:** it describes `ratchet_key` under "Added" with the phrasing "for forward-secure long-lived connections", suggesting it is actually enabled, whereas (as in item 1) it is library code not yet wired.

**Proposed implementation:** change the line to: "Session Key Ratchet (library primitive, not yet wired into connection lifecycle — see issue #N)" until item 1 is done, then move it to a new release section describing it as actually enabled.

---

## Priority table

| # | Item | Priority | Note |
|---|---|---|---|
| 1 | Enable `ratchet_key` in the connection lifecycle | 🟡 medium | Forward secrecy for long sessions only |
| 2 | `client_proof` (HMAC key confirmation) | 🟡 medium | Better diagnostics, not an exploitable hole |
| 3 | Real `cargo-fuzz` | 🟡 medium-high | Specifically targets the new handshake frames |
| 4 | `criterion` benchmarks | 🟡 medium | Necessary for credible performance documentation instead of manual figures |
| 5 | CUBIC TCP-friendly region | 🟢 low | Network coexistence fairness |
| 6 | GSO/GRO batching | 🟢 low | Deferred until benchmarks show the need |
| 7 | Active-MITM test + documenting the security assumption | 🟢 low-medium | Documenting design limits, not fixing a hole |
| 8 | Correct the CHANGELOG regarding ratchet_key | 🟡 medium (credibility) | Immediate and independent |

**There is no "critical" (🔴) item in this list** — which itself reflects that the foundational work (real security for the connection path) is complete. The recommended practical priority: (3 + 8) immediately since both are independent and quick to implement → (1 + 2) together since they serve the same handshake path → (4) to formally document actual performance → (5 + 6 + 7) later per operational priority.
