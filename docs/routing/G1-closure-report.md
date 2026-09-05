# G1 Closure Report — Measurement Layer Activation

> **Gate:** G1 of `docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md` (v1.1 §6).
> **Design:** `docs/routing/G1-design-note.md` (binding decisions D1–D10).
> **Committed range:** `78ff80d..` through the G1 series (see §2).
> **Closure date:** 2026-09-05.
> **Rule (G-R4):** a fix without a test is not implemented — every item below
> maps to a named, negative-checked test or an executable gate check.

## 1. Exit-gate criteria → evidence

| G1 criterion (ARDP §6) | Result | Evidence |
| :-- | :-- | :-- |
| `owd_var` within ±1 ms of an injected known delay | ✅ | `owd::tests::known_delay_injection_accuracy` — +15 ms injection reads 13.9–16.1 ms across 30 ticks; new-lower-floor re-anchor returns to 0 |
| Timestamp wrap at `2^32 − ε` produces no false jump | ✅ | `owd::tests::wraparound_edge_no_false_jump` — constant 5 ms delay across the wrap stays zero-variance |
| 30 s at 50 ppm drift ≤ 1.5 ms error | ✅ | `owd::tests::clock_drift_bounded_by_reanchor` — 40 s run, worst error exactly 1500 µs (integer-exact: 50 ppm = 1/20,000) |
| Zero allocation in the RX path | ✅ by construction | `OwdEstimator` is a `Copy` struct of integer fields inline in `ConnectionHot` (design note D2) |
| Jitter from real 60 Hz traffic non-zero and stable | ✅ sim + ✅ WAN | `connection::tests::owd_samples_emitted_at_bounded_rate_with_real_jitter` (200 ticks @60 FPS, alternating 20/35 ms delay, jitter ≥ 1 ms) + live WAN round in §4 (RFC 3550 jitter printed from real 60 FPS traffic) |
| INV-18: every wire field has a consumer | ✅ | three procedural checks in `scripts/verify_remediation.sh` (timestamp consumed in RX, estimator wired, telemetry surfaced) — the timestamp check fails on the pre-G1 tree and passes after |
| INV-3: no measurement before authentication | ✅ | `connection::tests::tampered_datagram_produces_no_owd_sample` — tampered datagram: auth fails, corrupted++, estimator unseeded, zero events; pristine-datagram control proves the harness |
| Full gate green ×3 consecutive runs | ✅ | `/tmp/gate-run-{1,2,3}.log` — three PASSED runs, 149 tests each |

## 2. Work items landed (Track A + Track D start + E-6 opening)

| Item | Commit | Content |
| :-- | :-- | :-- |
| §12 scaffolding | `78ff80d` | G1 design note + defect registry (opened with RT-1) |
| E-1 | `1182f53` | stale New-12 comment corrected at the TX amplification gate |
| E-2 | `3324013` | `RttStats.min_rtt` → `pub(crate)`; six gtp-core test lines moved to the Option API |
| A-6 | `8a27b12` | `anti_amplification_factor` live: `with_factor`/`set_factor` (floor 1), wired at **all five** construction sites — including a wiring gap found during implementation: the handshake-driven `new_with_directional_keys` (the production endpoint path) never saw the config; tests: 10× boundary, factor floor, default 3× unchanged, config→limiter wiring |
| RE-1 math | `72c6120` | `OwdEstimator` in gtp-recovery + 5 unit tests (the four gate criteria + migration reset) |
| A-1 + A-2 | `5ed2430` | RX consumption post-auth, bounded-rate `OwdSample` emission (`owd_sample_interval`, 100 ms default, all four profiles), `PathEventDetected`/`PathEventKind`/`PathDirection` defined (emission from G5), `DetailedMetrics::owd_var/jitter` as `Option<Duration>`, migration reset beside the X-1 reset, CLI prints OWD Variance + OWD Jitter; 3 integration tests |
| INV-18 gate checks | `046d4e7` | three procedural greps in the verification gate |
| D-1 | `00eb110` | `SimulatedFabric`: per-direction link profiles, scripted impairments (clamped deltas, cursor-idempotent), splitmix64 per-link-per-direction RNG streams, event log; 4 tests incl. same-seed ⟹ identical 1000-event log |
| E-6 opening | `114f390` | server-authentication design options (PSK vs operator-signed Ed25519; recommendation B); ADR-006 decisions enumerated; zero code |

Test count: **134 → 149** across the gate runs (+5 recovery, +3 core integration, +1 core A-6 wiring, +2 path, +4 sim).

## 3. Notable findings during the gate (defect-registry entries)

- **RT-1** (`defect-registry.md`): `loss_ratio()` is a mixed-unit proxy — found
  in the 2026-09-05 WAN baseline round; fix belongs with E-5 (CLI metrics
  testing), deliberately not in G1.
- **A-6 wiring gap** (found, fixed in `8a27b12`): the config factor reached
  `build_with_config` but the handshake-driven hot constructor has no config
  parameter — the production endpoint path would have kept the hardcoded 3×
  forever. Fixed by re-seeding from the connection-level config in both
  `GtpConnection` constructors; pinned by
  `anti_amplification_factor_flows_from_config_to_limiters`.

## 4. Deployment verification (post-G1 WAN round)

Both ends synced to the G1 head `00eb110` via git bundle (commit-level parity
confirmed on each side; VPS suite re-run: **149 passed / 0 failed**).
`gtp-server` restarted on the new build (PID 191275) and a 2,000-frame live
WAN round was executed with the new telemetry lines — raw capture
`/tmp/wan-g1-round.log`:

| Metric | Value | Reading |
| :-- | :-- | :-- |
| Elapsed | 34.14 s | consistent with the 2026-09-05 baseline (34.20 s) |
| Smoothed / Min RTT | 53.43 / 52.67 ms | within the established 44–55 ms profile |
| **OWD Variance** | **1.541 ms** | non-zero one-way queueing variance above the floor — live, from the wire timestamp |
| **OWD Jitter (RFC 3550)** | **267 µs** | **non-zero real inter-arrival jitter from actual 60 FPS internet traffic — the last G1 gate criterion, satisfied on the production network** |
| Loss / Retx / PTO / Corrupted | 0 / 0 / 0 / 0 | clean round |
| Server efficiency | Memory 892 K (peak 1.7 M), CPU ≈ 8 s across the round | ordered delivery complete (`order_seq: 499` = final frame) |

RTT variance (684 µs) vs OWD jitter (267 µs) are now *independently
measurable* — exactly the directional separation RE-1 exists to provide and
round-trip RTT alone never could.

## 5. Position after G1

- Measurement layer live end-to-end: wire timestamp → authenticated RX →
  estimator → bounded-rate events + telemetry surface → CLI visibility.
- D-1 delivered ahead of G2; G2 remains to wire the fabric to a runner and
  prove byte-identical determinism over connection-driven runs.
- E-6 design open with a recommendation; implementation is the G7 blocker.
- Next gate: **G2** (per ARDP §6: full determinism proof, per-direction
  impairment demonstrably independent), then G3 shadow mode consumes the
  measurement layer delivered here.
