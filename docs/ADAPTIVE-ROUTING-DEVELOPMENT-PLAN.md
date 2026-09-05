# GTP-rs — Adaptive Routing Development Plan

> **Purpose.** Turn `GTP_Adaptive_Routing_Technical_Paper.md` into an executable
> engineering plan: every capability the paper asks for, checked against the code
> that exists today, reconciled with the architectural decisions already adopted
> in `GTPrs_Integrated_CrossLayer_Design_and_Audit__AR.md` (ICD-01), and ordered
> into phases with gates that can be run.
>
> **Verification basis.** Every "as-built" statement below was established by
> reading the source at the cited file and line on `main` @ `a47bb0e`, and by
> running the gate on that tree: **135 passed / 0 failed / 2 ignored**, clippy and
> `cargo fmt --check` clean, `scripts/verify_remediation.sh` → GATE: PASSED, plus
> the on-demand N-1 volume gate (1 passed). No claim here is carried over from a
> prior document without re-checking it.
>
> **Inputs.** The paper (§§1–10); ICD-01 (RE-1…RE-10, C-1…C-4, gates G0–G6);
> `docs/reaudit/Closure-Matrix-2026-09.md`; the codebase.
>
> **Status of this document.** Plan of record for the adaptive-routing work. It
> does not restate ICD-01's mechanism specifications — it schedules them, adds
> what the paper requires beyond them, and records where the paper and the code
> disagree.

---

## 1. Where the project actually stands

**Gate G0 of ICD-01 is complete.** It was the declared blocker for all
measurement work ("no measurement work may begin before G0 passes"), and every
item in it is now closed and pinned by a named test:

| G0 item | Status | Pinned by |
| :-- | :-- | :-- |
| N-1 — `is_long_header()` gate before handshake-type inspection | ✅ closed | `short_header_ciphertext_colliding_with_handshake_types_is_still_routed` + 5000-datagram volume gate |
| X-1 / C-2 — `RttStats` reset on validated migration, `Option` instead of the `u64::MAX` sentinel | ✅ closed | `min_rtt_follows_the_new_path_after_migration` |
| N-4 — PTO separated from RTO; no `cc.on_timeout` from the PTO path | ✅ closed | `test_pto_probe_does_not_collapse_cwnd`; `connection.rs:862` gates collapse on `PERSISTENT_CONGESTION_PTO_COUNT` |
| X-2 / X-3 — AAD conflict and nonce description corrected in `GTP-SEC-01` | ✅ closed | documentation round `f636948` |

Beyond G0, the 2026-09-04 round also closed N-2, N-3, N-5, N-7, FR-5, FR-7,
FR-8, FU-4, FU-5, WIR-2, WIR-4, SEC-14, X-19 and New-8/New-12. There are **no
open critical or high protocol defects.**

**Therefore: the project is positioned exactly at the start of G1.** The
prerequisite work is done and the engine can begin. What follows is what G1
onward actually requires, expanded to cover everything the paper asks for.

---

## 2. Reconciliation — the paper against the code

This is the core of the plan. Each row is a capability the paper assumes or
requires. "As built" is what the source says today.

### 2.1 Transport primitives the paper builds on

| Paper reference | Claim | As built | Verdict |
| :-- | :-- | :-- | :-- |
| §1.2, §3.1.1 | Four delivery semantics | `MessageClass` — all four present and exercised | ✅ **true** |
| §1.2, §3.1.2 | CUBIC + token-bucket pacing | `gtp-cc/src/cubic.rs`, pacing engine live; RFC 8312 TCP-friendly region and fast convergence implemented | ✅ **true** |
| §1.2, §3.3 | 5 priority tiers with DRR | `PriorityTier::ALL` (P0Control…P4BulkCosmetic); DRR with persistent cursor and deficit cap at `scheduler.rs:203-258` | ✅ **true** |
| §1.2, §3.2.1 | AEAD encryption | ChaCha20-Poly1305, AAD covers the full header | ✅ **true** |
| §1.2, §3.2.1 | 128-bit sliding replay window | `REPLAY_WINDOW_SIZE = 128`, `bitmap: [u64; 2]` (`replay.rs:3-9`) | ✅ **true** |
| §3.2.1 | "Stateless Cookie Tokens — verify exit-point identity without state" | `StatelessTokenManager` exists in `gtp-path/src/stateless_token.rs` and is fully unit-tested — **but has zero production callers.** It is never constructed outside its own tests | ❌ **not wired** → **A-8** |
| §3.2.1 | "3-Way Path Challenge/Response" | Path validation is **two-way**: `start_challenge` → `validate_response`. There is no third leg, and the N-6 documentation round explicitly corrected the docs to say two-way | ❌ **paper is wrong** → §3.1 |
| §5.2.2 | MTU optimizer with a per-path MTU cache | `initial_mtu`/`min_mtu`/`max_mtu`/`mtu_probe_interval` exist in config; `trigger_mtu_probe` and `ControlEvent::MtuUpdated` exist. **No PLPMTUD loop, no per-path cache, no overhead model** | ⚠️ **partial** → **B-6** |
| §4.1.1, §5.3 | "Congestion level" as a scoring input | `DetailedMetrics.queue_bytes_per_tier` is hardcoded `[0,0,0,0,0]` and all three ECN counters hardcoded `0` (`control/handle.rs:187,199-201`). `CongestionController::on_ecn` exists but **has zero callers** | ❌ **no signal exists** → **A-4, A-5** |
| §2.2.3, RE-1 | Directional / one-way measurements | `PacketHeader.timestamp_micros` is encoded and decoded but **consumed nowhere in `gtp-core`** — only printed by `gtp-cli dissect`. The field is on the wire, inside the AAD, and free | ❌ **not consumed** → **A-1** |

### 2.2 Capabilities the paper needs that do not exist at all

| Paper reference | Capability | Plan item |
| :-- | :-- | :-- |
| §2.2.2, §4.1 | Route scoring over multiple candidate paths | **B-1, B-3** |
| §4.2 | Flap suppression (hysteresis, hold, confirm, penalty, rate cap) | **B-4** |
| §4.3 | Asymmetric-routing detection and handling | **B-5** |
| §5.1.1 | Real-time BGP monitoring (RIPE RIS, RouteViews) | **C-1** |
| §5.1.2 | ML failure prediction | **C-2** |
| §2.2.1 | Smart entry point, route cache, exit-point fleet management | **C-3** |
| §6.1 | Multi-path simulation with per-direction impairment | **D-1** |
| §3.2.2 | Server certificates / path signing | **E-6 — blocking, see §4** |

---

## 3. Corrections the paper needs

These are places where the paper states something about GTP-rs that is not true,
or specifies something that would misbehave. They are listed because the paper is
an input to implementation: left uncorrected, each one becomes a defect.

### 3.1 Security features described as present but not wired

§3.2.1 tabulates "Stateless Cookie Tokens" and "3-Way Path Challenge/Response" as
GTP-rs features the routing system can lean on. Neither is available as
described. `StatelessTokenManager` is dead code with no caller, and path
validation is a two-way challenge/response. Any design that assumes an exit point
can be identity-checked statelessly today is building on nothing.

### 3.2 The scoring function is discontinuous at its own decision thresholds

`calculate_latency_score` (§4.1.2) is piecewise:

- at `rtt = 49.9ms` → `1.0`; at `rtt = 50.0ms` → `0.9 - 0/500 = 0.9`
- at `rtt = 199.9ms` → `≈0.70`; at `rtt = 200.0ms` → `0.0`

Two step discontinuities, one of them a **0.70 → 0.0 cliff**, sitting exactly
where paths cluster. A path oscillating by ±0.1ms around 50ms produces a 0.1
score swing with no change in real quality; around 200ms it produces a 0.7 swing.
That is a flap generator built into the scorer — and §4.2 then spends five
mechanisms suppressing flapping the scorer itself manufactures.

**Resolution:** adopt ICD-01 RE-4's continuous normalization. Keep the paper's
*weights* (they are a reasonable starting point) and its *criteria*; replace the
step function with a monotone continuous mapping. Calibrate against the 24-hour
shadow dataset from G3 rather than shipping the constants as final.

### 3.3 Priority is not derived from delivery semantics

§3.3's `determine_priority` maps `DeliverySemantics` → queue index. In GTP-rs,
`PriorityTier` is an independent 5-tier enum carried in `MessageOptions`; a
`ReliableOrdered` message can legitimately be `P1Input`, and an `Unreliable` one
`P4BulkCosmetic`. Collapsing the two would silently re-prioritize traffic and
break the DRR weights that N-3 was fixed to honour. The routing engine must read
`PriorityTier`, never infer it.

### 3.4 Multipath congestion control is already rejected

§5.3.2 specifies a `MultipathCongestionController` allocating bandwidth across
paths. **ICD-01 §6.4 decided against multipath inside a connection**, on the
grounds that `GTP-ARCH-01` fixes "one connection owns one active path, one
congestion controller, one pacing engine", and breaking it requires rewriting
`gtp-cc`, `gtp-recovery`, `gtp-path` and `gtp-core` — invalidating the audited
invariants and the whole test suite.

**The adopted alternative stands: separate the data plane from the measurement
plane.** One data connection with one CC; a small independent measurement
connection per candidate (RE-2). This gives per-path `RttStats` structurally,
costs no new wire format, reuses the existing `Ping { nonce }` frame as the probe
carrier, and removes the classic adaptive-routing pathology where measurement
perturbs what it measures. §5.3.2 is therefore **out of scope**, and this plan
does not schedule it.

### 3.5 The integration test in §6.3 uses an API that does not exist

`AsyncGtpConnection::new("test_config.toml")` — the real constructor is
`AsyncGtpConnection::new(conn: GtpConnection, rx_channel: mpsc::Receiver<..>)`,
and there is **no file-based configuration anywhere in the workspace**: no
`serde` dependency, no TOML loader, `GtpConfig` is built in code. Likewise
`cubic.congestion_window()` (§3.1.2) is `CongestionController::cwnd(&cc)`.

**Decision required (see A-7):** either add configuration loading, or correct the
paper's examples. Adding `serde` to the core has a real cost and no current
consumer; the recommendation is to add file-based config in the *adapter* crate
(`gtp-route-tokio`), where operators actually need it, and leave `gtp-core`
dependency-free.

### 3.6 The MTU overhead model understates the GTP header

§5.2.2 uses `gtp_overhead = 8`. The GTP long header carries CID, packet number,
timestamp and flags, plus a 16-byte AEAD tag; 8 bytes is not the real figure.
Since **fragmentation does not exist** (CORE-2 deferred; `FR-1` makes oversized
sends fail cleanly rather than stall), an MTU governor that overestimates the
usable payload converts directly into `PayloadTooLarge` errors at the API.
Hence RE-8's raise-only constraint and gate G6's "zero `PayloadTooLarge`
attributable to a routing decision" (INV-19).

---

## 4. The blocking issue the paper does not address

**SEC-5 / DEF-1 — there is no server authentication.**

The handshake is anonymous X25519. Nothing binds the key exchange to an identity,
so an active on-path attacker can impersonate an entry or exit point and
terminate the tunnel. The deferral rationale on record is that the scope is
"simulation / LAN".

The paper's system is not simulation or LAN. It is a product that carries
players' traffic across the public internet through third-party VPS hops, and
§3.2.2 itself asks for "server certificates" and "path signing" — without noting
that neither the certificate mechanism nor any signing primitive exists.

This is not a routing-engine feature; it is a precondition for the product. A
route-selection layer that steers traffic between exit points it cannot
authenticate is choosing which unauthenticated party to trust.

**Consequence for this plan:** SEC-5 is reclassified from *deferred* to
**blocking for any deployment carrying real user traffic** (item **E-6**). It
does not block G1–G3, which are measurement and shadow mode on hosts we control.
It blocks G4 onward in production, and it blocks beta.

---

## 5. Work breakdown

Five tracks. Track A unblocks Track B; Tracks C and D run in parallel; Track E is
the standing backlog.

### Track A — core enablers (`gtp-core`, `gtp-cc`, `gtp-io`)

Total core footprint stays small and mostly negative-cost, as ICD-01 §6.3 argues.

| ID | Item | Files | Effort | Gate | Depends |
| :-- | :-- | :-- | :-- | :-- | :-- |
| **A-1** | Consume `timestamp_micros` in RX → sliding `base_d` (30s window), OWD variance, RFC 3550 §6.4.1 jitter. Zero allocation in the RX path | `gtp-core/src/connection.rs` | ~30 lines | G1 | — |
| **A-2** | `ControlEvent::OwdSample`, `ControlEvent::PathEventDetected`; surface `owd_var` and `jitter` on `DetailedMetrics` | `control/events.rs`, `control/metrics.rs`, `control/handle.rs` | ~20 lines | G1 | A-1 |
| **A-3** | Multi-slot path challenges **or** explicit cancellation (X-8). Folds in the expired-challenge cleanup: `pending_challenge` is currently cleared on success only (`path_validator.rs:43-59`), so `pending_addr()` can name an address whose challenge timed out | `gtp-path/src/path_validator.rs`, `gtp-core/src/control/handle.rs` | ~40 lines | G4 | — |
| **A-4** | Real per-tier queue accounting in `DetailedMetrics` (CC-11). The data already exists in the scheduler — there is no getter | `gtp-scheduler`, `control/handle.rs` | small | G3 | — |
| **A-5** | ECN end to end: read/set `IP_TOS`/`IPV6_TCLASS` on the socket, populate `RecvDatagram.ecn`, call `CongestionController::on_ecn`, export real counters | `gtp-runtime-tokio`, `gtp-core`, `gtp-cc` | medium | G6 | A-4 |
| **A-6** | New-11: `anti_amplification_factor` is declared in `control/config.rs:40` and set in four profiles (one to `10`) but read nowhere — the limiter hardcodes `saturating_mul(3)`. Make it live or delete it | `gtp-path/src/anti_amplification.rs`, `control/config.rs` | small | G1 | — |
| **A-7** | Decide configuration strategy (§3.5): file-based config in the adapter crate, `gtp-core` stays dependency-free | design | — | G3 | — |
| **A-8** | Wire `StatelessTokenManager` into the handshake as a Retry-style address-validation cookie, or formally record it as unused and correct §3.2.1 | `gtp-runtime-tokio/src/endpoint.rs`, `gtp-path` | medium | G4 | — |

**A-1 and A-3 are the only hard prerequisites for the engine.** A-1 is the whole
of RE-1; A-3 is what lets more than one candidate be probed at a time.

### Track B — the routing engine (new crates, outside the hot path)

Two new crates per ICD-01 §6.2: `gtp-route` (pure, no tokio, no `gtp-core`,
injected clock, fully deterministic) and `gtp-route-tokio` (thin adapter).

| ID | Item | Mechanism | Gate |
| :-- | :-- | :-- | :-- |
| **B-1** | `gtp-route` skeleton: `PathStats<P>`, `trait RouteTarget`, epoch tagging, deterministic clock injection | RE-3 | G3 |
| **B-2** | `gtp-route-tokio`: independent measurement connection per candidate, batched probing over the existing `Ping { nonce }` frame, 1 Hz `DetailedMetrics` + `drain_events` polling, **shadow mode**, single kill switch restoring baseline exactly (INV-15) | RE-2 | G3 |
| **B-3** | Continuous scorer over the paper's five criteria and weights (RTT 0.30, jitter 0.25, loss 0.20, stability 0.15, congestion 0.10), replacing the discontinuous step function of §4.1.2 (see §3.2) | RE-4 | G3 |
| **B-4** | `DecisionFSM` (STEADY→LEADING→CONFIRM→SWITCH→VERIFY) + `FlapSuppressor`: hysteresis ≥20%, hold ≥30s, confirmation window 10s, flap penalty capped at 0.5, ≤3 switches/minute — the paper's §4.2 parameters | RE-5 | G4/G5 |
| **B-5** | Path-event discriminator: reroute vs congestion; directional degradation diagnosis from A-1's OWD data (paper §4.3) | RE-6 | G5 |
| **B-6** | `RevertGuard`: post-switch verification and automatic rollback when the candidate measured better but performs worse | RE-7 | G5 |
| **B-7** | Constrained MTU governor: PLPMTUD, **raise-only**, real overhead model (see §3.6), per-path cache | RE-8 | G6 |
| **B-8** | Feed `total_stale_drops` into scoring; reference-path comparison (`tunnel_advantage_ms`) so the engine can recommend the direct path | RE-9, RE-10 | G6 |

### Track C — external services (no GTP-rs dependency, parallel)

These carry no protocol risk and can proceed independently at any time.

| ID | Item | Notes |
| :-- | :-- | :-- |
| **C-1** | BGP monitoring ingest (RIPE RIS, RouteViews); hijack and large-change detection; alerting | Paper §5.1.1. Consumes public feeds; produces advisory input to the scorer |
| **C-2** | Failure-prediction model on the shadow dataset | Paper §5.1.2. **Requires G3's 24-hour dataset first** — there is nothing to train on before that |
| **C-3** | Control plane: exit-point fleet management, deployment automation, dashboards, Redis/PostgreSQL state | Paper §2.2.1, §8.2, §9.2.2 |

### Track D — simulation and test infrastructure

| ID | Item | Notes |
| :-- | :-- | :-- |
| **D-1** | `SimulatedFabric`: replace the single-pipe `SimulatedNetwork` (`BinaryHeap`, one profile per call) with per-candidate links, **independent forward/reverse profiles**, a time-scripted impairment schedule, and an optional shared bottleneck | Per-direction profiles are not a nicety: without them the directional detection in A-1/B-5 **cannot be tested at all** |
| **D-2** | Determinism gate: two runs with the same master seed produce byte-identical event sequences (`splitmix64(master, i)` per link) | G2 |
| **D-3** | WAN measurement protocol and baseline capture on a pinned build, before any engine judgement | Principle P5 |

### Track E — standing backlog (from the 2026-09-05 re-examination)

| ID | Severity | Item |
| :-- | :-- | :-- |
| **E-1** | Medium | The comment above the anti-amplification send gate (`connection.rs:1083-1101`) still describes New-12 as an open, unbounded defect and instructs the reader that the reflection hole is unfixed. New-12 closed it (`connection.rs:595-648`). Comment edit only |
| **E-2** | Low | `RttStats.min_rtt` is a `pub` field still holding the `u64::MAX` sentinel (`rtt.rs:12,23`); N-5 converted only the consumer surfaces. Make it `pub(crate)` |
| **E-3** | Low | Expired-challenge cleanup — folded into **A-3** |
| **E-4** | Low | New-13: non-directed control frames drained *before* a directed frame ride the same datagram to `override_dest` (`connection.rs:974-1000`). New-12 removed remote triggerability, so this is now a bounded local-information concern |
| **E-5** | Note | `gtp-cli` is 1,106 lines with zero tests — and it is the verification tool (`dissect`, `stress-suite`, `sim-benchmark`) |
| **E-6** | **Blocking for production** | SEC-5 server authentication — see §4 |

---

## 6. Phases and gates

G0 is closed. G1–G6 are ICD-01's, with the additions this plan introduces marked.

| Phase | Contents | Exit gate |
| :-- | :-- | :-- |
| **G1 — measurement layer** | A-1, A-2, A-6 | `owd_var` within ±1 ms of an injected known delay; timestamp wrap at `2^32 − ε` produces no false jump; 30 s at 50 ppm clock drift ≤ 1.5 ms error; **zero allocation** in RX; jitter from real 60 Hz gameplay traffic is non-zero and stable |
| **G2 — multi-path simulation** | D-1, D-2 | Full determinism: same seed ⟹ byte-identical event sequence. Per-direction impairment demonstrably independent |
| **G3 — shadow mode** | B-1, B-2, B-3, A-4, A-7 | 24 h of WAN shadow operation: complete score log, **zero migrations**; performance matches baseline within measurement noise (negative test for INV-15); batch-measured jitter ≈ passively measured jitter on the same path; **weights calibrated from the 24 h dataset** |
| **G4 — failover only** | B-4 (failover class only), A-3, A-8 | Simulated active-path death ⟹ recovery < 500 ms with `detect`/`confirm`/`switch` exported separately; real WAN path cut ⟹ session survives and reliable delivery completes; 10 min at 5% non-catastrophic loss ⟹ **zero** false migrations |
| **G5 — degradation + verification** | B-4 (full), B-5, B-6 | `t=30s` degradation of +40 ms/+5% ⟹ migration within < 2 s; a candidate that measures better but performs worse ⟹ automatic revert inside the verification window; 30 min of ±5 ms RTT oscillation around 50 ms ⟹ `switch_count = 0`; return-path-only degradation diagnosed correctly with no blind entry migration |
| **G6 — optimization + governance** | B-7, B-8, A-5 | `switch_count ≤ 2/min` across all scenarios; `post_switch_delta > 0` in ≥ 80% of switches; `tunnel_advantage_ms > 0` or the engine recommends the direct path; **zero** `PayloadTooLarge` attributable to a routing decision |
| **G7 — production readiness** *(added by this plan)* | E-6, C-3 | Server authentication implemented and adversarially tested; exit-point fleet under management; no deployment carrying real user traffic before this gate |

Every phase inherits the standing rules: the local verification gate must stay
green (`cargo test --workspace`, clippy `-D warnings`, `cargo fmt --check`,
`scripts/verify_remediation.sh`), each fix carries a negative-checked test, and
the kill switch is exercised as a negative test in every phase (P4).

**Ordering rule (P1): fix the seams the measurement depends on before building
the measurement.** G0 satisfied this; A-1 and A-4 extend it. An engine on
corrupt inputs is worse than no engine.

---

## 7. Acceptance criteria from the paper

The paper's §6.2 targets are adopted as the engine's service-level objectives.
They are product metrics, not unit-test assertions — measured by D-3's WAN
protocol.

| Metric | Minimum | Target | Measured by |
| :-- | :-- | :-- | :-- |
| RTT | < 100 ms | < 50 ms | active probing (B-2) |
| Jitter | < 30 ms | < 10 ms | A-1, RFC 3550 §6.4.1 |
| Packet loss | < 2% | < 0.5% | loss detector counters |
| Route switch time | < 500 ms | < 200 ms | G4 `detect`/`confirm`/`switch` split |
| Route flap rate | < 3/min | < 1/min | `switch_count`, gate G6 |
| End-to-end latency | < 150 ms | < 80 ms | D-3 |

The paper's forecast improvements (§10.3: 30–50% stability, 20–40% latency,
60–80% loss reduction) are **projections, not commitments.** They must be
restated against G3's measured baseline before they appear in any external
material. Principle P5 exists for exactly this.

---

## 8. Traceability

| Paper § | Subject | Plan item | State |
| :-- | :-- | :-- | :-- |
| 2.2.1 | Smart entry point | C-3 | not started |
| 2.2.2 | Adaptive routing engine | B-1…B-4 | not started |
| 2.2.3 | Advanced telemetry | A-1, A-2, A-4 | not started |
| 3.1.1 | Delivery semantics | — | ✅ already built |
| 3.1.2 | CUBIC integration | — | ✅ built; §3.5 corrects the example API |
| 3.2.1 | Security feature table | A-8 | ❌ two entries incorrect — §3.1 |
| 3.2.2 | Certificates, path signing | E-6 | ❌ blocking — §4 |
| 3.3 | DRR scheduling | — | ✅ built; §3.3 corrects the priority mapping |
| 4.1 | Route scoring | B-3 | scorer redesigned — §3.2 |
| 4.2 | Anti-flapping | B-4 | parameters adopted as-is |
| 4.3 | Asymmetric routing | A-1, B-5 | needs OWD first |
| 5.1.1 | BGP monitoring | C-1 | parallel track |
| 5.1.2 | ML prediction | C-2 | blocked on G3 dataset |
| 5.2 | MTU optimization | B-7 | overhead model corrected — §3.6 |
| 5.3.1 | Gaming-tuned CUBIC | — | partly present (`competitive_fps` profile); revisit after G6 |
| 5.3.2 | Multipath CC | — | ❌ **out of scope** — §3.4 |
| 6.1–6.3 | Test strategy | D-1…D-3 | §3.5 corrects the test API |
| 7.2 | Cost and ROI | — | business track, out of engineering scope |
| 9.1 | Roadmap | §6 | superseded by the gate model |

---

## 9. Risks

| # | Risk | Mitigation |
| :-- | :-- | :-- |
| R1 | Measurement perturbs what it measures | Structurally removed by the measurement-plane separation (§3.4) |
| R2 | Scorer manufactures the flapping the suppressor then fights | Continuous scoring (§3.2) + weights calibrated on real data, not assumed |
| R3 | Engine ships on projections rather than a measured baseline | P5; §7 restates the paper's forecasts as projections; G3 produces the baseline |
| R4 | Core creep — the engine leaks into the hot path | `gtp-route` has no dependency on `gtp-core`; core footprint capped at C-1…C-4 (~100 lines); zero hot-path additions |
| R5 | Unauthenticated exit points | E-6 / G7 — no production traffic before server authentication |
| R6 | MTU governor produces `PayloadTooLarge` because fragmentation does not exist | Raise-only governor (RE-8) + explicit G6 gate |
| R7 | Single-disk continuity: 37 commits and the whole audit history exist in one place | Radicle repo initialized (`rad:z37j412vAtH1SGVYda6Uw81GXaK68`) but **private**, so seeds do not replicate it. An off-machine copy remains an open action |

---

## 10. Immediate next actions

1. **E-1** — correct the stale send-gate comment. Minutes, zero behavioural risk,
   and it currently misinforms anyone reading the anti-amplification path.
2. **A-1 + A-2** — the measurement layer. This is G1, it is the whole of RE-1, it
   is ~50 lines of core change, and it turns a field already on the wire into the
   engine's primary input.
3. **D-1** — `SimulatedFabric` with per-direction profiles, in parallel. Nothing
   in A-1/B-5 can be tested without it.
4. **A-6** — resolve `anti_amplification_factor` one way or the other; dead
   configuration that four profiles set is a trap.
5. **E-6** — open the server-authentication design (PSK or signature). It is the
   longest-lead item and the only one that blocks the product rather than a
   phase.

Items 1–4 are unblocked today. Item 5 is a design milestone, not an
implementation task, and should start now precisely because it is slow.

---

*Plan of record. Supersedes the roadmap in `GTP_Adaptive_Routing_Technical_Paper.md` §9.1;
builds on `GTPrs_Integrated_CrossLayer_Design_and_Audit__AR.md` (ICD-01) §§6–9, which remains
the mechanism specification. Verified against `main` @ `a47bb0e`, 2026-09-05.*
