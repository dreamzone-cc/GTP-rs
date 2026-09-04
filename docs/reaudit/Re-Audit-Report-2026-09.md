# GTP-rs Re-Audit Report — Full Re-Verification of All Tracked Defects

> **Date:** 2026-09-04
> **Audited tree:** `integration/all-fixes` @ `1c0e488` plus the uncommitted New-12 work-tree changes on `fix/New-12-path-response-reflection`; branch `fix/New-8-per-path-validation` @ `7918821` verified in a separate worktree.
> **Methodology:** every status below was established by (a) running the named regression test(s) with the pinned toolchain and (b) inspecting the actual source at the cited lines. Prior audit documents were used only as the checklist of items to re-verify — never as evidence. This report records the **as-found state before this round's remediation**; the post-remediation outcome of each in-scope item is recorded in the Final Closure Matrix (`Closure-Matrix-2026-09.md`).
> **Environment:** cargo/rustc 1.85.0 (pinned by `rust-toolchain.toml`), Linux x86_64. Local rustup shims are broken, so the toolchain binaries were invoked directly and placed first in `PATH` for every command.

---

## 1. Baseline verification (this session, before any change)

| Check | Command | Result |
| :--- | :--- | :--- |
| Workspace tests (current tree incl. New-12 work) | `cargo test --workspace --all-targets` | **95 passed, 0 failed** (+1 doc-test = 96 total) |
| Format | `cargo fmt --all -- --check` | clean |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| Remediation gate | `bash scripts/verify_remediation.sh` | **GATE: PASSED** (95 tests + 7 procedural checks) |
| CI-parity smoke | `sync_game_loop`, `async_tokio_server` examples; `gtp-cli sim-benchmark --ticks 500` | all exit 0; extreme-loss scenario delivers 50/50 |
| New-8 branch tests | same, in worktree @ `7918821` | **111 passed, 0 failed**; fmt + clippy clean |
| New-12 work-tree tests | `cargo test -p gtp-core --lib challenge / queue_drops / many_challenges` | all 5 New-12 tests pass |

Complete ground-truth test inventory at audit time: 84 unit + 12 integration + 1 doc-test across 13 crates (full name list reproduced in Appendix A).

---

## 2. Re-verification matrix — security & path (gtp-crypto, gtp-path, gtp-runtime-tokio)

| ID | Defect (first audit) | Sev | As-found status | Evidence |
| :-- | :--- | :--: | :--- | :--- |
| SEC-1 | One key/IV pair reused in both directions | Critical | **CLOSED (tested)** | `directional_keys_distinct_per_role` PASS; `crypto_security_test` 6/6 PASS; `DirectionalKeys::for_role` with `c2s`/`s2c` HKDF labels — `gtp-crypto/src/handshake.rs:70-96,134-137` |
| SEC-2 | Top 32 bits of PN ignored in nonce | High | **CLOSED (tested)** | `nonce_uses_full_packet_number` PASS; full 64-bit PN XOR — `gtp-crypto/src/aead.rs:30-42` |
| SEC-3 | Replay window burned before AEAD auth | Critical | **CLOSED (tested)** | `spoofed_high_pn_does_not_burn_replay_window` + `failed_auth_simulation_does_not_burn_window` PASS; check at `connection.rs:307-310` (pre-open), commit only on Ok at `:361-364`; split API `replay.rs:23/51` |
| SEC-4 | Stateless cookie XOR-forgable, unauthenticated timestamp | Critical | **CLOSED (tested)** | 4 `cookie_*` tests PASS; HMAC-SHA256 `"GTP-COOKIE-V1"`, future-ts rejection, `ct_eq` — `gtp-path/src/stateless_token.rs:35-74` |
| SEC-5 | Anonymous DH, no server authentication (active MITM) | High | **DEFERRED (by decision DEF-1/SEC-A7)** | Still unauthenticated: `endpoint.rs:433-454` matches ServerHello only by assigned CID; no PSK/signature anywhere. Deferred with documented rationale; also tracked as X-13 input |
| SEC-6 | Key ratchet uncoordinated on wire | High | **CLOSED as safe-enablement; in-band FR-6 remains open** | `coordinated_ratchet_keeps_link_alive` PASS; KEY_PHASE-driven RX key selection `connection.rs:324-337`; 256-datagram prev-key grace `state.rs:87`; lockstep-by-agreement limitation documented `state.rs:368-370` |
| SEC-7 | x25519-dalek zeroize disabled | High | **CLOSED (code)** | workspace `Cargo.toml` features `["static_secrets","zeroize"]`; `ZeroizeOnDrop` on `GtpAeadProtector` (`aead.rs:18`), `HandshakeSharedSecret` (`handshake.rs:9`), `HandshakeSecret` Drop (`kdf.rs:30-34`) |
| SEC-8 | Deprecated static master-secret path | High | **PARTIAL — decision SEC-A8 open** | Deprecated constructors present with hardcoded secret `b"gtp_default_session_master_secret_2026"` (`state.rs:173-185`), still used by gtp-sim (`sim_runner.rs:27,29`) and insecure endpoint connect (`endpoint.rs:95-97`) |
| SEC-9 | Proof keyed by AEAD traffic key (no separation) | Medium | **CLOSED (code)** — better than tracker claimed | `confirmation_key()` = HKDF with dedicated salt `b"GTP_V1_1_CONFIRM_SALT"`; proof HMACs with derived key, not traffic key — `handshake.rs:169-196` |
| SEC-10 | `payload_len + TAG` overflow panic | Medium | **CLOSED (tested)** | `seal_rejects_overflowing_payload_len` PASS; `checked_add` — `aead.rs:64-66` |
| SEC-11 | ClientHello rate limit 1000/s vs documented 20/s | Medium | **CLOSED (code)** | per-IP limiter, 1s window, `<= 20` — `endpoint.rs:351-361` |
| SEC-12 | No `was_contributory()` check | Medium | **CLOSED (tested)** | `low_order_public_key_rejected` PASS; `handshake.rs:58-60` |
| SEC-13 | Version not negotiated/bound (downgrade) | Medium | **OPEN (deferred DEF-3)** | decoded version never compared (`header.rs:172-179`); ClientHello version ignored (`endpoint.rs:343-347`) |
| SEC-14 | Derived Debug leaks raw keys | Med/Low | **PARTIAL** — scope narrower than claimed | aead redacted (`debug_does_not_leak_keys` PASS). Still leaking: `DirectionalKeys` (`handshake.rs:70`) and `SessionDirectionalKeys` (`kdf.rs:76`) derive raw `Debug`. `ConnectionHot` derives no Debug at all (contrary to the tracker), so its key fields cannot leak |
| SEC-15 | Anti-amp bytes counted pre-auth | Medium | **READY-IN-BRANCH** | current tree counts on arrival pre-auth (`connection.rs:295-297`); New-8 branch moves `on_bytes_received` into the post-AEAD branch (`connection.rs:360-380`) with test `unauthenticated_datagrams_earn_no_anti_amplification_budget` PASS |
| SEC-16 | Anti-amp not path-scoped, no migration reset | Medium | **READY-IN-BRANCH (= New-8)** | per-probe budget slot (`state.rs:151-152`), RX credit follows `src_addr`, PATH_RESPONSE promotes probe (`connection.rs:379-392,608-613`); 5/5 branch tests PASS |
| SEC-17 | `PacketNumber::next` unchecked add | Medium | **CLOSED (tested)** | `test_packet_number_next_wraps_safely` PASS; `wrapping_add` — `identifiers.rs:50-54` |
| SEC-18 | `replay_window_size` config ignored | Low | **CLOSED (code)** | config consumed in state construction |
| SEC-19 | Plaintext protector skips length validation | Low | **OPEN** | `plaintext.rs:9-18` returns `Ok(payload_len)` with no bounds check |
| PATH-1 | Initial→Closed illegal transition | Medium | **CLOSED (tested)** | `force_close_fresh_connection_succeeds` + `test_valid_and_invalid_state_transitions` PASS; `state_machine.rs:33` |
| PATH-2 | Anti-amplification effectively disabled | Medium | **PARTIAL → fixed in New-8 branch** | current tree: TX gate real (`connection.rs:988-997`) but every construction site passes `pre_validated=true` (`endpoint.rs:188-194,505-511`) so the 3x cap never binds in production; New-8 branch enforces per-path probe budgets (tests PASS) |
| PATH-3 | Challenge timeout consulted only on response | Medium | **OPEN (residual X-8)** | 3s timeout read only inside `validate_response` (`path_validator.rs:42`); single pending slot (`:15`), no timer/retry/second slot |
| PATH-4 | Two sources of truth for active_path | Low | **CLOSED (code)** | `ConnectionHot.active_path` sole owner (`state.rs:126`), assigned in exactly one runtime place (`connection.rs:611`) |
| PATH-5 | Challenge echo sent to active_path, not challenger | Low/Crit | **CLOSED (tested)** | `path_migration_via_protocol` PASS; echo queued with `dest=src_addr` (`connection.rs:588-593`) |
| R-6 | Previous RX key never retired | Medium | **CLOSED (tested)** | `previous_rx_key_is_retired_after_grace_window` + `outbound_traffic_does_not_age_the_rx_key_grace_window` PASS; `tick_rx_key_grace` retires+zeroizes (`state.rs:424-433`), ticked on RX only |
| R-8 | Blind dual-try key open | Low | **CLOSED (code)** | KEY_PHASE bit selects key first, single fallback (`connection.rs:317-359`); `failed_open_leaves_buffer_intact` PASS |

## 3. Re-verification matrix — recovery & congestion (gtp-recovery, gtp-cc)

| ID | Defect | Sev | As-found status | Evidence |
| :-- | :--- | :--: | :--- | :--- |
| REC-1 | ack_delay subtracted unconditionally | High | **CLOSED (tested)** | `test_rtt_stats_update` PASS; clamp `rtt.rs:44-47` (RFC 9002 §5.3) |
| REC-2 | min_rtt from adjusted sample | Medium | **CLOSED (tested)** | raw-sample min_rtt before adjustment (`rtt.rs:39-40`), asserted in test |
| REC-3 / N-5 | `min_rtt = u64::MAX` sentinel leaks | Medium | **OPEN — confirmed** | sentinel `rtt.rs:23`; leaks into `calculate_backpressure` (`handle.rs:150-155`, inflation≈0 pre-sample) and metrics→CLI display (`main.rs:222,433,545` prints ≈18446744073709s) |
| REC-4 | No PTO granularity floor | Low | **CLOSED (tested)** | 1ms floor `rtt.rs:84` |
| REC-5 | ACK intervals never pruned | High | **CLOSED (tested)** | `ack_intervals_pruned_and_coalesced` PASS; retention 1024 (`ack_tracker.rs:14,115-121`) |
| REC-6 | 32-range cap silently drops | High | **CLOSED (tested)** | capped at 32 via truncation + window prune (`ack_tracker.rs:210`); note: enforcement is truncate-to-32, `merge_adjacent` merges truly adjacent intervals only |
| REC-7 | ACK policy hardcoded, negotiation no-op | High | **CLOSED (tested)** | `policy_config_drives_ack_behavior` PASS; `with_policy/set_policy` (`ack_tracker.rs:56-68`), consumed from AckFrequency frame (`connection.rs:629-632`) |
| REC-8 | ACK in every datagram | Medium | **DEFERRED (intentional)** | default preset `ack_frequency_packets: 1` (`config.rs:48,61`); per-packet ACK is a deliberate low-latency choice, changeable via policy |
| REC-9 | ack_delay_us truncated to u32 | Low | **OPEN** | `ack_tracker.rs:195` |
| REC-10 / Recon-0.7 / Recon-0.8 | ACK ranges unvalidated (34GB alloc); decoder gap; optimistic ACK | Critical/High | **CLOSED (tested)** | `oversized_ack_ranges_capped`, `optimistic_ack_rejected`, `ack_ranges_multi_gap_roundtrip_exact`, `ack_ranges_property_sweep_seeded` all PASS; cap 16384/frame (`loss_detector.rs:12,167-169`), `largest_acked>largest_sent` reject (`:192-207`) |
| REC-11 | PTO re-enqueue storm | High | **CLOSED (tested)** | `pto_burst_capped_and_drains_records` PASS; burst cap 2 oldest + backoff 2^8 (`loss_detector.rs:14,125-134,343-382`) |
| REC-12 | No loss timer state machine | High | **OPEN** | loss declared only inside `on_ack_received` (`loss_detector.rs:269-312`); no timer anywhere |
| REC-13 | DeliveryRateSample discarded | Medium | **OPEN** | computed `loss_detector.rs:247-267`, dropped at `connection.rs:407` |
| REC-14 / FR-3 / A-3 | Duplicate in-flight accounting | High | **CLOSED (tested)** | `inflight_is_a_single_source_and_pto_drain_sheds_it`, `pto_drain_reports_bytes_for_congestion_settlement`, `ack_only_packets_never_declared_lost` PASS; no CUBIC mirror (`cubic.rs:171-173`) |
| CC-1 | Timeout epoch not reset | High | **CLOSED (tested)** | `cubic_timeout_resets_epoch_state` PASS (`cubic.rs:206-218`) |
| CC-2 | cwnd grows on empty ACKs | High | **CLOSED (tested)** | `cubic_does_not_grow_on_empty_acks` PASS (`cubic.rs:182-184`) |
| CC-3 | Phantom in-flight leak / spurious reduction | High | **CLOSED (tested)** | via `ack_only_packets_never_declared_lost`; `in_flight` filter (`loss_detector.rs:220-223,288`) |
| CC-4 / FR-4 / A-4 | CC RTT is a raw sample; duplicate active_path | Medium | **CLOSED (tested)** | `controller_rtt_mirrors_the_single_rtt_source_after_ack` PASS; `on_rtt` store (`cubic.rs:199-204`), sync (`connection.rs:418-420`) |
| CC-5 | W_tcp uses min_rtt, no expiry | Medium | **CLOSED (code) — tracker was stale** | W_tcp uses `smoothed_rtt` (`cubic.rs:106-111`); `min_rtt` absent from cubic.rs entirely |
| CC-6 | CC knobs dead | Medium | **CLOSED (tested)** | `cubic_config_is_honored` PASS (`cubic.rs:70-89`) |
| CC-7 | W_cubic(t) not W_cubic(t+RTT) | Low | **OPEN** | `cubic.rs:100-104` |
| CC-8 | No HyStart / idle restart / max clamp | Low | **OPEN** | absent (only min_cwnd clamp `cubic.rs:117`) |
| CC-9 | Pacing fractional truncation | Medium | **CLOSED (tested)** | `test_pacing_tokens_accumulation_and_consumption` PASS; remainder carry (`pacing.rs:57-59`) |
| CC-10 | Backpressure no hysteresis; min_rtt sentinel disables axis | Medium | **OPEN** | pure threshold compare (`backpressure.rs:29-39`); sentinel division (`:23-27`) |
| CC-11 | Hardcoded metrics | Low | **PARTIAL** | real: RTT/cwnd/inflight/tokens/counters (`handle.rs:157-186`); hardcoded 0: `queue_bytes_per_tier` (`:171`), ECN counters (`:183-185`) |
| CC-12 / X-14 | `on_ecn` never called; ECN dead end-to-end | Low | **OPEN** | trait method defined (`controller.rs:14`, `cubic.rs:193-197`), zero call sites; `udp.rs:66` `ecn = 0` |
| FU-4 / N-4 | PTO treated as RTO; inert `cc.on_loss` calls + stale comments | High | **OPEN** | PTO sweep calls `cc.on_timeout` (`connection.rs:773`) collapsing cwnd to min (`cubic.rs:210-211`); `cc.on_loss(&loss_ev)` with empty event = inert no-op (`connection.rs:778`); stale R-1 comments (`:774-777`) |

## 4. Re-verification matrix — scheduler, semantics, ordered delivery (gtp-scheduler, gtp-core)

| ID | Defect | Sev | As-found status | Evidence |
| :-- | :--- | :--: | :--- | :--- |
| SCH-1 / N-3 | DRR degenerates to strict priority | High | **OPEN** | P0 drained first (`scheduler.rs:114-125`); fixed tier order every call with immediate return (`:128-160`), no persistent round index |
| SCH-2 / X-19 | Uncapped deficit accrual | Medium | **OPEN** | `deficit += weight*100` every visit (`scheduler.rs:143`), no cap |
| SCH-3 | P0 no share cap (reflection echo storm) | Medium | **PARTIAL** | New-12 caps PathResponses (1/datagram, 2 pending — `state.rs:107,120`, tests PASS); P0 tier itself byte-capped only (`scheduler.rs:47-48`) |
| SCH-4 / FR-7 / A-8 | O(n) enqueue byte accounting | Medium | **OPEN** | `iter().map(size_bytes).sum()` per enqueue/requeue (`scheduler.rs:47,97`) |
| SCH-5 | Tier capacity counts expired items | Low | **OPEN** | capacity sums include not-yet-pruned stale items (`scheduler.rs:47-48` vs `effective_queue_bytes :175-182`); prune only in produce (`connection.rs:803`) |
| SCH-6 | "P4 shed first" advertised, no mechanism | Low | **OPEN** | BackpressureLevel never consulted by scheduler (`connection.rs:653-662` events only) |
| SEM-1 / ORD-1 | Plain integer compares instead of RFC 1982 | High | **CLOSED (tested)** | `test_generation_id_modulo_arithmetic`, `order_seq_wraparound_still_delivers` PASS; half-space compare (`identifiers.rs:143-146,175-178`; `ordered_group.rs:29-33`) |
| SEM-2 | No RX drop-late for UnreliableSequenced | High | **CLOSED (tested)** | `rx_drop_late_sequenced` PASS; `should_admit` gate (`connection.rs:456-470`) |
| SEM-3 | `supersedable` written, never read | Medium | **OPEN** | written at `semantics.rs:66,74,90-91` / `item.rs:11` / `connection.rs:168,203,232,275,696` — zero reads |
| SEM-4 | Eviction searches only new item's tier | Medium | **OPEN** | `retain` on `queues[tier_idx]` only (`scheduler.rs:74`) |
| SEM-5 | StateTable never pruned | Low | **CLOSED (tested)** | `state_table_is_bounded_under_adversarial_keys` PASS; FIFO 4096 (`state_table.rs:11,56-64`) |
| ORD-2 | Full reorder buffer aborts whole datagram | Critical | **CLOSED (code)** | frame isolated: dropped frame counted `total_dropped_frames+=1`, datagram continues (`connection.rs:512-528`) |
| ORD-3 | Silent reliable enqueue loss | High | **CLOSED (code)** | retx enqueue failures counted (`connection.rs:442-444,794-796`); first-send failures propagate `Err` to caller |
| ORD-4 | ReliableUnordered no dedup | High | **CLOSED (tested)** | `reliable_unordered_dedup` PASS; `delivered_index.insert_if_new` (`connection.rs:494-501`) |
| ORD-5 / FR-2 / A-2 | Ordered-group map unbounded (16GB) | High | **CLOSED (tested)**; **FU-5 OPEN** | `ordered_group_map_stays_bounded_under_many_wire_group_ids` PASS; cap 256 (`state.rs:97`) with FIFO eviction (`:377-391`); eviction order is FIFO-by-creation, **not** LRU — and the comment at `state.rs:374-375` incorrectly claims idle-longest-first (FU-5 confirmed) |
| ORD-6 / FR-5 / A-6 | No gap timeout in reorder store | Medium | **OPEN** | `on_incoming(order_seq, payload)` takes no time parameter (`ordered_group.rs:35`) |
| ORD-7 / FR-8 | Batch delivery labeled with triggering frame's order_seq | Medium | **OPEN** | ready items all labeled with incoming frame's order_seq (`connection.rs:514-523`); `on_incoming` returns bare payloads (`ordered_group.rs:35,41-54`) |
| N-7 | Zero-byte items bypass byte caps | Medium | **OPEN** | byte-only tier caps (`scheduler.rs:47-52`), `size_bytes = payload.len()` (`item.rs:24-26`); reorder buffer bytes-only (`ordered_group.rs:57,66-67`) |

## 5. Re-verification matrix — wire, core, runtime (gtp-wire, gtp-core, gtp-runtime-tokio)

| ID | Defect | Sev | As-found status | Evidence |
| :-- | :--- | :--: | :--- | :--- |
| WIR-1 | Encode-side `offset + 4 + padding_len` overflow | High (lib) | **OPEN** | unchecked adds `frame.rs:378` (+ same pattern 390, 407, 423, 440, 457); `codec.rs:78` |
| WIR-2 | ACK `range_count` written raw — encoder emits frame its own decoder rejects | High | **OPEN** | raw byte write `frame.rs:231` vs clamp at `:219/:234` and decoder reject >32 (`:488`); roundtrip test only uses 2 ranges |
| WIR-3 | Silent `payload_len as u16` truncation | Medium | **CLOSED (code)** | reject instead of truncate (`frame.rs:258`; `connection.rs:956-960` with requeue) |
| WIR-4 | Close reason truncated mid-UTF-8 | Medium | **OPEN** | byte-slice cut at 255 without char boundary (`frame.rs:389,397-398`); peer decode fails `from_utf8` (`:619-620`) |
| WIR-5 | Padding/MtuProbe swallow trailing frames | Medium | **DEFERRED (DEF-3)** | terminal frames set `offset = buf.len()` (`frame.rs:604-605,704-705`) |
| WIR-6 | `header_len` written but unenforced | Medium | **DEFERRED — decode side actually validates** (tracker stale) | decode enforces bounds and honors it (`header.rs:184-221`); residual: encode writes raw `header_len` (`:136`) without cross-check against bytes written |
| WIR-7 | Frame ≈288B, double move | Medium | **OPEN (perf)** | `[AckRange; 32]` = 256B in Ack variant; `#[allow(clippy::large_enum_variant)]` (`frame.rs:34,39`) |
| WIR-9 | Unchecked `+1` on ids | Low | **CLOSED (tested)** | `test_packet_number_next_wraps_safely` PASS; wrapping/saturating (`identifiers.rs:50-53,82-83,118-119,148-149`) |
| WIR-11 | VarInt dead surface | Doc/Low | **OPEN** | zero protocol usage outside `varint.rs` + re-exports |
| WIR-12 | Fuzz crate never built/run | Medium | **PARTIAL — claim outdated** | `.github/workflows/ci.yml:64-92` `fuzz_smoke` job builds + runs both targets 120s on nightly (non-blocking) on every push; not built locally (no nightly installed) |
| Core-C1 | Control frames double-wrapped in Data — Ping/Close/PathChallenge dead | Critical | **CLOSED (tested)** | `control_frames_reach_peer_as_frames` PASS; `OutgoingControlFrame` queue drained as real frames (`connection.rs:882-909,1042-1048`) |
| CORE-2 / FR-1 / A-1 | Oversized payload stalls tier forever | High | **CLOSED for rejection (tested); fragmentation absent (X-10 deferred)** | `oversized_payload_is_rejected_and_never_stalls_a_tier` PASS; `fragment_id`/`total_fragments` hardcoded decorative (`connection.rs:1111-1147`) |
| CORE-3 | RX loop dies on first socket error | High | **CLOSED (code)** | continues on ConnectionReset/Refused/WouldBlock/Interrupted and logs others (`endpoint.rs:304-317`) |
| CORE-4 | CID table eviction / duplicate CID | Medium | **PARTIAL** | eviction on Closed (`endpoint.rs:553-559`) CLOSED; **TX-task leak OPEN**: re-insert at `:519` replaces map entry without stopping old `spawn_tx_loop` task |
| CORE-5 / N-2 | RX head-of-line blocking on slow consumer | High | **READY-IN-BRANCH** | current: `tx.send(msg).await` under lock (`endpoint.rs:547-550`); New-8 branch: `try_send` + isolation, `slow_consumer_test` 6/6 PASS |
| CORE-6 | Hot/cold cache-line claim cosmetic | Medium | **OPEN (doc)** | no `repr(align)` anywhere in `state.rs` |
| CORE-7 | `unwrap_or(32)` masks encode failure | Medium | **OPEN** | `endpoint.rs:415`; magic 32 ≠ MIN_LONG_HEADER_LEN (28) |
| CORE-8 | Plain Unreliable indistinguishable on RX | Low | **CLOSED (code)** | `is_plain_unreliable` bypasses RX state table both admission and update (`connection.rs:459-475`) |
| CORE-9 | Pending-handshake cleanup only on next hello | Low | **PARTIAL — better than tracker** | pending map cleaned on timeout/channel-drop/receipt (`endpoint.rs:141-150,446`); **residual OPEN**: `hello_rate_limiter` per-IP entries never expire (`endpoint.rs:44,352-361`) |
| D-2 | Dropped frames still recorded in-flight | Medium | **CLOSED (tested)** | `drained_control_is_restored_when_the_datagram_is_rejected` PASS; requeue on every post-drain rejection (`connection.rs:948,958,980,991`) |
| R-1 | PTO sweep never settles in-flight debt | Critical | **CLOSED (tested)** | `pto_drain_reports_bytes_for_congestion_settlement` + `inflight_is_a_single_source_and_pto_drain_sheds_it` PASS |
| R-2 | StateTable unbounded | High | **CLOSED (tested)** | `state_table_is_bounded_under_adversarial_keys` PASS |
| R-3 | ACK consumed even if datagram rejected | Medium | **CLOSED (tested)** | `peek_does_not_consume_pending_ack_state` PASS; `commit_ack_sent` after seal/send (`connection.rs:1027-1031`) |
| R-4 / R-5 | Post-pop failures lose items | Medium | **CLOSED (code)** | `requeue_popped` on all failure exits (`connection.rs:925,949,959,981,992`); budget-bounded pop (`:915-916`) |
| R-7 | Directed datagram carries ACK | Medium | **CLOSED (tested)** | `directed_control_frame_carries_no_ack` + `directed_control_behind_a_frame_still_carries_no_ack` PASS |
| N-1 | ~1.17% silent data-datagram loss (handshake-type ciphertext misroute) | Critical | **READY-IN-BRANCH** | current: cleartext first-byte inspection ungated (`endpoint.rs:335-338`); New-8 branch gates on `header.flags.is_long_header()` (`endpoint.rs:59-61`); branch crate tests 13 passed + 1 volume test `#[ignore]`d by design |
| FR-5 | Closed connection keeps decrypting/dispatching | Medium | **OPEN** | `handle_incoming_datagram` begins with counters→header→CID→replay; no ConnectionState gate (`connection.rs:287-310`) |
| FR-6 / A-7 | Ratchet not coordinated in-band; `packets_since_ratchet` dead | Medium | **OPEN (residual)** | counter defined/init/reset/incremented but never read to trigger (`state.rs:169,364,418`; `connection.rs:1023`); ratchet only via control API (`handle.rs:142`); limitation documented (`state.rs:368-370`) |
| X-1 | `min_rtt` not reset on path migration | P0-engine | **READY-IN-BRANCH** | current tree: zero `reset_for_new_path`; New-8 branch: `rtt.rs:62`, `loss_detector.rs:99` (`on_path_migration`), called on validated migration (`connection.rs:625`); branch tests incl. `min_rtt_follows_the_new_path_after_migration` PASS |
| X-2 | AAD spec/impl contradiction | P0-doc | **OPEN** | `docs/specs/GTP-SEC-01.md:13-14` omits fields; actual AAD = full header slice incl. `timestamp_micros` (`connection.rs:314,968-972`) |
| X-3 | Nonce description inaccurate; INV-9 undocumented | P1-doc | **OPEN** | nonce uses full CID+PN64 (`aead.rs:30-42`) — in-code comment outdated |
| X-4 | `timestamp_micros` dead on RX | P1 | **OPEN** | written TX; sole reader is CLI dissect (`main.rs:108`); zero RX consumers |
| X-8 / PATH-3 | Single challenge slot, no retry/cancel | P0-engine | **OPEN** | `pending_challenge: Option<…>` single slot (`path_validator.rs:15`), silent overwrite (`:25-26`) |
| New-8 / A-5 / SEC-15/16 | Per-path post-auth anti-amplification | High | **READY-IN-BRANCH** | see SEC-15/SEC-16 rows; 5/5 branch tests PASS |
| New-12 | PathChallenge reflection amplifier | High | **FIXED IN WORK-TREE (uncommitted)** | caps `MAX_PATH_RESPONSES_PER_DATAGRAM=1` / `MAX_PENDING_PATH_RESPONSES=2` (`state.rs:107,120`); answer-policy active-path-only; 5/5 tests PASS; **stale doc comment at `state.rs:111-113`** claims "two addresses" — to be corrected before commit |

## 6. Discrepancies found versus the prior audit/tracker claims

1. **CC-5 is CLOSED, not partial** — W_tcp uses `smoothed_rtt`; `min_rtt` does not exist in cubic.rs at all (`cubic.rs:106-111`).
2. **WIR-12 is partially outdated** — CI does build and run both fuzz targets (non-blocking nightly job, `ci.yml:64-92`). The residual gap is local-only.
3. **CORE-9 is partially fixed** — pending handshake map IS cleaned on failure paths; only `hello_rate_limiter` grows unboundedly per unique source IP.
4. **WIR-6 decode side is sound** — decode validates bounds and honors `header_len`; the residual defect is encode-side cross-checking only.
5. **SEC-14 scope is narrower** — `ConnectionHot` derives no Debug (cannot leak); only `DirectionalKeys` and `SessionDirectionalKeys` still leak via derived Debug.
6. **PATH-2 neutralized in production until New-8 merges** — the 3x limiter exists but every endpoint construction site passes `pre_validated=true`, so it never binds; New-8 branch makes it real per-path.
7. **FU-5 confirmed with misleading comment** — `state.rs:374-375` claims idle-longest-first eviction; implementation is FIFO-by-creation.
8. **N-6 confirmed live** — running `gtp-cli dissect` on README's own example fails with `Invalid header length` (example is 24B header_len; long header requires ≥28). Preset β=0.75 (`config.rs:69`) contradicts docs/default 0.7 — both directions need alignment.
9. **Line-number drift** — PTO block moved to `connection.rs:763-800` (was 722-723 in tracker).

## 7. Items in scope for this remediation round (per approved plan)

Merging (Phase 1): New-12 commit → New-8 branch (N-1, N-2, X-1, A-5/SEC-15, New-8) → New-12.
Code fixes: N-3, FR-7, N-7 (scheduler); N-4/FU-4, FR-8, N-5, FU-5, FR-5 (recovery/integrity); WIR-2, WIR-4, SEC-14 (wire/crypto hygiene).
Documentation: N-6 (README/CLI hex, β, StressSuite verdicts), X-2 (GTP-SEC-01 AAD), X-3 (nonce note), CHANGELOG.
Test infrastructure: 5 cross-layer integration scenarios + unit gap-fill + 3× reproducibility gate.

## 8. Deferred register (documented, with rationale — not fixed this round)

| Item | Rationale |
| :--- | :--- |
| SEC-5 / SEC-A7 (server authentication) | Architectural decision DEF-1: requires PSK/signature design; tracked for a dedicated security milestone |
| SEC-8 / SEC-A8 (static master-secret removal) | Affects gtp-sim determinism; needs a migration design for sim-only keys before removal |
| CORE-2 / X-10 (fragmentation) | Large feature (PLPMTUD + fragmentation); current contract rejects >MTU at API — acceptable, documented |
| WIR-5, WIR-6-residual, WIR-10 / SEC-13 (wire deferred set DEF-3) | Wire-format governance: any change requires version negotiation design |
| REC-12 (loss timer), REC-13 (rate sample), REC-9 (u32 ack_delay) | Recovery refinements below criticality bar; PTO path currently drains correctly |
| CC-7, CC-8, CC-10, CC-12/X-14 (ECN) | CC enhancements / ECN requires OS + runtime support; engine agenda |
| SEM-3, SEM-4, SCH-5, SCH-6 | Scheduler/semantics refinements; supersession currently functions per-tier |
| CORE-4 TX-task leak, CORE-9 rate-limiter map, CORE-7 | Runtime hygiene, low severity, bounded impact |
| X-4, X-8 (timestamp consumption, multi-slot challenges) | Inputs to the future routing-engine build-out (RE agenda, gates G1+) |
| WIR-7, WIR-11, CORE-6, CC-11, X-18 | Performance/documentation cleanups |
| REC-8 | Intentional design (per-packet ACK default, policy-changeable) |

---

## Appendix A — ground-truth test inventory (as of audit)

**gtp-types (4):** test_generation_id_modulo_arithmetic, test_packet_number_next_wraps_safely, test_state_key_encoding, test_state_sequence_modulo_arithmetic
**gtp-wire (7 + 3 integ):** varint roundtrips; short/long header roundtrips; ACK / data+reliable / control frame roundtrips; builder+iterator roundtrip; truncated frames/header no-panic; 1000-iter mutation sweep
**gtp-recovery (15):** rtt stats update; ack tracker contiguous/sparse; prune+coalesce; policy-driven ACK; non-destructive peek; packet-threshold loss; multi-gap ACK exact; seeded property sweep; optimistic-ACK reject; oversized ranges capped; ack-only never lost; PTO burst cap + drain; PTO settlement reports; single in-flight source
**gtp-cc (5):** slow-start+loss reduction; timeout epoch reset; no growth on empty ACKs; config honored; pacing token accumulation
**gtp-scheduler (6):** stale drop/deadline; state supersession eviction; bounded state table; ordered group in/out-of-order; order_seq wraparound
**gtp-path (7):** 3x anti-amp boundary; path validation + NAT rebinding; valid/invalid transitions; cookie secret/refresh/future/wrong-addr-expiry (4)
**gtp-crypto (12 + 6 integ):** AEAD seal/open, tamper, nonce-PN64, overflow reject, failed-open intact, redacted Debug; x25519 roundtrip, low-order reject; replay window (2); HKDF isolation; plaintext passthrough; integration: multi-connection entropy, directional isolation, protector dispatch, eavesdropper, ratchet forward security, active MITM rejected
**gtp-core (22):** send/receive pipeline; oversized payload rejection; bounded ordered-group map; RTT mirror; spoofed high-PN; late sequenced drop; force close; directional keys per role; coordinated ratchet; path migration; reliable-unordered dedup; control frames as frames; prev-key retirement; outbound doesn't age grace; directed no-ACK (×2); drained-control restore; New-12 challenge matrix (5)
**gtp-io (1):** UDP loopback batch
**gtp-runtime-tokio (1 + 2 integ):** endpoint e2e; handshake e2e (dynamic accept + 10-client fan-in)
**gtp-sim (2):** reliable-ordered recovery under 20% loss; 60 FPS sequenced streaming
**gtp SDK (1 + doc-test):** facade roundtrip
