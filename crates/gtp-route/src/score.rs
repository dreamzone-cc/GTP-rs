//! Continuous scoring and confidence (B-3 / B-9 slices).
//!
//! Every axis maps lower-is-better onto `[0, 1]` with a **linear saturation**
//! — no step functions, no cliffs (the paper §4.1.2 step scorer is a flap
//! generator per ARDP §3.2, and is not reimplemented here). A missing axis is
//! excluded and the remaining weights renormalize: unknown is never scored
//! as excellent.

use crate::PathStats;

/// `owd_var` values at and above this read as a fully degraded axis.
pub const OWD_VAR_SATURATION_US: u32 = 50_000;
/// Jitter values at and above this read as a fully degraded axis.
pub const JITTER_SATURATION_US: u32 = 20_000;
/// RTT values at and above this read as a fully degraded axis.
pub const RTT_SATURATION_US: u32 = 300_000;

/// Within-direction weight of the variance axis (jitter takes the rest).
pub const VAR_AXIS_WEIGHT: f64 = 0.6;

/// Path-level weights: forward direction, reverse direction, round trip.
/// Starting points only — the shipped defaults are a calibration output of
/// the G3 24-hour shadow dataset (ARDP §11.1), not constants.
pub const PATH_WEIGHTS: (f64, f64, f64) = (0.30, 0.30, 0.25);

/// F4 (RT-1/E-5): loss rate at and above this reads as a fully degraded
/// axis. 5%: beyond this, interactive gameplay is impossible regardless of
/// how good the latency axes are.
pub const LOSS_SATURATION: f64 = 0.05;
/// F4: weight of the loss axis relative to the latency axes (the remaining
/// weight is distributed among the latency axes by their own weights).
pub const LOSS_AXIS_WEIGHT: f64 = 0.15;

/// Sample count at which [`confidence`] reaches 1.0.
pub const CONFIDENCE_FULL_SAMPLES: f64 = 30.0;
/// Below this confidence a candidate may be confirmed but never fast-picked
/// (B-9).
pub const CONFIDENCE_FLOOR: f64 = 0.5;

/// RT-3 / B-9 recency: evidence at most this old is fully fresh. One report
/// interval of carry-over is normal (the CLI reports at 1 s), so the grace
/// covers it without penalty.
pub const FRESHNESS_GRACE_US: u64 = 1_000_000;
/// RT-3 / B-9 recency: evidence at or beyond this age contributes nothing
/// (`freshness() == 0.0`) — a path nobody has heard from for 5 s must not be
/// trusted over a measured live one.
pub const FRESHNESS_SATURATION_US: u64 = 5_000_000;

/// Linear lower-is-better normalization onto `[0, 1]` with the documented
/// saturation point. Monotone non-increasing; exact at the boundaries.
pub fn normalize_lower_better(value: u32, saturation: u32) -> f64 {
    if saturation == 0 {
        return 0.0;
    }
    1.0 - (value as f64 / saturation as f64).min(1.0)
}

/// One direction's quality from its two axes; `None` when both are unknown.
fn direction_score(var_us: Option<u32>, jitter_us: Option<u32>) -> Option<f64> {
    match (var_us, jitter_us) {
        (Some(v), Some(j)) => Some(
            VAR_AXIS_WEIGHT * normalize_lower_better(v, OWD_VAR_SATURATION_US)
                + (1.0 - VAR_AXIS_WEIGHT) * normalize_lower_better(j, JITTER_SATURATION_US),
        ),
        (Some(v), None) => Some(normalize_lower_better(v, OWD_VAR_SATURATION_US)),
        (None, Some(j)) => Some(normalize_lower_better(j, JITTER_SATURATION_US)),
        (None, None) => None,
    }
}

/// Continuous path quality in `[0, 1]` (higher is better) over the axes that
/// are present. `None` when no axis is measured at all.
///
/// INV-11: the input must describe one path in one epoch; mixing scopes
/// here is the caller's bug, not a scoring feature.
pub fn score(stats: &PathStats) -> Option<f64> {
    let fwd = direction_score(stats.fwd_owd_var_us, stats.fwd_jitter_us);
    let rev = direction_score(stats.rev_owd_var_us, stats.rev_jitter_us);
    let rtt = stats
        .rtt_us
        .map(|r| normalize_lower_better(r, RTT_SATURATION_US));

    let (w_fwd, w_rev, w_rtt) = PATH_WEIGHTS;
    let mut weight_sum = 0.0;
    let mut acc = 0.0;
    if let Some(s) = fwd {
        weight_sum += w_fwd * (1.0 - LOSS_AXIS_WEIGHT);
        acc += w_fwd * (1.0 - LOSS_AXIS_WEIGHT) * s;
    }
    if let Some(s) = rev {
        weight_sum += w_rev * (1.0 - LOSS_AXIS_WEIGHT);
        acc += w_rev * (1.0 - LOSS_AXIS_WEIGHT) * s;
    }
    if let Some(s) = rtt {
        weight_sum += w_rtt * (1.0 - LOSS_AXIS_WEIGHT);
        acc += w_rtt * (1.0 - LOSS_AXIS_WEIGHT) * s;
    }
    // F4: loss axis — lower-is-better onto [0,1] with 5% saturation.
    // Unknown (None) is excluded and the remaining weights renormalize.
    if let Some(lr) = stats.loss_rate {
        let loss_score = 1.0 - (lr / LOSS_SATURATION).min(1.0);
        weight_sum += LOSS_AXIS_WEIGHT;
        acc += LOSS_AXIS_WEIGHT * loss_score;
    }
    if weight_sum == 0.0 {
        None
    } else {
        Some(acc / weight_sum)
    }
}

/// B-9 sample-count confidence in `[0, 1]`: linear up to
/// [`CONFIDENCE_FULL_SAMPLES`], saturated after. Applied multiplicatively to
/// [`score`] by the caller (see [`crate::select`]).
pub fn confidence(sample_count: u32) -> f64 {
    (sample_count as f64 / CONFIDENCE_FULL_SAMPLES).min(1.0)
}

/// RT-3 / B-9 recency factor in `[0, 1]`. Within the grace window evidence
/// is fully fresh; beyond it the factor decays linearly to zero at the
/// saturation age. `None` (unknown age — e.g. a legacy GTPRP1 report) is
/// **neutral 1.0, surfaced as unknown in the decision record**: freshness
/// enforcement is applied to the ages the endpoints carry, never invented.
pub fn freshness(age_us: Option<u64>) -> f64 {
    let Some(age) = age_us else {
        return 1.0;
    };
    if age <= FRESHNESS_GRACE_US {
        1.0
    } else {
        let span = (FRESHNESS_SATURATION_US - FRESHNESS_GRACE_US) as f64;
        ((FRESHNESS_SATURATION_US as f64 - age as f64) / span).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_exact_at_boundaries() {
        assert_eq!(normalize_lower_better(0, 50_000), 1.0);
        assert_eq!(normalize_lower_better(25_000, 50_000), 0.5);
        assert_eq!(normalize_lower_better(50_000, 50_000), 0.0);
        assert_eq!(
            normalize_lower_better(500_000, 50_000),
            0.0,
            "clamped, never negative"
        );
        assert_eq!(
            normalize_lower_better(7, 0),
            0.0,
            "degenerate saturation is inert"
        );
    }

    #[test]
    fn score_renormormalizes_over_present_axes() {
        // Only forward present: the score IS the forward direction score.
        let fwd_only = PathStats {
            path_id: 1,
            fwd_owd_var_us: Some(0),
            fwd_jitter_us: Some(0),
            ..Default::default()
        };
        assert_eq!(score(&fwd_only), Some(1.0));

        // Forward perfect + RTT fully saturated: (0.30*1 + 0.25*0) / 0.55.
        let mixed = PathStats {
            rtt_us: Some(RTT_SATURATION_US),
            ..fwd_only
        };
        let expected = 0.30 / 0.55;
        let got = score(&mixed).unwrap();
        assert!(
            (got - expected).abs() < 1e-12,
            "got {got}, expected {expected}"
        );

        // Nothing measured: unknown, not excellent.
        assert_eq!(score(&PathStats::default()), None);
    }

    #[test]
    fn score_is_monotone_in_every_axis() {
        let base = PathStats::full(0, 10_000, 2_000, 10_000, 2_000, 40_000, 30, 0, 0);
        let s0 = score(&base).unwrap();
        for worse in [
            PathStats {
                fwd_owd_var_us: Some(30_000),
                ..base
            },
            PathStats {
                fwd_jitter_us: Some(12_000),
                ..base
            },
            PathStats {
                rev_owd_var_us: Some(45_000),
                ..base
            },
            PathStats {
                rev_jitter_us: Some(19_000),
                ..base
            },
            PathStats {
                rtt_us: Some(250_000),
                ..base
            },
        ] {
            assert!(
                score(&worse).unwrap() < s0,
                "degrading any axis must lower the score ({:?})",
                worse
            );
        }
    }

    /// F4: loss degrades the score monotonically; 5%+ saturates to zero.
    #[test]
    fn loss_axis_degrades_and_saturates() {
        let base = PathStats::full(0, 1_000, 300, 1_000, 300, 30_000, 60, 0, 0);
        let s0 = score(&base).unwrap();

        for (rate, expect_lower) in [
            (0.005, true),
            (0.01, true),
            (0.03, true),
            (0.05, true),
            (0.10, true), // saturated: same as 0.05
        ] {
            let degraded = PathStats {
                loss_rate: Some(rate),
                ..base
            };
            let s = score(&degraded).unwrap();
            if expect_lower {
                assert!(
                    s < s0,
                    "loss_rate={rate} must lower the score ({s} >= {s0})"
                );
            }
        }
        // 5% and 10% both saturate — identical scores.
        let l5 = score(&PathStats {
            loss_rate: Some(0.05),
            ..base
        })
        .unwrap();
        let l10 = score(&PathStats {
            loss_rate: Some(0.10),
            ..base
        })
        .unwrap();
        assert_eq!(l5, l10, "beyond saturation the axis is clamped");
        // Unknown loss renormalizes — the latency axes still work.
        let unknown = score(&PathStats {
            loss_rate: None,
            ..base
        })
        .unwrap();
        assert_eq!(
            unknown, s0,
            "unknown loss is neutral (excluded, not excellent)"
        );
    }

    #[test]
    fn confidence_is_linear_then_saturated() {
        assert_eq!(confidence(0), 0.0);
        assert_eq!(confidence(15), 0.5);
        assert_eq!(confidence(30), 1.0);
        assert_eq!(confidence(30_000), 1.0);
    }

    #[test]
    fn freshness_grace_then_linear_decay_to_zero() {
        assert_eq!(freshness(None), 1.0, "unknown age is neutral, not punished");
        assert_eq!(freshness(Some(0)), 1.0);
        assert_eq!(
            freshness(Some(FRESHNESS_GRACE_US)),
            1.0,
            "grace window is free"
        );
        // Midway between grace (1 s) and saturation (5 s): factor 0.5.
        let mid = (FRESHNESS_GRACE_US + FRESHNESS_SATURATION_US) / 2; // 3 s
        assert_eq!(freshness(Some(mid)), 0.5);
        assert_eq!(freshness(Some(FRESHNESS_SATURATION_US)), 0.0);
        assert_eq!(freshness(Some(60_000_000)), 0.0, "clamped at zero");
        // Monotone non-increasing across the whole domain.
        let mut prev = 1.0;
        for age in [
            0u64, 500_000, 1_000_000, 2_000_000, 3_000_000, 4_000_000, 5_000_000, 9_000_000,
        ] {
            let f = freshness(Some(age));
            assert!(f <= prev, "freshness must be non-increasing (age {age})");
            prev = f;
        }
    }
}
