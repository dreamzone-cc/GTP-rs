# GTP-rs Engineering Reference — Remediation History, Workflows & Development Guide

> **Status:** Primary engineering reference for current and future contributors.
> **Last updated:** 2026-09-04 (covers everything up to commit `83e5e09`).
> **Language policy:** official repository documentation is English-only; personal
> Arabic mirrors live in the git-ignored `arabic-local/` directory.

This document consolidates the entire audit → remediation → live-verification
process into one place: what was changed and why, the exact commits, the API
changes a developer must know, the local environment quirks, how to run every
test tier, and the operational runbook for the verification VPS. Evidence-level
detail lives in the specialised reports it links to (§9).

---

## 1. Project overview

GTP-rs is a QUIC-inspired game transport protocol (GTP/1.1) over UDP, implemented
entirely in-house (no QUIC/TLS libraries) across 13 workspace crates:

```
gtp-types        foundation: ids (RFC 1982 semantics), time, delivery classes, errors
gtp-wire         framing: 24/28-byte headers, 14 frame types, varint, builder/iterator
gtp-recovery     ACK tracking, RTT estimation, loss detection, PTO, sent records
gtp-cc           CUBIC (RFC 8312), token-bucket pacing, backpressure signalling
gtp-scheduler    5-tier DRR scheduler, state-supersession table, ordered-group RX
gtp-path         connection state machine, anti-amplification, path validation, cookies
gtp-crypto       X25519 handshake, ChaCha20-Poly1305 AEAD, HKDF, replay window
gtp-core         the engine hub: GtpConnection (TX/RX pipelines, hot/cold state)
gtp-io           UDP abstraction (currently NOT used by the live runtime — see §8)
gtp-runtime-tokio  async endpoint: handshake, CID routing, RX/TX loops
gtp-sim          deterministic seeded network simulation
gtp-cli          CLI: dissect, net-server, net-client, stress-suite, sim-benchmark, control-demo
gtp              SDK facade + prelude
```

Dependency DAG (no cycles): `types → wire → {recovery, scheduler} → cc →
{path, crypto} → core → {io, sim, runtime-tokio} → cli/gtp`.

Toolchain: **Rust 1.85.0**, pinned by `rust-toolchain.toml`.

## 2. Remediation history — two rounds at a glance

### Round 1: comprehensive re-audit & remediation (2026-09-04 morning)

Entry state: `integration/all-fixes` @ `1c0e488` + uncommitted New-12 work; 95
tests. Two ready branches awaited integration; the adopted master plan defined
five remediation stages.

| # | Commit | Content |
| :-- | :--- | :--- |
| 1 | `3fada65` | New-12 fix committed on its branch (PathChallenge reflection caps + 5 tests) |
| 2 | `e756d7d` | merge `fix/New-8-per-path-validation` (N-1 ciphertext misroute, N-2 RX HoL, X-1 RTT reset, A-5 post-auth counting, New-8 per-path budgets) |
| 3 | `33db814` | merge New-12 into `integration/all-fixes`; resolved a semantic conflict by correcting the source-address modeling in two New-8 tests (`65daea8`) |
| 4 | `1dfabc0` | **scheduler round**: N-3 fair DRR with persistent cursor, X-19 deficit cap ×4, FR-7 O(1) byte/item accounting, N-7 item caps (5 new tests) |
| 5 | `60d02a0` | **recovery round**: N-4/FU-4 PTO discipline (RFC 9002 §7.5), FR-8 per-message order sequences, N-5 optional min_rtt, FU-5 LRU group eviction, FR-5 closed-state gate (7 new tests) |
| 6 | `26174ed` | **wire/crypto round**: WIR-2 clamped ACK range_count, WIR-4 UTF-8-safe close reason, SEC-14 redacted key Debug (3 new tests) |
| 7 | `6bc0612` | cross-layer integration suite (`crates/gtp/tests/cross_layer_integration_test.rs`) + gate script auto-detects cargo |
| 8 | `f636948` | documentation round: N-6 (dissect hex, two-way path validation wording, β=0.75 preset note, real stress-suite verdicts), X-2/X-3 (GTP-SEC-01 AAD + nonce spec alignment), closure matrix, CHANGELOG |
| 9 | `c419cc7` | merge `integration/all-fixes` → `main` (local only) |
| 10 | `6cb6e6e` | session documentation set stored in-repo (7 root-level files) |

Test progression: **95 → 111 → 116 → 121 → 128 → 132 → 134** (+1 doc-test = 135).
Three consecutive identical full-gate runs proved reproducibility.

### Follow-up round: verification of later additions + live WAN (2026-09-04, later sessions)

| # | Commit | Content |
| :-- | :--- | :--- |
| 11 | `09f1d8a` *(external)* | CLI stress-harness backpressure yield/backoff retry loop + three Arabic verification reports in `docs/reaudit/` |
| 12 | `ba2a486` | fmt fix for `09f1d8a` (its match arm broke `cargo fmt --check` / CI) |
| 13 | `83e5e09` | official live-WAN verification report; corrections to the follow-up reports (fabricated commit hash, wrong defect descriptions, loopback-vs-WAN misattribution); closure-matrix addendum; CHANGELOG |

**Later rounds (2026-09-05):** the adaptive-routing program began executing
against `docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md` — gate G1 (measurement
layer: `timestamp_micros` → `OwdEstimator` → OWD/jitter telemetry, A-6 factor
wiring, E-1/E-2 hygiene) at `a19bbff`, then a development round closing gate
G2: RT-2 (bounded event queue + net-server drain), `gtp-sim::FabricRunner`
(connection-driven determinism + directional independence), the pure
`gtp-route` crate (scorer/confidence/explainable selection), the
`MeasurementReport` bidirectional exchange and the `route-probe` CLI shadow
verdict — **168 tests green, parity `07cbc47`**, with three live device↔VPS
probes confirming real forward≠reverse separation. First actual switching
remains gated at G4. Details: `docs/routing/G1-closure-report.md` and
`docs/routing/G2-and-route-proto-closure.md`.

**Earlier reference point: `83e5e09`** — identical on the local machine and the
verification VPS (`92.222.80.200`, synced via git bundles over SSH; no GitHub
pushes) at the time the WAN phase closed.

## 3. What changed technically (fix-by-fix, with entry points)

Every fix is pinned by a named regression test; the full evidence tables are in
`docs/reaudit/Re-Audit-Report-2026-09.md` (as-found state of ~100 tracked
defects) and `docs/reaudit/Closure-Matrix-2026-09.md` (outcomes + deferred
register). The developer-relevant essence:

### Security & path
- **SEC-14 (residual):** `DirectionalKeys` / `SessionDirectionalKeys` implement a
  redacted `Debug` (`[REDACTED]` per field) — raw key bytes never render in logs
  or panics. (`gtp-crypto/src/handshake.rs`, `kdf.rs`)
- **New-12:** inbound `PathChallenge` is answered only when it arrives from the
  active path; at most one `PathResponse` per inbound datagram
  (`MAX_PATH_RESPONSES_PER_DATAGRAM = 1`) and at most two queued
  (`MAX_PENDING_PATH_RESPONSES = 2`, overflow drops the NEW one). Dropped
  challenges are treated as wire loss. (`gtp-core/src/connection.rs`,
  `state.rs` constants)
- **New-8 / A-5 / SEC-15/16 (merged):** anti-amplification bytes count only
  after AEAD success; the 3× budget is a per-path probe slot promoted on a
  validated `PathResponse`. (`connection.rs`, `state.rs::anti_amplification_probe`)
- **X-1 (merged):** the RTT estimator (incl. `min_rtt`) resets on validated path
  migration via `RttStats::reset_for_new_path()`; pre-migration packets cannot
  re-seed the new path's sampling floor.

### Scheduler & delivery
- **N-3 / X-19:** `GameScheduler::pop_next` now runs textbook DRR over P1–P4
  with a persistent cursor (`drr_pointer`), once-per-round quantum accrual
  (`accrued[]`), and a deficit cap of 4× the tier quantum. P3/P4 receive their
  35:15:5 weight shares under full P1 saturation. (`gtp-scheduler/src/scheduler.rs`)
- **FR-7 / N-7:** per-tier byte AND item counts maintained in O(1) on every
  mutation; tier caps default 512 KB bytes + 4096 items
  (`DEFAULT_MAX_QUEUE_ITEMS_PER_TIER`); ordered-group reorder buffer caps at
  256 KB + 1024 items per group (`DEFAULT_MAX_GROUP_BUFFER_ITEMS`).
- **FU-5:** ordered-group eviction is LRU — every access refreshes recency
  (`ConnectionHot::ordered_group_mut`); a busy old group survives the 256-group bound.
- **FR-8:** `OrderedGroupReceiver::on_incoming` returns `(order_seq, payload)`
  pairs; the RX loop labels every drained message with its own sequence.
- **FR-5:** `handle_incoming_datagram` returns `Ok(vec![])` immediately when the
  connection is `Closed` — no decryption, dispatch, or bookkeeping.

### Recovery & congestion
- **N-4 / FU-4:** a PTO probe no longer collapses `cwnd`. The window shrinks via
  `cc.on_timeout` only on persistent congestion — `pto_count >= 3` consecutive
  probe rounds without a single ACK (`PERSISTENT_CONGESTION_PTO_COUNT` in
  `connection.rs`; resets on every ACK). The inert `cc.on_loss` call on the PTO
  path is gone. (RFC 9002 §7.5)
- **N-5:** `min_rtt` is `Option<Duration>` on every consumer surface; the
  `u64::MAX` sentinel never leaks. `RttStats::min_rtt_sample()` is the accessor;
  `calculate_backpressure` treats `None` as a neutral RTT-inflation axis;
  `DetailedMetrics::summary_line()` and `gtp-cli` render `n/a`.

### Wire
- **WIR-2:** the ACK encoder writes the **clamped** range count (`min(range_count,
  MAX_ACK_RANGES) as u8`) — it can no longer emit a frame its own decoder rejects.
- **WIR-4:** the Close reason trims at a UTF-8 character boundary (continuation-
  byte scan, stable Rust) instead of splitting multi-byte characters.

### Runtime (merged from New-8 branch)
- **N-1:** handshake-frame demux is gated on `header.flags.is_long_header()`;
  encrypted data whose ciphertext begins with `0x0B/0x0C/0x0D` is routed and
  decrypted normally (was ~1.17% silent loss).
- **N-2:** the RX loop never awaits application delivery (`try_send` +
  per-class slow-consumer isolation with drop counters) — no endpoint-wide HoL.

### CLI
- **`09f1d8a`:** the stress-suite high-burst tier retries on
  `ResourceLimitExceeded` (yield + 50 µs backoff, periodic yield every 64 sends)
  instead of aborting — sustains ~84k msg/s. Design note: retries are unbounded
  (safe in practice; a cap would be more defensive).
- **N-6:** `dissect` examples in README/CLI are valid 28-byte long-header
  packets; stress-suite verdicts are computed from real counters.

## 4. Breaking & new API surface (what a developer must know)

| Item | Before | After | Why |
| :--- | :--- | :--- | :--- |
| `NetworkFeedback::min_rtt` | `Duration` | `Option<Duration>` | N-5 sentinel leak |
| `DetailedMetrics::min_rtt` | `Duration` | `Option<Duration>` | N-5 |
| `calculate_backpressure(..., min_rtt)` | `Duration` | `Option<Duration>` | N-5 |
| `OrderedGroupReceiver::on_incoming` return | `Result<Vec<Vec<u8>>>` | `Result<Vec<(u32, Vec<u8>)>>` | FR-8 true order sequences |

New public surface: `RttStats::min_rtt_sample()`,
`RttStats::reset_for_new_path()`, `GameScheduler::with_item_limit`,
`GameScheduler::queue_tier_bytes/queue_tier_items`, `RttStats::reset_for_new_path`
constants `MAX_PATH_RESPONSES_PER_DATAGRAM`, `MAX_PENDING_PATH_RESPONSES`,
`DEFAULT_MAX_QUEUE_ITEMS_PER_TIER`, `DEFAULT_MAX_GROUP_BUFFER_ITEMS`,
`PERSISTENT_CONGESTION_PTO_COUNT` (private const in `connection.rs`).

## 5. Environment setup & known quirks (read before building)

1. **Toolchain:** Rust 1.85.0 via `rust-toolchain.toml`. The release profile is
   fat-LTO/opt-3 — dev builds are fine for tests, use `--release` for benchmarks
   and WAN runs.
2. **Broken rustup shims (this machine):** the `cargo`/`rustc` proxies on PATH
   are mangled (`unknown proxy name: 'ZCode-3.8.1-linux-x64'`). Workaround:
   ```bash
   export PATH=$HOME/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin:$PATH
   ```
3. **Compiler mixing (E0514):** if a different rustc ends up in PATH partway
   through, cached `.rmeta` files become incompatible. Either keep the pinned
   PATH for the whole session or `cargo clean`.
4. **Verification gate** `scripts/verify_remediation.sh` resolves cargo
   automatically: `CARGO_BIN` override → toolchain pinned by rust-toolchain.toml
   → a working PATH cargo → newest installed toolchain. It runs fmt (report),
   clippy (report), the full test suite (hard gate), and 7 procedural checks.
5. **Fuzzing:** `fuzz/` is a separate libfuzzer package requiring nightly; it is
   deliberately outside workspace members. CI (`fuzz_smoke`) builds and runs it
   non-blocking. Locally there is no nightly toolchain.
6. **`gtp-io` is currently unused by the live runtime** (`gtp-runtime-tokio`
   uses `tokio::net::UdpSocket` directly). Its 2 MB socket buffers /
   `send_batch` loops exist for a future integration — do not assume it is on
   the hot path. ECN is likewise plumbed nowhere end-to-end (deferred X-14).

## 6. Test tiers & how to run them

| Tier | Command | What it proves |
| :--- | :--- | :--- |
| Full unit+integration | `cargo test --workspace --all-targets` | **134 tests, all crates** (current green count) |
| Doc-test | `cargo test --workspace` | +1 runnable doc example (135 total) |
| N-1 volume gate | `cargo test -p gtp-runtime-tokio --test n1_routing_gate_test -- --ignored` | 5,000-datagram loopback, ciphertext-collision routing, <0.05% loss |
| Cross-layer seams | `cargo test -p gtp --test cross_layer_integration_test` | full recovery path (loss→reorder→PTO→ordered delivery with true seqs); DRR fairness under P1 saturation |
| Seam isolation | `cargo test -p gtp-runtime-tokio --test slow_consumer_test` | no endpoint-wide HoL (N-2) |
| Local stress | `gtp-cli stress-suite --mode all --count 10000` | load tiers, 5-profile impairment matrix, 50k-packet endurance, 200 concurrent sessions, NAT-rebind. **Loopback-only** — its `--server` arg is dead code |
| Live WAN rounds | `gtp-cli net-client --server 92.222.80.200:7777 --count {100..5000}` | real-internet handshake + all four delivery semantics + Control API in one session |
| VPS-side suite | (on VPS) `cargo test --workspace --all-targets` | parity verification on the deployment host |
| CI parity smoke | examples + `gtp-cli sim-benchmark --ticks 500` | the two examples CI runs + deterministic sim |

Test-count history: 95 (entry) → 111 (New-8 branch) → 116 (post-merge) → 121 →
128 → 132 → 134 (+1 doc-test). The gate expectation ">117" from the adopted plan
was met and exceeded.

Reproducibility rule: full gates were run three times consecutively with
identical outcomes before round-1 closure; re-run the same way after any
multi-commit change.

## 7. Quality gates, workflow rules & repository policy

- **Every fix ships with a named regression test** that fails before and passes
  after; the whole suite must stay green after each stage (no regressions).
- **Clippy is CI-blocking** (`--workspace --all-targets -- -D warnings`); so is
  `cargo fmt --check`. (The only violation ever merged — `09f1d8a` — was fixed
  in `ba2a486`; keep the gate in your loop.)
- **Evidence-based documentation:** every status claim must cite a run command +
  test name or `file:line`. Historical reports found with unverifiable claims
  (e.g. a fabricated commit hash in the follow-up round) are corrected in place
  with verification addenda — see `Closure-Matrix-2026-09.md` addendum and the
  report headers in `docs/reaudit/`.
- **Branch/merge flow:** work lands on `integration/all-fixes`, gates green, then
  merges into `main` locally. Feature branches follow `fix/<ID>-<slug>` naming.
- **Radicle-only sync:** development and sync are local/Radicle. GitHub is
  out of scope until the project's final publication step. `rad` is installed
  globally (alias `DreamZone`), but the repo is not yet a Radicle project
  (`rad init` pending) and the node is not running — commits are local until
  then (documented decision, approved fallback).
- **VPS sync protocol** uses signed git bundles over SSH (no external push):
  ```bash
  git bundle create /tmp/gtp.bundle main --not <vps-head>
  scp /tmp/gtp.bundle ubuntu@92.222.80.200:/tmp/
  ssh ubuntu@92.222.80.200 'cd ~/GTP-rs && git pull /tmp/gtp.bundle main'
  ```

## 8. VPS operations runbook (verification server)

- **Host:** `ubuntu@92.222.80.200`, repo `/home/ubuntu/GTP-rs`, toolchain
  `~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu`, 2 vCPU, kernel
  `7.0.0-28-generic`.
- **Service** (transient systemd unit):
  ```bash
  sudo systemd-run --unit=gtp-server \
    /home/ubuntu/GTP-rs/target/release/gtp-cli net-server --bind 0.0.0.0:7777
  systemctl status gtp-server
  sudo journalctl -u gtp-server --no-pager
  ```
- **Pre-test parity protocol (mandatory):** `git rev-parse HEAD` on both ends;
  document any difference; sync the newer side via bundle; rebuild release;
  restart the service; run the 134-test suite on the VPS; only then test.
- **Reference telemetry (at `83e5e09`):** server RSS ≈ 5.85 MB flat, CPU <5%
  under 60 FPS encrypted load; WAN RTT ≈ 45–55 ms with jitter < ~2 ms;
  5,000-frame round ≈ 85 s at 0.00% loss; one genuine mid-stream loss observed
  (round 2,000) recovered by a single selective retransmission.
- Known historical incident (for context): an old pre-remediation binary
  (commit `1c0e488`, with N-1/N-2 defects) ran on the VPS until 2026-09-04
  06:55 UTC; it was replaced before the first live round. Always check the
  deployed commit before measuring.

## 9. Documentation map (what lives where)

| Document | Purpose |
| :--- | :--- |
| `docs/ENGINEERING-REFERENCE.md` | **this document** — process history + dev guide |
| `docs/routing/MEASUREMENT-AND-SELECTION-REFERENCE.md` | **primary reference for measurement mechanisms** (estimator math, telemetry surfaces, bidirectional report format, scoring/confidence/selection semantics) + the extension recipe for new measurement patterns + the accomplishment record — the first stop for any telemetry/routing work |
| `docs/reaudit/Re-Audit-Report-2026-09.md` | as-found re-verification matrix of ~100 tracked defects (all families), evidence per item |
| `docs/reaudit/Closure-Matrix-2026-09.md` | post-remediation outcomes, master-plan closure table, deferred register, commit record, test-count progression (+ follow-up addendum) |
| `docs/reaudit/Live-WAN-Verification-Report-2026-09-04.md` | official live-WAN phase report: parity audit, 5 stepped rounds, stress suite, server telemetry, doc-corrections record |
| `docs/reaudit/GTP-rs_*_AR.md` (3 files) | Arabic verification reports from the follow-up session — **read together with the corrections** recorded in their verification addenda and in the official WAN report §7 |
| `GTP-rs-Comprehensive-Audit-and-Remediation-Plan.md` | the adopted master plan of the 2026-09-04 round (bilingual) |
| `GTP-rs-Technical-Specification.md` | developer requirements & traceability (FR/SEC-A/PERF/TEST/BR IDs) |
| `GTP-rs-Architecture-Protocol-Audit-Paper-v1.0.md` + reconciliation/tracker/remediation-plan docs (root) | the original first-audit registry (~69 defects) and early remediation records |
| `GTPrs_Integrated_CrossLayer_Design_and_Audit__AR.md` | cross-layer audit, system invariants (INV-*), and the future adaptive-routing engine spec (RE-1..10, gates G0–G6) |
| `GTP++.md`, `GTP_Adaptive_Routing_Technical_Paper.md`, `Technical_Paper_Gaming_VPN_Adaptive_Routing.md` | adaptive-routing design papers (concept-level) |
| `GTP-rs_SESSION_CONTEXT_GUIDE.md` | session-continuity process guide |
| `CHANGELOG.md` | user-facing change log (Unreleased section covers both rounds) |
| `docs/specs/GTP-*-01.md` | normative sub-specifications (SEC-01 aligned with the implementation in round 1) |
| `arabic-local/` *(git-ignored)* | personal Arabic mirrors of all reports |

## 10. Open items & future work (approved deferrals)

Nothing critical or high remains open. The maintained deferred register (with
rationales) is in `Closure-Matrix-2026-09.md` §3; headline items:

- **SEC-5 / SEC-A7** server authentication (active-MITM defense) — architectural
  decision DEF-1; needs a PSK/signature design milestone.
- **SEC-8 / SEC-A8** removal of the deprecated static master secret (gtp-sim
  determinism depends on it).
- **CORE-2 / X-10** fragmentation + PLPMTUD (API currently rejects >MTU safely).
- **Wire deferred set** WIR-5 / WIR-6-residual / WIR-10 / SEC-13 — gated on
  version negotiation design (DEF-3).
- **REC-12** independent loss timer; **REC-13** DeliveryRateSample consumption.
- **CC enhancements:** CC-7 (W_cubic(t+RTT)), CC-8 (HyStart/idle restart),
  CC-10 (backpressure hysteresis), CC-12/X-14 (ECN end-to-end).
- **Runtime hygiene:** CORE-4 residual TX-task leak on live-CID replacement,
  CORE-9 rate-limiter map growth, CORE-7 `unwrap_or(32)`.
- **Routing-engine agenda** (RE-1..10 behind gates G1–G6, timestamp
  consumption X-4, multi-slot path challenges X-8) — the next major feature
  track, specified in the ICD paper.
- **CLI nit:** the stress-suite retry loop could take an explicit retry cap.
- **Multi-day WAN endurance** and local nightly fuzz runs — environment gaps,
  not code gaps.

## 11. Engineering principles established by this process

1. **Decrypt before demux** — ciphertext is uniform noise; never classify or
   route on pre-decryption bytes (the N-1 lesson, now pinned by volume test).
2. **Task-per-session isolation** — never `await` one consumer inside a shared
   loop (the N-2 lesson, pinned by slow-consumer tests).
3. **Entropy-rich CIDs** — port ^ pid ^ nanotime for client-generated connection
   IDs (the handshake-collision lesson from live testing).
4. **Fairness must be measured, not assumed** — DRR needs a persistent cursor and
   caps, or it degenerates to strict priority (N-3/X-19).
5. **PTO ≠ congestion** — window collapse requires persistent-congestion
   evidence (N-4, RFC 9002 §7.5).
6. **One source of truth per quantity** — in-flight (FR-3), RTT (FR-4), active
   path (FR-4); mirrors drift.
7. **Bounds on everything wire-reachable** — bytes AND item counts (N-7),
   per-path amplification budgets (New-8), reflection caps (New-12).
8. **A fix without a named test is not implemented** — and reports must cite
   verifiable evidence (commands, hashes) — unverifiable claims get corrected
   in place.
