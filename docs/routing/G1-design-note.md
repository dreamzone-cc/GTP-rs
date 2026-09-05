# G1 Design Note — Measurement Layer Activation (RE-1 + A-2)

> **Gate:** G1 of `docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md` (ARDP v1.1 §6).
> **Mechanism spec:** ICD-01 §7 RE-1 (algorithm, numeric bounds), items A-1/A-2.
> **Status:** approved for implementation 2026-09-05.

## 1. Scope

Turn the wire field `PacketHeader.timestamp_micros` — sent in every packet,
covered by the AAD, and consumed by nothing on RX today (X-4) — into the
routing engine's primary measurement input: one-way-delay variance and RFC
3550 jitter, at zero wire cost and zero RX-path allocation.

Also lands in this gate (ARDP §5 Track A / §10): E-1 (stale comment), E-2
(`min_rtt` visibility), A-6 (`anti_amplification_factor` live config), the
INV-18 gate check, and — on the parallel Track D — D-1 (`SimulatedFabric`
start). E-6 (server authentication) gets its design document opened, not
implemented.

## 2. Design decisions (binding)

| # | Decision | Rationale |
| :-- | :-- | :-- |
| D1 | `OwdEstimator` lives in `gtp-recovery` next to `RttStats` | Recovery owns measurement estimators; keeps `gtp-route` (G3) dependency-free of the core |
| D2 | `Copy` struct of integer fields only (`base_d`, `prev_owd_var`, `jitter`, `last_floor`, `has_sample`) | Zero allocation in the RX path by construction (INV-18 / G1 gate) |
| D3 | Algorithm exactly per ICD-01 RE-1: `d = ts_local.wrapping_sub(ts_peer)`; `delta = d.wrapping_sub(base_d) as i32`; `if delta < 0 { base_d = d; }`; `owd_var = delta.max(0)`; RFC 3550 §6.4.1 jitter `jitter += (abs(owd_var − prev) − jitter) / 16` | Signed wrap-safe interpretation is correct for all `\|delta\| < 2^31`; clock offset cancels by subtraction — no clock sync needed |
| D4 | Sliding floor window of 30 s: if no new floor is registered within 30 s, re-anchor `base_d = d` | Bounds crystal-drift error to ≤ 1.5 ms at 50 ppm (ICD-01 RE-1 table); proven by unit test |
| D5 | Called **only after** `replay_window.commit` (the authenticated point, `connection.rs:375`) | INV-3: no state mutation from unauthenticated input |
| D6 | Bounded event emission: `ControlEvent::OwdSample` emitted at most once per `owd_sample_interval` (new `GtpConfig` field, default 100 ms) | `event_queue` is an unbounded `Vec` (`connection.rs:34`); per-packet emission at 60–144 Hz would flood it |
| D7 | `ControlEvent::PathEventDetected` is defined now but **not emitted** until G5 | Avoids a later breaking enum change; variant is dead-but-documented |
| D8 | `DetailedMetrics` gains `owd_var: Option<Duration>` and `jitter: Option<Duration>` (`None` before first sample) | Follows the N-5 `Option` discipline — the sentinel never leaks into telemetry |
| D9 | `owd.reset_for_new_path()` is called beside `loss_detector.on_path_migration` (`connection.rs:688-691`) | Consistency with the X-1 fix; pre-positions INV-13 (epoch discipline, G3) |
| D10 | `epoch: u8` field in `OwdSample` is constant `0` until RE-3 lands in G3 | Wire/event shape frozen early, semantics later |

## 3. Touch points

| File | Change |
| :-- | :-- |
| `gtp-recovery/src/owd.rs` (new) | `OwdEstimator`, `OwdSample`, unit tests (injection ±1 ms, wrap edge `2^32−ε`, 50 ppm/30 s drift ≤ 1.5 ms, RFC 3550 spot values) |
| `gtp-recovery/src/lib.rs` | export `owd` |
| `gtp-core/src/connection.rs` | RX call after replay commit (:375); bounded emission; reset on migration (:688-691); E-1 stale comment (:1080-1101) |
| `gtp-core/src/state.rs` | `owd: OwdEstimator` field on `ConnectionHot`; init in `build_with_config` |
| `gtp-core/src/control/events.rs` | `OwdSample { owd_var_us, jitter_us, epoch }`, `PathEventDetected { kind, owd_step_us, direction }` |
| `gtp-core/src/control/metrics.rs` | `owd_var: Option<Duration>`, `jitter: Option<Duration>` |
| `gtp-core/src/control/handle.rs` | populate both in `query_metrics` |
| `gtp-core/src/control/config.rs` | `owd_sample_interval: Duration` (default 100 ms, all four profiles) |
| `gtp-cli/src/main.rs` | net-client report prints OWD variance + jitter lines |
| `scripts/verify_remediation.sh` | INV-18 `check_present` for the `timestamp_micros` consumer |
| `gtp-path/src/anti_amplification.rs` + `state.rs` + `config.rs` | A-6: `with_factor`, wired from config at both limiter constructions |
| `gtp-recovery/src/rtt.rs` | E-2: `min_rtt` → `pub(crate)`; six gtp-core test lines switch to `min_rtt_sample()` |
| `gtp-sim/src/fabric.rs` (new) | D-1: `SimulatedFabric` per-direction links, scripted impairments, splitmix64 seeding, determinism test |

## 4. Exit gate — criterion → evidence

| Criterion | Evidence |
| :-- | :-- |
| `owd_var` within ±1 ms of an injected known delay | unit test `known_delay_injection_accuracy` |
| Timestamp wrap at `2^32 − ε` produces no false jump | unit test `wraparound_edge_no_false_jump` |
| 30 s at 50 ppm drift ≤ 1.5 ms error | unit test `clock_drift_bounded_by_reanchor` |
| Zero allocation in RX | by construction (D2); reviewed |
| Jitter from real 60 Hz traffic non-zero and stable | sim test `owd_sample_emitted_under_jittery_profile` + WAN round via CLI printout |
| INV-18: every wire field has a consumer | `verify_remediation.sh` procedural check |
| INV-3: no measurement before authentication | negative test `tampered_datagram_produces_no_owd_sample` |
| Full gate green ×3 consecutive runs | closure report |

## 5. Invariants exercised

INV-3 (auth before state), INV-18 (no dead wire surface), INV-2 family
(measurement beside `RttStats`), pre-positioning of INV-13 (epoch reset).
