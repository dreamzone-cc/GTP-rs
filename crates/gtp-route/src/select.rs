//! Selection among candidates and single-path health (B-10 slice).
//!
//! [`select`] is pure and total: same candidates, same answer, plus a
//! machine-readable reason code for the decision log. It NEVER actuates
//! anything — executing a choice is the (future) adapter's job; today the
//! honest execution is "print it" (shadow discipline, INV-15).

use crate::score::{confidence, score, CONFIDENCE_FLOOR};
use crate::PathStats;

/// Relative effective-score margin below which the top two are treated as a
/// tie and broken by the lower `path_id` (deterministic tie-break).
pub const CLEAR_WINNER_MARGIN: f64 = 0.05;

/// Why a selection came out the way it did — the B-10 reason code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionReason {
    /// The candidate list was empty.
    NoCandidates,
    /// The best candidate has no measured axis at all.
    InsufficientData,
    /// Every candidate sits below [`CONFIDENCE_FLOOR`] (B-9): confirmable,
    /// never fast-pickable. No winner is declared.
    ConfidenceFloorHold,
    /// Exactly one candidate with usable data — trivially selected.
    SingleCandidate,
    /// The winner cleared [`CLEAR_WINNER_MARGIN`] over the runner-up.
    ClearWinner,
    /// Top two within the margin — broken deterministically by lower id.
    TieBreakLowerId,
}

impl SelectionReason {
    /// Stable machine-readable code for the decision log.
    pub fn code(&self) -> &'static str {
        match self {
            SelectionReason::NoCandidates => "NO_CANDIDATES",
            SelectionReason::InsufficientData => "INSUFFICIENT_DATA",
            SelectionReason::ConfidenceFloorHold => "CONFIDENCE_FLOOR_HOLD",
            SelectionReason::SingleCandidate => "SINGLE_CANDIDATE",
            SelectionReason::ClearWinner => "CLEAR_WINNER",
            SelectionReason::TieBreakLowerId => "TIE_BREAK_LOWER_ID",
        }
    }
}

/// One candidate's scored record — the structured per-decision entry (B-10).
#[derive(Clone, Copy, Debug)]
pub struct ScoredCandidate {
    pub path_id: u32,
    /// Raw continuous score, `None` when no axis was measured.
    pub score: Option<f64>,
    /// B-9 confidence in `[0, 1]`.
    pub confidence: f64,
    /// `score * confidence`, the comparison key.
    pub effective: Option<f64>,
}

/// The outcome of [`select`].
#[derive(Clone, Debug)]
pub struct Selection {
    /// The chosen `path_id`, or `None` when no winner may be declared
    /// (empty input, no data anywhere, or the confidence floor).
    pub chosen: Option<u32>,
    pub reason: SelectionReason,
    /// Runner-up `path_id` when a comparison happened.
    pub runner_up: Option<u32>,
    /// Every candidate's scored record, input order.
    pub scored: Vec<ScoredCandidate>,
}

impl Selection {
    /// One-line human form for logs and the shadow report.
    pub fn summary(&self) -> String {
        match self.chosen {
            Some(id) => format!("SELECT path {} ({})", id, self.reason.code()),
            None => format!("HOLD ({})", self.reason.code()),
        }
    }
}

/// Rank candidates by effective score (score × confidence), with a
/// deterministic tie-break (higher confidence, then lower id).
fn rank(candidates: &[PathStats]) -> Vec<ScoredCandidate> {
    let mut scored: Vec<ScoredCandidate> = candidates
        .iter()
        .map(|c| {
            let s = score(c);
            let conf = confidence(c.sample_count);
            ScoredCandidate {
                path_id: c.path_id,
                score: s,
                confidence: conf,
                effective: s.map(|v| v * conf),
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        let key = |s: &ScoredCandidate| (s.effective.unwrap_or(f64::MIN), s.confidence);
        // Higher effective/confidence first; equal keys fall through to id.
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path_id.cmp(&b.path_id))
    });
    scored
}

/// Choose the best candidate — pure, deterministic, explainable.
///
/// Rules, in order:
/// 1. empty input → [`SelectionReason::NoCandidates`];
/// 2. the best candidate has no measured axis → `InsufficientData`;
/// 3. the best candidate's confidence is below [`CONFIDENCE_FLOOR`] →
///    `ConfidenceFloorHold` with **no winner** (B-9: confirm, never
///    fast-pick);
/// 4. a single candidate with data → `SingleCandidate`;
/// 5. a margin ≥ [`CLEAR_WINNER_MARGIN`] over the runner-up → `ClearWinner`;
/// 6. otherwise → `TieBreakLowerId` (deterministic).
pub fn select(candidates: &[PathStats]) -> Selection {
    let scored = rank(candidates);
    let mut selection = Selection {
        chosen: None,
        reason: SelectionReason::NoCandidates,
        runner_up: None,
        scored,
    };
    if candidates.is_empty() {
        return selection;
    }

    let best = &selection.scored[0];
    if best.score.is_none() {
        selection.reason = SelectionReason::InsufficientData;
        return selection;
    }
    if best.confidence < CONFIDENCE_FLOOR {
        selection.reason = SelectionReason::ConfidenceFloorHold;
        return selection;
    }

    let usable: Vec<&ScoredCandidate> = selection
        .scored
        .iter()
        .filter(|s| s.score.is_some())
        .collect();
    if usable.len() == 1 {
        selection.chosen = Some(best.path_id);
        selection.reason = SelectionReason::SingleCandidate;
        return selection;
    }

    let runner = usable[1];
    selection.runner_up = Some(runner.path_id);
    let best_eff = best.effective.unwrap_or(0.0);
    let runner_eff = runner.effective.unwrap_or(0.0);
    let margin = if best_eff > 0.0 {
        (best_eff - runner_eff) / best_eff
    } else {
        0.0
    };
    if margin >= CLEAR_WINNER_MARGIN {
        selection.chosen = Some(best.path_id);
        selection.reason = SelectionReason::ClearWinner;
    } else {
        // Within the margin: deterministic lower-id tie-break.
        let chosen = best.path_id.min(runner.path_id);
        selection.chosen = Some(chosen);
        selection.reason = SelectionReason::TieBreakLowerId;
    }
    selection
}

/// Single-path health verdict for probe tooling (no comparison involved).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthVerdict {
    /// All measured axes inside the degradation thresholds.
    Healthy,
    /// The worst axis exceeded its threshold; carries the axis name.
    Degraded(&'static str),
    /// No axis measured.
    InsufficientData,
}

/// Thresholds for [`health`] — a flat 50%-of-saturation reading is degraded.
pub const HEALTH_VAR_THRESHOLD_US: u32 = 25_000;
pub const HEALTH_JITTER_THRESHOLD_US: u32 = 10_000;
pub const HEALTH_RTT_THRESHOLD_US: u32 = 150_000;

/// Judge one path's health from its measured axes (worst offender wins).
pub fn health(stats: &PathStats) -> HealthVerdict {
    let mut worst: Option<(&'static str, f64)> = None;
    let mut consider = |name: &'static str, value: Option<u32>, threshold: u32| {
        if let Some(v) = value {
            if v > threshold {
                let ratio = v as f64 / threshold as f64;
                if worst.map(|(_, r)| ratio > r).unwrap_or(true) {
                    worst = Some((name, ratio));
                }
            }
        }
    };
    consider("fwd_owd_var", stats.fwd_owd_var_us, HEALTH_VAR_THRESHOLD_US);
    consider("rev_owd_var", stats.rev_owd_var_us, HEALTH_VAR_THRESHOLD_US);
    consider(
        "fwd_jitter",
        stats.fwd_jitter_us,
        HEALTH_JITTER_THRESHOLD_US,
    );
    consider(
        "rev_jitter",
        stats.rev_jitter_us,
        HEALTH_JITTER_THRESHOLD_US,
    );
    consider("rtt", stats.rtt_us, HEALTH_RTT_THRESHOLD_US);

    match worst {
        Some((axis, _)) => HealthVerdict::Degraded(axis),
        None => {
            let any = stats.fwd_owd_var_us.is_some()
                || stats.fwd_jitter_us.is_some()
                || stats.rev_owd_var_us.is_some()
                || stats.rev_jitter_us.is_some()
                || stats.rtt_us.is_some();
            if any {
                HealthVerdict::Healthy
            } else {
                HealthVerdict::InsufficientData
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(id: u32) -> PathStats {
        PathStats::full(id, 1_000, 300, 1_000, 300, 30_000, 60)
    }

    #[test]
    fn empty_input_and_no_data_are_explained() {
        let s = select(&[]);
        assert_eq!(s.reason, SelectionReason::NoCandidates);
        assert_eq!(s.chosen, None);

        let s = select(&[PathStats::default()]);
        assert_eq!(s.reason, SelectionReason::InsufficientData);
        assert_eq!(s.chosen, None);
        assert_eq!(s.summary(), "HOLD (INSUFFICIENT_DATA)");
    }

    #[test]
    fn confidence_floor_holds_without_a_winner() {
        // Excellent numbers but only 5 samples, and no confident competitor:
        // B-9 forbids declaring the fast pick even for the sole candidate.
        let thin = PathStats::full(1, 100, 100, 100, 100, 10_000, 5);
        let s = select(&[thin]);
        assert_eq!(s.reason, SelectionReason::ConfidenceFloorHold);
        assert_eq!(
            s.chosen, None,
            "below the floor: confirmable, never fast-picked"
        );
        assert_eq!(s.summary(), "HOLD (CONFIDENCE_FLOOR_HOLD)");

        // Multiplicative confidence at work: against a confident competitor
        // with similar numbers, the confident candidate outranks the thin
        // one on the EFFECTIVE score — no hold needed.
        let s = select(&[thin, clean(2)]);
        assert_eq!(s.chosen, Some(2));
        assert_eq!(s.reason, SelectionReason::ClearWinner);
        let best = s.scored[0];
        assert_eq!(
            best.path_id, 2,
            "confidence lifts the trustworthy candidate"
        );
    }

    #[test]
    fn single_candidate_is_trivial() {
        let s = select(&[clean(9)]);
        assert_eq!(s.chosen, Some(9));
        assert_eq!(s.reason, SelectionReason::SingleCandidate);
        assert_eq!(s.summary(), "SELECT path 9 (SINGLE_CANDIDATE)");
    }

    #[test]
    fn clear_winner_beats_a_degraded_candidate() {
        let degraded = PathStats::full(2, 45_000, 18_000, 40_000, 16_000, 250_000, 60);
        // Higher id on the good path: winning must be by score, not order/id.
        let s = select(&[degraded, clean(7)]);
        assert_eq!(s.chosen, Some(7));
        assert_eq!(s.reason, SelectionReason::ClearWinner);
        assert_eq!(s.runner_up, Some(2));
    }

    #[test]
    fn ties_break_to_the_lower_id() {
        let a = clean(4);
        let b = clean(9);
        let s = select(&[b, a]);
        assert_eq!(s.reason, SelectionReason::TieBreakLowerId);
        assert_eq!(s.chosen, Some(4), "deterministic lower-id tie-break");
        assert_eq!(s.runner_up, Some(9));
    }

    #[test]
    fn health_flags_the_worst_axis() {
        assert_eq!(health(&clean(1)), HealthVerdict::Healthy);

        let mut bad = clean(1);
        bad.rev_owd_var_us = Some(40_000);
        assert_eq!(health(&bad), HealthVerdict::Degraded("rev_owd_var"));

        assert_eq!(
            health(&PathStats::default()),
            HealthVerdict::InsufficientData
        );
    }
}
