# GTP-rs — Reconciling the Two Audits and Independent Verification of the New Findings

**Repository:** `e1c9ef7` — local copy `/home/ggonlinux/GTP/`
**Documents compared:**
- (A) `GTP-rs_Technical_Audit_and_Optimization_Plan_AR.md` — the first report *(Arabic original; see `arabic-local/`)*
- (B) `GTP-rs-Architecture-Protocol-Audit-Paper-v1.0.md` — the new paper v1.0
- (C) `GTP-rs_Deep_Audit_Verified_AR.md` — my previous audit *(Arabic original; see `arabic-local/`)*

**Date:** August 30, 2026

---

## 0. Verdict on the new paper (B)

Paper (B) is **stronger and more comprehensive than (A) and (C) combined** in coverage: 6 critical + 17 high + 34 medium defects, with precise `file:line` locations, and an actual `cargo test` run (48 passing) — something I could not do (no Rust toolchain available in my environment).

Most importantly: **paper (B) and my audit (C) independently reached the same central thesis** — "the layers are sound internally, and all the defects sit on the seams between them, which is precisely what the tests do not cover." Two independent audits converging on this point raises confidence considerably and makes it the correct axis for the remediation plan.

**Net outcome of the comparison:**
- **21 defects in (B) that I never touched** — I verified 13 of them directly in the source: **all correct**, and two of them (SEC-1, Core-C1) are more dangerous than anything in my audit.
- **Two substantive findings of mine are absent from (B)** — one of them (the ACK range accounting defect) is **explicitly classified by paper (B) as "correct"**, which is an error in the paper that I re-verified three times.
- **One item needs a technical correction in (B)** (the CC-3 mechanism) — and the correction exposes a third, even more certain defect that no document had detected.

---

## 1. ✱ The most dangerous discovery in the entire file: cross-direction nonce reuse (SEC-1)

**This is the most dangerous thing in the repository, and I missed it entirely. I verified it directly and it is fully correct.**

The verification chain I executed:

```
1) grep over crates/gtp-crypto + state.rs + endpoint.rs for
   role / direction / client_tx / server_tx / is_client  →  zero results
2) Client  (endpoint.rs:183): GtpConnection::new_with_session_keys(cid, .., key, iv, true, ..)
3) Server  (endpoint.rs:408): GtpConnection::new_with_session_keys(cid, .., key, iv, true, ..)
   ← the same (key, iv) both produced by derive_handshake_session_keys
4) state.rs:59  →  next_packet_number: PacketNumber(1)  for both peers
5) aead.rs:21   →  nonce = IV ⊕ (CID_be ‖ PN_be[4..8])   a purely deterministic function
```

⇒ **The client's packet 1 and the server's packet 1 use the same key and the same nonce.** And every packet number N collides across both directions for the whole session.

**Why this is more dangerous than my note in (C §3.8):** I detected the nonce repetition at 2³² packets — after days of continuous operation. Paper (B) detected that the collision occurs at **the very first packet**, in every session, without exception.

**The full technical impact** (ChaCha20-Poly1305 / RFC 8439):
1. **Confidentiality collapse:** `C₁ ⊕ C₂ = P₁ ⊕ P₂`. Game traffic is highly structured and predictable (fixed frame headers, sequential identifiers, coordinates within a narrow range) ⇒ plaintext recovery is practical.
2. **Integrity collapse — the worst part:** the one-time Poly1305 key is derived from (key, nonce) only. A nonce collision discloses it ⇒ **forgery of fully authenticated packets**. The differing AAD between directions (the timestamps) **does not help**: AAD feeds Poly1305 only and never touches the keystream.
3. The attacker needs no key at all — only the ability to observe both directions, which any network intermediary has.

**An additional detail I confirm:** the test at `handshake.rs:155-163` contains `assert_eq!(client_key, server_key)` — i.e., **the dangerous state is enshrined in the test suite as required behavior**. Any correct fix will make this test fail, and that is a success signal, not a failure.

**The fix (as in (B) 0.1, with my elaboration):**

```rust
// In derive_handshake_session_keys: derive four materials, not two
let (c2s_key, c2s_iv) = expand(&master, b"gtp/v1 client->server");
let (s2c_key, s2c_iv) = expand(&master, b"gtp/v1 server->client");
// Each endpoint: seals with tx_key and opens with rx_key according to its role
```

Mandatory test: `assert_ne!(c2s_key, s2c_key)` + proof that opening client traffic with the client key **fails**.

---

## 2. ✱ The second discovery I missed: control frames double-wrapped (Core-C1)

**I verified it directly in `control/handle.rs` — fully correct, and the chain is clearer than what I described in (C §3.16).**

Every control function (`set_ack_frequency`, `trigger_path_challenge`, `trigger_mtu_probe`, `send_ping`, `graceful_close`) follows literally the same pattern:

```rust
let frame = Frame::PathChallenge { data: nonce };
let mut buf = [0u8; 16];
let written = frame.encode(&mut buf)?;          // a full frame encoded
let item = SchedulableItem {
    class: MessageClass::Unreliable,            // ← later wrapped in Frame::Data
    payload: buf[..written].to_vec(),           // frame bytes as an "application payload"
    ...
};
```

And the TX path (`connection.rs:576-586`) wraps **every** `Unreliable` item inside `Frame::Data`. The result: the peer receives a `Data` frame whose payload is raw control-frame bytes, **and delivers them to the application as a game message**. The `PathChallenge`/`PathResponse`/`Close`/`AckFrequency`/`Ping` handlers in `handle_incoming_datagram` **never fire** for any locally originated traffic.

**The crowning point paper (B) spotted and I confirm:** `graceful_close` transitions to `Draining` **before** sending, while the TX gate is:

```rust
if !self.hot.state.is_active() && !matches!(self.hot.state, Handshaking) { return Ok(None); }
```

and `is_active()` means `Established` only ⇒ **graceful close never transmits a single byte at all**.

**This corrects and deepens my note in (C §3.16):** I said path migration was dead because `start_challenge` is never invoked from the receive path. The truth is worse: even the explicit invocation via `trigger_path_challenge` — which **does** call `start_challenge` — never reaches the peer because of the double wrapping. The automatic echo at `connection.rs:389` (`b[..9].to_vec()` inside an `Unreliable` item) suffers the same defect. I noticed that intrusion into the code but never traced its consequence to the end.

**The dead end-to-end features:** path migration, NAT rebinding, graceful close, Ping/keepalive, PMTU discovery, ACK-frequency negotiation. All of them **documented and unit-tested** — and all disabled in practice.

---

## 3. The rest of (B)'s findings that are new to me — verification results

I verified the items marked ✅ directly; the rest is structurally consistent with what I read but I did not inspect that exact line.

| ID | The claim | Verification |
| :--- | :--- | :--- |
| **SEC-7** | `x25519-dalek` with `default-features = false` ⇒ the `zeroize` feature is off, private keys never wiped | ✅ **correct** — `Cargo.toml:50` confirmed. A striking irony: `handshake.rs` carefully uses `ZeroizeOnDrop` on `HandshakeSharedSecret` while the private key itself is never zeroized. (It also disables `precomputed-tables` ⇒ a DH performance loss) |
| **SEM-1** | `GenerationId::is_newer_than` uses plain `>` while `StateSequence` implements RFC 1982 correctly **in the same file** | ✅ **correct** — `identifiers.rs:141-144` versus `:170-172`. Two contradictory ordering systems for two counters traveling in the same frame. I had missed it |
| **SEC-11** | The ClientHello rate limiter: the comment says "20/second" and the code does `entry.0 <= 1000` | ✅ **correct** — `endpoint.rs:290`. A ×50 gap from the documented intent |
| **SEC-10** | An overflow in `payload.len() < payload_len + AEAD_TAG_LEN` | ✅ **correct** — `aead.rs:45`. The fix: `checked_add`. Not remotely triggerable (the value is internal) but a panic in a public API |
| **WIR-1** | The same pattern in `offset + 4 + *padding_len` for `MtuProbe`/`Padding` | ✅ **correct** — `frame.rs:378`. The fields are public `usize` |
| **CORE-3** | The RX loop `while let Ok(..)` dies on the first socket error | ✅ **correct** — `endpoint.rs:247`. **And I strengthen it:** on Linux/Windows a UDP socket returns `ConnectionReset`/`ECONNREFUSED` upon receiving an ICMP port-unreachable ⇒ **a remote party can kill the entire endpoint** (all connections) with a single ICMP packet. The paper rates it "high"; operationally I consider it closer to critical |
| **WIR-12** | The `fuzz` crate is outside the workspace members ⇒ never built or run | ✅ **correct** — `fuzz/Cargo.toml` is a standalone package, and the root `Cargo.toml` does not list it. I noted in (C) that CI does not run fuzz; the paper added that it is not even compiled, so it may have rotted |
| **WIR-10** | `endpoint.rs:113` sends `version = 1` while `GTP_V1_1 = 0x00010001`, with no version check anywhere | ✅ **correct** — two internal implementations disagree, and nobody notices because the field is never read |
| **gtp-io dead** | `PacketIo` is not imported by any live path | ✅ **correct** — declared as a dependency in 3 crates, the only usage being `pub use gtp_io as io` in the facade. No `sendmmsg`/`recvmmsg`/GSO |
| **REC-6** | The 32-range cap silently drops acknowledgements without coalescing | ✅ **correct** — `take(MAX_ACK_RANGES - 1)`. It complements my note (C §3.10): I detected the missing pruning, the paper detected that the cap causes **forced retransmission via PTO** for packets that actually arrived |
| **REC-8** | An ACK frame in **every** outgoing datagram — bypassing adaptation entirely | ✅ **correct** — the condition `should_ack \|\| has_queued_data` and `generate_ack_frame` returns `Some` whenever a `largest_received` exists. **Very important for §4 below** |
| **REC-7** | `ack_frequency`/`max_ack_delay` hardcoded (2 and 25ms), never fed from `GtpConfig`; the incoming AckFrequency frame mutates a config nobody reads ⇒ the negotiation is a no-op | ✅ **structurally correct** — `AckTracker::default()` pins them, and `connection.rs:423-432` writes only to `self.config` |
| **CC-6** | Every CC parameter in `GtpConfig` is dead (the units are built with `Default`) | ✅ **correct** — `state.rs` builds `CubicCongestionController::default()` and `PacingEngine::default()` without passing the config. The `competitive_fps()`/LAN/Mobile presets are cosmetic on the CC side |
| **ORD-4** | `ReliableUnordered` (group 0) is delivered without deduplication | ✅ **correct** — there is no tracking of received `message_id`s at all. I spotted the risk for `Frame::Retx` and did not generalize it to the main path |
| **ORD-3** | `let _ = scheduler.enqueue(item)` for "reliable" retransmission ⇒ silent loss when P3 is full | ✅ **correct** — two sites (RX:326, PTO:525), both `let _`. Complements (C §3.12) with a third loss path I had missed |
| **PATH-1** | `Initial → Closed` is not allowed ⇒ closing a connection whose handshake failed is impossible | ✅ **correct** — I mentioned it in passing in (C) without drawing the practical conclusion |
| **WIR-6** | `header_len` is written but never honored; a header with `header_len=100` makes the receiver skip 76 bytes of frames silently as "extensions" | ✅ **correct** — I noted in (C §3.20) that `header_len` is peer-controlled, and the paper extracted the complete practical scenario |
| **SEC-9** | The HMAC proof reuses the AEAD key itself (role mixing) | ✅ **correct** — `compute_client_proof(&key, ...)` where `key` is the AEAD key. A separate `finished_key` must be derived |
| **CC-2** | The window grows on a duplicate/empty ACK (`bytes_acked = 0`) | ✅ **correct** — `on_ack` calls `update_w_cubic` with no gate on `bytes_acked > 0` |
| **CC-1** | `on_timeout` does not reset `k`/`origin_point`/`w_max` ⇒ `max()` instantly restores the old window | ✅ **correct** — I spotted the `max()` trap in (C §3.15), and the paper tied it to RFC 8312 §4.7 and showed that post-timeout slow start is practically gutted |
| **ORD-2** | A full group buffer aborts the datagram **including ACK frames** and skips `ack_tracker.on_packet_received` ⇒ the packet is never acknowledged, is retransmitted, fails the same way = **livelock** | ✅ **correct** — I detected the abort in (C §3.18); the paper extracted the livelock loop, which is the more important conclusion |
| **CORE-4/5** | The CID table without eviction + `.await` delivery inside the single RX loop (endpoint-level HOL) | ✅ consistent with the structure of `endpoint.rs` |

---

## 4. ✱ The defect paper (B) classifies as "correct" that is actually broken

**This is my most important remaining contribution, and I present it carefully because it contradicts an explicit verdict in paper (B).**

Paper (B) lists under §6.2 in the **"Correct"** section:

> "a descending sorted interval set with adjacent merging; **the range encoding semantics match QUIC**"

The encoder is indeed semantically identical to QUIC (with a ±1 offset in the `gap` convention). **But the paper examined the encoder and not the decoder.** And the decoder in `loss_detector.rs` consumes the `gap` in the reverse order:

```rust
// The encoder — gap_i describes the gap preceding range i
let gap = prev_start.saturating_sub(interval.end + 1);

// The decoder — uses range i first, then subtracts gap_i
let start = current_pn.saturating_sub(range.length as u64);   // ← before subtracting the gap
for pn in (start..=current_pn).rev() { acked_numbers.push(pn); }
current_pn = current_pn - range.length - range.gap - 1;       // ← the subtraction is a full step late
```

The decoder is written as if `gap_i` described the gap **following** range i, while the encoder writes it as the gap **preceding** it. The two halves use opposite conventions. The result is correct only when `gap = 0` for every range — i.e., in the complete absence of loss, which is exactly what the current tests cover (`range_count == 1`).

**Numerical re-verification** (the receiver got `1..5`, `8..10`, `20..22`):

```
The encoded ranges (gap,len): [(0,2), (9,2), (2,4)]
The truth        : 1 2 3 4 5 | 8 9 10 | 20 21 22
What the sender decodes: 3 4 5 6 7 | 17 18 19 | 20 21 22

Falsely acked (never arrived): 6, 7, 17, 18, 19
Lost acknowledgements (did arrive): 1, 2, 8, 9, 10
```

The falsely acked packets are removed from `sent_packets` and counted as delivered ⇒ **their reliable payloads are never retransmitted**.

### The composition that makes this defect permanent, not incidental

This is where three results from the two documents meet, and the composition is more dangerous than any of them alone:

| Source | Result |
| :--- | :--- |
| (B) REC-5 | ACK intervals are **never pruned** |
| The design | Retransmissions use **new packet numbers** ⇒ the gap of any lost packet is **permanent and never fills** |
| (B) REC-8 | An ACK frame is attached to **every** outgoing datagram |
| (C) this defect | The encode/decode is broken whenever a gap exists (≥ 2 ranges) |

**The composite conclusion:** as soon as **a single packet** is lost, the range count becomes ≥ 2 and never decreases, and every ACK frame sent after that moment — i.e., **every outgoing datagram for the rest of the connection's life** — carries wrong acknowledgements. The defect is not an edge case; it becomes the permanent state after the first loss.

This is the root cause that makes a "reliable channel" silently lose messages under loss — the very condition it exists for.

**The proof test** (fails conclusively today — full text in my previous audit §7-a): build an `AckTracker` receiving `1..5, 8..10, 20..22`, generate the ACK frame, feed it to a `LossDetector`, and assert that the set of packets the sender considers acked equals exactly the received set.

---

## 5. Other findings of mine absent from (B)

| # | The defect | Status in (B) |
| :--- | :--- | :--- |
| 1 | **The ACK range accounting defect** (§4 above) | Classified "correct" — the only substantive disagreement |
| 2 | **The optimistic-ACK attack**: no check that `largest_acked ≤ largest_sent`. A peer can acknowledge packets never sent ⇒ `cwnd` inflation and congestion-control subversion | REC-10 addresses the memory exhaustion from large `length`s but does not mention window inflation. **Complementary, not contradictory** — one fix: semantic validation of the frame |
| 3 | **No black-hole recovery**: no timer-based loss detection + `cc.inflight` never drains ⇒ `send_budget = cwnd − inflight = 0` **even after the path returns** | REC-12 notes the missing timer state machine and CC-3 the inflight leak, but the "the connection never recovers" consequence is not drawn. An integration, not a conflict |
| 4 | **A header-protection recommendation**: the plaintext CID and PN are exactly what makes the SEC-3 attack possible remotely at zero cost, and they enable session linkability | (B) detects SEC-3 precisely but does not connect it to the absence of header protection or propose it as a root fix. A useful architectural addition |

---

## 6. One technical correction to paper (B) — and a third defect it reveals

### CC-3 — the described mechanism is imprecise, and the reality is worse from another angle

**The claim in (B):** the bytes of ACK-only packets are charged to `cc.inflight`, and the peer "never acknowledges them by design" ⇒ **monotonic** growth until `cwnd − inflight` is exhausted and sending stops, with their records accumulating "forever" in `sent_packets`.

**What I verified:** the first half is entirely correct — `cc.on_packet_sent(pn, total_datagram_len, now)` is called **unconditionally** (`connection.rs:689`) even when `ack_eliciting == false`, while the record carries `in_flight: ack_eliciting`, i.e., `false`.

**However**, the loss-detection loop in `on_ack_received` walks **all** of `sent_packets` with no `in_flight` filter:

```rust
for (&pn, record) in &self.sent_packets {
    if pn > largest_pn { continue; }
    if largest_pn >= pn + PACKET_THRESHOLD || time_threshold_exceeded { lost_pns.push(pn); }
}
```

So the ACK-only records **are** eventually deleted as "lost", enter `bytes_lost`, and `cc.on_loss` subtracts them from `inflight`. Therefore:

- The monotonic growth until stall **occurs only when ACKs stop arriving** — not in the general case as the paper describes.
- The "accumulate forever" in `sent_packets` is **inaccurate** on the normal path.

**And most importantly — the defect this correction reveals, detected by no document:**

> **Every ACK-only packet is later falsely declared "lost", which invokes `cubic.on_congestion_event` and reduces `cwnd` by the β = 0.7 factor with no real loss whatsoever.**

The practical impact is harsher than the leak: in a **typical game client** — receiving a world-state stream while sending few inputs ⇒ a high proportion of its packets are ACK-only — the spurious reductions recur constantly. The only brake is the "once per RTT" condition in `on_congestion_event`, i.e., **one spurious reduction every RTT, permanently** ⇒ `cwnd` pinned at `min_cwnd = 2×SMSS` in practice regardless of how clean the network is.

**One fix addresses all three:** exclude ACK-only packets from `inflight` accounting in the CC, **and** filter the loss-detection loop on `record.in_flight == true` (which RFC 9002 §2 mandates anyway: loss is declared only for in-flight packets).

### Other minor corrections to (B)

| Location | The note |
| :--- | :--- |
| §3.2, the header table | `connection_id` is listed with a size of **10 bytes** at offset 2 — the correct value is **8** (the offsets: 0, 1, 2..10, 10..18, 18..22, 22..24). A typo in the table, not in the analysis |
| REC-10 "~34GB" | The figure is correct for a single range with `length = u32::MAX` (4.29×10⁹ × 8 bytes). In practice the process dies at the first range, so the estimate is the most accurate for usage |
| WIR-1 classification | The dual rating (high as a library / low as a service) is accurate and I agree — the decode path never generates an unbounded `usize` |

---

## 7. The unified critical list after the merge

| # | The defect | Source | Impact |
| :--- | :--- | :--- | :--- |
| **1** | Cross-direction nonce reuse from the very first packet | (B) SEC-1 | Collapse of confidentiality **and integrity** — forging authenticated packets with no key knowledge |
| **2** | The broken ACK range encode/decode ⇒ false acknowledgements ⇒ permanent silent loss on the reliable channel, **permanent after the first loss** | (C) + composition with (B) REC-5/REC-8 | The reliable channel is unreliable under loss |
| **3** | Control frames double-wrapped + graceful close never sent | (B) Core-C1 | 6 advertised features dead end-to-end |
| **4** | The replay window updated before AEAD authentication | (A) + (B) SEC-3 + (C) | Permanent connection cutoff with one off-path UDP packet |
| **5** | The XOR cookie ⇒ 24 secret bytes recoverable from one cookie | (A) + (B) SEC-4 + (C) | Collapse of address verification and DoS resistance |
| **6** | Anonymous handshake + no `was_contributory` check | (B) SEC-5/12 + (C) | Full active MITM |
| **7** | Peer-controlled allocation from an ACK frame (~34GB) + no `largest_acked ≤ largest_sent` check | (B) REC-10 + (C) | Process kill / cwnd inflation from a single peer |
| **8** | PTO without record removal, without exponential backoff, without a cap + `ReliableUnordered` without deduplication | (A) + (B) REC-11/ORD-4 + (C) | An escalating storm + duplicate application delivery |
| **9** | A full group aborts the datagram including the ACKs ⇒ livelock | (B) ORD-2 | An endless retransmission loop |
| **10** | An uncoordinated ratchet (KEY_PHASE unused) ⇒ invoking it kills the connection | (B) SEC-6 + (C) | An advertised feature that bricks the connection |
| **11** | Every ACK-only packet is falsely declared lost ⇒ a cwnd reduction every RTT | **new — this report §6** | `cwnd` pinned at the minimum in receiving clients |
| **12** | The silent RX death of the endpoint on the first socket error (ICMP) | (B) CORE-3 | A remote party freezes every connection on the node |

---

## 8. The recommended execution order

The Phase 0 plan in paper (B) is excellent and I adopt it as-is, with **two additions and one reordering**:

**Phase 0 (immediate) — in addition to items (B) 0.1–0.6:**
- `0.7` **Fix the decoder in `loss_detector.rs`** to subtract the `gap` before computing `start`, with a property test for the round-trip over random gap sets. **This item must precede any later performance measurement** — any p99 or goodput figure measured before it is measured on a protocol that loses data.
- `0.8` Fold the `largest_acked ≤ largest_sent` check into item `0.6` (semantic ACK validation) — a single fix closing two holes.

**Phase 1 — in addition to (B) 1.3:**
- Filter the loss-detection loop on `record.in_flight == true` (RFC 9002 §2) in parallel with excluding ACK-only bytes from `cc.inflight` — this addresses the spurious window reduction (§6 above) that the inflight exclusion alone does not.

**Phase 2 — a priority bump:**
- `CORE-3` (socket-error handling) moves from "high" to the top of Phase 2: an ICMP vector makes it remotely triggerable against every connection at once.

**Phase 4** (the seam-test harness) — I fully agree with the conclusion of Appendix (D): this is the highest-return investment in the project. I add the missing scenario to its list:

> **11.** ACK under multiple gaps: generate a receive pattern with random gaps, then assert that the set of packets the sender considers acked equals exactly the received set — this exposes defect #2 in the table above.

---

## 9. Conclusion

Paper (B) is now **the primary reference** for the project: the widest coverage, precise locations, real empirical verification, and an executable remediation plan. Its two discoveries, SEC-1 and Core-C1, exceed in danger everything in my audit, and I verified them directly and confirm them without reservation.

My three remaining contributions:
1. **The ACK range accounting defect** — classified "correct" by the paper while actually broken, and its composition with REC-5/REC-8 makes it permanent after the first loss.
2. **The spurious window reduction from ACK-only packets** — revealed by the correction to the CC-3 mechanism, and detected by no document.
3. **Complementary additions**: the optimistic-ACK attack, the missing black-hole recovery, and the header-protection recommendation.

The shared thesis remains the most accurate description of the situation: **the layers are sound, the seams between them are broken, and the tests are designed so they never pass through the seams.** Every one of the twelve defects above sits on a boundary between two components — including my findings: the ACK encoder ↔ the ACK decoder, and the CC accountant ↔ the loss detector.

**Verification commands for your local environment** (`/home/ggonlinux/GTP/`):

```bash
cd /home/ggonlinux/GTP
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo test --workspace --all-targets
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo clippy --workspace --all-targets -- -D warnings
# First bring the fuzz crate into the workspace (WIR-12), then:
cargo +nightly fuzz run decode_frame -- -max_total_time=600
```
