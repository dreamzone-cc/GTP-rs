# Design Note — Gate G2 Completion + Route-Selection Prototype Round

> **Gate:** completes G2 (D-2) and opens a labeled G3 prelude (a slice of
> B-1/B-3/B-9/B-10). Written **before** implementation, per ARDP v1.1 §12.1.
> **Round scope approved by the project owner:** full foundation + G2 + initial
> pure selector + a live bidirectional measurement prototype, validated over the
> real device↔VPS link.
> **Non-goals (stay per plan):** no path switching of any kind (G4), no
> measurement-connection plane or shadow engine/kill-switch (full B-2 at G3),
> no wire-format changes (exchanges ride `ReliableOrdered` app messages, §2.3),
> no 24 h dataset (G3).

## 1. Work items and target files

| Item | Target | Content |
| :--- | :--- | :--- |
| **RT-2 fix** (defect found 2026-09-05 inspection) | `gtp-core/src/connection.rs`, `control/handle.rs`, `control/config.rs`, `control/metrics.rs`, `state.rs` | Bound the control event queue: new `GtpConfig::event_queue_capacity` (default 1024, all four presets + builder); central `push_event` helpers on `GtpConnection` and `ConnectionControl` that drop the **oldest** event when full and count `ConnectionCold::total_dropped_events`; surface the counter in `DetailedMetrics`. Rationale: D6 bounded only the *emission rate* of `OwdSample`; the queue itself remains an unbounded `Vec` and no runtime loop drains it (only `control-demo` ever calls `drain_events`). |
| **D-2 / G2** | `gtp-sim/src/fabric_runner.rs` | `FabricRunner`: two `GtpConnection` endpoints driven through `SimulatedFabric` over N links (same constructor conventions as `SimulationRunner`: addrs, CID, opposite roles, virtual clock). `step()` = `tick_script(now)` → both endpoints produce → transmit per (link, direction) → `drain_ready` → dispatch by `dest`; per-link delivery counters. |
| **B-1 prelude + B-3/B-9/B-10 slices** | new crate `crates/gtp-route` (pure; no tokio, no gtp-core dep) | `PathStats` (per-direction `owd_var`/`jitter`, plus `rtt`/`loss_ratio_inputs` that exist today), continuous monotone `score()` over the RE-4 axes available now, a minimal multiplicative confidence factor (B-9: sample-count factor), and `select()` returning the chosen path **plus a machine-readable reason code** (B-10 slice). Pure functions, fully unit-testable, deterministic. |
| **Bidirectional exchange prototype** | `gtp-cli/src/main.rs` | `net-server`: per-connection 1 s task — `drain_events` (RT-2 companion) + send a compact `ReliableOrdered` report carrying the server-side measurements (client→node direction) to the client. New `route-probe` subcommand: 60 FPS traffic for `--duration`, collect local (node→device) metrics + server reports, print one **combined bidirectional table** plus the `gtp-route` shadow verdict (computes what it *would* select and why; executes nothing — INV-15 discipline). Runnable example under `crates/gtp/examples/`. |

## 2. Affected invariants (ICD-01 §3.4) and their tests

| Invariant | Impact | Test plan |
| :--- | :--- | :--- |
| **INV-15** (kill switch / shadow must not perturb) | The prototype computes a verdict and prints it; zero data-plane action | Negative test: `route-probe` loopback e2e asserts traffic + telemetry identical with verdict computed vs. not (same delivered counts); code review shows no actuation path exists |
| **INV-18** (every wire field has a consumer) | Unchanged — no wire change; reports are app payloads | Existing gate checks keep passing |
| **INV-13** (epoch separation) | `OwdSample.epoch` stays 0 (RE-3 is G3); the selector consumes per-connection stats only within one epoch by construction | Unit test: stats from a single window only; documented limitation |
| **INV-11** (measurement scope = decision scope) | Selector inputs are per-connection per-direction aggregates — the honest scope available today; cross-path comparison happens only in the fabric runner where both links share the same clock and traffic | Fabric integration test proves scope-consistent selection |
| **INV-14** (measurement independent of decision) | No feedback loop exists yet (shadow only) | By construction; noted for G4 |

## 3. G2 exit-criteria mapping (how this round closes the gate)

| Criterion (ARDP §6) | Evidence this round |
| :--- | :--- |
| Full determinism: same seed ⟹ byte-identical event sequence | `fabric_runner` test: two connection-driven runs with the same master seed produce identical fabric event logs **and** identical delivered-message sequences |
| Per-direction impairment demonstrably independent | `fabric_runner` test: +N ms on link-forward only ⇒ server-side `owd_var` rises while client-side stays ≈ 0 (and the symmetric case), asserted from `DetailedMetrics` |

## 4. Test inventory (new)

Unit: event-queue bound + drop-oldest + counter surfacing; report encode/decode;
`gtp-route` score/select/confidence/reason-codes/tie-breaks. Integration:
connection-driven determinism (2 runs); directional independence (both senses);
scripted mid-run impairment; selector picks the healthy link from real fabric
stats; `route-probe` loopback e2e (in-process endpoint pair) verifying the
bidirectional exchange end-to-end. Live: `route-probe` over the real WAN to the
VPS ×3 rounds (Stage 5), recorded in the round report.

## 5. Documentation plan (§12)

Defect-registry entry **RT-2** (event queue growth; found in the 2026-09-05
inspection, fixed here). Round closure report mapping every criterion above to
evidence; CHANGELOG; ARDP §8 traceability update; ENGINEERING-REFERENCE refresh;
README index entry; Arabic mirror under `arabic-local/` (git-ignored).
