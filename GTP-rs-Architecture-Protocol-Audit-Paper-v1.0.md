# Technical Paper: Comprehensive GTP-rs Architecture and Protocol Audit

| Field | Value |
| :--- | :--- |
| **Document title** | Comprehensive audit of the architecture, protocol structure, algorithms, and cross-layer functional integration of GTP-rs |
| **Project** | GTP-rs — a Rust implementation of the Game Transport Protocol (GTP/1.1) |
| **Repository** | `github.com/dreamzone-cc/GTP-rs` (local copy: `/home/ggonlinux/GTP`) |
| **Audited revision** | branch `main`, commit `e1c9ef7` ("feat(security,cc): implement HMAC key confirmation, live key ratchet, CUBIC RFC 8312 ...") |
| **Document version** | v1.0 — 2026-08-30 |
| **Audit scope** | All 13 crates + tests + fuzz targets + benchmarks + specification documents |
| **Methodology** | Full source read-through; conformance checking against RFC 9000/9002/8312/8439 and RFC 1982 plus the GTP/1.1 spec; `cargo test --workspace` execution; line-by-line RX/TX path tracing; cross-layer seam analysis |

---

## Table of Contents

1. [Executive Summary and Final Verdict](#1-executive-summary-and-final-verdict)
2. [Audit Methodology and Scope](#2-audit-methodology-and-scope)
3. [Overall Architecture and Protocol Structure](#3-overall-architecture-and-protocol-structure)
4. [Functional Integration and Interlocking Verification Results](#4-functional-integration-and-interlocking-verification-results)
5. [Security Audit — Crypto and Path Layers](#5-security-audit--crypto-and-path-layers)
6. [Loss Recovery and Congestion Control Algorithm Audit](#6-loss-recovery-and-congestion-control-algorithm-audit)
7. [Scheduling Algorithm and Delivery Semantics Audit](#7-scheduling-algorithm-and-delivery-semantics-audit)
8. [Wire Format Layer Audit](#8-wire-format-layer-audit)
9. [Core Engine and Runtime Layer Audit](#9-core-engine-and-runtime-layer-audit)
10. [Performance and Efficiency Analysis](#10-performance-and-efficiency-analysis)
11. [Stability Analysis](#11-stability-analysis)
12. [Unified Findings Registry](#12-unified-findings-registry)
13. [Prioritized Remediation Plan](#13-prioritized-remediation-plan)
14. [Appendices](#14-appendices)

---

## 1. Executive Summary and Final Verdict

### 1.1 Overall verdict

The codebase is **not ready for production use** in its current state, despite the fact that:

- The layered architecture is **cleanly designed and easy to fix** (13 crates with clear responsibilities).
- The data path is **entirely free of panics** (`panic`/`unwrap`) outside tests — verified by direct inspection.
- All existing tests pass (**48 passed, 0 failed, 1 ignored**).
- The `gtp-wire` crate is genuinely zero-allocation and depends on no `alloc` at all.

The essential blocker: **the critical defects concentrate at the seams between layers** — precisely the places the current test suite does not cover. Every layer looks sound and tested in isolation, but the hand-offs of control and data between layers break fundamental guarantees.

### 1.2 The five most dangerous findings (verified directly in the code)

| # | Defect | Severity | Impact |
| :--- | :--- | :--- | :--- |
| 1 | **Cross-direction nonce reuse**: one key/IV pair for both peers + a nonce derived only from the CID and the low bits of the packet number → the first packet in each direction shares the same key and nonce (an RFC 8439 violation) | Critical | Confidentiality break and potential forgery; the existing test at `handshake.rs:155-163` asserts key equality — i.e., it tests the dangerous state as if it were correct |
| 2 | **Control frames double-wrapped**: `Ping`/`PathChallenge`/`Close`/`AckFrequency`/`MtuProbe` are encoded, then wrapped inside `Frame::Data` → the peer never executes their handlers | Critical | **Path migration, NAT rebinding, graceful close, and Ping are dead features end-to-end**, although each is unit-tested in isolation |
| 3 | **Replay window updated before AEAD authentication**: a single forged datagram with a huge packet number permanently burns the number space | Critical | Permanent connection DoS with one packet, for anyone who knows the CID |
| 4 | **Forgeable handshake cookie**: a simple XOR construction instead of HMAC, with an unauthenticated, plaintext timestamp | Critical | 24 bytes of the secret recoverable from one cookie; lifetime bypass to infinity |
| 5 | **Phantom in-flight leak + retransmission storms**: ACK-only bytes are charged and never acknowledged; `on_timeout` re-enqueues without removing records and with no exponential PTO backoff | High | Progressively throttles sending until stall; duplicate application delivery in `ReliableUnordered` |

### 1.3 Functional integration verdict

The current tests cover **only the happy paths** (a clean datagram with no loss, a live handshake with one message, crypto algebra, 20% loss for ordered messages in the simulator). **There is no test at all** for any of the following seams — which are exactly where the real defects live:

- The PTO path when all ACKs are lost
- Path migration through the protocol (not by calling `PathValidator` directly)
- Receiving control frames and reacting to them end-to-end
- Triggering the key ratchet mid-session (it would fail — the peer never rotates)
- Corrupted/replayed/wrong-CID datagrams through the full connection
- Payloads larger than MTU (which would expose the permanent tier-head blockage)

---

## 2. Audit Methodology and Scope

### 2.1 What was audited

- **Full read** of the source files in: `gtp-types`, `gtp-wire`, `gtp-recovery`, `gtp-cc`, `gtp-scheduler`, `gtp-path`, `gtp-crypto`, `gtp-core`, `gtp-io`, `gtp-runtime-tokio`, `gtp-sim`, `gtp`, `gtp-cli`
- **Tests**: embedded unit tests (`#[cfg(test)]`), integration tests in `crates/*/tests/*.rs`, the fuzz targets in `fuzz/fuzz_targets/`, and the benchmarks
- **Standards conformance**: RFC 8439 (AEAD), RFC 8312 (CUBIC), RFC 9002 (loss recovery), RFC 9000 (related QUIC concepts), RFC 1982 (serial arithmetic), the internal `GTP_1_1_Comprehensive_Technical_Specification.md` spec, and `docs/specs/GTP-SEC-01.md` / `GTP-RUST-01.md`
- **Empirical verification**: `cargo test --workspace` on Rust toolchain 1.85.0 (matching `rust-toolchain.toml`)

### 2.2 Severity classification

| Level | Definition |
| :--- | :--- |
| **Critical** | Remotely exploitable, or breaks a fundamental security/functional guarantee, or fully disables an advertised feature |
| **High** | Data corruption, escalating performance degradation, a binding RFC violation, or conditional DoS |
| **Medium** | Incorrect edge-case behavior, dead code behind advertised guarantees, spec deviation |
| **Low** | Code quality, documentation inconsistencies, improvements |

> **Verification note**: the `file:line` locations in this paper were verified against the working tree at commit `e1c9ef7`. The four critical items in §1.2 were additionally reviewed line-by-line by hand, on top of the automated analysis reports.

---

## 3. Overall Architecture and Protocol Structure

### 3.1 Crate map (13 crates)

```
┌─────────────────────────────────────────────────────────────────┐
│  gtp (unified SDK facade)   gtp-cli (command-line tools)        │
│  gtp-runtime-tokio (async Tokio endpoint)                       │
├─────────────────────────────────────────────────────────────────┤
│  gtp-core ──── GtpConnection engine: RX/TX pipelines + Control  │
│      ├── gtp-scheduler ── 5-tier DRR + StateTable              │
│      ├── gtp-recovery ──── RTT/ACK/loss detection (RFC 9002)   │
│      ├── gtp-cc ─────────── CUBIC (RFC 8312) + pacing + backp. │
│      ├── gtp-crypto ─────── AEAD + HKDF + X25519 + replay wnd. │
│      ├── gtp-path ──────── state machine + anti-amp + cookies  │
│      ├── gtp-io ────────── UDP abstraction (⚠ dead code — §9.5)│
│      └── gtp-sim ───────── deterministic virtual-time simulator │
├─────────────────────────────────────────────────────────────────┤
│  gtp-wire ─── long/short headers + 14 TLV frames + builder     │
│  gtp-types ── identifiers + monotonic time + RFC 1982 + errors │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 Wire format as implemented

**Short header — 24 bytes** (`gtp-wire/src/header.rs:4`), all fields fixed-width big-endian:

| Offset | Size | Field |
| :--- | :--- | :--- |
| 0 | 1 | flags (bit7: long header, bit6: KEY_PHASE, bit5: ACK_PRESENT, bits3-4: ECN) |
| 1 | 1 | header_len (u8) |
| 2 | 8 | connection_id (u64 BE) |
| 10 | 8 | packet_number (u64 BE — **full, no varint or truncation**) |
| 18 | 4 | timestamp_micros (u32 BE) |
| 22 | 2 | payload_len (u16 BE) |

**Long header — 28 bytes** (`header.rs:5`): same fields + a 4-byte `version` at offset 1.

**Core format notes**:

- Reserved flag bits 0-2 are **never validated as zero on decode** — no forward-compatibility gate.
- The `payload_len` and `header_len` fields have **contradictory dual semantics**: `PacketBuilder::finish` writes `payload_len` as the plaintext frame byte count, then `gtp-core` **overwrites bytes 22-24 of the sealed datagram** by hand with `plaintext_len + AEAD_TAG_LEN` (`gtp-core/src/connection.rs:651-656`) so the AAD matches; nobody reads it on receive.
- The `VarInt` unit (RFC 9000 2-bit prefix) is **implemented correctly but is dead code** — no field uses it, while the README advertises it as a feature. The fixed layout means ~5.6% overhead on a 300-byte gameplay datagram.
- The `version` field is **never validated on any receive path**; worse, `gtp-runtime-tokio/src/endpoint.rs:113` sends version `1` while the constant `GTP_V1_1 = 0x00010001` (`header.rs:3`) — the two in-repo implementations disagree and nobody notices.

**The fourteen TLV frames** (`gtp-wire/src/frame.rs:8-23`): types `0x00..=0x0D` — Padding, Ack, Data, ReliableData, Retx, Ping, PathChallenge, PathResponse, MtuProbe, Close, AckFrequency, ClientHello, ServerHello, HandshakeFinish. All fourteen `encode` branches were verified internally consistent (the `needed` precomputation versus the bytes actually written) — **no encode-side buffer overrun exists** apart from the two critical overflow cases, W-1 below.

### 3.3 Receive (RX) path — actual order in the code

`GtpConnection::handle_incoming_datagram` — `gtp-core/src/connection.rs:212-471`:

```
1. Cold counters + anti-amplification credit       (218-222)  ← before any validation ⚠
2. Header decode PacketHeader::decode               (225)
3. CID match check                                  (228-230)
4. Replay window check_and_update                   (232-235)  ← before authentication ⚠ CRITICAL
5. AEAD decrypt + authenticate (open)               (238-256)
6. Frame dispatch loop:                             (263-449)
     Ack → loss_detector + cc + retx re-enqueue     (283-327)
     Data/ReliableData/Retx → delivery              (330-385)
     PathChallenge → automatic echo                 (389-406)
     PathResponse → path migration                  (408-421)
     AckFrequency → GtpConfig only (dead)           (423-432)
     Close → state transition + return Err          (434-445)
7. ACK tracker update                                (452-457)
8. Backpressure level-change event                   (460-468)
```

**Deviations from the correct order**:

- Step 4 before step 5: the **SEC-3 critical defect** (§5) — burning packet-number space before proving authenticity.
- **No path-state gate at all**: `src_addr` is never compared against `active_path`, and there is no `ConnectionState` gate — a Closed/Draining connection still fully processes data frames.
- The credit in step 1 is counted **before header decode, authentication, and the CID check** — junk datagrams inflate the 3x send budget.
- A datagram with a corrupt frame (the error at 266-269 breaks the loop) **is still acknowledged as received** in step 7.
- `Close` interrupts bookkeeping: the `return Err` at 444 skips step 7 — the peer's Close packet is never acknowledged.

### 3.4 Transmit (TX) path — actual order in the code

`GtpConnection::produce_outgoing_datagram` — `gtp-core/src/connection.rs:477-698`:

```
1. State gate is_active()                           (482-486)
2. PTO check + retransmission injection              (489-528)
3. Stale-item pruning prune_stale                    (531)
4. Pacing token update + send_budget = min(cwnd−inflight, tokens)  (535-541)
5. should_ack / has_data gates                       (543-551)
6. Header + PacketBuilder                            (554-558)
7. ACK frame attach                                  (564-567)
8. Scheduler pop + encode within budget              (571-647)  ← let _ = append_frame ⚠
9. finish() + patch payload_len to sealed length     (650-656)
10. AEAD seal                                        (660-666)
11. Anti-amplification check                         (671-676)  ← after the pop ⚠
12. In-flight/CC/pacing/packet-number bookkeeping    (679-695)
```

**Retransmission injection points**: two — ACK-driven loss (RX at 302-327) and the PTO sweep (500-525); both re-enqueue at P3 priority.

**Critical deviations**:

- Step 8: `let _ = builder.append_frame(&frame)` at `585/599/611/633` — a frame that does not fit is silently dropped while still being recorded as in-flight for reliable classes (`612-619, 634-641`) — phantom bytes that skew CC and loss detection (defect D-2).
- Step 11 after the pop: a `can_send` rejection drops items already popped from the scheduler — **silent loss of reliable messages**.
- **No fragmentation**: `fragment_id=0` and `total_fragments=1` always (`605-606, 627-628`) — a payload larger than ~1400 bytes is pushed back to its tier head and blocks it **forever** (head-of-line blockage inside the tier).

### 3.5 Task and timer structure of the runtime layer

- One RX task per endpoint (`endpoint.rs:245-458`) processes **all** connections serially and forwards messages to application channels with `.await` — one slow consumer (a full 1024-slot channel) freezes the whole endpoint.
- One TX task per connection ticking every 500µs (`endpoint.rs:464`) — N connections = N tasks waking 2000 times/second even when idle.
- CID routing table: `Arc<RwLock<FxHashMap<ConnectionId, (Arc<Mutex<GtpConnection>>, Sender)>>>` — **never evicted on close**; a duplicate CID silently replaces a live connection's entry.
- No shutdown signal or graceful endpoint close exists; the RX loop **dies silently on the first socket error** (`endpoint.rs:247`).

---

## 4. Functional Integration and Interlocking Verification Results

### 4.1 Test run results

```
$ cargo test --workspace    (rustc 1.85.0)
Total: 48 passed / 0 failed / 1 ignored
```

| Test file | What it actually covers |
| :--- | :--- |
| `crates/gtp/tests/integration_test.rs` | The four send classes → one clean datagram → delivery. Seam: scheduler→wire→crypto→decrypt→deliver |
| `crates/gtp-runtime-tokio/tests/handshake_e2e_test.rs` | Live X25519 handshake over UDP (two tests) + 10 concurrent clients with one message each |
| `crates/gtp-crypto/tests/crypto_security_test.rs` | HKDF per-CID isolation, eavesdropper rejection, ratchet distinctness, tampered-proof rejection |
| `crates/gtp-wire/tests/malformed_inputs_test.rs` | Truncations + 1000 pseudo-random buffers (≤64 bytes only) |
| `crates/gtp-sim/src/sim_runner.rs:109-173` | **The only** test of the scheduler→recovery retransmission loop: 20% loss for ordered messages (one deterministic seed) |

### 4.2 Seam matrix — tested versus untested

| Seam | Status | Note |
| :--- | :--- | :--- |
| scheduler → wire → crypto → delivery (clean path) | ✅ tested | |
| handshake crypto → CID routing → app channel | ✅ tested | One message only, no ACK/RTT |
| crypto algebra (HKDF/X25519/proof) | ✅ tested | Mostly equality/inequality; no known-answer vectors |
| wire decode robustness | ✅ partially tested | Buffers ≤64 bytes only — long headers and handshake frames never fuzzed |
| **crypto ↔ wire under corruption** (tampered/replayed/wrong-CID through the connection) | ❌ untested | Would have caught SEC-3 |
| **PTO under 100% ACK loss** | ❌ untested | Would have caught the retransmission storm |
| **Control frames end-to-end** (Ping/Close/PathChallenge/AckFrequency/MtuProbe) | ❌ untested | Would have caught Core-C1 (dead features) |
| **Path migration through the protocol** | ❌ untested | The only test calls `PathValidator` directly, bypassing the protocol |
| **Mid-session ratchet** | ❌ untested | Would have exposed that the peer cannot decrypt afterwards |
| **Anti-amplification gate through produce** | ❌ untested | Including the popped-item drop |
| **Payload > MTU / fragmentation** | ❌ untested | Would have caught the permanent tier-head blockage |
| **Draining → Closed lifecycle** | ❌ untested | No timeout exists at all |
| **gtp-io PacketIo with runtime/sim** | ❌ untested | The code is dead to begin with |
| **ACK cadence negotiation** | ❌ untested | Would have caught the no-op mechanism |

### 4.3 Integration verdict

The inter-layer machinery **works as one entity on the happy path only**. Under any deviation (loss, corruption, duplication, migration, close, large payload, key rotation) the integration breaks — and the current tests are constructed so they never pass through those states. The simulation infrastructure (`gtp-sim`) exists and is capable of most of these scenarios, but is exercised in exactly one.

---

## 5. Security Audit — Crypto and Path Layers

### 5.1 Implemented construction

- **AEAD**: ChaCha20-Poly1305 (RFC 8439) via RustCrypto `chacha20poly1305` 0.10, 256-bit keys, 96-bit nonces, 128-bit tags — correct use of the detached in-place API (no heap allocation).
- **Key derivation**: X25519 → IKM = `shared ‖ client_nonce ‖ server_nonce` (96 bytes) → HKDF-SHA256 with a fixed salt → master key → `derive_session_keys` (labels `gtp_key_`/`gtp__iv_` + CID).
- **Nonce**: `nonce = IV ⊕ (CID_be[0..8] ‖ PN_be[4..8])` — `aead.rs:21-33`.
- **AAD**: the full short header (24 bytes) — supplied by core, not enforced by gtp-crypto.
- **Replay window**: a 128-bit sliding bitmap (two u64 words) — the shift arithmetic is mathematically correct for all cases (1-63, 64, 65-127, ≥128).
- **Handshake schedule**: ClientHello(client pk+nonce) → ServerHello(server pk+nonce+cookie+CID) → HandshakeFinish(cookie+HMAC proof) — all in **plaintext** on the wire.

### 5.2 Critical findings

**SEC-1 (Critical) — immediate cross-direction nonce reuse.**
`derive_handshake_session_keys` (`handshake.rs:58-84`) returns a single (key, IV) pair **for both peers with no direction or role separation whatsoever**. Both peers start at PN=1 (`state.rs:49-133`), and `derive_nonce` is deterministic in (IV, CID, PN). The client and server share the CID — so the client's packet 1 and the server's packet 1 use **the same key and the same nonce**. Consequence: `P_client ⊕ P_server` leaks from the ciphertext XOR, and Poly1305 one-time-key forgery becomes possible. The test at `handshake.rs:155-163` declares `assert_eq!(client_key, server_key)` — the dangerous state is enshrined as "correct behavior".
**Fix**: derive two key/IV pairs with direction labels (`"gtp client tx"`/`"gtp server tx"`) and select by role, plus a test forbidding equality.

**SEC-2 (High) — the top 32 bits of the packet number are ignored in the nonce.**
`aead.rs:29-31` discards `pn_bytes[0..4]` — packet 1 and packet 2³²+1 produce the same nonce under the same key. No guard prevents reaching 2³².
**Fix**: use all 64 bits (e.g., `IV ⊕ PN_be[0..8]` in bytes 4..12) or cap the PN at 32 bits with a mandatory rekey before exhaustion.

**SEC-3 (Critical) — replay window updated before authentication.**
`connection.rs:232-235` runs before `open()` at `240`. The CID and PN are plaintext — an attacker who knows the CID sends PN=u64::MAX: the window jumps, the bitmap clears, and every subsequent legitimate packet is rejected as a replay **forever**. The internal security spec `GTP-SEC-01.md:16-19` explicitly requires the bit to be set only after successful AEAD — the code violates its own spec.
**Fix**: authenticate first, then update; or copy the window, update the copy, and commit only on success.

**SEC-4 (Critical) — the cookie is not a real MAC.**
`stateless_token.rs:24-35`: `cookie[i] = secret[i%32] ^ (addr_byte + ts_byte + i) mod 256`, then **the timestamp is written in plaintext into bytes 0..8**. Every non-secret input is known to an attacker (the timestamp is inside the cookie; the address is their own) → `secret[8..32]` (24 of 32 bytes) is recoverable from a single cookie, cookies can be forged for any address and any time, and an old cookie's lifetime can be refreshed forever. The constant-time comparison (`subtle`) exists but is pointless over a dismantlable construction. The current savior: the HandshakeFinish HMAC proof depends on X25519, which a spoofed-source attacker never sees — meaning the advertised address-ownership protection does not actually exist and the burden falls entirely on another layer.
**Fix**: `HMAC-SHA256(secret, "gtp cookie" ‖ addr ‖ ts)` + reject `ts > now + skew`.

**SEC-5 (High) — anonymous handshake with no server authentication.**
No certificate, no PSK, no static key, no server Finished proof — anonymous DH vulnerable to an active MITM who presents itself as the server. The test named `test_active_mitm_key_tamper_rejected` only exercises a tampered proof, not server impersonation. Acceptable for a first version **if documented** — but the project markets itself as "production-grade".

**SEC-6 (High) — the key ratchet is uncoordinated on the wire.**
`ratchet_key` is a one-way hash chain only: the `KEY_PHASE` header bit exists but is never set; the receive path never reads it; `key_rotation_interval_packets` is configured but unenforced; `packets_since_ratchet` is incremented and never read. After `ratchet_key()` **the peer cannot decrypt anything** (and no test passes through this path). The one-way chain also provides no post-compromise security, and old keys are never zeroized.

### 5.3 Additional security findings

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| SEC-7 | High | `Cargo.toml:50` | The `x25519-dalek` `zeroize` feature is disabled → ephemeral private keys are **never wiped from memory** |
| SEC-8 | High | `kdf.rs:32-55`, `state.rs:94-108` | The deprecated static path is deterministic (same secret+CID ⇒ same key/IV and the whole nonce sequence) and still public; the simulator uses it with a hardcoded published master secret (`sim_runner.rs:24-25`) |
| SEC-9 | Medium | `handshake.rs:104-119` | The HMAC confirmation proof reuses the AEAD traffic key itself (role mixing, no key separation) |
| SEC-10 | Medium | `aead.rs:45-47` | Integer overflow in the `payload_len + AEAD_TAG_LEN` check → slice-index panic at `payload_len = usize::MAX` (public API) |
| SEC-11 | Medium | `endpoint.rs:280,290` | ClientHello rate limiter: the comment says "20/s" while the code allows `<= 1000` — 50x the documented intent; and ClientHello has no freshness token (replay forces crypto work) |
| SEC-12 | Medium | `handshake.rs:49-54` | No `was_contributory()` check on the X25519 result (low-order/all-zero keys accepted) |
| SEC-13 | Medium | `header.rs`, `endpoint.rs:113` | The version is neither negotiated nor bound into the transcript — no downgrade protection once a second version exists |
| SEC-14 | Medium | `aead.rs:9-13`, `kdf.rs:13-27` | Derived `Debug` on key containers → keys can leak through logs/panic messages |
| SEC-15 | Medium | `connection.rs:218-222` | Anti-amplification credit counted before header decode/CID check/authentication |
| SEC-16 | Medium | `anti_amplification.rs` | No reset API — the budget is not path-scoped on migration (contrary to QUIC semantics); validation is lifted by decryption rather than address validation |
| SEC-17 | Medium | `identifiers.rs:50-52` | `PacketNumber::next` is an unchecked add — debug panic at u64::MAX, unsafe wrap in release with permanent replay rejection |
| SEC-18 | Low | `config.rs` vs `state.rs:73-75` | `replay_window_size` in the config is ignored (hardcoded 128) |
| SEC-19 | Low | `plaintext.rs:8-18` | The plaintext protector does not validate `payload_len` at all |

---

## 6. Loss Recovery and Congestion Control Algorithm Audit

### 6.1 RTT estimation (RFC 9002) — `gtp-recovery/src/rtt.rs`

**Correct**: the EWMA coefficients (rttvar 3/4+1/4, SR 7/8+1/8), the first-sample branch (SR=sample, rttvar=half), the ack_delay clamp to max_ack_delay, and saturating `Duration` arithmetic that cannot panic on zero/negative.

**Defects**:

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| REC-1 | High | `rtt.rs:36-48` | ack_delay subtracted **unconditionally** and on the first sample — an untrusted wire value drives SR/min_rtt toward zero (a feedback loop) |
| REC-2 | Medium | `rtt.rs:43-44` | min_rtt is updated from the **adjusted** sample rather than the raw one — drifts below the true path RTT and inflates time-threshold and backpressure computations |
| REC-3 | Medium | `rtt.rs:23` | The `u64::MAX µs` sentinel leaks to the game through `DetailedMetrics.min_rtt` until the first sample, keeping backpressure `Low` meanwhile |
| REC-4 | Low | `rtt.rs:68-70` | No timer-granularity floor (kGranularity) on PTO |

### 6.2 ACK tracker — `ack_tracker.rs`

**Correct**: a descending sorted interval set with adjacent merging; range encoding semantics matching QUIC; immediate ACK on gap + count-based + max_ack_delay timer; zero-allocation ACK frame encoding (stack array).

**Defects**:

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| REC-5 | High | `ack_tracker.rs:148-188` | **Intervals are never pruned** — every gap ever seen stays in the Vec for the whole session; unbounded growth + re-serialization of the oldest ranges in every ACK |
| REC-6 | High | `ack_tracker.rs:166` | The 32-range cap **silently drops acknowledgements** with no coalescing — packets received beyond the cap are never ACKed, forcing needless PTO retransmissions |
| REC-7 | High | `ack_tracker.rs:35-36` vs `config.rs` | `ack_frequency`/`max_ack_delay` are **hardcoded** (2 and 25ms) and never fed from `GtpConfig`; the incoming AckFrequency frame only mutates the config (`connection.rs:423-432`) which nothing reads; **the advertised negotiation is a complete no-op** |
| REC-8 | Medium | `connection.rs:543-568` | After the early gate, `should_ack || has_queued_data` is always true → **an ACK frame in every outgoing datagram** (~53 B + 8/range) even with nothing new — bypassing frequency adaptation entirely |
| REC-9 | Low | `ack_tracker.rs:150` | `ack_delay_us` truncated to u32 (>71 minutes) |

### 6.3 Loss detector — `loss_detector.rs`

**Correct**: packet threshold k=3 (`:185`), time threshold 9/8×max(SR, latest) (`:169-177`), RTT sampling only from the largest newly-acked PN (RFC 9002 §5), one-pass ACK+loss processing.

**Defects**:

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| REC-10 | Critical | `loss_detector.rs:100-113` + `frame.rs:483-515` | **Attacker-controlled expansion**: Ack decoding validates the range *count* but not the *contents* (`gap`/`length` are attacker u32s), then `(start..=current_pn).rev()` builds a vec — one range with `length=0xFFFFFFFF` allocates **~34 GB** before any sent-packet lookup. Requires the session key (a malicious peer, not the internet), but the wire layer hands over completely unvalidated semantics |
| REC-11 | High | `loss_detector.rs:219-233` + `connection.rs:489-528` | `on_timeout` returns all unacknowledged retransmittables **without removing them from sent_packets**, with no per-message cap and no exponential PTO backoff (`pto_count` is computed and ignored; `pto_max_duration` is dead config) — a **fixed-period retransmission storm** + duplicate application delivery |
| REC-12 | High | the design as a whole | **No timer state machine**: no arming/cancelling API; loss detection only runs inside `on_ack_received` and the PTO check only inside `produce` — an application that stops producing gets no PTO; total ACK loss means no time-based loss declaration ever |
| REC-13 | Medium | `loss_detector.rs:142-162` | `DeliveryRateSample` is computed on every ACK and discarded (`connection.rs:283` throws it away) — dead weight |
| REC-14 | Medium | `loss_detector.rs:76-82` vs `cubic.rs:124-126` | Asymmetric in-flight accounting: the detector counts ack-eliciting bytes only, CC counts every byte — the two gauges diverge |

### 6.4 CUBIC (RFC 8312) — `cubic.rs`

**Correct** (verified mathematically against the RFC text): the growth function `W_cubic(t)=C(t−K)³·SMSS+W_max` in byte domain with correct dimensions; `K=cbrt(W_max(1−β)/(C·SMSS))`; `W_tcp` per Eq. 4; β=0.7 and C=0.4; multiplicative decrease floored at 2·SMSS with a once-per-RTT guard; fast convergence semantically equivalent to §4.6; standard slow start; identical loss and ECN handling.

**Defects**:

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| CC-1 | High | `cubic.rs:154-158` + `:84` | `on_timeout` does not reset `k`/`origin_point`/`w_max` (contrary to §4.7, which requires K=0 and W_max=cwnd) — the moment slow start exits after a timeout, W_cubic is computed against the pre-timeout trajectory and `max()` **restores the old window instantly**; post-timeout slow start is gutted |
| CC-2 | High | `cubic.rs:128-134` | The window grows even on a duplicate/empty ACK (`bytes_acked=0`) — reflected ACK traffic or a peer can inflate cwnd with no data delivered |
| CC-3 | High | `connection.rs:689` + `cubic.rs:124-126` | **Phantom in-flight leak**: ACK-only datagram bytes are charged to `cc.inflight` while the peer never acknowledges them by design → monotonic growth until `cwnd−inflight` is consumed and **sending stalls**; the same records accumulate forever in sent_packets |
| CC-4 | Medium | `cubic.rs:150` | The CC's "smoothed_rtt" is the **raw last sample** with no EWMA — the once-per-RTT loss guard, pacing rate, and W_tcp all jitter with single observations |
| CC-5 | Medium | `cubic.rs:68` | W_tcp uses min_rtt instead of the connection RTT — maximizes the growth bound (aggressive); min_rtt has no expiry window |
| CC-6 | Medium | `state.rs:70,120` vs `config.rs` | **Every CC knob is dead**: `cubic_beta`/`cubic_c`/`initial_cwnd_packets`/`min_cwnd_packets`/`smss`/`pacing_gain`/`max_pacing_burst_bytes` never reach the constructor (built with `Default`) — the LAN/Mobile presets are cosmetic |
| CC-7 | Low | `cubic.rs:84` | The per-ACK candidate is W_cubic(t), not W_cubic(t+RTT) (§4.1) — milder than the RFC |
| CC-8 | Low | — | No HyStart (allowed, a MAY), no idle restart, no max-cwnd clamp |

### 6.5 Pacing and backpressure

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| CC-9 | Medium | `pacing.rs:33-38` | Fractional truncation in `(rate*elapsed) as u64` while `last_update_time` always advances — a systematic accumulating under-credit with no remainder accumulator; refill happens only inside `produce` — a 100ms sleep earns a full 12KB burst at once (pacing granularity = poll rate) |
| CC-10 | Medium | `backpressure.rs` | **No hysteresis** — a pure function that can flap the level per packet (core dedupes the event, not the level); recomputed only on incoming datagrams — a pure sender sees stale backpressure |
| CC-11 | Low | `handle.rs:211-228` | Metrics lie to operators: `pacing_tokens_remaining=0`, `queue_bytes_per_tier=[0;5]`, ECN counters=0 — **hardcoded** although the data exists (no getter at all) |
| CC-12 | Low | `controller.rs:14` | `on_ecn` is defined and implemented but never called — ECN counters are decoded off the wire, carried in ACK frames, and dropped (`connection.rs:281`) — **no ECN loop whatsoever** despite the header bits |

---

## 7. Scheduling Algorithm and Delivery Semantics Audit

### 7.1 The 5-tier DRR scheduler — `scheduler.rs`

**Structure**: 5 `VecDeque` tiers + `deficits:[usize;5]` + a per-tier byte cap (512KB default). P0 is strict-priority and never touches the deficit; P1-P4 receive 3500/3000/1500/500 bytes per visit.

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| SCH-1 | High | `scheduler.rs:109-134` | **DRR degenerates into strict priority**: `pop_next` scans tiers in fixed order, adds the quantum to every non-empty tier, and returns after one item; the caller (`connection.rs:573`) immediately calls again, restarting the scan at P1 and re-adding P1's quantum — P1's quantum (3500) exceeds any realistic frame (<~1400), so P1 is essentially always eligible ⇒ **the advertised weights never materialize under load** and P2-P4 starve until P1 empties — violating the spec requirement ("starvation must be prevented using weighted service") |
| SCH-2 | High | `scheduler.rs:109-117` | **Uncapped deficit accrual, amplified ×4**: each call adds up to 4 quanta to every non-empty tier (including calls returning None under small budgets — the pacing gate allows 64 bytes) with no cap (the classic: quantum+MTU) — a budget-blocked tier accumulates unlimited credit and bursts unfairly once freed |
| SCH-3 | Medium | `scheduler.rs:88-99` | P0 has no share cap — a PathChallenge echo storm starves every tier (the 512KB queue cap is the only brake) |
| SCH-4 | Medium | `scheduler.rs:47,74` | enqueue is O(n) (sums the whole tier per item) + O(n) `retain` for eviction ⇒ O(n²) cumulative — a running byte counter makes it O(1) |
| SCH-5 | Low | `scheduler.rs:47` vs `:149-156` | Tier capacity counts expired-but-unpruned items — they consume admission |
| SCH-6 | Low | `semantics.rs:36` | "P4 shed first under congestion" is advertised with no mechanism (deadlines and caps only) |

**Correct**: expired-item dropping at pop, deficit reset on empty queue, HOL avoidance via push-front + tier break when an item exceeds the budget (letting lower tiers fill the datagram tail).

### 7.2 State scheduling and supersession — `state_table.rs`

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| SEM-1 | High | `identifiers.rs:170-172` | `GenerationId::is_newer_than` is a plain `>` — **not RFC 1982**: a u32 generation wrap permanently breaks supersession for that key; in the same file `StateSequence` is handled correctly (141-144) — two contradictory ordering disciplines for two counters traveling in the same frame |
| SEM-2 | High | `connection.rs:330-345` | **No receive-side drop-late**: the StateTable runs at TX only; every `Frame::Data` is delivered to the application with no freshness check — half of the UnreliableSequenced semantics is missing, and the receiver can observe stale state after fresh state (especially with T4) |
| SEM-3 | Medium | `scheduler.rs:55-76` | The `supersedable` flag is written everywhere **and read nowhere** — a producer marking a snapshot non-supersedable still has it evicted silently |
| SEM-4 | Medium | `scheduler.rs:74` | Eviction searches only the new item's tier — an older update in another tier (priority changed) means both transmit, and with SEM-2 the stale one lands after the fresh one |
| SEM-5 | Low | `state_table.rs:7` | The table is never pruned — unbounded growth for the connection lifetime + a double lookup per enqueue |

### 7.3 Ordered groups — `ordered_group.rs`

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| ORD-1 | High | `ordered_group.rs:29` | The `order_seq < next_expected` comparison is a **plain integer compare** — a u32 wrap makes predecessors buffer as out-of-order **forever**; the group freezes and pins its reorder memory permanently (order_seq never received the RFC 1982 treatment) |
| ORD-2 | High | `connection.rs:367` | A full reorder buffer (256KB) propagates the error with `?`, aborting **the rest of the datagram including the ACK frames the peer needs**, and skipping `ack_tracker.on_packet_received` (452) — the packet is never acknowledged, is retransmitted, and fails the same way: a **potential livelock** |
| ORD-3 | High | `connection.rs:326` | `let _ = scheduler.enqueue(item)` for "reliable" retransmissions — a full P3 tier (`ResourceLimitExceeded`) or an expired item ⇒ **silent loss of a guaranteed-delivery message** without even a counter |
| ORD-4 | High | `connection.rs:347-358` | `ReliableUnordered` (group 0) is delivered **with no deduplication** — retransmissions (loss-driven REC-11 or PTO) reuse the same message_id and the payload reaches the application multiple times; ordered groups protect themselves via `next_expected` — nothing protects the unordered path |
| ORD-5 | Medium | `state.rs:24` + `connection.rs:361-365` | Unbounded group count — a peer can open up to 65,536 groups × 256KB = **16 GB** of frozen buffers (with ORD-1/ORD-2) |
| ORD-6 | Medium | the design | No missing-sequence tracking, no NACK, no timeout eviction — gap filling depends entirely on sender-side loss detection |
| ORD-7 | Low | `connection.rs:367-376` | Batch-delivered items are labeled with the triggering frame's order_seq rather than their own |

### 7.4 Path state machine, anti-amplification, and path validation

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| PATH-1 | Medium | `state_machine.rs:30-42` | `Initial → Closed` is not permitted — `force_close()` on a never-handshaked connection returns an error instead of closing: **closing a failed handshake is impossible** |
| PATH-2 | Medium | `endpoint.rs:183-190, 408-415` + `connection.rs:36` | **Anti-amplification is effectively disabled**: every live connection-creation path passes `pre_validated: true`; the only path that could engage the gate (`connect_with_session_keys(..., false)`) is unused |
| PATH-3 | Medium | `path_validator.rs` | The 3-second challenge timeout is consulted **only when a response happens to arrive** — no timer fires a failure, no retry, no cleanup; an abandoned challenge remains matchable; a `PathValidationFailed` event is never emitted |
| PATH-4 | Low | `state.rs:19` vs `path_validator.rs` | Two sources of truth for the active path (`ConnectionHot.active_path` and `PathValidator.active_path`) updated in two places with no enforced coordination |
| PATH-5 | Low | `connection.rs:389-406` | The PathChallenge echo is enqueued toward `active_path`, not the challenger's `src_addr` — **NAT rebinding can never succeed even if the frame arrives** (compounds with Core-C1); the `b[..9].to_vec()` slicing is hardcoded to the frame layout |

---

## 8. Wire Format Layer Audit

### 8.1 Strengths

- `FrameIterator` is genuinely zero-allocation zero-copy; the crate is `no_std` with no `alloc` — the property is structural.
- `PacketBuilder` writes into a caller-provided buffer and re-encodes the header in place.
- The decode path is panic-free: every read helper pre-checks lengths (`frame.rs:110-175`), headers are guarded by the `min_len` check (`header.rs:167,184`), and the `unreachable!()` in varint is genuinely unreachable.
- `StateSequence::is_newer_than` is a fully correct RFC 1982 implementation with the conservative midpoint treatment.

### 8.2 Defects

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| WIR-1 (W-1) | High | `frame.rs:378, 467` | **Integer overflow in encode-side bounds checks**: `offset + 4 + padding_len` with `padding_len = usize::MAX` (a public field) wraps in release, passes the check, then panics on the slice range; the same pattern for `Padding{len}` — reachable through the public API without unsafe (not remotely: decode never produces unbounded `usize`, but it is a library panic) |
| WIR-2 (W-2) | High | `frame.rs:219` vs `:231` | **Asymmetric ACK encoding**: the size and the range loop are clamped to 32, but the `range_count` byte is written raw — an ACK with 40 ranges encodes 32 and declares 40, which its own decoder rejects (`ResourceLimitExceeded`, a completely untested path) |
| WIR-3 | Medium | `codec.rs:90` + `connection.rs:653` | Silent `payload_len as u16` truncation for payloads >65535 (constructible: three 30KB frames in a 128KB buffer) — must return `BufferOverflow` |
| WIR-4 | Medium | `frame.rs:389` | The Close reason is silently truncated to 255 bytes with no error or signal |
| WIR-5 | Medium | `frame.rs:604-605, 704-705` | Decoding `MtuProbe`/`Padding` **consumes the rest of the packet** — any frame appended after them is silently destroyed; the encoder happily emits mid-packet Padding — **the encoder can produce datagrams its own decoder misparses**; the "padding is terminal" invariant exists only implicitly |
| WIR-6 | Medium | `header.rs:131,136,151` + `codec.rs:53-57` | `header_len` is written but never honored: encode always writes 24/28 bytes while stamping the caller's arbitrary value; PacketBuilder recomputes and ignores the field — a hand-built header with header_len=100 yields a datagram the receiver decodes with consumed=100, **silently skipping 76 bytes of frames as "extensions"**; the extension path has zero test coverage |
| WIR-7 | Medium | `frame.rs:34,39` + `connection.rs:263-275` | `Frame` is ≈ **288 bytes** due to the inline `[AckRange;32]` (`large_enum_variant` suppressed) and is moved **twice** per frame in the dispatch loop (~576 bytes of memcpy even for a 1-byte Padding) |
| WIR-8 | Low | `connection.rs:651-656` | Manual header-byte patching outside gtp-wire — layout knowledge duplicated across crates; any gtp-wire layout change silently corrupts the AAD |
| WIR-9 | Low | `identifiers.rs:50-52, 80-82` vs `:116-118` | Unchecked `+1` for PacketNumber/MessageId while `TransmissionId` uses `saturating_add` — inconsistent within one file |
| WIR-10 | Low | `header.rs:3` vs `endpoint.rs:113,167` | The version is never validated and the two internal values disagree (1 vs 0x00010001) — no version negotiation despite the spec requiring it |
| WIR-11 | Doc | `varint.rs` + `README.md:49` | VarInt is advertised as a feature and is **dead code** (no reference beyond the re-export) |
| WIR-12 | Medium | `Cargo.toml:2-16` + CI | The fuzz crate is **outside the workspace members** — never built by `cargo test/clippy --workspace`, no fuzz step in CI — it rots silently |

### 8.3 Wire-layer test coverage

Round-trip tests cover 9 of 14 frame types (nothing for Retx/PathResponse/MtuProbe/AckFrequency/Padding); the `ResourceLimitExceeded` and `MalformedFrame` (non-UTF-8 Close reason) branches are **never exercised by any test**; the input fuzzing is capped at ≤64 bytes, never touching long headers, handshake frames, or a full ACK; spec-mandated varint fuzzing (§372) is absent.

---

## 9. Core Engine and Runtime Layer Audit

### 9.1 The composed critical defect — dead control frames (Core-C1)

The complete chain:

1. `ConnectionControl::send_ping/trigger_path_challenge/trigger_mtu_probe/set_ack_frequency/graceful_close` (`control/handle.rs:47-172`) encode a control frame and enqueue it as the payload of a `MessageClass::Unreliable` item at P0.
2. The TX path (`connection.rs:576-586`) wraps **every** Unreliable item inside `Frame::Data`.
3. The peer sees a Data frame whose payload is raw frame bytes — no handler decodes the embedded frame; the `PathChallenge/PathResponse/AckFrequency/Close/Ping` handlers (`connection.rs:387-445`) **never fire for locally originated traffic**.
4. The crowning touch: `graceful_close` transitions to Draining **before** sending, while the TX loop refuses to produce unless `is_active()` (`endpoint.rs:471-473`) — **graceful close never transmits a single byte**.

Net result: **path migration, NAT rebinding, graceful close, keepalive/Ping, and PMTU discovery are all disabled end-to-end**, while every individual component (PathValidator, state machine, encoder) is sound and unit-tested. This pattern — sound layers, broken seam — is the recurring theme of this audit.

**Fix**: pass raw `Frame`s to the builder (a path independent of message classes) + allow the Close frame to be sent before exiting Draining.

### 9.2 Other core defects

| ID | Severity | Location | Description |
| :--- | :--- | :--- | :--- |
| CORE-2 | High | `connection.rs:571-647` | **No fragmentation** — a payload >MTU blocks its tier head forever (intra-tier HOL); if reliable, it is never sent, never acknowledged, never declared lost |
| CORE-3 | High | `endpoint.rs:247` | The RX loop `while let Ok(..)` **dies on the first socket error** — the whole endpoint (all connections, all timers) silently freezes |
| CORE-4 | Medium | `endpoint.rs:223,421` | CID table entries are never evicted on close; a duplicate CID silently replaces a live connection's routing entry — leaked tasks, locks, and channels, plus routing to the dead |
| CORE-5 | Medium | `endpoint.rs:448-455` | Messages are forwarded to application channels with `.await` inside the single serial RX loop — one slow consumer starves every connection (endpoint-level HOL) |
| CORE-6 | Medium | `state.rs:12` + `connection.rs:218-219,253,694-695` | The "cache-line-optimized hot/cold separation" claim is **cosmetic**: no `#[repr(align)]`, no layout control; the cold counters are written on every packet; the `next_send_time` field is dead |
| CORE-7 | Medium | `endpoint.rs:320-323` | `s_hdr.encode(&mut resp_buf).unwrap_or(32)` — masking a header-encode failure with a magic length; the only non-test `unwrap_or` in the runtime |
| CORE-8 | Low | `connection.rs:580-590` vs `:337-344` | Plain unreliable sends are encoded with `StateKey::default()/StateSequence(0)/GenerationId(0)` while RX interprets every Data frame as sequenced — all Unreliable traffic shares one key at sequence zero; any future freshness filter (SEM-2) would swallow legitimate messages, and the application cannot distinguish semantics on receive |
| CORE-9 | Low | `endpoint.rs:134-159` vs `:291-297` | Server pending-handshake cleanup happens **only when the next ClientHello arrives** — no independent timer |

### 9.3 Timer table — who drives each timer?

| Timer | Defined | Who polls it | The flaw |
| :--- | :--- | :--- | :--- |
| PTO | `rtt.rs:68-70` | the TX tick (500µs) or the app calling produce | Fires only when `inflight>0` **and** production continues; a stopped app gets no PTO; no exponential backoff (REC-11) |
| Time-threshold loss (9/8 RTT) | `loss_detector.rs:169-199` | inside `on_ack_received` only | **No standalone timer** — total ACK loss means no loss declaration ever |
| Path validation (3s) | `path_validator.rs:4` | **nobody** — consulted only when a response arrives | An abandoned challenge never expires, retries, or reports (PATH-3) |
| Draining → Closed | none | nobody | The transition never happens automatically |
| Keepalive | manual only | the app | Ping itself is dead (Core-C1) |
| Handshake retransmit | 400ms × 8 (client) | the connect task | sound; server cleanup is conditional (CORE-9) |
| max_ack_delay | `ack_tracker.rs:140-144` | the TX tick | Also freezes when production stops |

**There is no unified event loop** — timers scattered across tasks with no coordination, three of them driven by nobody at all.

### 9.4 Hot-path allocations (claim versus reality)

The claim (`docs/specs/GTP-RUST-01.md:29-32`, README): "zero-allocation strategy on the hot path". Reality:

- ✅ `gtp-wire`: zero allocation (structurally) — the claim is honest only here.
- ✅ AEAD: zero heap allocation (detached in-place), no `Box<dyn>` (the `Protector` enum dispatches statically).
- ❌ RX in core: `payload.to_vec()` per data frame (`connection.rs:341,352,376`); a `delivered_messages: Vec` per datagram; a full datagram copy per receive in the endpoint (`endpoint.rs:447`); `to_vec()` for ordered groups; `retransmittable_frames.clone()` per loss sweep; a full record clone per PTO.
- ❌ ACK processing: five `Vec`s per ACK frame + frame clones.
- ⚠ A `ChaCha20Poly1305` key schedule is created **per seal/open** instead of caching the cipher (`aead.rs:50,75`) — not heap, but wasted work per packet.

### 9.5 gtp-io is dead code

`PacketIo`/`RecvDatagram`/the advertised batching are **not imported by any live path**: the runtime uses `tokio::net::UdpSocket` with single datagrams; the dependency is declared in `gtp-runtime-tokio` but unused. No `sendmmsg`/`recvmmsg`, no GSO/GRO (the internal technical paper lists them as future recommendations). The only bytes flowing through it are its own unit test.

### 9.6 Simulation and CLI

- `gtp-sim`: the infrastructure is sound (deterministic virtual time, configurable impairments) but is exercised by only two scenarios (20% ordered loss + a 60fps sequenced stream) with one seed — below what the §4.2 seam matrix requires.
- `gtp-cli`: the `StressSuite` tool (`main.rs:445-1013`) has **no assertions** — report printing only, and its "nat-rebind" stage constructs a `PathValidator` directly (984-989), bypassing the protocol and testing something other than what it claims.
- The simulator builds connections through the deprecated static path with a published hardcoded master secret (`sim_runner.rs:24-25`).

---

## 10. Performance and Efficiency Analysis

### 10.1 Genuine strengths

1. **Structurally zero-allocation wire decoding** — slices borrowed straight from the receive buffer, no_std without alloc.
2. **Heap-free AEAD** with static dispatch and no vtable.
3. **Stack-built ACK frames** (a `[AckRange;32]` array) — no allocation to generate.
4. **TX/RX buffer reuse** in the main loops (`[u8;1500]` and `[u8;2048]`).
5. **Modest header overhead** (24-byte short header) — acceptable for games.

### 10.2 Structural throughput ceilings (ordered by impact)

1. **One serial RX task per endpoint does everything** (decrypt+authenticate+dispatch+await-forward) — a throughput ceiling and a single HOL point (CORE-5).
2. **A 500µs TX tick per connection** — 2000 wakeups/second/connection even idle, with the default `MissedTickBehavior::Burst` bursting after stalls.
3. **Per-packet/per-frame allocations** in core and runtime (§9.4) — the advertised claim breaks above the wire layer.
4. **~288 bytes ×2 moved per frame** in the dispatch loop (WIR-7).
5. **O(n) enqueue** in the scheduler (SCH-4) — quadratic cumulatively under load.
6. **No batched syscalls** (no sendmmsg/recvmmsg/GSO) and the abstraction built for it is dead (§9.5).
7. **Per-packet ChaCha key scheduling** (§9.4).
8. **Read helpers copy into stack arrays** then `from_be_bytes` instead of a direct `try_into` — one redundant copy in the innermost loop.

### 10.3 Benchmark quality

- The open benchmark clones its buffer **inside** each measured iteration — it measures allocation+memcpy along with AEAD (`crypto_bench.rs:28-39`); the DH benchmark generates a key pair per iteration, measuring keygen+DH.
- No coverage of: derive_nonce, HKDF, ratchet, the replay window, the ACK frame (the heaviest), FrameIterator, PacketBuilder — and no benchmarks at all in gtp-recovery/gtp-cc/gtp-scheduler/gtp-core.

---

## 11. Stability Analysis

### 11.1 Self-degrading paths (emerge with time/load)

| Path | Mechanism | Outcome |
| :--- | :--- | :--- |
| **ACK-only throttling** | ACK bytes charged to cc.inflight and never acknowledged (CC-3) | The `cwnd−inflight` budget shrinks monotonically until sending stalls completely |
| **PTO storm** | on_timeout re-enqueues without removal/cap/backoff (REC-11) | Escalating duplicate sending under sustained loss + duplicate application delivery (ORD-4) |
| **Full-buffer livelock** | A full ordered group aborts the datagram with its ACKs (ORD-2) | The same packet is retransmitted and fails the same way — an unbounded loop |
| **Large-payload tier blockage** | No fragmentation (CORE-2) | The tier never advances; its reliable traffic never completes |
| **Group freeze on wrap** | order_seq without RFC 1982 (ORD-1) | After 2³² messages per group it halts permanently, pinning memory |
| **Silent RX death** | The first socket error ends the loop (CORE-3) | An apparently-alive dead endpoint — every connection frozen with no signal |

### 11.2 Unbounded memory growth (six sites)

1. ACK intervals never pruned (REC-5).
2. ACK-only records in `sent_packets` forever (CC-3).
3. Ordered-group count uncapped with 256KB buffers each (ORD-5) — up to 16GB from a peer.
4. The CID routing table without eviction (CORE-4).
5. `StateTable` without pruning (SEM-5).
6. Attacker-controlled allocation up to ~34GB from a single ACK range (REC-10).

### 11.3 Positive stability factors

- No panics in the data path (verified directly — no unwrap/expect/panic outside tests in connection/endpoint/state/async_connection/udp).
- All time arithmetic is saturating — no panic on regression or zero.
- Panic-free decoding with comprehensive length checks.
- The deterministic simulator exists and is 100% reproducible.

---

## 12. Unified Findings Registry

The full registry, ordered by severity then location. IDs are referenced by the remediation plan.

### Critical — 6

| ID | Location | Summary |
| :--- | :--- | :--- |
| SEC-1 | `gtp-crypto/src/handshake.rs:58-84` + `aead.rs:21-33` + `gtp-core/src/state.rs:49-133` | One key/IV for both directions + a deterministic nonce ⇒ immediate nonce reuse (RFC 8439) |
| SEC-3 | `gtp-core/src/connection.rs:232-235` (before `:240`) | Replay window updated before AEAD authentication — permanent DoS with one packet |
| SEC-4 | `gtp-path/src/stateless_token.rs:24-35` | A dismantlable XOR cookie: secret recovery and lifetime forgery |
| Core-C1 | `gtp-core/src/control/handle.rs:47-172` + `connection.rs:576-586` + `endpoint.rs:471-473` | Double-wrapped control frames ⇒ Ping/Close/PathChallenge/MtuProbe/AckFrequency dead end-to-end |
| REC-10 | `gtp-wire/src/frame.rs:483-515` + `gtp-recovery/src/loss_detector.rs:100-113` | Unvalidated ACK range contents ⇒ ~34GB allocation by an authenticated peer |
| ORD-2 | `gtp-core/src/connection.rs:367` (with `:452`) | A full group aborts the datagram with its ACKs ⇒ a retransmission livelock |

### High — 17

| ID | Location | Summary |
| :--- | :--- | :--- |
| SEC-2 | `gtp-crypto/src/aead.rs:29-31` | Top 32 PN bits ignored in the nonce — collision at 2³² |
| SEC-5 | `gtp-runtime-tokio/src/endpoint.rs` (the whole handshake) | Anonymous DH with no server authentication or server Finished — active MITM |
| SEC-6 | `gtp-core/src/state.rs:86-92` + `gtp-wire/src/header.rs:13` | Uncoordinated ratchet (KEY_PHASE unused) ⇒ the peer cannot decrypt afterwards |
| SEC-7 | `Cargo.toml:50` | X25519 private keys never zeroized (the zeroize feature is disabled) |
| SEC-8 | `gtp-crypto/src/kdf.rs:32-55` + `gtp-sim/src/sim_runner.rs:24-25` | The static path is deterministic, still public, and used with a published secret |
| REC-1 | `gtp-recovery/src/rtt.rs:36-48` | Unconditional ack_delay subtraction incl. the first sample — attacker-corrupted RTT |
| REC-5 | `gtp-recovery/src/ack_tracker.rs:148-188` | ACK intervals never pruned — unbounded growth |
| REC-6 | `gtp-recovery/src/ack_tracker.rs:166` | The 32-range cap silently drops acknowledgements ⇒ forced PTO retransmission |
| REC-7 | `ack_tracker.rs:35-36` + `connection.rs:423-432` | ACK adaptation/negotiation completely dead (hardcoded values, config with no readers) |
| REC-11 | `loss_detector.rs:219-233` + `connection.rs:489-528` | PTO without exponential backoff/cap/record removal ⇒ storms and duplicate delivery |
| REC-12 | `gtp-recovery/src/loss_detector.rs` (structural) | No timer state machine: time-threshold conditioned on ACKs, PTO on production |
| CC-1 | `gtp-cc/src/cubic.rs:154-158` + `:84` | Post-timeout CUBIC instantly restores the old window (contrary to RFC 8312 §4.7) |
| CC-2 | `gtp-cc/src/cubic.rs:128-134` | Window growth on duplicate/empty ACKs |
| CC-3 | `connection.rs:689` + `cubic.rs:124-126` | In-flight leak on ACK-only traffic ⇒ gradual send stall |
| SCH-1 | `gtp-scheduler/src/scheduler.rs:109-134` | DRR degenerates to strict priority — weights have no effect, P2-P4 starve |
| SEM-1 | `gtp-types/src/identifiers.rs:170-172` | GenerationId without RFC 1982 — supersession breaks on wrap |
| ORD-4 | `connection.rs:347-358` | ReliableUnordered without dedup — duplicate application delivery |

### Medium — 34

| ID | Location | Summary |
| :--- | :--- | :--- |
| WIR-2 | `frame.rs:219/231` | range_count unclamped on encode — a frame its own decoder rejects |
| WIR-3 | `codec.rs:90` + `connection.rs:653` | Silent u16 payload_len truncation |
| WIR-4 | `frame.rs:389` | Close reason silently truncated to 255 |
| WIR-5 | `frame.rs:604-605, 704-705` | Padding/MtuProbe silently swallow trailing frames |
| WIR-6 | `header.rs:131-151` + `codec.rs:53-57` | header_len written but unenforced — frames silently skipped as extensions |
| WIR-7 | `frame.rs:34,39` + `connection.rs:263-275` | Frame ≈288 bytes, double move per frame |
| WIR-12 | `Cargo.toml` + CI | fuzz outside the workspace — never built or run |
| SEC-9 | `handshake.rs:104-119` | HMAC proof keyed by the AEAD key itself |
| SEC-10 | `aead.rs:45-47` | Overflow in the length check ⇒ a public-API panic |
| SEC-11 | `endpoint.rs:280,290` | Rate limiter 1000/s versus a documented 20/s + no ClientHello freshness |
| SEC-12 | `handshake.rs:49-54` | No X25519 contributory check |
| SEC-13 | `endpoint.rs:113` + `handshake.rs` | The version is not negotiated/bound into the transcript |
| SEC-14 | `aead.rs:9-13`, `kdf.rs:13-27` | Derived Debug on key containers |
| SEC-15 | `connection.rs:218-222` | Anti-amp credit before any validation |
| SEC-16 | `anti_amplification.rs` | No path-scoped reset; validation lifted by decryption |
| SEC-17 | `identifiers.rs:50-52` | Unchecked `next()` — debug panic / unsafe release wrap |
| REC-2 | `rtt.rs:43-44` | min_rtt from the adjusted sample |
| REC-3 | `rtt.rs:23` | The u64::MAX sentinel leaks into metrics |
| REC-8 | `connection.rs:543-568` | An ACK in every outgoing datagram — adaptation bypassed |
| REC-13 | `loss_detector.rs:142-162` | DeliveryRateSample computed and discarded |
| REC-14 | `loss_detector.rs:76-82` vs `cubic.rs:124-126` | Two unequal in-flight gauges |
| CC-4 | `cubic.rs:150` | The CC "smoothed_rtt" is a raw sample |
| CC-5 | `cubic.rs:68` | W_tcp on min_rtt (aggressive), no expiry |
| CC-6 | `config.rs` vs `state.rs:70,120` | All CC knobs/presets dead |
| CC-9 | `pacing.rs:33-38` | Fractional drift + production-conditioned refill |
| CC-10 | `backpressure.rs` | No hysteresis; receive-side-only refresh |
| CC-12 | `controller.rs:14` | on_ecn has no caller — no ECN loop |
| SCH-2 | `scheduler.rs:109-117` | Uncapped, ×4-amplified deficit accrual |
| SCH-3 | `scheduler.rs:88-99` | P0 without a share cap |
| SCH-4 | `scheduler.rs:47,74` | O(n) enqueue ⇒ O(n²) |
| SEM-2 | `connection.rs:330-345` | No receive-side drop-late — half of sequenced semantics |
| SEM-3/4 | `scheduler.rs:55-76`/`:74` | supersedable ignored; same-tier-only eviction |
| ORD-1 | `ordered_group.rs:29` | order_seq without wrap handling — permanent group freeze |
| ORD-3 | `connection.rs:326` | `let _` on reliable enqueue — guaranteed loss |
| ORD-5 | `state.rs:24` | Uncapped groups (16GB from a peer) |
| ORD-6 | the design | No NACK/gap eviction/timeout |
| PATH-1 | `state_machine.rs:30-42` | Initial→Closed illegal — closing a failed handshake impossible |
| PATH-2 | `endpoint.rs:183-190` | Anti-amp effectively disabled (always pre_validated) |
| PATH-3 | `path_validator.rs` | A challenge timeout nobody drives |
| CORE-2 | `connection.rs:571-647` | No fragmentation — permanent HOL for >MTU |
| CORE-3 | `endpoint.rs:247` | Silent RX death on a socket error |
| CORE-4 | `endpoint.rs:223,421` | CID table without eviction/duplicate replacement |
| CORE-5 | `endpoint.rs:448-455` | Endpoint-level HOL on one slow consumer |
| CORE-6 | `state.rs:12` | Hot/cold separation is cosmetic |
| D-2 | `connection.rs:566-641` | `let _ = append_frame` — dropped frames still recorded in-flight |
| CC-11 | `handle.rs:211-228` | Hardcoded metrics (tokens/tiers/ECN = 0) |
| CORE-7 | `endpoint.rs:323` | The magic `unwrap_or(32)` |

### Low — 12

WIR-1 (encode overflow — technically high as a library panic but requires local API use), WIR-8 (header patching outside gtp-wire), WIR-9 (unchecked +1), WIR-10 (unvalidated version/value mismatch), WIR-11 (dead advertised VarInt), REC-4 (no kGranularity), REC-9 (u32 truncation), CC-7/CC-8 (W_cubic(t), no HyStart/clamp), SEC-18 (configured window size ignored), SEC-19 (plaintext protector skips the length check), SCH-5/6, SEM-5, ORD-7, PATH-4/5, CORE-8/9, CC-20/21/22 (bench quality), T-3.

> Classification note: WIR-1 is rated "high" from a library perspective (a public-API panic) but is not remotely triggerable; it is listed here as low from a networked-service perspective since the decode path never produces unbounded values.

---

## 13. Prioritized Remediation Plan

### Phase 0 — stop the security bleeding (immediate, before any deployment)

| # | Action | Fixes | Effort |
| :--- | :--- | :--- | :--- |
| 0.1 | Derive two direction-labeled key/IV pairs in `derive_handshake_session_keys` and select by role + a test forbidding equality and proving cross-decryption fails | SEC-1 | Small |
| 0.2 | Reverse the authentication/replay order in `handle_incoming_datagram` (authenticate first, or copy-then-commit) + a test: a forged high-PN packet must not burn the window | SEC-3 | Small |
| 0.3 | Replace the cookie with `HMAC-SHA256(secret, "gtp cookie"‖addr‖ts)` + reject future timestamps + forgery/lifetime tests | SEC-4 | Small |
| 0.4 | Include all 64 PN bits in the nonce + a test of PN=1 versus PN=2³²+1 | SEC-2 | Small |
| 0.5 | Enable `x25519-dalek/zeroize` + manually redacted Debug for keys + the `was_contributory` check | SEC-7, SEC-14, SEC-12 | Small |
| 0.6 | Semantic ACK-range validation on decode (a sane byte/count cap, consistency with largest_acked) | REC-10 | Small |

### Phase 1 — revive functional integration (highest return per line changed)

| # | Action | Fixes |
| :--- | :--- | :--- |
| 1.1 | Pass control frames as raw `Frame`s to the builder (independent of message classes) + allow Close to be sent in Draining + end-to-end tests for Ping/Close/PathChallenge/AckFrequency | **Core-C1, PATH-5, part of PATH-3** |
| 1.2 | Actually wire `GtpConfig`: AckTracker (frequency/delay) and Cubic (β/C/IW/clamps) and Pacing (gain/burst) — delete the dead fields or connect them | REC-7, CC-6, SEC-18 |
| 1.3 | Do not charge ACK-only bytes to cc.inflight + drop their records from sent_packets immediately | CC-3, REC-14 |
| 1.4 | Fix the PTO completion path: remove re-enqueued records + exponential `PTO×2^pto_count` + a transmission cap + a ReliableUnordered message-id dedup ledger | REC-11, ORD-4 |
| 1.5 | Guard the SEM-2/ORD-1 transitions: RFC 1982 for order_seq and GenerationId + receive-side drop-late via an RX-side StateTable | ORD-1, SEM-1, SEM-2 |

### Phase 2 — operational stability

- A standalone armed/cancelled loss timer + a TX-tick-driven path-validation timer + a Draining→Closed timeout (REC-12, PATH-3).
- Handle socket errors in the RX loop (log+continue/rebuild) + CID-table eviction on close + duplicate-CID rejection (CORE-3, CORE-4).
- Fix ORD-2: isolate the full-group error without aborting the ACKs (accept the overflow as a dropped item and deliver the remaining frames) + an LRU cap on group count.
- Post-timeout CUBIC (reset k/origin/w_max) + a growth gate on `bytes_acked>0` + a real EWMA (CC-1, CC-2, CC-4) and raw min_rtt with an expiry window (REC-2, CC-5).
- Fragment >MTU payloads or reject them at admission with an explicit error — no silent blockage (CORE-2).
- Propagate `append_frame` errors instead of `let _`, dropping the matching in-flight record (D-2), and gate anti-amp before the pop (§3.4 step 11).

### Phase 3 — scheduler fairness and memory

- Real DRR: multi-item service per visit (keep draining a tier until its deficit is exhausted) + a quantum+MTU deficit cap (SCH-1, SCH-2).
- Prune stale ACK intervals + coalesce at the cap instead of dropping (REC-5, REC-6).
- A per-tier running byte counter (O(1) enqueue) + cross-tier supersession eviction + honoring `supersedable` (SCH-4, SEM-3/4).

### Phase 4 — the integration test harness (prevents a repeat of everything above)

The specifically missing seam tests — each would have caught an existing defect:

1. PTO under 100% ACK loss (catches REC-11/ORD-4).
2. Path migration through the protocol under loss (catches Core-C1/PATH-3/PATH-5).
3. Control frames end-to-end (catches Core-C1).
4. Mid-session ratchet followed by data (catches SEC-6).
5. Tampered/replayed/wrong-CID datagrams through the full connection with window-integrity assertions (catches SEC-3).
6. A payload >MTU (catches CORE-2).
7. A full group with concurrent ACK flow (catches ORD-2).
8. Long-run fairness: per-tier byte shares under saturation (catches SCH-1/2).
9. A simulation bridge: reuse gtp-sim as the bed for these scenarios across multiple seeds.
10. Bring the fuzz crate into the workspace + CI (WIR-12) + known-answer vectors (KAT) for RFC 8439 and HKDF.

### Phase 5 — performance (after correctness)

- Remove `to_vec()` from the delivery path (borrowed delivery or preallocated pools) + stop the per-datagram copy in the endpoint.
- Shrink `Frame` (Box/SmallVec for ACK ranges or split the enum) and the double move in the dispatch loop.
- Cache the ChaCha cipher instead of rescheduling per packet.
- Activate gtp-io (sendmmsg/recvmmsg) or delete it; shard RX across SO_REUSEPORT/multiple tasks.
- Wake TX with a minimum-gap deadline instead of a fixed 500µs idle tick.

---

## 14. Appendices

### Appendix A — verification environment

| Item | Value |
| :--- | :--- |
| System | CachyOS Linux (kernel 7.2.2-1-cachyos) x86_64 |
| Toolchain | rustc/cargo 1.85.0 (matching `rust-toolchain.toml`) — note: invoking rustup through the system proxies fails in this environment (mangled `argv[0]`: `unknown proxy name: 'ZCode-3.8.1-linux-x64'`); the direct path `~/.rustup/toolchains/1.85.0-.../bin` was used successfully. Fixing the rustup installation or using the direct path in local CI scripts is recommended. |
| Verification command | `cargo test --workspace` |
| Result | Full pass: 48 passed / 0 failed / 1 ignored |

### Appendix B — binding RFC conformance

| Standard | Status | Main deviations |
| :--- | :--- | :--- |
| RFC 8439 (AEAD) | ❌ violated | SEC-1 (nonce uniqueness), SEC-2 |
| RFC 8312 (CUBIC) | ⚠ partial | The math is correct; CC-1 (§4.7), CC-2, CC-4/5, no HyStart/clamp |
| RFC 9002 (recovery) | ⚠ partial | EWMA/thresholds correct; REC-1 (§5.2/5.3), REC-11/12 (backoff/timer state machine) |
| RFC 1982 (serials) | ⚠ patchy | StateSequence correct; GenerationId and order_seq are not (SEM-1, ORD-1) |
| RFC 9000 (QUIC concepts) | ⚠ inspired, not bound | Anti-amplification not path-scoped (SEC-16), no key spaces, no version negotiation |

### Appendix C — defect distribution per crate

| Crate | Critical | High | Medium | Low |
| :--- | :--- | :--- | :--- | :--- |
| gtp-crypto | 2 | 4 | 5 | 3 |
| gtp-path | 1 | — | 4 | 3 |
| gtp-wire | 1 | 1 | 5 | 5 |
| gtp-recovery | 1 | 5 | 5 | 2 |
| gtp-cc | — | 3 | 6 | 5 |
| gtp-scheduler | — | 2 | 4 | 3 |
| gtp-types | — | 1 | 1 | 1 |
| gtp-core | 1 | 2 | 8 | 3 |
| gtp-runtime-tokio | 1 | — | 4 | 2 |
| gtp-io / gtp-sim / gtp-cli | — | — | 3 (dead code/single scenario) | 2 |

*(Counts are approximate for items spanning two crates — see the detailed registry in §12; cross-crate criticals are counted for both since fixing them touches the seam.)*

### Appendix D — the central theme of the audit

The pattern recurring across every finding: **the layers are built and internally tested, but the control-transfer points between them are unsoldered**. Frames encoded in one layer and interpreted in another with different semantics (Core-C1, WIR-5/6); counters updated in one layer assuming a guarantee another layer does not provide (SEC-3, CC-3, D-2); mathematically correct algorithms fed unvalidated wire values (REC-1, REC-10); and advertised settings written but never read (REC-7, CC-6, SEC-18). The highest-leverage investment in this codebase is therefore not fixing any single defect but **building the seam-test harness (Phase 4)** that would have caught all six current criticals — and will hold every future inter-layer seam.

---

*End of document — GTP-rs Architecture & Protocol Audit Paper v1.0*
