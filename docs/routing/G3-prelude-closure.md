# G3-Prelude Round — Closure Report

> **Round:** the first executable slice of the post-G2 gap paper
> (`GTP_Protocol_G2_Gap_Analysis_and_Validation_Plan.md`, verified in its
> Appendix C): its P0 items *freshness/staleness model*, *structured decision
> log*, and the *adversarial scenario suite* (§27).
> **Design note (pre-implementation, §12.1):** `G3-prelude-freshness-design.md`.
> **Committed range:** `f226612..` (this round).
> **Closure date:** 2026-09-07.

## 1. Exit criteria → evidence

| Criterion (design note §4) | Result | Evidence |
| :--- | :--- | :--- |
| RT-3: stale reports detectable | ✅ | `since_last_rx_tracks_only_authenticated_traffic` (INV-3 negative: tampered traffic never moves the basis), `v1_reports_parse_with_unknown_age`, `stale_evidence_holds_and_a_live_competitor_wins` (S04), S11 |
| Freshness factor semantics | ✅ | `freshness_grace_then_linear_decay_to_zero` (grace free, exact 0.5 at 3 s, clamped, monotone over the full domain); unknown age neutral, surfaced as unknown |
| `STALE_EVIDENCE_HOLD` in selection order | ✅ | enforced after the confidence floor, before any winner; no winner on stale best |
| Structured decision log | ✅ | `record_roundtrips_exactly` (line-invariant replay), `tracker_counts_switches_and_holds` (switch/revert/dwell exact), `ring_is_bounded_and_drops_are_counted` (INV-18), `malformed_log_lines_are_rejected_whole` |
| Adversarial suite | ✅ | S01–S06, S08–S12 pass; S07 the standing loss-axis placeholder (`#[ignore]` with the reason); S12 cross-checks RT-2 across capacities 0/1/16/64 with exact accounting |
| INV-15 (no actuation) | ✅ by construction | everything records/prints; no actuation path exists; S05 proves the observation side only |
| Full gate ×3 | ✅ | three consecutive runs **187/187**, fmt+clippy clean, `verify_remediation.sh` PASSED |

## 2. Work landed

| Item | Content |
| :--- | :--- |
| RT-3 fix | `last_rx_time` post-auth (INV-3) → `DetailedMetrics::since_last_rx`; `MeasurementReport` **GTPRP2** with `since_last_rx` (v1 parses, age = unknown); `PathStats` both-direction ages, staler governs; `freshness()` (1 s grace → 0 at 5 s; unknown neutral); `effective = score × confidence × freshness`; `STALE_EVIDENCE_HOLD` |
| Decision log | `DecisionRecord` (§17 field set + per-candidate scored entries, `GTPDL1` strict-parse lines) and `DecisionTracker` (bounded ring + dropped counter, switch/revert/dwell KPIs; revert = switch straight back to the prior-prior path) |
| Adversarial suite | S01 stable winner (×100 reproducible); S02 near-tie deterministic today (`TIE_BREAK_LOWER_ID`) with the G4 HOLD mapping recorded; S03 confidence floor; S04 stale rejection; S05 flap record accounting; S06 directional axis flagging; S08 transient spike (RFC 3550 EWMA tail decays to clean within 2 s; variance axis back immediately); S09 sustained degradation detectable ≤ 0.5 s; S10 recovery to the clean neighbourhood + fresh basis; S11 delayed/duplicate reports stale-aware and inert; S12 RT-2 cross-check |
| CLI | server reports are v2; `route-probe` prints per-direction evidence freshness + freshness column in the verdict; e2e asserts < 1 s basis under 60 FPS |

Test count **168 → 187** (+1 core INV-3, +6 route unit, +4 decision-log, +11
suite, minus reorganizations). The suite's sim-driven tests (S08–S10) forced
three honest corrections to the round's own expectations — the EWMA tail
property (geometric decay, not instant), a 1 ms/1 s unit fix, and the
post-step zero age — each now pinned as the documented behavior.

## 3. Live validation (device ↔ VPS, 2026-09-07)

Both ends synced to the round head (bundle over SSH; VPS suite 187/187;
release rebuilt; service restarted). One live `route-probe` round with the
new freshness surface — captured in the round's raw output (`/tmp/`):
the combined table now shows **per-direction evidence ages** (fwd from the
server's `since_last_rx` via GTPRP2, rev from the client's own), and the
verdict line carries the freshness column, confirming the RT-3 fix end-to-end
on the real network.

## 4. Position after this round

- The gap paper's P0 items *freshness*, *decision log*, and *adversarial
  suite* are closed; its Definition-of-Done rows for staleness (§34) and
  explainability are checkable from `GTPDL1` records.
- Remaining G3 core per plan: **B-2** (shadow engine + measurement-connection
  plane + the single INV-15 kill switch), wiring the tracker into the 1 Hz
  poll loop, **A-4** (tier accounting), **A-7** (config strategy), and the
  24-hour calibration dataset (which the adversarial suite now seeds with
  deterministic scenarios). First actuation remains gated at **G4** with
  B-4's hold/confirm semantics — S02's recorded mapping.
- S07 (burst loss) is the suite's standing placeholder until the loss axis
  (RT-1/E-5) provides a per-packet unit.
