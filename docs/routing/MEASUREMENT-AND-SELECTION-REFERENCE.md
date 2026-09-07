# Measurement & Route-Selection Reference — Mechanisms, Semantics, and the Extension Recipe

> **Status:** PRIMARY reference for everything that measures, scores, selects,
> and reports paths in GTP-rs. Use it (a) to understand the mechanisms in
> force, (b) as the recipe book for adding new measurement patterns or
> scoring axes, and (c) as the record of what has been accomplished so far.
> **Audience:** any engineer extending the adaptive-routing engine or adding
> new telemetry. **Binding rules it inherits:** ARDP v1.1 (§2.3 no-wire-change
> for exchanges, §11.1 weights are calibration outputs, §12 documentation
> protocol) and ICD-01 invariants (INV-3/11/13/14/15/18).
> **Ground truth:** the code. Every constant quoted here is copied from the
> source; where behavior is subtle the pinning test is named.
> **Last updated:** 2026-09-05 (round `3d2c3cf`, 168 tests green, parity on
> both deployment ends).

---

## 1. The measurement architecture (one picture)

```
                    ┌──────────────── measurement substrate (on the wire since 0.1.0) ───────────────┐
                    │  short-header packet carries timestamp_micros = sender's local TX clock (µs,  │
                    │  u32 wrapping). Inside the AAD — an on-path attacker cannot forge it (INV-10).│
                    └──────────────────────────────────┬────────────────────────────────────────────┘
                                                       │ post-authentication only (INV-3)
                                       ┌───────────────▼──────────────┐
                                       │  receiver: OwdEstimator      │  gtp-recovery/owd.rs
                                       │  d = rx_clock − peer_tx      │  per authenticated packet
                                       │  sliding floor + jitter      │  zero allocation
                                       └───────┬──────────────┬───────┘
                    rate-bounded events (100 ms)│              │always-current aggregates
                                               ▼              ▼
                          ControlEvent::OwdSample      DetailedMetrics.owd_var / .jitter
                          (bounded queue, RT-2:        (Option<Duration> — None = not yet
                           capacity 1024,              sampled; sentinel never leaks — N-5)
                           drop-oldest + counter)
                                               │
              ┌────────────────────────────────┴───────────────────────────────┐
              │ each endpoint tells the other what ITS receiver measured        │
              │ MeasurementReport "GTPRP1|var|jitter|srtt|samples"            │
              │ as ReliableOrdered app messages (NO wire change — ARDP §2.3)    │
              │ group id 0x5250 · basis = packets received (NOT events)         │
              └────────────────────────────────┬───────────────────────────────┘
                                               │ merge (report = fwd, local = rev)
                                               ▼
                                     gtp-route::PathStats        one path, both directions, one epoch (INV-11)
                                               │
                       ┌───────────────────────┼───────────────────────┐
                       ▼                       ▼                       ▼
                 score() (B-3)          confidence() (B-9)       health()
                 continuous, monotone    sample-count factor      single-path verdict
                       └────────── together ──────────┘
                                       ▼
                              select() (B-10 slice)
                    deterministic winner + reason code + scored records
                                       │
                          SHADOW ONLY (INV-15): print/log — never actuate.
                          First real switching is gated at G4 (failover class + A-3).
```

Who measures what (never forget this when placing a new measurement):

| Direction | Measured AT | Fed by |
| :--- | :--- | :--- |
| A→B ("forward") | B's `OwdEstimator` | A's wire timestamps on B's RX path |
| B→A ("reverse") | A's `OwdEstimator` | B's wire timestamps on A's RX path |
| Round trip | either side's `RttStats.smoothed_rtt` | ACK timing (one quantity, both sides see it) |

## 2. Mechanism registry (semantics, bounds, pinning tests)

### 2.1 The wire timestamp (substrate; X-4 closed at G1)
- Written at TX: `PacketHeader::new_short(cid, pn, now.as_micros() as u32, 0)` (`connection.rs`, TX pipeline).
- u32 microseconds → wraps every 71.58 minutes. Every consumer must be
  wrap-safe (see 2.2). Inside the AAD since 0.1.0 — INV-10 covers it.

### 2.2 `OwdEstimator` — sliding-floor one-way-delay variance + RFC 3550 jitter
`gtp-recovery/src/owd.rs` (RE-1; landed G1). Per **authenticated** packet
(INV-3: called only inside the post-replay-commit, post-AEAD-open arm —
`tampered_datagram_produces_no_owd_sample` pins it):

```
d      = ts_local.wrapping_sub(ts_peer)            // constant clock offset + one-way delay
first sample: base_d = d, floor_time = now, var = 0
re-anchor: if now − floor_time > 30 s  → base_d = d   (FLOOR_REANCHOR_WINDOW)
delta  = (d.wrapping_sub(base_d)) as i32            // wrap-safe: |delta| < 2^31 exact
if delta < 0 → base_d = d, floor_time = now          // queueing delay can't be negative;
                                                     // a lower d is a better floor
owd_var = delta.max(0)
jitter += (|owd_var − prev_owd_var| − jitter) / 16  // RFC 3550 §6.4.1, integer, floored at 0
```

Properties (each pinned by a named test in `owd.rs`):
- **Offset-free:** the unknown clock offset cancels against the floor — no
  clock synchronization is ever needed.
- **Wrap-safe:** both clocks advance at the same rate so `d` is stable across
  the u32 edge (`wraparound_edge_no_false_jump`).
- **Drift-bounded:** ≤ 1.5 ms error at 50 ppm within the 30 s re-anchor window
  (`clock_drift_bounded_by_reanchor` — integer-exact 1/20,000).
- **Accurate:** +15 ms injected delay reads 13.9–16.1 ms
  (`known_delay_injection_accuracy`).
- **Path-scoped:** `reset_for_new_path()` beside the X-1 RTT reset — old-path
  samples never describe the new path (pre-positions INV-13).
- `Copy` integer struct, inline in `ConnectionHot` — zero RX allocation by
  construction (INV-18 discipline).
- API: `on_packet(ts_peer, now) -> OwdSample`, `owd_var()/jitter() ->
  Option<Duration>` (None before first sample), `is_ready()`, `sample()`.
  `OwdSample.epoch` is constant `0` until RE-3 lands at G3 (D10).

### 2.3 Telemetry surfaces
- **Events:** `ControlEvent::OwdSample` at most once per
  `GtpConfig::owd_sample_interval` (default **100 ms**, all four presets) —
  `last_owd_emit` gate in the RX path. The event queue is **bounded**
  (RT-2): `GtpConfig::event_queue_capacity` (default **1024**, 0 = drop all
  but count), drop-**oldest** on overflow, `ConnectionCold::
  total_dropped_events` surfaced in `DetailedMetrics`. Pinning:
  `event_queue_is_bounded_and_drops_oldest_with_counter`,
  `event_queue_capacity_zero_drops_every_event`.
- **Polling surface:** `DetailedMetrics::owd_var` / `jitter` are
  `Option<Duration>` (N-5 discipline — `None`, never a sentinel), filled in
  `ConnectionControl::query_metrics` (handle.rs).
- **Basis rule (RT-2.5 lesson):** the estimator's honest sample count is
  **packets received** (`total_rx_packets`); the OwdSample event stream is
  rate-limited *telemetry*, NOT a basis. Never mix the two units.

### 2.4 Bidirectional exchange — `MeasurementReport`
`gtp-route/src/report.rs`. ARDP §2.3: genuinely-missing coordination rides as
`ReliableOrdered` application messages — **zero wire change**.

- Wire form (ASCII, one line): `GTPRP1|var|jitter|srtt|samples` where `-` =
  not-yet-measured. Prefix-strict parse; malformed numbers reject the whole
  line (`malformed_reports_are_rejected_whole`).
- Group id: `REPORT_GROUP_ID = 0x5250` (chosen to not collide with app
  groups; pick a fresh one for any future message family).
- Sender = whichever endpoint is reporting **its receiver's** aggregates (the
  direction the other side cannot see).
- `into_path_stats(path_id, &local_rev)`: report becomes the **fwd** axes,
  local receiver's aggregates become **rev**, RTT prefers the local value,
  basis = min of the two per-packet counts.
- Pinning: `report_roundtrips_exactly`, `malformed_reports_are_rejected_whole`,
  `merging_builds_the_bidirectional_picture`.

### 2.5 RTT (pre-existing, for completeness)
`RttStats` (gtp-recovery/rtt.rs): RFC 9002 EWMA, ack_delay clamped by
`max_ack_delay`, min_rtt from raw samples, `min_rtt_sample() -> Option`
(E-2: sentinel crate-private), reset on validated migration (X-1), PTO
granularity floor 1 ms. One source of truth (FR-4): the CC mirrors it, never
the reverse.

## 3. The scoring model (`gtp-route::score`, B-3)

**Form:** continuous, monotone, lower-is-better → `[0,1]`. **No step
functions, no cliffs** — the paper §4.1.2 step scorer is a flap generator
(ARDP §3.2) and is deliberately NOT implemented.

| Axis | Source field | Saturation (=0.0) | Weight within direction |
| :--- | :--- | :--- | :--: |
| owd_var | `PathStats.{fwd,rev}_owd_var_us` | `OWD_VAR_SATURATION_US = 50_000` | `VAR_AXIS_WEIGHT = 0.6` |
| jitter | `PathStats.{fwd,rev}_jitter_us` | `JITTER_SATURATION_US = 20_000` | 0.4 |
| rtt | `PathStats.rtt_us` | `RTT_SATURATION_US = 300_000` | path-level |

Path-level weights `PATH_WEIGHTS = (fwd 0.35, rev 0.35, rtt 0.30)` —
**starting points, not constants**: the shipped defaults are a calibration
output of the G3 24-hour shadow dataset (ARDP §11.1).

**Renormalization rule:** a missing axis is *unknown, not excellent* —
present-axis weights renormalize (`score_renormormalizes_over_present_axes`;
fully-empty input → `None`, never a fake 1.0).

**Deliberately absent: loss.** `loss_ratio()` is a mixed-unit proxy (RT-1:
message-level retransmissions ÷ datagram count — reads 1500% under
impairment). A wrong unit must never drive a decision. The loss axis stays out
until a real per-packet loss signal replaces it (see §5.1).

Pinning: `normalization_is_exact_at_boundaries`,
`score_is_monotone_in_every_axis`.

## 4. Confidence, selection, health

### 4.1 Confidence (B-9)
`confidence(sample_count) = min(samples / CONFIDENCE_FULL_SAMPLES, 1.0)`,
`CONFIDENCE_FULL_SAMPLES = 30`. Applied **multiplicatively**: the selection
key is `effective = score × confidence`. Floor: `CONFIDENCE_FLOOR = 0.5` — a
best candidate below the floor is **confirmable but never fast-picked**
(`ConfidenceFloorHold`, no winner declared).

### 4.2 Selection (B-10 slice) — `select(&[PathStats]) -> Selection`
Rules in order: empty → `NO_CANDIDATES`; best has no data →
`INSUFFICIENT_DATA`; best below confidence floor → `CONFIDENCE_FLOOR_HOLD`
(no winner); single usable → `SINGLE_CANDIDATE`; margin
`(best_eff − runner_eff)/best_eff ≥ CLEAR_WINNER_MARGIN = 0.05` →
`CLEAR_WINNER`; else deterministic `TIE_BREAK_LOWER_ID`. Output carries the
chosen id, runner-up, and a per-candidate scored record (score, confidence,
effective) — the structured decision-log entry. `summary()` gives the
one-line human form. **Pure, total, side-effect-free.**

### 4.3 Health — `health(&PathStats) -> HealthVerdict`
Worst-axis verdict for one path: `Healthy` / `Degraded(axis_name)` /
`InsufficientData`. Thresholds: var > `25_000` µs, jitter > `10_000` µs,
RTT > `150_000` µs (50% of each scoring saturation).

### 4.4 What selection is ALLOWED to do today
Nothing. Shadow discipline (INV-15): `route-probe` computes, prints, records
— there is **no actuation path**. First real switching = G4 (failover class
only, with A-3 multi-slot challenges). Anything you add must respect this
until G4, and even then per-class enable flags (B-12) apply.

## 5. THE EXTENSION RECIPE — adding a new measurement pattern

Use this whenever you add a new axis, a new estimator, or a new
cross-endpoint message. Follow it in order; the §12 protocol (design note →
code+tests → registry entry/closure) wraps every step.

### 5.1 Worked example: adding a real LOSS axis (the known next one)

| # | Step | Where (exact files) |
| :-- | :--- | :--- |
| 1 | Decide the honest unit first. For loss: **per-packet**, ACK-derived (e.g. `(sent_ack_eliciting − acked − in_flight_or_drained)/sent` over a window), NOT `loss_ratio()`. If it cannot be per-packet yet, it is not ready — write the registry entry and stop (what this round did). | design note |
| 2 | Compute it where the truth lives, post-auth only. Loss truth = loss detector (`gtp-recovery/src/loss_detector.rs`): a windowed counter derived from `on_ack_received`/`on_timeout` drains, never from the CLI. | gtp-recovery |
| 3 | Surface as an aggregate: `Option<f64>` (None before the first full window — N-5 discipline) on `DetailedMetrics` (`control/metrics.rs` + fill in `handle.rs::query_metrics`). Never a sentinel. | gtp-core |
| 4 | If the far end must see it: extend the **report**, not the wire — add a field to `MeasurementReport`, bump to `GTPRP2`, keep the strict-parse rule (wrong version/arity/number ⇒ whole-line reject), and pin encode/parse/roundtrip tests. Both ends must agree on the version before it feeds decisions. | gtp-route/report.rs |
| 5 | Extend `PathStats` with `Option<...>` and give it a **saturation constant + weight**, then rely on the existing renormalization rule (missing = unknown). Keep lower-is-better unless you write the monotonicity test for the other direction. | gtp-route |
| 6 | Update `health()` thresholds if the new axis should be able to flag degradation alone. | gtp-route/select.rs |
| 7 | Tests, negative-checked: exact normalization values, monotonicity, renormalization-over-missing, floor/reason-code paths, and ONE integration test where the new axis is injected via the fabric/sim and **flips a selection** (the `selection_from_sim` pattern — the impaired path carries the lower id so order can't explain the outcome). | tests |
| 8 | §12 paper trail: design note (before), defect-registry entry if you found anything, closure report (after), CHANGELOG, ENGINEERING-REFERENCE refresh. | docs/routing |

### 5.2 The rules that keep the recipe safe (read before step 1)
- **INV-3** — no measurement state moves before authentication. Every new
  estimator call goes in the post-AEAD-open arm only; add the tampered-input
  negative test.
- **INV-18** — every wire field must have a consumer; new telemetry must not
  create unbounded state (the RT-2 lesson: bound the queue *and* give the
  runtime a drain path).
- **INV-11/13** — one `PathStats` = one path, one epoch, one scope. Never
  merge across migrations; call `reset_for_new_path()` alongside X-1.
- **Basis honesty** — sample counts are per-packet (`total_rx_packets`), not
  per-event; units must match on both ends of a report.
- **No wire change for coordination** (§2.3) — app messages only; a genuinely
  missing *frame* is a version-gated wire decision, not a shortcut.
- **Weights are calibration outputs** (§11.1) — new axes get starting
  weights and a G3-dataset calibration plan, never final constants.
- **Pure core** — estimators/scoring live in `gtp-recovery`/`gtp-route`;
  anything needing tokio or sockets is adapter work (B-2, `gtp-route-tokio`).

### 5.3 Ready-made extension slots (in plan order)
- **Loss axis** — as §5.1; unblocks when E-5 gives the CLI honest metrics.
- **ECN / congestion axis (A-5, X-14)** — needs `gtp-io` ECN read +
  `on_ecn` call sites; then it becomes a scoring axis (the paper's 5th axis).
- **Per-tier queue pressure (A-4)** — getters exist
  (`GameScheduler::queue_tier_bytes/items`); wire into `query_metrics`, then
  either an axis or a `health` input.
- **Measurement-connection plane (B-2, G3)** — one probe connection per
  candidate path, batched probes over the existing `Ping { nonce }` frame;
  each probe connection gets its own `OwdEstimator`/`RttStats` per-path state
  by construction (INV-11). Shadow engine + single INV-15 kill switch.
- **Epoch tagging (RE-3, G3)** — fill `OwdSample.epoch`; selectors refuse to
  compare across epochs.
- **Switching (G4+)** — `DecisionFSM` + `FlapSuppressor` per-class parameters
  (§11.2), `RevertGuard` (G5), MTU governor (G6).

## 6. Accomplishment record (what exists, with proof)

| Milestone | Commit | Tests pinning it | Live evidence |
| :--- | :--- | :--- | :--- |
| G0 prerequisites (N-1 gate, X-1 reset, N-4 PTO, X-2/X-3 docs) | `1c0e488..f636948` series | named in `Closure-Matrix-2026-09.md` | 2026-09-04 WAN rounds, 0.00% loss |
| G1 measurement layer (A-1/A-2 timestamp→estimator→telemetry; A-6 factor; E-1/E-2) | `72c6120`, `5ed2430`, `8a27b12`, `3324013`, `1182f53` | 4 gate tests + integration, see `G1-closure-report.md` | live 2000-frame WAN: owd_var 1.541 ms, jitter 267 µs |
| Fabric (D-1) — per-direction links, scripted impairments, splitmix64 streams | `00eb110` | 4 fabric tests | — |
| RT-2 bounded event queue + net-server drain | `fe1046c` | 2 negative tests | production shape confirmed live (no drop warnings, EV lines in journalctl) |
| **G2 (D-2) connection-driven determinism + directional independence** | `1463e6e` | `same_seed_connection_driven_runs_are_byte_identical`, `per_direction_impairment_is_independently_visible` | — |
| `gtp-route` pure crate (B-1 prelude, B-3/B-9/B-10 slices) | `37afbb7` | 10 unit + `selection_from_sim` | — |
| Bidirectional exchange + `route-probe` shadow verdict + e2e + example | `07cbc47` | 3 report tests + `route_probe_e2e` | **3 live device↔VPS probes** (below) |
| Round closure (docs, §12) | `88fdd10`, `3d2c3cf` | gate ×3 at 168/168 | — |

Live probe numbers (device ↔ `92.222.80.200:7777`, 10 s @ 60 FPS, parity
`07cbc47`): forward owd_var **1486 / 1389 / 815 µs** vs reverse **892 / 1043 /
690 µs** — real directional separation every round; scores 0.931 / 0.933 /
0.943; 11/11 reports each; mid-probe loss burst on the report stream
recovered live by reliable retransmission (journalctl `Msg#36-42`).

**Test-count trajectory:** 95 (2026-09-04 entry) → 134 → 149 (G1) → **168**
(now). Raw captures: `/tmp/route-probe-{1,2,3}.txt` on the dev machine.

## 7. Quick reference — files & commands

| Need | Where |
| :--- | :--- |
| Estimator math | `crates/gtp-recovery/src/owd.rs` |
| Event/metrics surfaces | `crates/gtp-core/src/control/{events,metrics,handle,config}.rs`, `connection.rs` RX path |
| Report format | `crates/gtp-route/src/report.rs` |
| Scoring/confidence | `crates/gtp-route/src/score.rs` |
| Selection/health/reason codes | `crates/gtp-route/src/select.rs` |
| Multi-link simulation | `crates/gtp-sim/src/{fabric,fabric_runner}.rs` |
| CLI probe | `gtp-cli route-probe --server <addr> --duration <s>` |
| Runnable example | `cargo run -p gtp --example route_selection` |
| E2E exchange test | `crates/gtp/tests/route_probe_e2e.rs` |
| Plans/gates | `docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md`, `docs/routing/*` |

```bash
# Run the whole measurement+selection test surface
cargo test -p gtp-route -p gtp-sim -p gtp --all-targets
# Live bidirectional probe against the verification VPS
gtp-cli route-probe --server 92.222.80.200:7777 --duration 10
```
