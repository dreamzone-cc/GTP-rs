//! Adaptive route switching controller (B-2 / F3).
//!
//! Wraps [`crate::select`] with the hysteresis and safety policies that turn
//! a pure ranking into an actuation decision. This module is STILL PURE —
//! no tokio, no sockets; the caller (the connection layer) feeds it measured
//! [`PathStats`] and executes whatever it returns.
//!
//! Policies (B-2 design, validated against S01–S12):
//! - **Dwell floor**: a newly selected path must be held for `min_dwell`
//!   before another switch is even considered — prevents flap on noise.
//! - **Revert window**: if the path we JUST LEFT scores better than the one
//!   we switched to within `revert_window`, we switch back immediately
//!   (the previous path recovered — the degradation was transient).
//! - **Shadow mode**: the controller returns decisions but the caller
//!   executes nothing (INV-15 — the safe rollout posture).

use crate::select::{self, SelectionReason};
use crate::{DecisionTracker, PathStats};
use gtp_types::{Duration, MonotonicTime};

/// Default minimum dwell on a selected path before another switch.
pub const DEFAULT_MIN_DWELL: Duration = Duration::from_millis(200);
/// Default revert window: if the OLD path beats the NEW one within this
/// window, switch back (the degradation was transient).
pub const DEFAULT_REVERT_WINDOW: Duration = Duration::from_millis(500);

/// Whether the caller should actually actuate the controller's decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchPolicy {
    /// Log decisions, never actuate (safe rollout — INV-15).
    Shadow,
    /// Actuate: switch the active path to the controller's selection.
    Enforce,
}

/// What the controller decided on one evaluation.
#[derive(Clone, Debug, PartialEq)]
pub enum SwitchDecision {
    /// Stay on the current path (with the selector's reason for the record).
    Hold {
        current: u32,
        reason: SelectionReason,
    },
    /// Switch to a new path (the selector's winner cleared every gate).
    Switch {
        from: u32,
        to: u32,
        reason: SelectionReason,
    },
    /// Switch BACK to the path we just left — it recovered within the
    /// revert window; the degradation was transient.
    Revert { from: u32, to: u32 },
}

/// Adaptive switching controller — pure decision engine with hysteresis.
#[derive(Debug)]
pub struct SwitchController {
    policy: SwitchPolicy,
    min_dwell: Duration,
    revert_window: Duration,
    /// Currently selected path (set on the first evaluation or a switch).
    selected: Option<u32>,
    /// The path we were on before the last switch (for revert detection).
    previous: Option<u32>,
    /// When the last switch happened.
    last_switch_time: MonotonicTime,
    /// Decision counter (for the structured log).
    decision_count: u64,
}

impl SwitchController {
    pub fn new(policy: SwitchPolicy) -> Self {
        Self {
            policy,
            min_dwell: DEFAULT_MIN_DWELL,
            revert_window: DEFAULT_REVERT_WINDOW,
            selected: None,
            previous: None,
            last_switch_time: MonotonicTime::ZERO,
            decision_count: 0,
        }
    }

    pub fn policy(&self) -> SwitchPolicy {
        self.policy
    }

    pub fn selected(&self) -> Option<u32> {
        self.selected
    }

    /// Evaluates the candidates and returns what the caller should do.
    ///
    /// This is the ONLY method the connection layer calls: feed it the
    /// current measurements, get back a decision (which may be "hold").
    pub fn evaluate(
        &mut self,
        candidates: &[PathStats],
        now: MonotonicTime,
        tracker: &mut DecisionTracker,
    ) -> SwitchDecision {
        self.decision_count += 1;
        let selection = select::select(candidates);

        // Record the decision (shadow or enforce — the log is identical).
        tracker.record(0, crate::PolicyClass::Shadow, None, &selection);

        let winner = match selection.chosen {
            Some(w) => w,
            None => {
                return SwitchDecision::Hold {
                    current: self.selected.unwrap_or(0),
                    reason: selection.reason,
                };
            }
        };

        // First evaluation: adopt the winner without a switch event.
        if self.selected.is_none() {
            self.selected = Some(winner);
            self.last_switch_time = now;
            return SwitchDecision::Hold {
                current: winner,
                reason: selection.reason,
            };
        }

        let current = self.selected.unwrap();

        // Same path: hold.
        if winner == current {
            return SwitchDecision::Hold {
                current,
                reason: selection.reason,
            };
        }

        // Revert detection runs BEFORE the dwell floor: a revert is not a
        // "new switch" — it undoes a recent one whose cause proved transient.
        if let Some(prev) = self.previous {
            if winner == prev && now.duration_since(self.last_switch_time) < self.revert_window {
                self.previous = Some(current);
                self.selected = Some(winner);
                self.last_switch_time = now;
                return SwitchDecision::Revert {
                    from: current,
                    to: winner,
                };
            }
        }

        // Dwell floor: a newly selected path must be held before switching again.
        if now.duration_since(self.last_switch_time) < self.min_dwell {
            return SwitchDecision::Hold {
                current,
                reason: SelectionReason::TieBreakLowerId, // reuse: "cooldown"
            };
        }

        // The winner is genuinely better and the dwell floor has passed.
        if self.policy == SwitchPolicy::Enforce {
            self.previous = Some(current);
            self.selected = Some(winner);
            self.last_switch_time = now;
        }
        // In Shadow mode we return the decision but DON'T update state —
        // the caller is expected to ignore it; the next evaluate sees the
        // same "current" and re-evaluates cleanly.
        else {
            return SwitchDecision::Hold {
                current,
                reason: SelectionReason::ClearWinner, // the winner exists but we're shadowing
            };
        }

        SwitchDecision::Switch {
            from: current,
            to: winner,
            reason: selection.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::SelectionReason;

    fn clean(id: u32, rtt_us: u32) -> PathStats {
        PathStats::full(id, 1_000, 300, 1_000, 300, rtt_us, 60, 0, 0)
    }

    fn degraded(id: u32, rtt_us: u32) -> PathStats {
        PathStats::full(id, 45_000, 18_000, 40_000, 16_000, rtt_us, 60, 0, 0)
    }

    #[test]
    fn first_evaluation_adopts_without_switching() {
        let mut sc = SwitchController::new(SwitchPolicy::Enforce);
        let mut tracker = DecisionTracker::new(32);
        let now = MonotonicTime::from_micros(1_000_000);

        let d = sc.evaluate(&[clean(1, 30_000), clean(2, 50_000)], now, &mut tracker);
        assert!(matches!(d, SwitchDecision::Hold { current: 1, .. }));
    }

    #[test]
    fn sustained_degradation_switches_after_dwell() {
        let mut sc = SwitchController::new(SwitchPolicy::Enforce);
        let mut tracker = DecisionTracker::new(32);
        let t0 = MonotonicTime::from_micros(1_000_000);

        // t0: path 1 is better → adopt.
        sc.evaluate(&[clean(1, 30_000), clean(2, 50_000)], t0, &mut tracker);
        assert_eq!(sc.selected(), Some(1));

        // t0+100ms: path 1 degrades, path 2 stays clean — but we're still
        // inside the dwell floor (200ms), so hold.
        let t1 = t0 + Duration::from_millis(100);
        let d = sc.evaluate(&[degraded(1, 250_000), clean(2, 50_000)], t1, &mut tracker);
        assert!(
            matches!(d, SwitchDecision::Hold { .. }),
            "inside dwell floor"
        );

        // t0+300ms: past the dwell floor — switch to path 2.
        let t2 = t0 + Duration::from_millis(300);
        let d = sc.evaluate(&[degraded(1, 250_000), clean(2, 50_000)], t2, &mut tracker);
        assert!(
            matches!(d, SwitchDecision::Switch { from: 1, to: 2, .. }),
            "sustained degradation must switch after dwell, got {d:?}"
        );
        assert_eq!(sc.selected(), Some(2));
    }

    #[test]
    fn transient_spike_does_not_switch() {
        let mut sc = SwitchController::new(SwitchPolicy::Enforce);
        let mut tracker = DecisionTracker::new(32);
        let t0 = MonotonicTime::from_micros(1_000_000);

        // t0: path 1 better → adopt.
        sc.evaluate(&[clean(1, 30_000), clean(2, 50_000)], t0, &mut tracker);

        // t0+100ms: path 1 spikes — but we're INSIDE the 200ms dwell floor,
        // so the switch to path 2 is blocked (the spike is a single sample).
        let t1 = t0 + Duration::from_millis(100);
        let d = sc.evaluate(&[degraded(1, 250_000), clean(2, 50_000)], t1, &mut tracker);
        assert!(
            matches!(d, SwitchDecision::Hold { .. }),
            "inside dwell floor: spike must not switch"
        );

        // t0+300ms: path 1 is clean again by the time the dwell floor passes.
        // The selector ranks path 1 back on top — no switch happened at all.
        let t2 = t0 + Duration::from_millis(300);
        let d = sc.evaluate(&[clean(1, 30_000), clean(2, 50_000)], t2, &mut tracker);
        assert!(
            matches!(d, SwitchDecision::Hold { current: 1, .. }),
            "a transient spike must NOT cause a permanent switch, got {d:?}"
        );
        assert_eq!(sc.selected(), Some(1), "still on the original path");
    }

    #[test]
    fn revert_within_window_switches_back() {
        let mut sc = SwitchController::new(SwitchPolicy::Enforce);
        let mut tracker = DecisionTracker::new(32);
        let t0 = MonotonicTime::from_micros(1_000_000);

        // t0: path 1 better → adopt.
        sc.evaluate(&[clean(1, 30_000), clean(2, 50_000)], t0, &mut tracker);

        // t0+300ms: path 1 degrades → switch to path 2.
        let t1 = t0 + Duration::from_millis(300);
        let _ = sc.evaluate(&[degraded(1, 250_000), clean(2, 50_000)], t1, &mut tracker);
        assert_eq!(sc.selected(), Some(2));

        // t0+400ms: path 1 RECOVERS (the degradation was transient, 100ms).
        // Within the 500ms revert window: switch back.
        let t2 = t0 + Duration::from_millis(400);
        let d = sc.evaluate(&[clean(1, 30_000), clean(2, 50_000)], t2, &mut tracker);
        assert!(
            matches!(d, SwitchDecision::Revert { from: 2, to: 1 }),
            "recovery within the revert window must revert, got {d:?}"
        );
        assert_eq!(sc.selected(), Some(1));
    }

    #[test]
    fn shadow_mode_never_switches() {
        let mut sc = SwitchController::new(SwitchPolicy::Shadow);
        let mut tracker = DecisionTracker::new(32);
        let t0 = MonotonicTime::from_micros(1_000_000);

        sc.evaluate(&[clean(1, 30_000), clean(2, 50_000)], t0, &mut tracker);
        let t1 = t0 + Duration::from_millis(300);
        let d = sc.evaluate(&[degraded(1, 250_000), clean(2, 50_000)], t1, &mut tracker);
        assert!(
            matches!(d, SwitchDecision::Hold { .. }),
            "shadow never actuates"
        );
        assert_eq!(sc.selected(), Some(1), "shadow keeps the original path");
    }
}
