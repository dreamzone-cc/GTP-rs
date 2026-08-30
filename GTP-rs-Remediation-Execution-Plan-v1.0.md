# GTP-rs Remediation Execution Plan — Comprehensive

| Field | Value |
| :--- | :--- |
| **Binding primary reference** | `GTP-rs-Architecture-Protocol-Audit-Paper-v1.0.md` (the technical audit paper) |
| **Approved amendment** | `GTP-rs-Cross-Audit-Reconciliation.md` — its items 0.7 and 0.8, the §6 addition (the phantom window reduction), and the CORE-3 priority bump are **mandatorily merged** into this plan |
| **Scope** | All findings of the audit paper (~70 defects, 6 phases) — execution starts with Phases 0 and 1 in full plus the high-priority Phase 2 items |
| **Review mechanism** | `GTP-rs-Remediation-Tracker.md` + `scripts/verify_remediation.sh` — see §8 |
| **Plan version** | v1.0 — 2026-08-30 |

---

## 1. Binding working rules

1. **The audit paper is the governing reference**: every work item cites its registry ID (paper §12). Any conflict between the implementation and the paper is resolved in favor of the paper, except where the reconciliation document explicitly amends it.
2. **No item is declared "done" without three pieces of evidence**: (a) the code change merged, (b) a regression test that fails before the fix and passes after (or proves the new property), (c) a green full verification gate (`cargo test --workspace` + clippy).
3. **Every fix is local, not architectural**: no protocol redesign within this plan — the documented exceptions (SEC-5 server authentication, header protection) are classified "deferred by architectural decision".
4. **No regression in existing tests**: a legacy test failing because of a fix (e.g., `assert_eq!(client_key, server_key)`) is rewritten to assert the new correct behavior, with the reason documented.
5. **The verification gate runs before closing any phase**, and the tracker document is updated immediately with status and evidence.

## 2. Phase 0 — stop the security bleeding (critical)

| Item | Fixes | Target files | Execution summary | Acceptance criteria |
| :--- | :--- | :--- | :--- | :--- |
| **P0-1** | SEC-1 | `gtp-crypto/handshake.rs`, `gtp-crypto/kdf.rs`, `gtp-core/state.rs`, `gtp-core/connection.rs`, `gtp-runtime-tokio/endpoint.rs` | Derive **four key materials** (key+IV per direction) with two separate HKDF labels; each endpoint seals with its outbound direction key and opens with its inbound. New `DirectionalKeys` type. The deprecated static path also becomes directional | `assert_ne!` between direction keys; cross-decryption of client traffic with the client key fails (cross test); the live e2e handshake passes |
| **P0-2** | SEC-2 | `gtp-crypto/aead.rs` | Nonce = `IV ⊕ PN_be[0..8]` in bytes 4..12 (all 64 bits). The CID leaves the nonce (the key is already CID-scoped via HKDF info) — removes the overlap | Test: nonce(PN=1) ≠ nonce(PN=2³²+1); AEAD round-trip passes |
| **P0-3** | SEC-3 | `gtp-crypto/replay.rs`, `gtp-core/connection.rs` | Split the window API: `check(pn)` (no commitment) before authentication, `commit(pn)` only after successful AEAD | A forged unauthenticated PN=u64::MAX packet does not burn the window — the subsequent legitimate packet with that PN is accepted |
| **P0-4** | SEC-4 | `gtp-path/stateless_token.rs`, `gtp-path/Cargo.toml` | Replace XOR with `HMAC-SHA256(secret, "GTP-COOKIE-V1"‖addr‖ts)`; the timestamp stays plaintext in bytes 0..8, the remaining 24 bytes are the truncated MAC; **reject future timestamps** (`token_time > now`) | Tests: refreshing an old cookie's timestamp **fails**; recovering secret bytes from a cookie **is impossible** (changing the secret changes the HMAC) |
| **P0-5** | SEC-7/10/12/14 | `Cargo.toml`, `aead.rs`, `handshake.rs` | Enable `x25519-dalek/zeroize`; `checked_add` in the seal/open length checks; enforce `was_contributory()` (`compute_shared_secret` returns `Result`); manually redacted `Debug` for `GtpAeadProtector` and `HandshakeSecret` | Clean build; a low-order key is rejected; `format!("{:?}")` contains no key bytes |
| **P0-6** | REC-10 + reconciliation 0.7/0.8 | `gtp-wire/frame.rs`, `gtp-recovery/loss_detector.rs` | (a) **Fix the range decoder**: apply `gap` before computing `start` (the encoder's convention: the gap precedes the block). (b) Semantic validation: `largest_acked ≤ largest sent PN`, a per-frame acked-count cap, reject underflow | **Property test**: a seeded random gap pattern → the acked set equals the received set exactly; a frame demanding a huge allocation is rejected |

## 3. Phase 1 — revive functional integration

| Item | Fixes | Target files | Execution summary | Acceptance criteria |
| :--- | :--- | :--- | :--- | :--- |
| **P1-1** | Core-C1 + PATH-5 + part of PATH-3 | `gtp-core/control/handle.rs`, `gtp-core/connection.rs`, `gtp-runtime-tokio/endpoint.rs` | A **dedicated control queue** in `ConnectionHot` (owned `Frame` items + an optional destination) spent directly in `produce` ahead of scheduler data; the PathResponse echo is routed to the challenger's `src_addr`; `graceful_close` enqueues the Close **before** the Draining transition, and the TX gate produces throughout Draining until the control queue empties (the loop exits on `Closed` only) | e2e tests: a ping reaches the peer as a Ping frame; Close moves the peer to Draining; a protocol-level path challenge switches `active_path` to the new address |
| **P1-2** | REC-7 + CC-6 + SEC-18 | `gtp-recovery/ack_tracker.rs`, `gtp-cc/cubic.rs`, `gtp-cc/pacing.rs`, `gtp-core/state.rs`, `gtp-core/control/config.rs` | Actually inject `GtpConfig`: `AckTracker::with_policy(freq, max_delay)`; Cubic accepts β/C/IW/min_cwnd/SMSS; Pacing accepts gain/burst; the deprecated `new()` path passes defaults. Delete the dead fields or wire them | Test: a non-default config produces measurably different ACK/cwnd behavior |
| **P1-3** | CC-3 + reconciliation §6 | `gtp-cc/cubic.rs`, `gtp-recovery/loss_detector.rs`, `gtp-core/connection.rs` | (a) ACK-only bytes never enter `cc.inflight`. (b) **Filter the loss-detection loop on `record.in_flight`** (RFC 9002 §2) — prevents the phantom per-RTT window reduction | Tests: 50 ACK-only packets then one ACK — `inflight` does not grow; cwnd is not spuriously reduced |
| **P1-4** | REC-11 + ORD-4 | `gtp-recovery/loss_detector.rs`, `gtp-core/connection.rs` | `on_timeout` drops the records it returns (burst cap = 2, oldest first, ack-eliciting only) + exponential backoff `PTO×2^min(pto_count,8)` capped by `pto_max_duration`; a **delivery index** (bounded FIFO set) blocks duplicate delivery of a `ReliableUnordered` message_id | Simulation test: 100% ACK loss — retransmissions per PTO ≤ 2 and non-escalating; the message is delivered exactly once |
| **P1-5** | ORD-1 + SEM-1 + SEM-2 | `gtp-types/identifiers.rs`, `gtp-scheduler/ordered_group.rs`, `gtp-core/connection.rs` | RFC 1982 for `GenerationId` and for `order_seq` comparisons (explicit modular arithmetic); **receive-side drop-late**: a Data frame with a non-default key and non-zero sequence is checked against an RX-side table before delivery | Tests: u32 wrap for group and generation; delivering 5→3 drops 3 |
| **P1-6** | REC-5 + REC-6 | `gtp-recovery/ack_tracker.rs` | Prune confirmed old intervals (below the sequential delivery point) + when 32 ranges would be exceeded: coalesce the smallest gaps instead of dropping acknowledgements | Test: 100 successive gaps — acknowledgements stay within 32 without silent drops; memory does not grow linearly |

## 4. Phase 2 — high-priority items (executed with Phases 0/1)

| Item | Fixes | Summary | Acceptance criteria |
| :--- | :--- | :--- | :--- |
| **P2-1** | CORE-3 | The RX loop continues on `ConnectionReset/WouldBlock` and logs other errors instead of dying — **promoted per the reconciliation decision** (ICMP is remotely sendable) | A simulated socket error does not kill the endpoint |
| **P2-2** | ORD-2 | A full reorder buffer **isolates the frame** (a dropped counter) without aborting the datagram or skipping `ack_tracker.on_packet_received` | The livelock test: full buffer + retransmission → the packet is acknowledged and progress resumes |
| **P2-3** | PATH-1 + Draining→Closed | Permit `Initial→Closed` and `Draining→Closed` (closing a failed handshake possible + terminating the drain) | `force_close` on a fresh connection succeeds |
| **P2-4** | CORE-4 | Evict the CID table entry on reaching Closed + refuse to replace a live (Established) connection's CID | Eviction and replacement-rejection tests |
| **P2-5** | SEC-6 (safe enablement) | The ratchet rotates **both** direction pairs together, sets the KEY_PHASE bit, and retains the previous RX key for a grace window; documentation states both peers must invoke it together (no wire negotiation yet) | Test: after a synchronized ratchet on both peers, data exchange continues |
| **P2-6** | D-2 + §3.4 ordering | `append_frame` errors are handled: the item is requeued (push-front) instead of being dropped while recording phantom in-flight | Test: a payload near datagram capacity is neither lost nor phantom-recorded |

## 5. Phases 3–5 (tracked, out of scope this round)

| Phase | Items | Initial status |
| :--- | :--- | :--- |
| **3 — scheduler fairness and memory** | SCH-1/2/3/4, the group cap ORD-5, StateTable pruning SEM-5 | Deferred — executed after Phases 0/1 stabilize, by measurement and verification |
| **4 — the seam-test harness** | Scenarios 1–11 (incl. the multi-gap ACK scenario from the reconciliation document) | Partially covered by the regression tests above; the remainder (CLI stress bodies, multiple seeds) deferred |
| **5 — performance** | Remove to_vec, shrink Frame, cached cipher, gtp-io, TX tick | Deferred — after correctness (paper §13 principle) |
| **Deferred by architectural decision** | SEC-5 (server authentication/Finished), header protection (reconciliation recommendation), WIR-6 (header_len extensions), WIR-10 (version negotiation) | Require a documented protocol design decision before implementation |

## 6. Implementation dependency map

```
P0-5 (Cargo zeroize) ─┐
P0-1 (directional) ───┼─→ P2-5 (ratchet) ─→ P1-1 (control) ─→ endpoint TX gate
P0-2 (nonce) ─────────┘
P0-3 (window) ──→ connection.rs RX reorder
P0-6 (ACK decode) ──→ P1-3 (in_flight) ──→ P1-4 (PTO/dedup)
P1-2 (config wiring) ──→ P1-6 (ACK pruning)
P1-5 (RFC1982) independent
```

## 7. Per-step execution and verification sequence

1. Read the target file + the paper's locations (`file:line`).
2. Apply the change + update/add the tests in the same logical commit.
3. `cargo test -p <crate>` for the layer, then `cargo test --workspace` at every convergence point.
4. Update the tracker: status + regression test name + gate result.

## 8. Review and verification mechanism

1. **Tracker document** `GTP-rs-Remediation-Tracker.md`: a matrix of every plan item × (status | regression test | evidence | notes). It is the single authoritative record of execution status.
2. **Verification gate** `scripts/verify_remediation.sh`: fmt + clippy (report) + `cargo test --workspace --all-targets` (hard gate) + procedural grep checks (the key-inequality test exists, no `assert_eq!` on keys, the future-timestamp rejection exists).
3. **Closure rule**: an item without a named regression test in the tracker is not implemented.
4. **Regression rule**: any test failure after merging an item returns the item to `in repair` automatically.
5. The arbiter for any dispute: the audit paper §12 (the registry), then the reconciliation document.
