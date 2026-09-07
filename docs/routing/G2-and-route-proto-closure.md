# G2 + Route-Prototype Round — Closure Report

> **Round:** G2 completion (D-2) + route-selection prototype (B-1 prelude,
> B-3/B-9/B-10 slices) + bidirectional exchange prototype, per the approved
> round scope and the pre-implementation design note
> (`G2-and-route-proto-design.md`, ARDP §12.1).
> **Committed range:** `a19bbff..07cbc47` (5 commits).
> **Closure date:** 2026-09-05.

## 1. Exit criteria → evidence

| Criterion | Result | Evidence |
| :--- | :--- | :--- |
| **G2**: same seed ⟹ byte-identical event sequence over CONNECTION-DRIVEN runs | ✅ | `fabric_runner::same_seed_connection_driven_runs_are_byte_identical` — two full `GtpConnection`-pair runs, same master seed/script/traffic: identical fabric event log AND identical delivered payloads (≥100 ordered messages); a different seed diverges |
| **G2**: per-direction impairment demonstrably independent | ✅ | `fabric_runner::per_direction_impairment_is_independently_visible` — scripted +40 ms FORWARD-only step: server `owd_var` ≥ 30 ms, client ≤ 5 ms; the reverse case mirrors exactly |
| RT-2 fixed (event queue unbounded, never drained) | ✅ | `event_queue_is_bounded_and_drops_oldest_with_counter`, `event_queue_capacity_zero_drops_every_event`; registry entry RT-2 |
| Pure selector correctness | ✅ | 13 gtp-route unit tests: exact normalization/monotonicity/renormalization, confidence floor hold, all six reason codes, deterministic tie-breaks, strict report parse/roundtrip, bidirectional merge |
| Selection from real measurements (INV-11) | ✅ | `selection_from_sim`: two real `SimulationRunner` pairs (clean vs jitter-impaired; clean carries the HIGHER id so order/id cannot explain the outcome) — `select` picks the measured-better path, CLEAR_WINNER |
| Exchange works end-to-end | ✅ | `route_probe_e2e`: in-process endpoint pair — server drains (RT-2), reports what its receiver measured; client parses ≥3 reports, merges, produces the shadow verdict |
| Shadow discipline (INV-15) | ✅ by construction | `route-probe` computes and prints; no actuation path exists anywhere in the round; the negative check is "the runners' data planes are untouched by computing a selection" (asserted in `selection_from_sim`) |
| Live device↔node validation, ×3 consistent | ✅ | §3 below — three consecutive `route-probe` rounds over the real WAN, all Healthy, consistent scores (0.931/0.933/0.943), 11/11 reports each |
| Full gate green ×3 | ✅ | three consecutive full-gate runs at 168/168 (local), plus 168/168 re-run on the VPS itself at parity `07cbc47` |

## 2. Work landed

| Commit | Content |
| :--- | :--- |
| `fe1046c` | RT-2: bounded event queue (capacity config, drop-oldest, drop counter in metrics) + net-server 1 s drain loop |
| `1463e6e` | D-2/G2: `FabricRunner` — connection-driven multi-link determinism + directional independence |
| `37afbb7` | `gtp-route` crate: pure continuous scorer (B-3), sample-count confidence (B-9), explainable deterministic selection (B-10 slice), health verdict; `selection_from_sim` integration test |
| `07cbc47` | `MeasurementReport` exchange (ReliableOrdered, no wire change — §2.3), net-server periodic reports, `gtp-cli route-probe` shadow verdict, `gtp::route` facade, runnable `route_selection` example, `route_probe_e2e` |

Test count: **149 → 168** (+2 RT-2, +2 fabric runner, +10 route unit, +1 sim
selection, +3 report, +1 e2e). Loss axis deliberately absent from scoring
(RT-1: `loss_ratio()` is a mixed-unit proxy; a wrong unit must never drive a
decision) — a real loss axis arrives with A-5/E-5 work.

## 3. Live validation (device ↔ VPS `92.222.80.200:7777`, 2026-09-05)

Both ends at parity `07cbc47` (bundle sync, VPS suite 168/168, release rebuilt,
service restarted PID 199749). Three consecutive 10 s / 60 FPS probes:

| Probe | Device→Node owd_var | Node→Device owd_var | Fwd jitter | Rev jitter | RTT | Reports | Score | Verdict |
| :--: | :--: | :--: | :--: | :--: | :--: | :--: | :--: | :--- |
| 1 | 1486 µs | 892 µs | 370 µs | 424 µs | 53.8 ms | 11/11 | 0.931 | Healthy |
| 2 | 1389 µs | 1043 µs | 410 µs | 422 µs | 50.6 ms | 11/11 | 0.933 | Healthy |
| 3 | 815 µs | 690 µs | 379 µs | 239 µs | 46.3 ms | 11/11 | 0.943 | Healthy |

Notable live findings:

- **Directional separation is real on the internet:** every probe measured
  forward ≠ reverse (e.g. 1486 vs 892 µs) — the exact quantity round-trip RTT
  cannot express and the reason RE-1 exists.
- **Recovery visible mid-probe:** during probe 1 the server's journal shows a
  brief loss burst on its report stream (`RetransmissionTriggered` Msg#36-42)
  and the reliable-ordered exchange recovered it — all 11 reports delivered.
  Measurement and delivery survived real impairment simultaneously.
- **RT-2 confirmed in production shape:** no drop warnings at capacity 1024
  under continuous 60 FPS traffic; the drain loop visibly executed (server EV
  lines in `journalctl`).

Raw captures: `/tmp/route-probe-{1,2,3}.txt`.

## 4. Position after this round

- **Gate G2 is complete** (D-1 + D-2 both delivered and pinned).
- The routing mechanism's measurement→selection→explanation spine exists:
  wire timestamp → authenticated RX → estimator → per-direction aggregates →
  cross-endpoint `MeasurementReport` → pure score/select with reason codes →
  shadow verdict. All of it pure, deterministic, and testable.
- **Next per plan:** G3 — measurement-connection plane + shadow engine with
  the INV-15 kill switch (B-2), full scorer calibration dataset (B-3/B-9),
  structured decision log (B-10), path lifecycle (B-11), per-class enable
  flags (B-12), A-4 tier accounting, A-7 config strategy; first actual
  switching remains gated at G4 (failover class + A-3 multi-slot challenges).
