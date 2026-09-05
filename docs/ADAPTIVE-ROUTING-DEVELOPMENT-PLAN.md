# GTP-rs — Adaptive Routing Development Plan

> **Version.** 1.1 — 2026-09-05 (v1.0 issued the same day). v1.1 adds: a
> re-verification round with two corrections to v1.0 (see "v1.1 corrections"
> below), reconciliation of the second, platform-level paper (§2.3), two
> resolved design conflicts (§11), a mandatory documentation protocol (§12),
> new work items B-9…B-13, C-4, C-5, D-4, and updates to the phase/gate table.
>
> **Purpose.** Turn `GTP_Adaptive_Routing_Technical_Paper.md` into an executable
> engineering plan: every capability the paper asks for, checked against the
> code that exists today, reconciled with the architectural decisions already adopted
> in `GTPrs_Integrated_CrossLayer_Design_and_Audit__AR.md` (ICD-01), and ordered
> into phases with gates that can be run.
>
> **Verification basis.** Every "as-built" statement below was established by
> reading the source at the cited file and line on `main` @ `a47bb0e`, and by
> running the gate on that tree: **135 passed / 0 failed / 2 ignored**, clippy and
> `cargo fmt --check` clean, `scripts/verify_remediation.sh` → GATE: PASSED, plus
> the on-demand N-1 volume gate (1 passed). No claim here is carried over from a
> prior document without re-checking it. Claims added in v1.1 were re-verified
> the same way against the same tree (see the v1.1 corrections list in §1).
>
> **Inputs.** The paper (§§1–10); the second paper
> (`GTP Adaptive Routing — ورقة تقنية تنفيذية ومعمارية النظام.md` — a
> platform-architecture treatment written independently of the GTP-rs
> internals; reconciled in §2.3); ICD-01 (RE-1…RE-10, C-1…C-4, gates G0–G6);
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

> **v1.1 update (2026-09-05, later the same day): gate G1 is complete.**
> A-1, A-2, A-6, E-1, E-2 landed; D-1 delivered ahead of G2; E-6 design
> opened (ADR-006 decisions enumerated); INV-18 checks added to the gate.
> 134 → 149 tests green (three consecutive full-gate runs); deployed to both
> ends at parity `00eb110`; live WAN round shows non-zero RFC 3550 one-way
> jitter (267 µs) from real 60 FPS traffic. Evidence:
> `docs/routing/G1-closure-report.md`. **The project is now at the start of
> G2.**

### v1.1 corrections to this plan

A second verification pass against the same tree (`main` @ `a47bb0e`) corrected
two v1.0 claims and confirmed the rest of the standing backlog:

| v1.0 claim | v1.1 verdict | Evidence |
| :-- | :-- | :-- |
| A-8: `StatelessTokenManager` has zero production callers | **Refuted — it is wired.** Constructed in `GtpEndpoint::bind` (`gtp-runtime-tokio/src/endpoint.rs:89`), issues cookies on ClientHello (`endpoint.rs:398`), and strictly verifies them in the `HandshakeFinish` handler (`endpoint.rs:496`). The CHANGELOG 0.2.0 entry matches reality | `endpoint.rs:67,89,398,496` |
| A-4: "the scheduler has the data but there is no getter" | **Half refuted — the getter exists.** `GameScheduler::queue_tier_bytes` (`gtp-scheduler/src/scheduler.rs:299`) and `queue_tier_items` (`:304`) already expose per-tier accounting; the fix shrinks to calling them from `query_metrics` instead of hardcoding zeros | `scheduler.rs:289,299,304`; `control/handle.rs:164,187` |
| A-6: `anti_amplification_factor` is dead config; the limiter hardcodes ×3 | **Confirmed** | `control/config.rs:40,92,129,166,209`; `gtp-path/src/anti_amplification.rs:31`; constructed no-arg at `gtp-core/src/state.rs:319` |
| E-1: stale New-12 comment at the send gate | **Confirmed** | `connection.rs:1080-1101` vs the closed fix at `:595-648` |
| E-2: `RttStats.min_rtt` is a `pub` field still on the `u64::MAX` sentinel | **Confirmed** | `rtt.rs:12,23,42` |
| E-5: `gtp-cli` is 1106 lines with zero tests | **Confirmed** | `crates/gtp-cli/src/main.rs`; no `tests/` dir |

Consequences: **A-8 is withdrawn** as an engineering item (reduced to the
§2.1/§3.1 corrections already applied below), and **A-4 shrinks** to wiring an
existing getter. Nothing else in v1.0 changed verdict.

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
| §3.2.1 | "Stateless Cookie Tokens — verify exit-point identity without state" | `StatelessTokenManager` **is wired end-to-end** (v1.1 correction of v1.0): constructed in `GtpEndpoint::bind`, issues cookies on ClientHello, constant-time verified in `HandshakeFinish` | ✅ **true** — `endpoint.rs:89,398,496` |
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

### 2.3 The second paper (platform architecture) against this plan — v1.1

`GTP Adaptive Routing — ورقة تقنية تنفيذية ومعمارية النظام.md` approaches the
same product from platform-engineering first principles, written **without**
knowledge of the GTP-rs internals. That independence cuts both ways: it
contributes operational machinery v1.0 of this plan lacked, and it proposes
rebuilding components that already exist and are audited. Reconciliation:

**Adopted — mapped into the existing tracks:**

| From the second paper | Value | Lands in |
| :-- | :-- | :-- |
| Confidence as a separate scoring dimension (§15: sample quality × count × recency) | Deterministic and testable; blocks decisions on thin evidence without adding a flap source | **B-9**, G3 |
| Explainable decision log (§16, §41: structured per-decision record with reason code, deltas, confidence, gate outcomes) | Debuggability now; training data for C-2 later | **B-10**, G3 onward |
| Path lifecycle `Unknown→Probing→Healthy→Degraded→Unhealthy→Recovering` with an observation window before re-qualification (§9.2, §77) | Complements the decision FSM; gives RE-5 cooldowns a first-class state instead of a timer | **B-11**, G3/G5 |
| Per-feature runtime kill switches (§65) | Generalizes INV-15's single kill switch without weakening it — each decision class fails safe independently | **B-12**, G3 |
| Switch-quality KPIs (§52): false-switch rate (reverts/switches), migration packet loss/reorder, probe success rate | Makes switch *quality* measurable, not just switch speed | **B-13**, G5/G6 |
| Chaos scenario catalog (§49–50: Tests A–F plus blackhole, node death, MTU mismatch) | Maps cleanly onto the existing gates | **D-4** |
| Exit-fleet diversity: provider / ASN / datacenter failure domains (§70–72) | Kills "fake multipath" — three exits on one provider are one path | **C-4**, G7 |
| Staged rollout 1→100% with automatic halt on regression, plus control/treatment experiments (§66–67) | Production governance; generalizes the mandatory parallel shadow round | **C-5**, G7 |
| Three-way benchmark: baseline VPN vs adaptive V1 vs adaptive+intelligence under identical conditions (§53) | Extends `tunnel_advantage_ms` into a repeatable methodology | **D-3** (extended) |
| Entry/exit node services as a distinct layer (§5–8, §37–38: sessions, forwarding, fleet control) | The one real gap in v1.0 — C-3 was a single line | **C-3** (expanded) |

**Rejected — would rebuild what exists or violate adopted invariants:**

| Proposal | Why rejected |
| :-- | :-- |
| New workspace with its own `gtp-core` / `scheduler` / `congestion` / `security` / `mtu` crates (§8) | All exist, are audited (135 tests green), and carry a closed defect history; rebuilding discards both |
| 4-tier priority derived from delivery semantics (§27) | GTP-rs carries an independent 5-tier `PriorityTier`; §3.3 of this plan already forbids inferring one from the other |
| 1 Hz probe interval as the measurement model (§11–12, §39) | X-5: 1 Hz structurally cannot measure jitter; RE-2 batch probing replaces it |
| Uniform hold-down (30 s) and uniform switch parameters (§17–20, §39) | ICD-01 RE-5 separates Failover/Degradation/Optimization budgets — see §11.2 |
| A new control protocol (§30: SESSION_INIT, PATH_*, METRIC_UPDATE, …) | Duplicates wire frames and handshake stages that exist (`PATH_CHALLENGE`/`RESPONSE`, `Ping`, ClientHello/ServerHello/HandshakeFinish); what is genuinely missing rides as `ReliableOrdered` application messages — no wire change |
| Weighted multipath allocation across active paths (§34–35) | §3.4 below: out of scope; GTP-ARCH-01 keeps one active path per connection. Flow-affinity (§36) is recorded as a design precondition for any future multipath |

**Consistent with ICD-01 — recorded as entry conditions for Track C:**
deterministic-before-AI (§Principle 3); ML advisory-only, never executing
switches directly (§33); BGP as a correlated risk signal, never an autonomous
decision input — "BGP changed + stable metrics = do not switch" (§24, §31).

---

## 3. Corrections the paper needs

These are places where the paper states something about GTP-rs that is not true,
or specifies something that would misbehave. They are listed because the paper is
an input to implementation: left uncorrected, each one becomes a defect.

### 3.1 Security features described as present — one correction stands, one retracted (v1.1)

§3.2.1 tabulates "Stateless Cookie Tokens" and "3-Way Path Challenge/Response"
as GTP-rs features the routing system can lean on. The **cookie entry is
correct**: v1.0 of this plan wrongly called the manager dead code — it is wired
end-to-end in the endpoint (`endpoint.rs:89,398,496`), and v1.1 retracts that
claim. The **challenge entry is still wrong as written**: path validation is
**two-way** (`start_challenge` → `validate_response`); there is no third leg,
and the N-6 documentation round already corrected the docs to say so. Any
design that assumes a three-way validation handshake exists is building on
nothing.

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
| **A-4** | Real per-tier queue accounting in `DetailedMetrics` (CC-11). v1.1: the getter already exists (`queue_tier_bytes` / `queue_tier_items`, `scheduler.rs:299,304`) — the fix is only to call it from `query_metrics` and stop hardcoding `[0,0,0,0,0]` (and the zeroed ECN counters until A-5 makes them real) | `control/handle.rs` (+ scheduler calls) | small | G3 | — |
| **A-5** | ECN end to end: read/set `IP_TOS`/`IPV6_TCLASS` on the socket, populate `RecvDatagram.ecn`, call `CongestionController::on_ecn`, export real counters | `gtp-runtime-tokio`, `gtp-core`, `gtp-cc` | medium | G6 | A-4 |
| **A-6** | New-11: `anti_amplification_factor` is declared in `control/config.rs:40` and set in four profiles (one to `10`) but read nowhere — the limiter hardcodes `saturating_mul(3)`. Make it live or delete it | `gtp-path/src/anti_amplification.rs`, `control/config.rs` | small | G1 | — |
| **A-7** | Decide configuration strategy (§3.5): file-based config in the adapter crate, `gtp-core` stays dependency-free | design | — | G3 | — |
| **A-8** | *Withdrawn in v1.1.* `StatelessTokenManager` is already wired as a strict HandshakeFinish cookie (`endpoint.rs:89,398,496`). Residue is documentation only, and it is done: §2.1 row and §3.1 above are corrected, and the paper's cookie row stands as verified-true | — | doc-only | closed (v1.1) | — |

**A-1 and A-3 are the only hard prerequisites for the engine.** A-1 is the whole
of RE-1; A-3 is what lets more than one candidate be probed at a time.

### Track B — the routing engine (new crates, outside the hot path)

Two new crates per ICD-01 §6.2: `gtp-route` (pure, no tokio, no `gtp-core`,
injected clock, fully deterministic) and `gtp-route-tokio` (thin adapter).

| ID | Item | Mechanism | Gate |
| :-- | :-- | :-- | :-- |
| **B-1** | `gtp-route` skeleton: `PathStats<P>`, `trait RouteTarget`, epoch tagging, deterministic clock injection | RE-3 | G3 |
| **B-2** | `gtp-route-tokio`: independent measurement connection per candidate, batched probing over the existing `Ping { nonce }` frame, 1 Hz `DetailedMetrics` + `drain_events` polling, **shadow mode**, single kill switch restoring baseline exactly (INV-15) | RE-2 | G3 |
| **B-3** | Continuous scorer replacing the discontinuous step function of §4.1.2 (see §3.2). v1.1 / §11.1: ICD-01 RE-4's axes (rtt_p50, tail, jitter, loss, stale_drop, instability) are the primary configuration; the paper's five axes/weights run alongside as a comparison configuration, both calibrated on the G3 shadow dataset | RE-4 | G3 |
| **B-4** | `DecisionFSM` (STEADY→LEADING→CONFIRM→SWITCH→VERIFY) + `FlapSuppressor`. v1.1 / §11.2: per-class parameters (ICD-01 RE-5) govern — Failover immediate, Degradation at the paper §4.2 defaults, Optimization slower; flap penalty capped at 0.5, decayable | RE-5 | G4/G5 |
| **B-5** | Path-event discriminator: reroute vs congestion; directional degradation diagnosis from A-1's OWD data (paper §4.3) | RE-6 | G5 |
| **B-6** | `RevertGuard`: post-switch verification and automatic rollback when the candidate measured better but performs worse | RE-7 | G5 |
| **B-7** | Constrained MTU governor: PLPMTUD, **raise-only**, real overhead model (see §3.6), per-path cache | RE-8 | G6 |
| **B-8** | Feed `total_stale_drops` into scoring; reference-path comparison (`tunnel_advantage_ms`) so the engine can recommend the direct path | RE-9, RE-10 | G6 |
| **B-9** | Confidence dimension in the scorer: `confidence = sample_quality × sample_count_factor × measurement_recency`, applied multiplicatively to the quality score; below a floor the FSM may confirm but never fast-switch (v1.1) | RE-4 extension | G3 |
| **B-10** | Explainable decision log: every decision — including shadow-mode non-decisions — emits a structured record (scores, per-axis deltas, confidence, hysteresis/hold/penalty outcomes, reason code); feeds the RE-10 export and later C-2 training data (v1.1) | RE-10 extension | G3 |
| **B-11** | Path lifecycle `Unknown→Probing→Healthy→Degraded→Unhealthy→Recovering`, with an observation window before a cooled-down path re-enters the candidate set (v1.1) | RE-5 extension | G3/G5 |
| **B-12** | Runtime enable flags per decision class (failover / degradation / optimization), layered under the single INV-15 kill switch; each flag's off-path is a negative test (v1.1) | INV-15 extension | G3 |
| **B-13** | Switch-quality KPIs: false-switch rate (`revert_count / switch_count`), migration packet loss and reordering, probe success rate — exported separately in RE-10 and gated in G5/G6 (v1.1) | RE-10 extension | G5/G6 |

### Track C — external services (no GTP-rs dependency, parallel)

These carry no protocol risk and can proceed independently at any time.

| ID | Item | Notes |
| :-- | :-- | :-- |
| **C-1** | BGP monitoring ingest (RIPE RIS, RouteViews); hijack and large-change detection; alerting | Paper §5.1.1. Consumes public feeds; produces advisory input to the scorer |
| **C-2** | Failure-prediction model on the shadow dataset | Paper §5.1.2. **Requires G3's 24-hour dataset first** — there is nothing to train on before that. Advisory-only per §2.3 |
| **C-3** | Control plane and node services — expanded in v1.1 from the second paper: entry/exit node binaries on top of `gtp-runtime-tokio` (session handling, forwarding, per-path MTU, telemetry agent), then fleet management, deployment automation, dashboards, Redis/PostgreSQL state. Hard rule: **the data plane never touches a database** | Papers 1 §2.2.1, §8.2, §9.2.2; paper 2 §5–8, §37–38 |
| **C-4** | Exit-fleet diversity: candidate exits must span providers / ASNs / datacenters (failure domains) — three exits on one provider are one path. Recorded as a deployment invariant checked at G7 (v1.1) | Paper 2 §70–72 |
| **C-5** | Staged rollout (1→5→10→25→50→100%) with automatic halt on regression (loss, disconnects, flap rate, CPU, migration failures), plus control/treatment experiments on weights, probe interval, hysteresis — generalizes the mandatory parallel shadow round (v1.1) | Paper 2 §66–67 |

### Track D — simulation and test infrastructure

| ID | Item | Notes |
| :-- | :-- | :-- |
| **D-1** | `SimulatedFabric`: replace the single-pipe `SimulatedNetwork` (`BinaryHeap`, one profile per call) with per-candidate links, **independent forward/reverse profiles**, a time-scripted impairment schedule, and an optional shared bottleneck | Per-direction profiles are not a nicety: without them the directional detection in A-1/B-5 **cannot be tested at all** |
| **D-2** | Determinism gate: two runs with the same master seed produce byte-identical event sequences (`splitmix64(master, i)` per link) | G2 |
| **D-3** | WAN measurement protocol and baseline capture on a pinned build, before any engine judgement. v1.1 extension: a three-way comparison — baseline VPN vs adaptive V1 vs adaptive+intelligence — under identical client / ISP / destination / duration / traffic pattern | Principle P5; paper 2 §53 |
| **D-4** | Chaos scenario catalog mapped to gates (v1.1): **A** initial best-path selection → G3 shadow validation; **B** degradation migration → G5; **C** continuous oscillation ⟹ zero switches → G5 stability test; **D** blackhole / hard failure → G4; **E** BGP-only event ⟹ no switch → C-1 entry test; **F** correlated BGP + RTT + loss spikes → C-1; plus node-death and MTU-mismatch drills → G6/G7 | Paper 2 §49–50 |

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
| **G3 — shadow mode** | B-1, B-2, B-3, **B-9, B-10, B-11, B-12**, A-4, A-7 | 24 h of WAN shadow operation: complete score log, **zero migrations**; performance matches baseline within measurement noise (negative test for INV-15); batch-measured jitter ≈ passively measured jitter on the same path; **weights calibrated from the 24 h dataset** (§11.1: both axis sets evaluated); **decision log complete with reason codes for every recorded decision** |
| **G4 — failover only** | B-4 (failover class only), A-3 | Simulated active-path death ⟹ recovery < 500 ms with `detect`/`confirm`/`switch` exported separately; real WAN path cut ⟹ session survives and reliable delivery completes; 10 min at 5% non-catastrophic loss ⟹ **zero** false migrations |
| **G5 — degradation + verification** | B-4 (full), B-5, B-6, **B-11 (completion), B-13** | `t=30s` degradation of +40 ms/+5% ⟹ migration within < 2 s; a candidate that measures better but performs worse ⟹ automatic revert inside the verification window; 30 min of ±5 ms RTT oscillation around 50 ms ⟹ `switch_count = 0`; return-path-only degradation diagnosed correctly with no blind entry migration; **false-switch rate reported from the first migration onward** |
| **G6 — optimization + governance** | B-7, B-8, A-5, **B-13** | `switch_count ≤ 2/min` across all scenarios; `post_switch_delta > 0` in ≥ 80% of switches; `tunnel_advantage_ms > 0` or the engine recommends the direct path; **zero** `PayloadTooLarge` attributable to a routing decision; **migration loss/reordering and probe success rate exported** |
| **G7 — production readiness** *(added by v1.0; extended v1.1)* | E-6, C-3, **C-4, C-5** | Server authentication implemented and adversarially tested; exit-point fleet under management with provider/ASN/datacenter diversity verified (C-4); staged rollout with auto-halt exercised in rehearsal (C-5); no deployment carrying real user traffic before this gate |

Every phase inherits the standing rules: the local verification gate must stay
green (`cargo test --workspace`, clippy `-D warnings`, `cargo fmt --check`,
`scripts/verify_remediation.sh`), each fix carries a negative-checked test, and
the kill switch is exercised as a negative test in every phase (P4). From v1.1,
the documentation protocol of §12 also applies to every gate, G1 through G7.

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
| 2.2.3 | Advanced telemetry | A-1, A-2, A-4 | ✅ A-1/A-2 done (G1); A-4 at G3 |
| 3.1.1 | Delivery semantics | — | ✅ already built |
| 3.1.2 | CUBIC integration | — | ✅ built; §3.5 corrects the example API |
| 3.2.1 | Security feature table | A-8 | ❌ two entries incorrect — §3.1 |
| 3.2.2 | Certificates, path signing | E-6 | ❌ blocking — §4 |
| 3.3 | DRR scheduling | — | ✅ built; §3.3 corrects the priority mapping |
| 4.1 | Route scoring | B-3 | scorer redesigned — §3.2 |
| 4.2 | Anti-flapping | B-4 | parameters adopted as-is |
| 4.3 | Asymmetric routing | A-1, B-5 | A-1 done (G1) — OWD live; B-5 at G5 |
| 5.1.1 | BGP monitoring | C-1 | parallel track |
| 5.1.2 | ML prediction | C-2 | blocked on G3 dataset |
| 5.2 | MTU optimization | B-7 | overhead model corrected — §3.6 |
| 5.3.1 | Gaming-tuned CUBIC | — | partly present (`competitive_fps` profile); revisit after G6 |
| 5.3.2 | Multipath CC | — | ❌ **out of scope** — §3.4 |
| 6.1–6.3 | Test strategy | D-1…D-3 | §3.5 corrects the test API |
| 7.2 | Cost and ROI | — | business track, out of engineering scope |
| 9.1 | Roadmap | §6 | superseded by the gate model |
| Paper 2 §15 | Confidence scoring | B-9 | adopted (v1.1) |
| Paper 2 §16, §41 | Explainable decision log | B-10 | adopted (v1.1) |
| Paper 2 §9.2, §77 | Path lifecycle / recovery window | B-11 | adopted (v1.1) |
| Paper 2 §65 | Per-feature kill switches | B-12 | adopted (v1.1) |
| Paper 2 §52 | Switch-quality KPIs | B-13 | adopted (v1.1) |
| Paper 2 §49–50 | Chaos scenario catalog | D-4 | adopted, mapped to gates (v1.1) |
| Paper 2 §70–72 | Provider/ASN/datacenter diversity | C-4 | adopted as deployment invariant (v1.1) |
| Paper 2 §66–67 | Staged rollout + experiments | C-5 | adopted at G7 (v1.1) |
| Paper 2 §53 | Three-way benchmark | D-3 | adopted as extension (v1.1) |
| Paper 2 §5–8, §37–38 | Entry/exit services, DB architecture | C-3 | adopted, expanded (v1.1) |
| Paper 2 §8 | Rebuild transport/scheduler/security crates | — | ❌ rejected — §2.3 |
| Paper 2 §27 | 4-tier semantics-derived priority | — | ❌ rejected — §3.3 |
| Paper 2 §11–12, §39 | 1 Hz probing, uniform hold-down | — | ❌ rejected — §2.3, §11.2 |
| Paper 2 §30 | New control protocol | — | ❌ rejected — §2.3 |
| Paper 2 §34–35 | Weighted multipath | — | ❌ rejected — §3.4 |

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
| R8 | Two scoring weight sets diverge (paper's five axes vs ICD-01 RE-4) and the wrong one ships | §11.1: RE-4 axes are primary, both run in shadow through G3; the 24-hour dataset picks the shipped defaults (v1.1) |

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

6. **v1.1 addition** — stand up the §12 documentation protocol *before* the
   first G1 commit: create `docs/routing/`, the gate design-note template, and
   the defect-registry file, so the paper trail exists from the first line of
   engine code rather than being reconstructed later. (A-8 needs no action:
   withdrawn — see §1.)

---

## 11. Resolved design conflicts (v1.1)

Two points where v1.0 of this plan, ICD-01, and the papers diverged. The
decisions below are binding for implementation; both are anchored at G3, where
the 24-hour shadow dataset settles them empirically.

### 11.1 Scoring axes and weights

| Source | Axes | Weights |
| :-- | :-- | :-- |
| Paper §4.1.1 (= v1.0 B-3; second paper §13 keeps the same set) | rtt, jitter, loss, stability, congestion | 0.30 / 0.25 / 0.20 / 0.15 / 0.10 |
| ICD-01 RE-4 | rtt_p50, tail (p95−p50), jitter, loss, stale_drop, instability | 0.22 / 0.13 / 0.18 / 0.22 / 0.15 / 0.10 |

**Decision: implement RE-4's axes as the primary scorer.** Rationale: the tail
axis catches what p50 hides at exactly the RTT band competitive games occupy;
`stale_drop` is the closest proxy to player experience (INV-17) and is already
exported; and a congestion axis has no signal until A-5 revives ECN (X-14).
The paper's five-axis set is implemented alongside as a **comparison
configuration** — both run in shadow through G3, and the dataset picks the
shipped defaults. In either case the weights are calibration output, not
constants (paper 2 §13 says the same).

### 11.2 Anti-flapping parameters

v1.0 B-4 quoted the paper's single uniform parameter set (hysteresis ≥ 20%,
hold ≥ 30 s, confirmation 10 s, penalty cap 0.5, ≤ 3 switches/min). ICD-01
RE-5 splits the budgets by decision class, because one hold cannot serve both
ends: 30 s is fatal for failover and toothless against optimization churn.

**Decision: per-class parameters (ICD-01 RE-5) govern.**

| Parameter | Failover | Degradation | Optimization |
| :-- | :-- | :-- | :-- |
| Switch margin | — (immediate) | 10% | 20% |
| Confirmation rounds | 0 | 1 | 2 |
| Post-switch hold | 1 s | 5 s | 15 s |
| Switches/minute cap | unlimited | 3 | 2 |
| Evicted-path cooldown | 10 s | 30 s | 60 s |

The paper's numbers survive intact as the defaults of the **Degradation**
class — which is the class its §4.2 was actually describing.

---

## 12. Documentation protocol (mandatory from G1 — v1.1)

Language rule first: **everything committed to this repository is in
English.** A condensed Arabic companion lives outside the repository
(`arabic-local/Adaptive_Routing_Execution_Plan_Summary_AR.md`, git-ignored by
existing convention). It is a personal tracking aid only and is never
committed.

### 12.1 Per-gate documentation lifecycle

Every gate G1…G7 follows the same cycle, whether it passes in a day or a
month:

1. **Before implementation** — a gate design note under `docs/routing/`:
   scope, target files with paths, affected INV invariants (ICD-01 G-R2: any
   change touching a D2–D6 seam ships with an impact analysis), and the test
   plan per invariant.
2. **During implementation** — a numbered defect-registry entry for every
   defect found and fixed (continuing the N-/X-/FR- series): ID, description,
   root cause, violated invariant, fix, the negative-checked test that pins
   it, and file:line evidence. A fix without a test is not implemented (G-R4).
3. **After the gate** — a closure report in the established format (cf.
   `docs/reaudit/Closure-Matrix-2026-09.md`) mapping every gate criterion to
   its evidence, plus: a CHANGELOG entry, a traceability update (§8), and a
   `docs/ENGINEERING-REFERENCE.md` refresh so onboarding reflects the new
   state.

### 12.2 Code-level documentation

- rustdoc on every public API of `gtp-route` / `gtp-route-tokio`, in the style
  already used by `gtp-core`'s control surface.
- Invariant tagging inside the code: every enforcement point carries its
  invariant reference — e.g. `/// INV-11: measurement scope must equal
  decision scope` — so the code and ICD-01 §3.4 stay cross-referenceable in
  both directions.
- Core changes C-1…C-4 are documented at their sites with the gate that
  required them.
- Every new capability ships a runnable example under `crates/gtp/examples/`
  (continuing `sync_game_loop.rs` / `async_tokio_server.rs`).

### 12.3 Architectural decision records

Each binding choice gets an ADR entry (extending
`GTP_Architecture_Decision_Paper_v1.0.md` or as numbered notes under
`docs/routing/`): measurement-plane separation; local epoch before header
bits; per-probe-connection keys; scoring axes (§11.1); per-class flap budgets
(§11.2); and the SEC-5 mechanism choice (PSK vs signature) when the E-6 design
opens.

### 12.4 Verification tooling

The verification tool is itself under test (E-5): the `gtp-cli` subcommands
whose outputs gate decisions (`dissect`, `stress-suite`, `sim-benchmark`) get
golden-output tests as those outputs become decision inputs. Every WAN report
records build SHA, seed, decision class, split `detect`/`confirm`/`switch`
timings, and the mandatory parallel shadow round (ICD-01 §9.3).

---

*Plan of record, v1.1. Supersedes the roadmap in `GTP_Adaptive_Routing_Technical_Paper.md` §9.1;
builds on `GTPrs_Integrated_CrossLayer_Design_and_Audit__AR.md` (ICD-01) §§6–9, which remains
the mechanism specification; reconciles the second (platform) paper per §2.3.
v1.0 verified against `main` @ `a47bb0e`, 2026-09-05; v1.1 additions re-verified
against the same tree the same day.*
