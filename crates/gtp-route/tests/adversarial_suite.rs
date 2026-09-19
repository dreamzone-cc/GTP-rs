//! Adversarial scenario suite S01–S12 (gap paper §27, adopted via Appendix C)
//! — the fixed regression set for the pre-switching safety envelope.
//!
//! Scenarios S01–S06 and S11 are pure-selector properties over crafted
//! stats; S08–S10 drive REAL estimator measurements through
//! `SimulationRunner` (transient spike, sustained degradation with bounded
//! virtual detection time, recovery); S07 is gated on the loss axis (RT-1 /
//! E-5 — a placeholder documents the gate); S12 cross-checks the RT-2
//! queue bound under sustained pressure.
//!
//! **Standing mapping (verification addendum C.3-1):** S02's HOLD
//! semantics belong to switching time (B-4 FSM, G4). Today's shadow
//! verdict for a near-tie is the deterministic `TIE_BREAK_LOWER_ID` —
//! the test pins the CURRENT contract and records the G4 mapping.

use gtp_core::state::OFFLINE_SIM_MASTER_SECRET;
use gtp_route::{select, DecisionTracker, HealthVerdict, PathStats, PolicyClass, SelectionReason};
use gtp_sim::{NetworkProfile, SimulationRunner};
use gtp_types::{Duration, PriorityTier};

fn clean(id: u32) -> PathStats {
    PathStats::full(id, 1_000, 300, 1_000, 300, 30_000, 60, 0, 0)
}

fn degraded(id: u32) -> PathStats {
    PathStats::full(id, 45_000, 18_000, 40_000, 16_000, 250_000, 60, 0, 0)
}

// ---------------------------------------------------------------------
// S01 — Stable Winner: one path clearly better ⇒ clear winner, stable
// across repeated identical decisions.
// ---------------------------------------------------------------------
#[test]
fn s01_stable_winner_is_reproducible() {
    for _ in 0..100 {
        let s = select(&[degraded(2), clean(7)]);
        assert_eq!(s.chosen, Some(7));
        assert_eq!(s.reason, SelectionReason::ClearWinner);
        assert_eq!(s.runner_up, Some(2));
    }
}

// ---------------------------------------------------------------------
// S02 — Near Tie: a 0.910 vs 0.911-scale difference must be decided
// deterministically. CURRENT shadow contract: TIE_BREAK_LOWER_ID (no
// random flapping); the gap paper's HOLD-at-margin semantics arrive with
// the B-4 decision FSM at G4 — this test pins today's behavior and that
// mapping.
// ---------------------------------------------------------------------
#[test]
fn s02_near_tie_is_deterministic_today_hold_semantics_at_g4() {
    let a = PathStats::full(3, 1_050, 310, 1_010, 305, 30_100, 60, 0, 0);
    let b = PathStats::full(9, 1_040, 305, 1_005, 300, 29_900, 60, 0, 0);
    // Determinism: input order must not matter, and repeats are identical.
    for _ in 0..50 {
        let s1 = select(&[a, b]);
        let s2 = select(&[b, a]);
        assert_eq!(s1.chosen, s2.chosen, "order-invariant");
        assert_eq!(s1.reason, SelectionReason::TieBreakLowerId);
        assert_eq!(s1.chosen, Some(3), "deterministic lower-id tie-break");
    }
}

// ---------------------------------------------------------------------
// S03 — Low-Confidence Winner: best score, thin evidence ⇒ no fast pick.
// ---------------------------------------------------------------------
#[test]
fn s03_low_confidence_winner_holds() {
    let thin = PathStats::full(1, 100, 100, 100, 100, 10_000, 5, 0, 0);
    let s = select(&[thin, degraded(2)]);
    assert_eq!(s.reason, SelectionReason::ConfidenceFloorHold);
    assert_eq!(s.chosen, None);
}

// ---------------------------------------------------------------------
// S04 — Stale Winner: excellent numbers on an old basis never win.
// ---------------------------------------------------------------------
#[test]
fn s04_stale_winner_is_rejected() {
    let stale = PathStats::full(1, 100, 100, 100, 100, 10_000, 60, 4_000_000, 0);
    let s = select(&[stale, degraded(2)]);
    assert_eq!(s.reason, SelectionReason::StaleEvidenceHold);
    assert_eq!(
        s.chosen, None,
        "stale excellence must not beat live honesty"
    );
}

// ---------------------------------------------------------------------
// S05 — Flapping: alternating verdicts feed the tracker; every decision
// stays explainable and the record stays bounded. (Actuation-side flap
// suppression is B-4 at G4; here we prove the OBSERVATION side: no
// dropped or malformed records, exact switch/revert accounting.)
// ---------------------------------------------------------------------
#[test]
fn s05_alternating_winners_are_recorded_exactly() {
    let mut tracker = DecisionTracker::new(4);
    let a_wins = select(&[clean(1), degraded(2)]);
    let b_wins = select(&[degraded(1), clean(2)]);
    let mut t_us = 0u64;
    for _ in 0..10 {
        t_us += 1_000_000;
        tracker.record(0xAB, PolicyClass::Shadow, Some(t_us), &a_wins);
        t_us += 1_000_000;
        tracker.record(0xAB, PolicyClass::Shadow, Some(t_us), &b_wins);
    }
    assert_eq!(
        tracker.decisions, 20,
        "every decision recorded, holds included"
    );
    assert_eq!(tracker.switch_count, 19, "first pick + 19 alternations");
    assert!(
        tracker.revert_count >= 18,
        "alternation is revert-shaped: got {}",
        tracker.revert_count
    );
    assert_eq!(
        tracker.dropped_records, 16,
        "ring bound holds under sustained pressure"
    );
    assert_eq!(
        tracker.records().count(),
        4,
        "the newest four records survive"
    );
    // Every stored record replays losslessly.
    for rec in tracker.records() {
        let line = rec.encode();
        let parsed = gtp_route::DecisionRecord::parse(&line).expect("valid record");
        assert_eq!(parsed.encode(), line);
    }
}

// ---------------------------------------------------------------------
// S06 — One-Sided Impairment: forward-only vs reverse-only degradation
// must flag the correct axis (directional diagnosis never collapses to
// a global "bad").
// ---------------------------------------------------------------------
#[test]
fn s06_one_sided_impairment_flags_the_correct_axis() {
    let fwd_bad = PathStats::full(1, 40_000, 15_000, 1_000, 300, 30_000, 60, 0, 0);
    match gtp_route::health(&fwd_bad) {
        HealthVerdict::Degraded(axis) => {
            assert!(
                axis.starts_with("fwd"),
                "forward impairment flags a fwd axis, got {axis}"
            );
        }
        other => panic!("forward impairment must degrade: {other:?}"),
    }
    let rev_bad = PathStats::full(2, 1_000, 300, 40_000, 15_000, 30_000, 60, 0, 0);
    match gtp_route::health(&rev_bad) {
        HealthVerdict::Degraded(axis) => {
            assert!(
                axis.starts_with("rev"),
                "reverse impairment flags a rev axis, got {axis}"
            );
        }
        other => panic!("reverse impairment must degrade: {other:?}"),
    }
}

// ---------------------------------------------------------------------
// S07 — Burst Loss: GATED. Requires the loss axis, which is deliberately
// absent until RT-1's mixed-unit proxy is replaced (E-5). The placeholder
// documents the gate so the suite is complete and the gap visible.
// ---------------------------------------------------------------------
#[test]
#[ignore = "requires the loss axis (RT-1 / E-5): re-enable when per-packet loss lands"]
fn s07_burst_loss_placeholder() {}

// ---------------------------------------------------------------------
// S08 — Transient Spike (measurement level): ONE impaired packet must
// leave no lasting damage on the estimator within bounded virtual time.
// ---------------------------------------------------------------------
#[test]
fn s08_single_packet_spike_leaves_no_lasting_damage() {
    let mut runner = SimulationRunner::new(
        88,
        NetworkProfile {
            one_way_delay: Duration::from_millis(20),
            jitter: Duration::from_millis(0),
            loss_rate: 0.0,
            reorder_rate: 0.0,
            duplicate_rate: 0.0,
            bandwidth_bytes_per_sec: 10_000_000,
        },
    );
    let tick = Duration::from_micros(16_667);
    let drive = |runner: &mut SimulationRunner, frames: u32| {
        for _ in 0..frames {
            runner
                .client
                .send_unreliable(
                    b"s08".to_vec(),
                    PriorityTier::P1Input,
                    None,
                    runner.current_time,
                )
                .unwrap();
            runner.step(tick);
        }
    };
    drive(&mut runner, 60); // 1 s clean baseline.
    let before = runner.server.control().query_metrics(runner.current_time);
    let base_jitter = before.jitter.unwrap_or(Duration::from_micros(0));

    // ONE spiked packet: 150 ms delay for exactly one step.
    runner.profile.one_way_delay = Duration::from_millis(150);
    runner
        .client
        .send_unreliable(
            b"spike".to_vec(),
            PriorityTier::P1Input,
            None,
            runner.current_time,
        )
        .unwrap();
    runner.step(tick);
    runner.profile.one_way_delay = Duration::from_millis(20);

    // RFC 3550 jitter is an EWMA with gain 1/16 — a spike legitimately
    // leaves a GEOMETRICALLY DECAYING tail (this is the standard's
    // designed memory, not a defect). The spike-read captures the tail's
    // start; after 2 s of clean traffic (120 ticks) it must have decayed
    // to the clean neighbourhood. The primary variance axis (weight 0.6)
    // must be back within one floor-re-anchor, i.e. immediately.
    // The spiked packet is still IN FLIGHT (150 ms delay): it lands ~9
    // ticks later, late among the clean packets — exactly the arrival
    // pattern a transient network spike produces. Advance past it.
    drive(&mut runner, 12); // 200 ms: the spike has landed and registered
    let spiked = runner
        .server
        .control()
        .query_metrics(runner.current_time)
        .jitter
        .unwrap_or(Duration::from_micros(0));
    assert!(
        spiked > base_jitter,
        "premise: the spike registers in the EWMA ({spiked:?})"
    );
    let v_mid = runner
        .server
        .control()
        .query_metrics(runner.current_time)
        .owd_var
        .unwrap_or(Duration::from_micros(0));
    assert!(
        v_mid <= Duration::from_millis(2),
        "the primary variance axis returns to the floor immediately after the spike: {v_mid:?}"
    );
    drive(&mut runner, 118); // 2 s of clean traffic: (15/16)^120 tail
    let after = runner.server.control().query_metrics(runner.current_time);
    let j = after.jitter.unwrap_or(Duration::from_micros(0));
    assert!(
        j <= Duration::from_millis(1),
        "the EWMA tail must decay to the clean neighbourhood within 2 s: {j:?}"
    );
    let v = after.owd_var.unwrap_or(Duration::from_micros(0));
    assert!(v <= Duration::from_millis(1), "no lasting variance: {v:?}");
}

// ---------------------------------------------------------------------
// S09 — Sustained Failure (measurement level): real degradation must be
// DETECTABLE within bounded virtual time (the honest precondition for
// any future bounded-time switch decision at G4).
// ---------------------------------------------------------------------
#[test]
fn s09_sustained_degradation_is_detectable_within_bounded_time() {
    let mut runner = SimulationRunner::new(
        89,
        NetworkProfile {
            one_way_delay: Duration::from_millis(20),
            jitter: Duration::from_millis(0),
            loss_rate: 0.0,
            reorder_rate: 0.0,
            duplicate_rate: 0.0,
            bandwidth_bytes_per_sec: 10_000_000,
        },
    );
    let tick = Duration::from_micros(16_667);
    let drive = |runner: &mut SimulationRunner, frames: u32| {
        for _ in 0..frames {
            runner
                .client
                .send_unreliable(
                    b"s09".to_vec(),
                    PriorityTier::P1Input,
                    None,
                    runner.current_time,
                )
                .unwrap();
            runner.step(tick);
        }
    };
    drive(&mut runner, 60); // clean baseline.

    // Degrade: +40 ms one-way with 8 ms jitter, sustained.
    runner.profile.one_way_delay = Duration::from_millis(60);
    runner.profile.jitter = Duration::from_millis(8);
    drive(&mut runner, 30); // 0.5 s of sustained degradation.

    let m = runner.server.control().query_metrics(runner.current_time);
    let v = m.owd_var.unwrap_or(Duration::from_micros(0));
    let j = m.jitter.unwrap_or(Duration::from_micros(0));
    assert!(
        v >= Duration::from_millis(30),
        "sustained degradation must be detectable within 0.5 s: var {v:?}"
    );
    assert!(
        j >= Duration::from_micros(1_000),
        "jitter from sustained 8 ms impairment must be visible: {j:?}"
    );
}

// ---------------------------------------------------------------------
// S10 — Recovery (measurement level): after the impairment lifts, the
// estimator must return to its clean neighbourhood within bounded
// virtual time — the precondition for hysteretic recovery decisions
// (RevertGuard stays B-6/G5; here we prove measurement recovery).
// ---------------------------------------------------------------------
#[test]
fn s10_recovery_returns_estimates_to_clean_neighbourhood() {
    let mut runner = SimulationRunner::new(
        90,
        NetworkProfile {
            one_way_delay: Duration::from_millis(20),
            jitter: Duration::from_millis(0),
            loss_rate: 0.0,
            reorder_rate: 0.0,
            duplicate_rate: 0.0,
            bandwidth_bytes_per_sec: 10_000_000,
        },
    );
    let tick = Duration::from_micros(16_667);
    let drive = |runner: &mut SimulationRunner, frames: u32| {
        for _ in 0..frames {
            runner
                .client
                .send_unreliable(
                    b"s10".to_vec(),
                    PriorityTier::P1Input,
                    None,
                    runner.current_time,
                )
                .unwrap();
            runner.step(tick);
        }
    };
    drive(&mut runner, 60); // clean.
    runner.profile.one_way_delay = Duration::from_millis(60);
    runner.profile.jitter = Duration::from_millis(8);
    drive(&mut runner, 60); // 1 s degraded.
                            // Lift the impairment.
    runner.profile.one_way_delay = Duration::from_millis(20);
    runner.profile.jitter = Duration::from_millis(0);
    drive(&mut runner, 120); // 2 s of clean recovery traffic.

    let m = runner.server.control().query_metrics(runner.current_time);
    let v = m.owd_var.unwrap_or(Duration::from_micros(0));
    assert!(
        v <= Duration::from_millis(5),
        "after 2 s of clean traffic the variance must return to the clean neighbourhood: {v:?}"
    );
    // The last step delivered traffic AT the current virtual time, so the
    // basis age reads exactly zero — fresh under continuous 60 FPS flow.
    assert_eq!(
        runner
            .server
            .control()
            .query_metrics(runner.current_time)
            .since_last_rx,
        Some(Duration::from_micros(0)),
        "the basis stays fresh at 60 FPS post-recovery"
    );
}

// ---------------------------------------------------------------------
// S11 — Report-Channel Impairment: delayed/duplicated reports surface as
// growing evidence ages and are rejected as stale; duplicates cannot
// change a decision built from the same data (determinism).
// ---------------------------------------------------------------------
#[test]
fn s11_delayed_and_duplicate_reports_are_stale_aware_and_inert() {
    use gtp_route::MeasurementReport;

    // A report that left the far end a while ago carries a large age.
    let old_report = MeasurementReport {
        owd_var_us: Some(100),
        jitter_us: Some(100),
        srtt_us: Some(50_000),
        samples: 500,
        since_last_rx_us: Some(4_200_000),
        loss_rate: None,
    };
    let line = old_report.encode();
    assert!(line.starts_with("GTPRP2|"));
    let parsed = MeasurementReport::parse(&line).unwrap();
    assert_eq!(parsed.since_last_rx_us, Some(4_200_000));

    // Merged against a fresh local side, the STALE forward evidence holds.
    let local = PathStats::full(2, 8_000, 3_000, 8_000, 3_000, 80_000, 60, 0, 300);
    let stale_stats = old_report.into_path_stats(1, &local);
    let s = select(&[stale_stats]);
    assert_eq!(s.reason, SelectionReason::StaleEvidenceHold);
    assert_eq!(s.chosen, None);

    // Duplicates are inert: the same parsed report twice yields the same
    // merged stats and the same decision — replay determinism.
    let merged_a = MeasurementReport::parse(&line)
        .unwrap()
        .into_path_stats(1, &local);
    let merged_b = MeasurementReport::parse(&line)
        .unwrap()
        .into_path_stats(1, &local);
    assert_eq!(merged_a, merged_b);
    assert_eq!(select(&[merged_a]).summary(), select(&[merged_b]).summary());
}

// ---------------------------------------------------------------------
// S12 — Queue Saturation (cross-check of RT-2): sustained over-pressure
// keeps memory bounded and accounting exact across capacity values.
// (The negative-path unit tests live in gtp-core; this exercises the
// sustained matrix from the gap paper §8 at the integration level.)
// ---------------------------------------------------------------------
#[test]
fn s12_queue_saturation_stays_bounded_with_exact_accounting() {
    use gtp_core::{ControlEvent, GtpConfig, GtpConnection};
    use gtp_types::ConnectionId;

    for capacity in [0usize, 1, 16, 64] {
        let config = GtpConfig {
            event_queue_capacity: capacity,
            ..GtpConfig::default()
        };
        let mut conn = GtpConnection::new_with_role(
            ConnectionId(0x5D12_0000_0000_0000 + capacity as u64),
            "127.0.0.1:6000".parse().unwrap(),
            true,
            true,
            OFFLINE_SIM_MASTER_SECRET,
            config,
        );
        // Sustained over-pressure: 1000 pushes against every capacity.
        for i in 0..1000u64 {
            conn.push_event(ControlEvent::PtoTriggered {
                pto_count: i as u32,
                inflight_bytes: i,
            });
        }
        let expected_len = capacity.min(1000);
        assert_eq!(
            conn.event_queue.len(),
            expected_len,
            "capacity {capacity}: bounded"
        );
        assert_eq!(
            conn.cold.total_dropped_events,
            1000 - expected_len as u64,
            "capacity {capacity}: every shed event counted exactly"
        );
        // The retained records are the NEWEST ones.
        if capacity > 0 {
            let drained = conn.drain_events();
            assert_eq!(drained.len(), expected_len);
            if let Some(gtp_core::ControlEvent::PtoTriggered { pto_count, .. }) = drained.last() {
                assert_eq!(*pto_count, 999, "newest information survives");
            }
        }
    }
}
