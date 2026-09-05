//! Selection fed by real connection measurements (integration, INV-11).
//!
//! Two `SimulationRunner` pairs — one clean path, one heavily impaired —
//! produce genuine per-direction `owd_var`/`jitter` and RTT aggregates from
//! actual GTP traffic; `gtp_route::select` must pick the clean path **by
//! score** (the clean path carries the HIGHER id, so order and id cannot
//! explain the outcome). This is the "choose the best path" behaviour of the
//! routing mechanism, demonstrated deterministically and shadow-only: the
//! data plane never moves (INV-15).

use gtp_route::{score, select, SelectionReason};
use gtp_sim::{NetworkProfile, SimulationRunner};
use gtp_types::{Duration, PriorityTier};

fn profile(one_way_delay_ms: u64, jitter_ms: u64) -> NetworkProfile {
    NetworkProfile {
        one_way_delay: Duration::from_millis(one_way_delay_ms),
        jitter: Duration::from_millis(jitter_ms),
        loss_rate: 0.0,
        reorder_rate: 0.0,
        duplicate_rate: 0.0,
        bandwidth_bytes_per_sec: 10_000_000,
    }
}

/// Drive 60 FPS traffic through one runner pair and read BOTH receivers'
/// one-way aggregates: the server's estimator sees the client→server
/// direction (forward), the client's sees server→client (reverse).
fn measure_pair(seed: u64, profile: NetworkProfile, path_id: u32) -> gtp_route::PathStats {
    let mut runner = SimulationRunner::new(seed, profile);
    let tick = Duration::from_micros(16_667);
    let frames = 120u32; // 2 s of virtual traffic → ample samples
    for i in 0..frames {
        runner
            .client
            .send_unreliable(
                format!("probe_frame_{i}").into_bytes(),
                PriorityTier::P1Input,
                None,
                runner.current_time,
            )
            .unwrap();
        runner.step(tick);
    }

    let server = runner.server.control().query_metrics(runner.current_time);
    let client = runner.client.control().query_metrics(runner.current_time);

    gtp_route::PathStats {
        path_id,
        fwd_owd_var_us: server.owd_var.map(|d| d.as_micros() as u32),
        fwd_jitter_us: server.jitter.map(|d| d.as_micros() as u32),
        rev_owd_var_us: client.owd_var.map(|d| d.as_micros() as u32),
        rev_jitter_us: client.jitter.map(|d| d.as_micros() as u32),
        rtt_us: Some(client.smoothed_rtt.as_micros() as u32),
        sample_count: frames,
    }
}

#[test]
fn selector_picks_the_measured_better_path() {
    // The impaired path has the LOWER id and appears FIRST: if selection
    // were order- or id-driven it would win. It must lose on measurement.
    let impaired = measure_pair(11, profile(20, 15), 2);
    let clean = measure_pair(22, profile(20, 0), 7);

    // Premises: the impairments actually surfaced in the measurements.
    let impaired_score = score(&impaired).expect("impaired path must be scorable");
    let clean_score = score(&clean).expect("clean path must be scorable");
    assert!(
        impaired_score < clean_score,
        "premise: impaired scores worse ({impaired_score} vs {clean_score})"
    );
    assert!(
        impaired.fwd_jitter_us.unwrap_or(0) > clean.fwd_jitter_us.unwrap_or(0),
        "premise: jitter visibly higher on the impaired path"
    );

    let selection = select(&[impaired, clean]);
    assert_eq!(selection.chosen, Some(7), "the measured-better path wins");
    assert_eq!(selection.reason, SelectionReason::ClearWinner);
    assert_eq!(selection.runner_up, Some(2));

    // Shadow discipline (INV-15): a selection is a RECORD — the runners'
    // data planes are untouched by computing it.
    assert!(
        selection.scored.iter().all(|s| s.score.is_some()),
        "every candidate carries a structured scored record (B-10)"
    );
    assert_eq!(selection.summary(), "SELECT path 7 (CLEAR_WINNER)");
}
