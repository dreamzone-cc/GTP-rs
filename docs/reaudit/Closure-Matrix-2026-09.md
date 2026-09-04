# GTP-rs Final Closure Matrix — 2026-09 Remediation Round

> **Scope:** outcome of every in-scope item from the approved master remediation plan
> (2026-09-04), with the verifying test or command actually run. The as-found state of
> every tracked defect (~100 items across all audit families) is recorded in
> `Re-Audit-Report-2026-09.md`; this matrix records the post-remediation outcome.
> **Environment:** cargo/rustc 1.85.0 (pinned), Linux x86_64.

---

## 1. Master-plan closure table (§5 of the adopted plan)

| Action | Target criterion | Verifying tool (run 2026-09-04) | Result |
| :--- | :--- | :--- | :---: |
| Branch merges + N-1/N-2 | Zero data-datagram drops; zero RX head-of-line blocking | `cargo test -p gtp-runtime-tokio` (15 passed incl. `slow_consumer_test` 6/6); volume gate `cargo test -p gtp-runtime-tokio --test n1_routing_gate_test -- --ignored` → 1 passed (5000-datagram loopback, <0.05% loss) | ✅ |
| Scheduler N-3 | Fair shares; no P3/P4 starvation | `test_drr_fairness_under_p1_saturation` (exact 350/150/50 window) + `p3_is_served_within_the_first_round_under_p1_saturation` (first P3 item at pop 36 of 1000) + SDK-level `p3_and_p4_traffic_flows_while_p1_is_saturated` | ✅ |
| PTO RFC 9002 discipline | cwnd stable on single PTO probe | `test_pto_probe_does_not_collapse_cwnd` (rounds 1–2 unchanged; round 3 collapses) | ✅ |
| FR-8 order sequences | Each message carries its true sequence | `test_ordered_group_drain_preserves_distinct_order_seq` + `full_recovery_path_delivers_ordered_stream_after_loss_and_pto` (delivers `[0..5]`, own seqs) | ✅ |
| Wire + crypto fixes | Encoding integrity; keys redacted in logs | `cargo test -p gtp-wire -p gtp-crypto` → 23 passed incl. `ack_frame_with_uncapped_range_count_encodes_self_consistently`, `close_reason_truncates_at_a_utf8_char_boundary`, both `debug_does_not_leak_*` tests | ✅ |
| Full quality gate | fmt + clippy + all tests + procedural checks | `bash scripts/verify_remediation.sh` → **GATE: PASSED**, 134 tests passed (+1 doc-test = 135; +1 on-demand volume gate) | ✅ |
| Reproducibility | Identical results across reruns | **3 consecutive full-gate runs, byte-identical outcomes** (134/0, 135/0 with doc-tests, fmt+clippy clean, volume gate 1/0 each run) | ✅ |

## 2. In-scope defect closure (all critical/high items)

| ID | Severity | Outcome | Pinned by |
| :-- | :--: | :--- | :--- |
| N-1 | Critical | **CLOSED** (merged from `fix/New-8`) | `short_header_ciphertext_colliding_with_handshake_types_is_still_routed` + volume gate |
| N-2 | High | **CLOSED** (merged) | `tests/slow_consumer_test.rs` (6 tests) |
| N-3 | High | **CLOSED** | `test_drr_fairness_under_p1_saturation` + 2 more |
| N-4 / FU-4 | High | **CLOSED** | `test_pto_probe_does_not_collapse_cwnd` |
| New-8 / A-5 / SEC-15/16 | High | **CLOSED** (merged) | 5-test per-path anti-amp matrix |
| New-12 | High | **CLOSED** | 5-test challenge matrix |
| X-1 | P0-engine | **CLOSED** (merged) | `min_rtt_follows_the_new_path_after_migration` + 2 more |
| FR-5 | Medium | **CLOSED** | `closed_connection_ignores_incoming_datagrams` |
| FR-7 | Medium | **CLOSED** | `queue_counters_stay_exact_across_all_mutation_paths` |
| FR-8 | Medium | **CLOSED** | `test_ordered_group_drain_preserves_distinct_order_seq` |
| N-5 | Medium | **CLOSED** | `unsampled_min_rtt_is_none_until_the_first_ack` + backpressure/metrics unit tests |
| N-7 | Medium | **CLOSED** | `zero_byte_flood_is_bounded_by_item_caps` + `zero_byte_flood_is_bounded_by_the_item_cap` |
| FU-5 | Medium | **CLOSED** | `ordered_group_eviction_is_lru_not_fifo` |
| WIR-2 | Medium | **CLOSED** | `ack_frame_with_uncapped_range_count_encodes_self_consistently` |
| WIR-4 | Medium | **CLOSED** | `close_reason_truncates_at_a_utf8_char_boundary` |
| SEC-14 (residual) | Medium/Low | **CLOSED** | `debug_does_not_leak_directional_keys`, `debug_does_not_leak_session_directional_keys` |
| X-19 / SCH-2 | P2 | **CLOSED** (deficit cap, with N-3) | deficit cap in DRR; exercised by fairness tests |
| N-6 | Docs | **CLOSED** | live `gtp-cli dissect` run on the README example succeeds; conditional verdicts in `stress-suite` |

**Open critical/high defects: none.** (Re-audit confirmed the remaining previously-closed
criticals — SEC-1..4, REC-10, Core-C1, PATH-5, R-1..R-8, FR-1..FR-4 — via their named
tests, all green in the final runs.)

## 3. Deferred register (documented, with rationale — unchanged by this round)

| Item | Rationale |
| :--- | :--- |
| SEC-5 / SEC-A7 (server authentication, active MITM) | Architectural decision DEF-1 — requires a PSK/signature design milestone; anonymous-DH scope is simulation/LAN |
| SEC-8 / SEC-A8 (static master-secret removal) | gtp-sim determinism depends on it; needs a sim-key migration design |
| CORE-2 / X-10 (fragmentation) | Large feature; the API contract rejects >MTU payloads safely (FR-1) |
| WIR-5, WIR-6 residual, WIR-10 / SEC-13 (wire deferred set, DEF-3) | Wire-format governance; any change requires version negotiation |
| REC-12 (loss timer), REC-13 (rate sample), REC-9 (u32 ack_delay) | Recovery refinements below the criticality bar; the PTO path drains correctly |
| CC-7, CC-8, CC-10, CC-12 / X-14 (ECN) | CC enhancements / ECN needs OS + runtime support |
| SEM-3, SEM-4, SCH-5, SCH-6 | Scheduler/semantics refinements; per-tier supersession functions |
| CORE-4 TX-task leak, CORE-9 rate-limiter map growth, CORE-7 | Runtime hygiene, low severity, bounded impact |
| X-4 (timestamp consumption), X-8 (multi-slot challenges) | Inputs to the future routing-engine build-out (RE agenda) |
| WIR-1 (encode overflow, library-level), WIR-7, WIR-11, CORE-6, CC-11, X-18 | Performance/documentation cleanups |
| REC-8 (per-packet ACK default) | Intentional low-latency design; policy-changeable |

## 4. Session commit record (Radicle-local workflow, no GitHub)

| Commit | Content |
| :--- | :--- |
| `3fada65` | New-12 fix committed on its branch |
| `e756d7d` | merge `fix/New-8-per-path-validation` (N-1, N-2, X-1, A-5, New-8) |
| `33db814` | merge New-12 into `integration/all-fixes` |
| `65daea8` | New-8 test source-address modeling fix + re-audit report |
| `1dfabc0` | N-3 / X-19 / FR-7 / N-7 scheduler round |
| `60d02a0` | N-4 / FU-4 / FR-8 / N-5 / FU-5 / FR-5 recovery round |
| `26174ed` | WIR-2 / WIR-4 / SEC-14 wire+crypto round |
| `6bc0612` | cross-layer integration scenarios + gate auto-detect |
| (this commit) | N-6 / X-2 / X-3 documentation round + closure matrix + CHANGELOG |

**Radicle note:** the workspace has Radicle tooling and an identity configured globally,
but this repository has not yet been initialized as a Radicle project (`rad init`) and
the node is not running — per the approved fallback, all work is committed locally on
`integration/all-fixes` (merged to `main` locally) and Radicle synchronization is
deferred until the project is initialized. No GitHub/origin operations were performed.

## 5. Test-count progression

| Point | Tests (all-targets) |
| :--- | :---: |
| Baseline (tree @ `1c0e488` + uncommitted New-12) | 95 |
| New-8 branch (@ `7918821`) | 111 |
| After Stage-2 merges | 116 |
| + scheduler round | 121 |
| + recovery round | 128 |
| + wire/crypto round | 132 |
| + cross-layer suite | **134** (+1 doc-test; +1 on-demand N-1 volume gate) |


---

## Addendum — follow-up round (2026-09-04, later session)

Follow-up changes were added after `6cb6e6e`: commit `09f1d8a` (CLI stress-harness
backpressure yield/backoff) and three Arabic reports in `docs/reaudit/`. An
independent verification session established:

1. **Code change verified**: implemented, effective (high-burst tier sustains
   84,360 msg/s, no failures), no conflicts, no regressions (134/0 on both ends).
   One CI-blocking fmt violation it introduced was fixed in `ba2a486`.
2. **Documentation corrected in place**: fabricated commit hash
   (`09f1d8a4e3fa…` → real `09f1d8a390…`), wrong author, wrong defect
   descriptions (N-4/N-6/N-7), CORE-4/REC-8 overclaims, loopback-vs-WAN
   misattribution, stale dissect hex, kernel version. Verification addenda
   appended to the affected reports.
3. **Live WAN phase executed end-to-end** at version parity `ba2a486`
   (local + VPS): five stepped rounds to `92.222.80.200:7777` all pass —
   including one genuine internet loss in round 2,000 recovered by a single
   selective retransmission — plus the full local stress suite and the 134-test
   suite run on the VPS itself. Full details and raw evidence:
   `Live-WAN-Verification-Report-2026-09-04.md`.

| Commit | Content |
| :--- | :--- |
| `ba2a486` | style fix for `09f1d8a`'s fmt violation |
| (this commit) | follow-up docs corrections + verification addenda + official live-WAN report |
