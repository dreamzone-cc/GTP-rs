# Design Note — G3-Prelude Round: Report Freshness (GTPRP2), Structured Decision Log, Adversarial Suite

> **Gate:** G3 prelude — the first executable slice of the post-G2 gap paper
> (`GTP_Protocol_G2_Gap_Analysis_and_Validation_Plan.md`, verified in its
> Appendix C): its P0 items "freshness/staleness model", "structured decision
> log", and the adversarial scenario suite (§27 S01–S12).
> **Written before implementation** per ARDP v1.1 §12.1.
> **Non-goals (stay per plan):** hysteresis FSM/anti-flap actuation semantics
> (B-4, G4), path-lifecycle state machine (B-11, G3/G5), loss axis (blocked
> on RT-1/E-5), any actuation whatsoever (first switching = G4).

## 1. Work items and target files

| Item | Target | Content |
| :--- | :--- | :--- |
| **RT-3 fix** (defect from the gap paper §3.3, confirmed in Appendix C) | `gtp-core/src/{connection.rs,state.rs}`, `control/{metrics.rs,handle.rs}` | `ConnectionHot::last_rx_time: Option<MonotonicTime>` set **post-authentication only** (beside the OWD feed, INV-3); surfaced as `DetailedMetrics::since_last_rx: Option<Duration>` (None before the first packet — N-5 discipline). This is the measurement-basis age in the endpoint's OWN clock: no clock synchronization is ever needed (the reporter knows when it last received data; the receiver of a report adds only its own arrival tracking). |
| **GTPRP2 report** | `gtp-route/src/report.rs` | `MeasurementReport` gains `since_last_rx_us: Option<u64>`; encode as `GTPRP2\|var\|jitter\|srtt\|samples\|since_last_rx`. Parser accepts BOTH versions: GTPRP1 (age → `None`) and GTPRP2 (strict arity; malformed ⇒ whole-line reject). App-layer message — **zero wire change** (§2.3). |
| **Freshness factor (B-9 recency)** | `gtp-route/src/{lib.rs,score.rs,select.rs}` | `PathStats::{fwd,rev}_age_us: Option<u64>`; path age = max of the two (either direction's stale evidence ages the whole picture); `freshness(age)` = 1.0 within `FRESHNESS_GRACE_US = 1 s`, linear decay to 0 at `FRESHNESS_SATURATION_US = 5 s`; unknown age (`None`, e.g. a GTPRP1 report) is **neutral 1.0, surfaced as unknown in the record** — we do not punish without evidence; enforcement applies once ages are carried. `effective = score × confidence × freshness`; a best candidate whose freshness falls below `CONFIDENCE_FLOOR` holds with a new reason code `STALE_EVIDENCE_HOLD` (the S04 requirement: stale evidence must never win). |
| **Structured decision log (§17 / B-10 / B-13 KPIs)** | `gtp-route/src/decision_log.rs` (new) | `DecisionRecord` with the §17 field set (decision_id, connection_id, path_set, window, per-candidate score/confidence/freshness/effective, thresholds snapshot, previous/selected path, reason code, policy class, state transition) with strict line encode/parse (`GTPDL1|…`). `DecisionTracker`: pure, deterministic — id counter, previous-path memory, bounded record ring (INV-18 discipline: capacity + dropped counter), KPI computation (decision count, switch count, revert count, minimum dwell) — the observability half of B-13, no actuation. |
| **Adversarial suite S01–S12** | `gtp-route/tests/adversarial_suite.rs` | The gap paper §27 scenarios as named deterministic tests. S01–S06, S11: pure-stats over `select`/`health`/tracker. S08/S09/S10: measurement-level via `SimulationRunner` (single-packet spike; sustained degradation with bounded virtual detection time; recovery with deterministic reversion — RevertGuard stays B-6/G5). S12: cross-reference to the RT-2 core tests (no duplication). S07 (burst loss): `#[ignore]` placeholder with the explicit reason "requires the loss axis (RT-1/E-5)" so the suite documents the gate. S02's expected outcome is the CURRENT deterministic tie-break, per verification addendum C.3-1 — HOLD-at-2% semantics arrive with B-4 (G4), and the test records that mapping. |
| **CLI wiring** | `gtp-cli/src/main.rs`, `gtp/tests/route_probe_e2e.rs`, `gtp/examples/route_selection.rs` | Server reports become GTPRP2 (carries `since_last_rx`); `route-probe` prints the per-direction freshness in the combined table and the shadow verdict now carries the freshness column; e2e/example updated to v2. |

## 2. Affected invariants (ICD-01 §3.4) and tests

| Invariant | Impact | Test plan |
| :--- | :--- | :--- |
| **INV-3** (no state mutation before authentication) | `last_rx_time` is measurement state | Set ONLY in the post-replay-commit / post-AEAD-open arm, beside the OWD feed; extend `tampered_datagram_produces_no_owd_sample` pattern with a `since_last_rx` assertion on the tampered path |
| **INV-15** (shadow never actuates) | Decision log/tracker record, print | Negative: suite asserts computing records never touches runner data planes; no actuation path exists |
| **INV-11** (measurement scope = decision scope) | Ages are per-path, per-epoch | `into_path_stats` maps report age → fwd, local age → rev; sim tests keep both sides in one virtual clock |
| **INV-18** (no unbounded state) | Tracker records | Bounded ring with capacity + `dropped_records` counter, pinned by unit test |
| **N-5 discipline** (no sentinels) | New metrics/records | `Option` everywhere; `None` = not-yet / unknown, pinned by unit tests |

## 3. Acceptance mapping (gap paper → this round)

| Gap-paper item | Delivered here |
| :--- | :--- |
| §3.3 freshness/staleness model | GTPRP2 + `since_last_rx` + `freshness()` + `STALE_EVIDENCE_HOLD` + S04/S11 tests |
| §17 structured decision logging | `DecisionRecord`/`DecisionTracker` + S05 KPI test |
| §27 S01–S12 | Suite (S07 gated, S12 cross-ref) |
| §24 Correctness KPIs (subset) | deterministic pass rate, reason-code correctness, decision-log replay |
| §28 Observability Q1–Q12 (subset) | Q1/Q2/Q3/Q4/Q8/Q9 answerable from a `DecisionRecord` |

## 4. Gate for this round

Local full gate ×3 green (expected 168 → ~185+), fmt/clippy clean,
`verify_remediation.sh` PASSED, VPS parity + one live `route-probe` round
showing the freshness column on the real network.
