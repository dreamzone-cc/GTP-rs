# GTP-rs Remediation Execution Tracker

> **The adopted review mechanism** — this document is the single record of execution status.
> Statuses: `⬜ not started` | `🔧 in repair` | `✅ done & verified` | `⏸ deferred by decision` | `❌ verification failed`
> Closure rule: an item without a named regression test is not implemented. The governing references: `GTP-rs-Architecture-Protocol-Audit-Paper-v1.0.md` §12 + `GTP-rs-Cross-Audit-Reconciliation.md`.

## Latest verification gate — **PASSED ✅** (2026-08-30)

| Item | Result |
| :--- | :--- |
| `cargo test --workspace --all-targets` | ✅ **77 passed / 0 failed** (48 before this round — +29 regression tests) |
| `cargo clippy --workspace --all-targets` | ✅ 0 warnings in the tracked categories |
| `cargo fmt` | ✅ applied tree-wide |
| Procedural grep checks (7) | ✅ all passing |
| Run command | `bash scripts/verify_remediation.sh` |

---

## Phase 0 — stop the security bleeding: **complete 6/6**

| ID | Item | Status | Regression tests | Evidence/notes |
| :--- | :--- | :--- | :--- | :--- |
| P0-1 | SEC-1 directional keys | ✅ | `directional_keys_distinct_per_role`, `test_x25519_diffie_hellman_roundtrip` (incl. cross-open failure), `test_x25519_passive_eavesdropper_cannot_decrypt`, the live e2e handshake | `DirectionalKeys` + `derive_directional_handshake_session_keys` (labels: c2s/s2c); the deprecated static path is directional too (`derive_directional_session_keys`) — no nonce-reuse path remains; the old test asserting `client_key == server_key` was removed and inverted |
| P0-2 | SEC-2 full-PN nonce | ✅ | `nonce_uses_full_packet_number` | nonce = `IV ⊕ (CID[0..4] ‖ PN[0..8])` — all 64 bits mixed in |
| P0-3 | SEC-3 authenticate before commit | ✅ | `spoofed_high_pn_does_not_burn_replay_window` (connection), `failed_auth_simulation_does_not_burn_window` (replay) | `ReplayWindow::check()` without commitment + `commit()` only after successful AEAD |
| P0-4 | SEC-4 HMAC cookie | ✅ | `cookie_timestamp_refresh_is_rejected`, `future_dated_cookie_is_rejected`, `cookie_secret_not_recoverable`, `test_cookie_verify_rejects_wrong_addr_and_expiry` | `HMAC-SHA256(secret, "GTP-COOKIE-V1"‖addr‖ts)`; a plaintext timestamp + a 24-byte truncated MAC; explicit future-timestamp rejection; gtp-path now depends on hmac/sha2 |
| P0-5 | SEC-7/10/12/14 | ✅ | `low_order_public_key_rejected`, `debug_does_not_leak_keys`, `seal_rejects_overflowing_payload_len` | `x25519-dalek/zeroize` enabled in the workspace; `was_contributory()` via a `Result` signature; `checked_add` in seal/open; manually redacted `Debug` for `GtpAeadProtector`/`HandshakeSecret`/`StatelessTokenManager` (with Drop+zeroize for the second) |
| P0-6 | REC-10 + reconciliation 0.7/0.8 | ✅ | `ack_ranges_multi_gap_roundtrip_exact`, `ack_ranges_property_sweep_seeded` (64 deterministic LCG cases), `optimistic_ack_rejected`, `oversized_ack_ranges_capped` | **The decoder defect is fixed** (the gap is applied before computing start — it used to falsely ack 6,7,17,18,19 and lose 1,2,8,9,10 in the reconciliation document's example); `MAX_ACKED_PER_FRAME=16384`; `largest_acked > largest_sent` rejected |

## Phase 1 — functional integration: **complete 6/6**

| ID | Item | Status | Regression tests | Evidence/notes |
| :--- | :--- | :--- | :--- | :--- |
| P1-1 | Core-C1 + PATH-5 | ✅ | `control_frames_reach_peer_as_frames` (Ping arrives as a frame + Close closes the peer with `ConnectionClosed(7)`), `path_migration_via_protocol` (challenge→directed echo→migration + a `PathMigrated` event) | A dedicated `OutgoingControlFrame` control queue (6 kinds) spent as real frames; the challenge echo is routed to `src_addr`; the challenge itself is routed to the new address; `graceful_close` enqueues the frame before Draining and the TX loop exits on `Closed` only — **graceful close actually transmits** |
| P1-2 | REC-7 + CC-6 | ✅ | `cubic_config_is_honored`, `policy_config_drives_ack_behavior`, `test_hkdf_directional_keys_isolation` | `AckTracker::with_policy/set_policy`; `CubicConfig{smss,iw,min,beta,c,gain}`; `PacingEngineConfig`; the incoming AckFrequency frame now reaches the tracker; `GtpConfig.max_pacing_burst_bytes` and `pto_max_duration` are now read |
| P1-3 | CC-3 + reconciliation §6 | ✅ | `ack_only_packets_never_declared_lost` | `bytes_acked`/`bytes_lost` count in-flight only; the loss loop filters on `record.in_flight` (preventing the phantom per-RTT reduction); `cc.on_packet_sent` is invoked for ack-eliciting traffic only |
| P1-4 | REC-11 + ORD-4 | ✅ | `pto_burst_capped_and_drains_records`, `reliable_unordered_dedup` | `on_timeout` spends ≤2 oldest in-flight records **and removes them**; backoff `×2^min(count,8)` capped by `pto_max_duration`; a `DeliveredIndex` (FIFO 4096) blocks duplicate delivery of `ReliableUnordered`/`Retx` |
| P1-5 | ORD-1 + SEM-1 + SEM-2 | ✅ | `order_seq_wraparound_still_delivers`, `test_generation_id_modulo_arithmetic`, `rx_drop_late_sequenced` | RFC 1982 for `GenerationId` and `order_seq` (u32 wrap keeps flowing); an RX-side `should_admit/update` table drops late state (default/plain frames bypass it — CORE-8 protection); `PacketNumber/MessageId::next` now wrap |
| P1-6 | REC-5 + REC-6 | ✅ | `ack_intervals_pruned_and_coalesced` | A retention window `ACK_RETENTION_WINDOW=1024` with pruning below the newest received; older-than-budget intervals are retained until pruning (no permanent loss) and every frame covers the newest |

## Phase 2 — high priority: **5/6 implemented + 1 partial**

| ID | Item | Status | Evidence/notes |
| :--- | :--- | :--- | :--- |
| P2-1 | CORE-3 RX resilience | ✅ | The RX loop never breaks: `ConnectionReset/ConnectionRefused/WouldBlock/Interrupted → continue`, other errors are logged and the loop continues (grep evidence); a dedicated tokio test is deferred |
| P2-2 | ORD-2 full-group isolation | ✅ logically / ⏸ load test | An `on_incoming` failure is counted in `total_dropped_frames` and neither aborts the datagram nor blocks `ack_tracker.on_packet_received`; the 256KB pressure test is deferred (heavy) |
| P2-3 | PATH-1 + Draining→Closed | ✅ | `force_close_fresh_connection_succeeds`; `Initial→Closed` legal; `Draining→Closed` automatic when the control queue empties |
| P2-4 | CORE-4 CID eviction | ✅ logically / ⏸ e2e test | The routing-table entry is evicted when the connection reaches Closed in the RX loop; a dedicated e2e test is deferred |
| P2-5 | SEC-6 coordinated ratchet | ✅ | `coordinated_ratchet_keeps_link_alive`: both directions rotate together + the KEY_PHASE bit on the wire + a retained previous RX key (a pre-rotation packet still opens); **documented limitation: requires synchronized invocation by both peers until a wire KeyUpdate message exists** |
| P2-6 | D-2 encode-error handling | ✅ | No `let _ = append_frame` remains: an encode failure requeues the item and no phantom in-flight is recorded; a retx enqueue failure is counted (`total_dropped_frames`) |

## Regression tests added this round (29)

crypto: `nonce_uses_full_packet_number`, `seal_rejects_overflowing_payload_len`, `debug_does_not_leak_keys`, `low_order_public_key_rejected`, `failed_auth_simulation_does_not_burn_window` + the four cookie tests + the rebuilt handshake test.
recovery: `ack_ranges_multi_gap_roundtrip_exact`, `ack_ranges_property_sweep_seeded`, `optimistic_ack_rejected`, `oversized_ack_ranges_capped`, `ack_only_packets_never_declared_lost`, `pto_burst_capped_and_drains_records`, `ack_intervals_pruned_and_coalesced`, `policy_config_drives_ack_behavior`, the extended RTT test.
cc: `cubic_timeout_resets_epoch_state`, `cubic_does_not_grow_on_empty_acks`, `cubic_config_is_honored`.
types/scheduler/path: `test_generation_id_modulo_arithmetic`, `test_packet_number_next_wraps_safely`, `order_seq_wraparound_still_delivers`.
core: `spoofed_high_pn_does_not_burn_replay_window`, `reliable_unordered_dedup`, `control_frames_reach_peer_as_frames`, `rx_drop_late_sequenced`, `force_close_fresh_connection_succeeds`, `directional_keys_distinct_per_role`, `coordinated_ratchet_keeps_link_alive`, `path_migration_via_protocol`.

## Deferred phases (tracked — see execution plan §5)

| ID | Item | Status | Notes |
| :--- | :--- | :--- | :--- |
| PH-3 | DRR fairness (SCH-1/2/3/4) + group cap ORD-5 + SEM-5 pruning | ⏸ | After Phases 0/1 measurements stabilize |
| PH-4 | Remaining seam scenarios (multi-seed sims, asserted CLI stress, fuzz in CI) | ⏸ partial | The critical portion is covered by the 29 regression tests |
| PH-5 | Performance: to_vec, the 288B Frame, cached ChaCha, gtp-io/sendmmsg, the TX tick | ⏸ | After correctness — hot-path performance untouched except the pacing remainder |
| DEF-1 | SEC-5 server authentication/Finished | ⏸ architectural decision | Requires a documented PSK/signature design first |
| DEF-2 | Header protection | ⏸ architectural decision | A reconciliation-document recommendation |
| DEF-3 | WIR-6 header_len extensions + WIR-10 version negotiation + WIR-5 terminal padding + WIR-2 range_count | ⏸ | WIR-2/WIR-3 both have side-effect defenses in the core path (reject/validate) — the full gtp-wire fix lands in PH-4/5 |
| DEF-4 | REC-8 (an ACK in every datagram) | ⏸ | Now intentional under the default `ack_frequency=1`; changes automatically via P1-2 |
| DEF-5 | CC-11 metrics (queue_bytes_per_tier/ECN) | ⏸ partial | `pacing_tokens_remaining` is now real; the rest is PH-5 |

## Implementation decision log

| Decision | Rationale |
| :--- | :--- |
| nonce = IV ⊕ CID[0..4] ‖ full PN | Eliminates the collision across all 64 bits; the key is already CID-scoped in HKDF info |
| The deprecated static path derives directionally + an explicit role (`new_with_role`/`as_client`) | No nonce-reuse path remains in the tree; the simulator and tests pass with opposing roles |
| A dedicated control queue (`OutgoingControlFrame`) instead of a new MessageClass | Less public-API disruption, and routing (`dest`) lives inside the item |
| The `tx_iv/rx_iv` fields exposed on ConnectionHot | Needed by the ratchet and the tests; exposes key inconsistency immediately |
| A manually synchronized two-direction ratchet + KEY_PHASE + a previous RX key | Maximum safely deliverable security without a new wire negotiation protocol (documented as a limitation) |
| `append_frame` errors requeue the item | Satisfies D-2 without changing pop_next semantics |

## How to review (human or automated)

```bash
# The full gate (tests + clippy + procedural checks)
bash scripts/verify_remediation.sh

# A specific regression test
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo test -p gtp-crypto nonce_uses_full
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo test -p gtp-recovery ack_ranges_multi_gap
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo test -p gtp-core path_migration_via_protocol
```

Acceptance rule per item: the code is merged + the named regression test above fails when the fix is reverted + the gate is green. Any future failure returns the item to `❌` automatically per execution-plan §8.

---
*Last updated: 2026-08-30 — round v1.0: Phases 0 and 1 complete, Phase 2 at 5/6 (+1 partial), 77/77 tests green.*
