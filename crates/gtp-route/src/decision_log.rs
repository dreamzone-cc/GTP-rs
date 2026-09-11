//! Structured decision logging (§17 of the gap paper; B-10 record + the
//! observability half of B-13) — every selection, **including shadow
//! non-decisions**, becomes a machine-readable, replayable record.
//!
//! Pure and deterministic: the tracker owns no clock and performs no I/O;
//! callers feed it records built from [`crate::select`]. Storage is a
//! bounded ring with a dropped-records counter (INV-18 discipline).
//! Serializes as one ASCII line, `GTPDL1|…`, strict-parse on the way back.

use crate::select::ScoredCandidate;
use crate::select::{Selection, SelectionReason};

/// Line prefix identifying the decision-log format (version 1).
pub const DECISION_LOG_PREFIX: &str = "GTPDL1";

/// Which decision class produced this record (per-class policies: B-12 /
/// gap paper §19). Shadow-mode selections are always `Shadow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyClass {
    /// Computed, recorded, actuated nothing (the only class until G4).
    Shadow,
    /// Failover-class decision (G4+).
    Failover,
    /// Degradation-class decision (G5+).
    Degradation,
    /// Optimization-class decision (G6+).
    Optimization,
}

impl PolicyClass {
    pub fn code(&self) -> &'static str {
        match self {
            PolicyClass::Shadow => "SHADOW",
            PolicyClass::Failover => "FAILOVER",
            PolicyClass::Degradation => "DEGRADATION",
            PolicyClass::Optimization => "OPTIMIZATION",
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "SHADOW" => Some(PolicyClass::Shadow),
            "FAILOVER" => Some(PolicyClass::Failover),
            "DEGRADATION" => Some(PolicyClass::Degradation),
            "OPTIMIZATION" => Some(PolicyClass::Optimization),
            _ => None,
        }
    }
}

/// One structured decision record — the §17 field set. All values are
/// copyable; the per-candidate table rides along so a decision can be
/// re-verified from the log alone (gap paper §16 replay requirement).
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionRecord {
    /// Monotonic decision sequence number (from [`DecisionTracker`]).
    pub decision_id: u64,
    /// The connection the decision was computed for.
    pub connection_id: u64,
    /// The policy class (see [`PolicyClass`]).
    pub policy_class: PolicyClass,
    /// The candidate path ids considered, input order.
    pub path_set: Vec<u32>,
    /// Measurement window length, µs, when the caller knows it.
    pub measurement_window_us: Option<u64>,
    /// The path chosen before this decision (tracker memory).
    pub previous_path: Option<u32>,
    /// The path this decision selects (`None` on holds).
    pub selected_path: Option<u32>,
    /// The runner-up, when a comparison happened.
    pub runner_up: Option<u32>,
    /// The reason code (B-10).
    pub reason: SelectionReason,
    /// Per-candidate scored entries, ranked order (score/confidence/
    /// freshness/effective + carried ages).
    pub candidates: Vec<ScoredCandidate>,
}

fn opt_u32(v: Option<u32>) -> String {
    match v {
        Some(v) => v.to_string(),
        None => "-".to_string(),
    }
}

fn opt_f(v: Option<f64>) -> String {
    match v {
        Some(v) => format!("{v:.4}"),
        None => "-".to_string(),
    }
}

impl DecisionRecord {
    /// Build the record from a [`Selection`] — pure; `decision_id` comes
    /// from the tracker, `connection_id`/`window` from the caller.
    pub fn from_selection(
        decision_id: u64,
        connection_id: u64,
        policy_class: PolicyClass,
        measurement_window_us: Option<u64>,
        selection: &Selection,
    ) -> Self {
        Self {
            decision_id,
            connection_id,
            policy_class,
            path_set: selection.scored.iter().map(|c| c.path_id).collect(),
            measurement_window_us,
            previous_path: None, // filled by the tracker
            selected_path: selection.chosen,
            runner_up: selection.runner_up,
            reason: selection.reason,
            candidates: selection.scored.clone(),
        }
    }

    /// One-line human form for logs.
    pub fn summary(&self) -> String {
        format!(
            "decision {} on conn {:X}: {} -> {} ({})",
            self.decision_id,
            self.connection_id,
            opt_u32(self.previous_path),
            opt_u32(self.selected_path),
            self.reason.code(),
        )
    }

    /// Encode as `GTPDL1|id|conn|class|window|prev|sel|runner|reason|cand…`
    /// where each candidate is `path:score:conf:fresh:eff:age` and `-`
    /// marks absent values. Strict-parse back (round-trip exact).
    pub fn encode(&self) -> String {
        let mut s = format!(
            "{}|{}|{:X}|{}|{}|{}|{}|{}|{}",
            DECISION_LOG_PREFIX,
            self.decision_id,
            self.connection_id,
            self.policy_class.code(),
            match self.measurement_window_us {
                Some(v) => v.to_string(),
                None => "-".to_string(),
            },
            opt_u32(self.previous_path),
            opt_u32(self.selected_path),
            opt_u32(self.runner_up),
            self.reason.code(),
        );
        for c in &self.candidates {
            s.push('|');
            let age = match c.age_us {
                Some(a) => a.to_string(),
                None => "-".to_string(),
            };
            // conf/fresh format inline ({:.4}) — same precision opt_f uses,
            // so encode->parse->encode replays exactly.
            s.push_str(&format!(
                "{}:{}:{:.4}:{:.4}:{}:{}",
                c.path_id,
                opt_f(c.score),
                c.confidence,
                c.freshness,
                opt_f(c.effective),
                age,
            ));
        }
        s
    }

    /// Strict parse of [`DecisionRecord::encode`] output: any malformed
    /// field rejects the whole line (a decision log that cannot be trusted
    /// is not half-accepted).
    pub fn parse(line: &str) -> Option<Self> {
        let rest = line.strip_prefix(DECISION_LOG_PREFIX)?.strip_prefix('|')?;
        let mut parts = rest.split('|');
        let decision_id = parts.next()?.parse::<u64>().ok()?;
        let connection_id = u64::from_str_radix(parts.next()?, 16).ok()?;
        let policy_class = PolicyClass::from_code(parts.next()?)?;
        let measurement_window_us = match parts.next()? {
            "-" => None,
            v => Some(v.parse::<u64>().ok()?),
        };
        let opt_u32 = |v: Option<&str>| -> Option<Option<u32>> {
            match v? {
                "-" => Some(None),
                s => s.parse::<u32>().ok().map(Some),
            }
        };
        let previous_path = opt_u32(parts.next())?;
        let selected_path = opt_u32(parts.next())?;
        let runner_up = opt_u32(parts.next())?;
        let reason = SelectionReason::from_code(parts.next()?)?;
        let mut candidates = Vec::new();
        for cand in parts {
            let mut f = cand.split(':');
            let path_id = f.next()?.parse::<u32>().ok()?;
            let score = match f.next()? {
                "-" => None,
                v => Some(v.parse::<f64>().ok()?),
            };
            let confidence = f.next()?.parse::<f64>().ok()?;
            let freshness = f.next()?.parse::<f64>().ok()?;
            let effective = match f.next()? {
                "-" => None,
                v => Some(v.parse::<f64>().ok()?),
            };
            let age_us = match f.next()? {
                "-" => None,
                v => Some(v.parse::<u64>().ok()?),
            };
            if f.next().is_some() {
                return None; // trailing fields inside a candidate: reject
            }
            candidates.push(ScoredCandidate {
                path_id,
                score,
                confidence,
                freshness,
                age_us,
                effective,
            });
        }
        Some(Self {
            decision_id,
            connection_id,
            policy_class,
            path_set: candidates.iter().map(|c| c.path_id).collect(),
            measurement_window_us,
            previous_path,
            selected_path,
            runner_up,
            reason,
            candidates,
        })
    }
}

/// Bounded, deterministic decision-log store with switch/revert KPIs —
/// the observability half of B-13 (actuation-side KPIs arrive with G4).
///
/// INV-18 discipline: the record ring is capacity-bounded; overflow sheds
/// the OLDEST record and counts it in `dropped_records`. `record()` returns
/// the stored record's id and updates the KPIs; the tracker never mutates
/// the selection itself (INV-15: logging is observing).
pub struct DecisionTracker {
    next_id: u64,
    previous_path: Option<u32>,
    /// The selected path from TWO selections ago — a switch straight back
    /// to it is a revert verdict (leaving a path must never count as one).
    before_previous_path: Option<u32>,
    ring: std::collections::VecDeque<DecisionRecord>,
    capacity: usize,
    /// Records shed by the capacity bound (surfaced, never silent).
    pub dropped_records: u64,
    /// KPI: total decisions recorded (all classes, incl. holds).
    pub decisions: u64,
    /// KPI: decisions that changed the selected path.
    pub switch_count: u64,
    /// KPI: switches that returned to the immediately-previous path
    /// (revert events — B-13's false-switch numerator once actuation
    /// exists; in shadow it counts revert-shaped verdicts).
    pub revert_count: u64,
    /// KPI: µs between the last two switches (dwell), when the caller
    /// stamps windows; `None` until two switches.
    pub last_dwell_us: Option<u64>,
    last_switch_window_us: Option<u64>,
}

impl DecisionTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            next_id: 0,
            previous_path: None,
            before_previous_path: None,
            ring: std::collections::VecDeque::new(),
            capacity,
            dropped_records: 0,
            decisions: 0,
            switch_count: 0,
            revert_count: 0,
            last_dwell_us: None,
            last_switch_window_us: None,
        }
    }

    /// Record one selection. `window_us` is the caller's clock stamp for
    /// dwell KPIs (virtual time in tests, wall-derived in the adapter).
    /// Returns the decision id. Holds still record (a non-decision is a
    /// decision — gap paper §17 requirement).
    pub fn record(
        &mut self,
        connection_id: u64,
        policy_class: PolicyClass,
        window_us: Option<u64>,
        selection: &Selection,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut rec =
            DecisionRecord::from_selection(id, connection_id, policy_class, window_us, selection);
        rec.previous_path = self.previous_path;

        if let (Some(prev), Some(sel)) = (self.previous_path, selection.chosen) {
            if prev != sel {
                self.switch_count += 1;
                if let (Some(now), Some(last)) = (window_us, self.last_switch_window_us) {
                    self.last_dwell_us = Some(now.saturating_sub(last));
                }
                self.last_switch_window_us = window_us;
                // A revert verdict: switching straight back to the path we
                // had BEFORE the one we are leaving (7→2→7), never merely
                // leaving a path.
                if self.before_previous_path == Some(sel) {
                    self.revert_count += 1;
                }
            }
        }
        if let Some(sel) = selection.chosen {
            self.before_previous_path = self.previous_path;
            self.previous_path = Some(sel);
        }

        self.decisions += 1;
        if self.capacity > 0 {
            if self.ring.len() >= self.capacity {
                self.ring.pop_front();
                self.dropped_records += 1;
            }
            self.ring.push_back(rec);
        } else {
            self.dropped_records += 1;
        }
        id
    }

    /// Stored records, oldest first.
    pub fn records(&self) -> impl Iterator<Item = &DecisionRecord> {
        self.ring.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select;
    use crate::PathStats;

    fn clean(id: u32) -> PathStats {
        PathStats::full(id, 1_000, 300, 1_000, 300, 30_000, 60, 0, 0)
    }
    fn degraded(id: u32) -> PathStats {
        PathStats::full(id, 45_000, 18_000, 40_000, 16_000, 250_000, 60, 0, 0)
    }

    #[test]
    fn record_roundtrips_exactly() {
        let selection = select(&[degraded(2), clean(7)]);
        let rec = DecisionRecord::from_selection(
            4,
            0xAC92,
            PolicyClass::Shadow,
            Some(10_000),
            &selection,
        );
        let line = rec.encode();
        assert!(line.starts_with("GTPDL1|4|AC92|SHADOW|10000|-|7|2|CLEAR_WINNER|"));
        // Line-invariant round trip: the parsed record re-encodes to the
        // exact same line (stronger than struct equality — it proves the
        // serialized form is lossless at its own precision).
        let parsed = DecisionRecord::parse(&line).expect("valid line must parse");
        assert_eq!(parsed.encode(), line, "log lines must replay exactly");
        // And the semantic core survives the trip bit-for-bit.
        assert_eq!(parsed.decision_id, rec.decision_id);
        assert_eq!(parsed.selected_path, rec.selected_path);
        assert_eq!(parsed.reason, rec.reason);
        assert_eq!(parsed.policy_class, rec.policy_class);
    }

    #[test]
    fn tracker_counts_switches_and_holds() {
        let mut t = DecisionTracker::new(8);
        // Decision 1: picks 7.
        let id1 = t.record(
            1,
            PolicyClass::Shadow,
            Some(1_000_000),
            &select(&[clean(7), degraded(2)]),
        );
        assert_eq!(id1, 0);
        assert_eq!(t.switch_count, 0, "no previous path: not a switch");
        // Decision 2: still 7 — no switch.
        t.record(
            1,
            PolicyClass::Shadow,
            Some(2_000_000),
            &select(&[clean(7), degraded(2)]),
        );
        // Decision 3: flips to 2 (degraded wins because 7 went stale).
        let stale7 = PathStats {
            fwd_age_us: Some(4_500_000),
            rev_age_us: Some(4_500_000),
            ..clean(7)
        };
        t.record(
            1,
            PolicyClass::Shadow,
            Some(3_000_000),
            &select(&[degraded(2), stale7]),
        );
        assert_eq!(t.switch_count, 1);
        assert_eq!(t.decisions, 3, "holds record too");
        // The stored record carries previous=7, selected=2.
        let last = t.records().last().unwrap();
        assert_eq!(last.previous_path, Some(7));
        assert_eq!(last.selected_path, Some(2));
        assert_eq!(last.reason, SelectionReason::ClearWinner);
        // Dwell: the switch at window 3_000_000 followed one at... none
        // before, so no dwell yet; flip once more for a dwell reading.
        t.record(
            1,
            PolicyClass::Shadow,
            Some(4_000_000),
            &select(&[clean(7), degraded(2)]),
        );
        assert_eq!(t.switch_count, 2);
        assert_eq!(
            t.last_dwell_us,
            Some(1_000_000),
            "second switch measures dwell"
        );
        // And that flip back to 7 with runner==previous(2)... runner is 2 == prev path? previous was 2 ⇒ revert-shaped.
        assert_eq!(
            t.revert_count, 1,
            "switch straight back to the prior-prior path is a revert verdict"
        );
    }

    #[test]
    fn ring_is_bounded_and_drops_are_counted() {
        let mut t = DecisionTracker::new(2);
        for i in 0..5u32 {
            t.record(9, PolicyClass::Shadow, None, &select(&[clean(i)]));
        }
        assert_eq!(t.decisions, 5);
        assert_eq!(t.dropped_records, 3, "shed the oldest beyond capacity");
        let ids: Vec<u64> = t.records().map(|r| r.decision_id).collect();
        assert_eq!(ids, vec![3, 4], "the newest records survive");
        assert_eq!(t.records().last().unwrap().selected_path, Some(4));
    }

    #[test]
    fn malformed_log_lines_are_rejected_whole() {
        assert_eq!(DecisionRecord::parse(""), None);
        assert_eq!(
            DecisionRecord::parse("GTPDL2|1|1|SHADOW|1|-|1|-|NO_CANDIDATES"),
            None
        );
        assert_eq!(
            DecisionRecord::parse("GTPDL1|x|1|SHADOW|1|-|1|-|NO_CANDIDATES"),
            None
        );
        assert_eq!(
            DecisionRecord::parse("GTPDL1|0|1|BOGUS|1|-|1|-|NO_CANDIDATES"),
            None,
            "unknown policy class rejects the line"
        );
    }
}
