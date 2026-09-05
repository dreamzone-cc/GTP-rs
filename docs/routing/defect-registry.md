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
