# Technical File: The Fixes and Updates That Must Be Implemented — GTP-rs (after commit b0beb91)

**The root diagnosis of everything below:** the last commit added correct cryptography components (X25519, HKDF, ratcheting, zeroize, stateless cookies) but they are **written as isolated modules not wired into the actual connection path**. Every item below is a missing "connecting wire" between an existing, correct component and the live operational path. No algorithm needs rewriting — the need is only to call what exists from the right place.

---

## 1. Wiring the real handshake sequence into the `GtpEndpoint::connect` path

**The primary affected file:** `crates/gtp-runtime-tokio/src/endpoint.rs`

**Current state:** `connect()` creates a `GtpConnection` directly in the `Established` state with a fixed secret, without sending any packet over the network. `connect_with_session_keys()` exists but has no callers.

**What must be implemented in detail:**

### 1.1 The client side
Inside `connect()` (or a new function replacing it with the same external signature so no consuming code breaks):
1. Generate `EphemeralKeyPair::generate()`.
2. Build `Frame::ClientHello { client_public_key, client_nonce, version: 1 }` and actually send it via `self.socket.send_to(...)` to `peer_addr`.
3. Await (`tokio::time::timeout`, 3 seconds suggested) the arrival of a `Frame::ServerHello` carrying the same context (via a temporary internal channel dedicated to the handshake, separate from the normal message channel in `start_rx_loop`).
4. On receipt: `client_pair.compute_shared_secret(&server_public_key)` then `derive_handshake_session_keys(...)` — **this call already exists and is tested; only the caller here is missing**.
5. (Optional but recommended, to close B.2 of the previous paper) send `Frame::HandshakeFinish { cookie_echo, client_proof }` to prove completion of the round before considering the connection `Established`.
6. Call `GtpConnection::new_with_session_keys(cid, peer_addr, key, iv, pre_validated: false, ...)` — **it already exists**; it is simply now called with real keys from the network instead of keys manually passed by the caller.

### 1.2 The server side
Inside `start_rx_loop()`, before the current `connections` lookup loop, an explicit handling branch must be added:
1. If a packet arrives with frame type `ClientHello` and a `connection_id` not present in `connections`:
   - Generate a server-specific `EphemeralKeyPair::generate()`.
   - Generate a `stateless_cookie` via `StatelessTokenManager::generate_cookie(src_addr, now)` — **the object already exists and is unused anywhere live; it only needs initialization (`StatelessTokenManager::new(server_secret)`) once at `GtpEndpoint::bind`, stored as a field on `GtpEndpoint`**.
   - Send `Frame::ServerHello { server_public_key, server_nonce, stateless_cookie, assigned_cid }` back to `src_addr` — **without creating any connection state yet** (this protects against memory exhaustion, see item 4 below).
2. Upon later receiving a `Frame::HandshakeFinish` with a correct `cookie_echo` (`StatelessTokenManager::verify_cookie`; I verified it genuinely uses `subtle::ConstantTimeEq` — good as is):
   - **Only now** is an actual `ConnectionHot` created via `new_with_session_keys(..., pre_validated: true)` — because the cookie proof proves address ownership.
   - If no `HandshakeFinish` arrives within a short timeout, the handshake attempt is silently discarded (no state was stored to begin with, so nothing to clean up).

**Acceptance criteria:** a real integration test (not merely unit-level inside `gtp-crypto`) in `crates/gtp-runtime-tokio/tests/` that runs an actual server and client on `127.0.0.1` over real UDP, verifying: (a) the connection succeeds and the derived keys match on both sides without any manual passing, (b) a third party intercepting only the `ClientHello`/`ServerHello` (without its private key) cannot decrypt any subsequent packet.

---

## 2. Converting the old `.connect()` at every live call site to the new secure path

After item 1 completes, `connect()` itself automatically becomes secure (it is the function rebuilt from the inside) — **none of the following six call sites need modification**, because the external signature stays as is:
- `crates/gtp-cli/src/main.rs:265, 315, 484, 867, 921, 940`

This is architecturally important: **fixing item 1 alone suffices to automatically cover all current and future tests without touching them**, provided the signature `connect(cid, peer_addr, secure: bool) -> AsyncGtpConnection` stays unchanged.

**A mandatory accompanying cleanup step:** convert `ConnectionHot::new_with_master_secret` and `ConnectionHot::new` (in their current forms using the fixed secret) to:
```rust
#[deprecated(note = "Uses a hardcoded shared secret; use the handshake-driven \
    GtpEndpoint::connect which derives real per-session keys via X25519. \
    Only safe for offline gtp-sim testing with secure=false.")]
```
This **does not delete** the code (it remains useful for `gtp-sim`, where a real network handshake makes no sense inside a deterministic single-process simulation), but it prevents falling into the same mistake in the future — any new usage will produce an explicit `deprecated` warning during `cargo build`.

---

## 3. Enabling conditional `mark_validated()` on all paths (not only the new one)

After item 1, the `new_with_master_secret` path becomes confined to `gtp-sim` only (a closed simulation environment where anti-amplification protection serves no purpose anyway). **However**, it must be ensured that any remaining usage of it (even inside `gtp-sim`) does not invoke an immediate `mark_validated()`, if the same simulator is later to be used for testing amplification-attack scenarios themselves (see item 6 below, a new required test).

**The change:** in `crates/gtp-core/src/state.rs`, inside `new_with_master_secret`:
```rust
// Before:
let mut anti_amp = AntiAmplificationLimiter::new();
if secure { anti_amp.mark_validated(); }

// After:
let anti_amp = AntiAmplificationLimiter::new(); // always remains unvalidated by default
```
and remove any automatic `mark_validated()`-on-creation call from this specific function (the new secure path from item 1 invokes it in the right place only: either immediately upon cookie verification, or immediately upon the first successful packet decryption, as already present at `connection.rs:246`).

---

## 4. A cap and expiry for "mid-handshake" connections (prevents memory exhaustion after item 1)

**Why this item appears now specifically:** once the server starts actually processing `ClientHello` (item 1.2), it becomes theoretically exposed to flooding with fake `ClientHello` messages at a high rate. The design proposed in 1.2 partially avoids this (no full connection state is stored before `HandshakeFinish`), but the following must also be defined:
- A maximum on `ServerHello` replies sent per IP address within a short window (e.g., 10 replies/second per IP) to prevent the server being used as a reflection amplifier even before the cookie stage.

**The affected file:** `gtp-runtime-tokio/src/endpoint.rs` (a simple counter with `FxHashMap<IpAddr, (u32, MonotonicTime)>` on `GtpEndpoint`, periodically reset).

**Performance impact:** negligible — this check happens only on `ClientHello` receipt (a relatively rare event, once per new connection), never touching the normal packet path after the connection exists.

---

## 5. Actually enabling `ratchet_key` (currently an isolated function, just as `derive_handshake_session_keys` used to be)

**Current state:** `ratchet_key` exists and is tested, with zero callers outside its own scope.

**What is required:**
1. Add a `packets_since_rotation: u32` field to `ConnectionHot` (or use `next_packet_number` itself as the indicator via `& KEY_ROTATION_MASK`).
2. In the send path (`connection.rs`, at the `protector.seal` call site), when a threshold is exceeded (proposed: every 2^24 packets as a conservative start, or every hour of connection time, whichever comes first): call `ratchet_key(current_key, cid)`, build a new `Protector::Aead` with the rotated key, and replace `self.hot.protector`.
3. **Critical:** the peer must know about the key change — add a "key phase" bit in the packet header (`gtp-wire/src/header.rs`) flipped on every rotation; upon receiving a packet with a bit different from the locally stored one, compute `ratchet_key` locally as well before attempting `open()`.

**Performance impact:** the rotation itself (one HKDF) is rare (every few hours/millions of packets); its cost is entirely negligible against the session volume.

**Priority note:** this item is less urgent than 1-4 (it poses no immediate security hole, only a forward-secrecy improvement for very long sessions) — it can be deferred until after the critical items close.

---

## 6. Fixing the NAT-rebinding stage in `StressSuite` so it actually verifies instead of printing a constant

**The affected file:** `crates/gtp-cli/src/main.rs`, the handler for `mode == "nat-rebind"`.

**The problem:** the line `println!("Path Challenge/Response: Dispatched & Validated")` prints unconditionally, while there is no actual invocation of `PathValidator::start_challenge`/`validate_response` in this path.

**What is required:** after reconnecting from the new address, the server must actually call `path_validator.start_challenge(new_addr, nonce, now)`, send a real `PathChallenge` frame (this must be checked: is this frame type even defined in `frame.rs`? If not, it is added in the same pattern as `ClientHello`/`ServerHello`), await the `PathResponse` from the client, then print the result **based on the actual return value of `validate_response()`**, not a constant string.

**Priority:** medium (a test-accuracy correction, not a security hole per se, but it gives the false impression that `PathValidator` is actually tested over a real network while it is unwired).

---

## 7. Correcting `CHANGELOG.md` to reflect the real state until item 1 completes

**The problem:** it currently describes X25519/Anti-Amplification/Ratcheting under "Added" in phrasing suggesting they are enabled in the live path, while they are currently an unwired library surface.

**What is required temporarily (until item 1 lands):** edit the `## [0.2.0]` section wording to distinguish explicitly between:
- **"Available (library-level, not yet wired into live handshake)"** for each of: the X25519 exchange, key ratcheting, stateless-cookie generation.
- **"Fixed (fully wired)"** for each of: ChaCha20-Poly1305 AEAD itself (this is actually enabled, as I previously verified), decode-path hardening, the static-dispatch enum, cargo-audit in CI.

After item 1 lands, these items officially move to a new `## [0.2.1] - Handshake Integration` section instead of remaining retroactively described as complete in 0.2.0.

---

## The priority and dependency table

| # | Item | Priority | Blocks what | Depends on |
|---|---|---|---|---|
| 1 | Wire the real handshake (client+server) | 🔴 critical, the foundation for everything after | Everything, security-wise | Nothing (the tools are ready) |
| 2 | `#[deprecated]` on the old path | 🔴 critical (prevents regression) | — | Item 1 |
| 3 | Remove the immediate `mark_validated()` on the old path | 🔴 critical | — | Independent, immediate |
| 4 | The memory-exhaustion cap for half-open connections | 🟡 high | — | Item 1 |
| 5 | Enable `ratchet_key` in the send path | 🟢 medium | Long-term forward secrecy only | Item 1 |
| 6 | Enable real NAT verification in the test | 🟢 medium | Test-result accuracy only | Nothing |
| 7 | Correct the CHANGELOG temporarily | 🟡 high (credibility) | — | Independent, immediate |

**The recommended practical order:** (3 + 7) immediately, both fully independent → 1 (the foundation) → 2 + 4 together as soon as 1 completes → 5 and 6 optional later. **Item 1 is the only one that transforms the system from "correct but unused cryptography tools" into "an actually secure protocol on the network" — everything else in this file is either a preparation for it or a follow-up cleanup.**
