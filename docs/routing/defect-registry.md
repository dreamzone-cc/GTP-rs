# Adaptive Routing — Defect Registry

> Numbered registry per ARDP v1.1 §12.1: every defect found and fixed during
> the routing-engine gates gets an entry — ID, description, root cause,
> violated invariant (ICD-01 §3.4), fix, the negative-checked test that pins
> it, and file:line evidence. Continues the project's N-/X-/FR- series culture
> with the new `RT-` prefix (routing-track defects).
>
> Rule (G-R4): **a fix without a test is not implemented.**

---

## RT-1 — `loss_ratio()` is a mixed-unit proxy, not a loss measurement

- **Found:** 2026-09-05 (WAN baseline round, stress-suite stage 2 output)
- **Description:** `DetailedMetrics::loss_ratio()` divides
  `total_retransmissions` (message-level retransmission events) by
  `total_tx_packets` (datagram count). Under heavy impairment the harness
  reports ratios like `1500.00%` (60 retransmissions over 4 datagrams); at
  WAN scale it silently under-reports (retransmissions land in both
  numerator and denominator). It is neither a packet-loss nor a message-loss
  measure.
- **Root cause:** two counters from different units combined when the
  original stress harness had a single pipe where the numbers happened to
  correlate.
- **Violated invariant:** none of the INV series directly; it is a
  measurement-honesty defect on the verification tool itself (extends
  standing item E-5 — `gtp-cli` and its metric surface are untested).
- **Impact:** cosmetic/verification-tooling only — the impairment verdicts
  are computed from delivered-message counters, not from this ratio.
- **Planned fix:** with E-5 — either report `retransmissions_per_datagram`
  honestly or derive loss from the ACK tracker; add golden-output tests for
  the CLI report. **Not fixed in G1** (no behavioral change to the protocol).
- **Evidence:** `crates/gtp-core/src/control/metrics.rs:45-51`;
  `crates/gtp-cli/src/main.rs:457,660-663`;
  `docs/reaudit/Live-WAN-Baseline-Report-2026-09-05.md` §6.1.

---

## RT-2 — Unbounded control event queue with no runtime consumer

- **Found:** 2026-09-05 (deep inspection round), fixed in the G2 development round (commit `fe1046c`)
- **Description:** `GtpConnection::event_queue` was an unbounded `Vec` and the
  only workspace consumer of `drain_events` was `control-demo` — live
  connections (`net-server`, `net-client`, the endpoint loops) never drained
  it. G1's bounded-rate `OwdSample` stream alone accumulated ~1.5 MB/hour per
  active connection on a long-lived server.
- **Root cause:** G1's design note (D6) bounded the *emission rate* because
  the queue was unbounded, but the queue itself was never bounded and the
  runtime consumers were never given a drain path.
- **Violated invariant:** INV-18-adjacent (measurement surfaces must not
  create unbounded state); operational memory-safety defect, not a protocol
  correctness defect.
- **Fix:** `GtpConfig::event_queue_capacity` (default 1024, all four presets +
  builder); central bounded `push_event` on both `GtpConnection` and
  `ConnectionControl` — overflow sheds the OLDEST event (newest information
  survives) and counts `ConnectionCold::total_dropped_events`, surfaced in
  `DetailedMetrics`; all nine internal push sites rewired; `net-server` drains
  on a 1 s cadence inside its per-connection select loop.
- **Pinned by (negative-checked):**
  `event_queue_is_bounded_and_drops_oldest_with_counter` (bound holds, oldest
  shed, counter exact, newest four survive, counter surfaces in metrics) and
  `event_queue_capacity_zero_drops_every_event` (most aggressive profile
  still counts what it sheds).
- **Evidence:** `crates/gtp-core/src/connection.rs` (`push_event`, tests),
  `control/handle.rs`, `control/config.rs`, `control/metrics.rs`,
  `state.rs` (`total_dropped_events`); live confirmation: `route-probe`
  sessions show zero drop warnings at 1024 capacity under 60 FPS traffic.

---

## RT-3 — Cross-endpoint reports carried no evidence age (stale reports undetectable)

- **Found:** 2026-09-05 (gap paper §3.3, confirmed by the independent verification in its Appendix C); fixed in the G3-prelude round.
- **Description:** the `MeasurementReport` format (`GTPRP1|var|jitter|srtt|samples`) carried no timestamp — a report describing the path state of minutes ago was indistinguishable from one describing now. A route decision could not detect stale evidence, violating the S04 requirement (stale excellence must never beat live honesty).
- **Root cause:** the G2 prototype's report was designed for a single 10 s probe round; age was implicitly bounded by the session. Nothing enforced it.
- **Violated invariant:** INV-11-adjacent (measurement scope: age is part of the scope) and the B-9 recency factor adopted in ARDP v1.1 (confidence = quality × count × **recency**) — the implementation had the count factor only.
- **Fix (per `G3-prelude-freshness-design.md`):**
  - `ConnectionHot::last_rx_time` set post-authentication only (INV-3), beside the OWD feed; surfaced as `DetailedMetrics::since_last_rx: Option<Duration>`.
  - `MeasurementReport` v2 (`GTPRP2|var|jitter|srtt|samples|since_last_rx`) — app-layer, zero wire change (§2.3); v1 still parses, its age reads `None` (unknown — never zero, never invented).
  - `PathStats::{fwd,rev}_age_us`; path age = the staler direction; `freshness()` — 1.0 within a 1 s grace, linear decay to 0 at 5 s; unknown age stays neutral (surfaced as `n/a`), enforcement applies only to carried ages.
  - `select()`: `effective = score × confidence × freshness`; a best candidate below the floor holds with the new reason code `STALE_EVIDENCE_HOLD` (no winner).
- **Pinned by (negative-checked):** `since_last_rx_tracks_only_authenticated_traffic` (INV-3: tampered traffic never moves the basis), `freshness_grace_then_linear_decay_to_zero`, `stale_evidence_holds_and_a_live_competitor_wins` (S04), `v1_reports_parse_with_unknown_age`, S11 `delayed_and_duplicate_reports_are_stale_aware_and_inert`, plus the e2e assertion that continuous 60 FPS traffic keeps the basis < 1 s.
- **Evidence:** `gtp-core/src/{state.rs,connection.rs,control/{metrics,handle}.rs}`; `gtp-route/src/{report.rs,score.rs,select.rs,lib.rs}`; live confirmation: the route-probe freshness line on the real WAN (see the round closure report).
