//! Compact cross-endpoint measurement report (app-layer, no wire change).
//!
//! ARDP §2.3: what is genuinely missing rides as `ReliableOrdered`
//! application messages — no new control protocol. A `MeasurementReport` is
//! one endpoint telling the other what ITS receiver measured (the direction
//! the receiver observes), encoded as a single ASCII line:

use crate::PathStats;

/// Line prefix identifying the report format (version 1).
pub const REPORT_PREFIX: &str = "GTPRP1";

/// The wire group id used by the CLI exchange for reports.
pub const REPORT_GROUP_ID: u16 = 0x5250;

/// One endpoint's measured aggregates for the direction it receives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeasurementReport {
    /// One-way delay variance above floor, µs (`None` before first sample).
    pub owd_var_us: Option<u32>,
    /// RFC 3550 inter-arrival jitter, µs (`None` before first sample).
    pub jitter_us: Option<u32>,
    /// The reporter's smoothed RTT, µs.
    pub srtt_us: Option<u32>,
    /// Measurement basis: packets RECEIVED by the reporter this session —
    /// the estimator is fed per authenticated packet, so this is its honest
    /// sample count (the rate-limited OwdSample event stream is telemetry,
    /// not the basis).
    pub samples: u32,
}

fn field(v: Option<u32>) -> String {
    match v {
        Some(v) => v.to_string(),
        None => "-".to_string(),
    }
}

impl MeasurementReport {
    /// Encode as `GTPRP1|var|jitter|srtt|samples` (`-` = not yet measured).
    pub fn encode(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            REPORT_PREFIX,
            field(self.owd_var_us),
            field(self.jitter_us),
            field(self.srtt_us),
            self.samples
        )
    }

    /// Strict parse: wrong prefix, wrong arity, or a malformed number yields
    /// `None` — a report that cannot be trusted is not half-accepted.
    pub fn parse(line: &str) -> Option<Self> {
        let rest = line.strip_prefix(REPORT_PREFIX)?.strip_prefix('|')?;
        let mut parts = rest.split('|');
        let next = |parts: &mut std::str::Split<'_, char>| -> Option<Option<u32>> {
            match parts.next()? {
                "-" => Some(None),
                // A malformed number rejects the WHOLE line: a report that
                // cannot be trusted is not half-accepted as "not measured".
                s => s.parse::<u32>().ok().map(Some),
            }
        };
        let owd_var_us = next(&mut parts)?;
        let jitter_us = next(&mut parts)?;
        let srtt_us = next(&mut parts)?;
        let samples = parts.next()?.parse::<u32>().ok()?;
        if parts.next().is_some() {
            return None; // trailing fields: not our format
        }
        Some(Self {
            owd_var_us,
            jitter_us,
            srtt_us,
            samples,
        })
    }

    /// Merge this report (the far end's observation of the **forward**
    /// direction) with the local end's own reverse-direction aggregates into
    /// one [`PathStats`] — the bidirectional picture neither side holds
    /// alone. `path_id` is assigned by the caller. The merged basis is the
    /// smaller of the two per-packet counts (both sides are per-packet
    /// units; never mix in the rate-limited event stream).
    pub fn into_path_stats(self, path_id: u32, rev: &PathStats) -> PathStats {
        PathStats {
            path_id,
            fwd_owd_var_us: self.owd_var_us,
            fwd_jitter_us: self.jitter_us,
            rev_owd_var_us: rev.rev_owd_var_us,
            rev_jitter_us: rev.rev_jitter_us,
            // RTT from either end is the same quantity; prefer the local one.
            rtt_us: rev.rtt_us.or(self.srtt_us),
            sample_count: self.samples.min(rev.sample_count.max(1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_roundtrips_exactly() {
        let r = MeasurementReport {
            owd_var_us: Some(4_032),
            jitter_us: Some(537),
            srtt_us: Some(51_379),
            samples: 812,
        };
        assert_eq!(r.encode(), "GTPRP1|4032|537|51379|812");
        assert_eq!(MeasurementReport::parse(&r.encode()), Some(r));

        // Pre-measurement report: every axis absent, basis zero.
        let empty = MeasurementReport::default();
        assert_eq!(empty.encode(), "GTPRP1|-|-|-|0");
        assert_eq!(MeasurementReport::parse(&empty.encode()), Some(empty));
    }

    #[test]
    fn malformed_reports_are_rejected_whole() {
        assert_eq!(
            MeasurementReport::parse("GTPRP1|1|2|3"),
            None,
            "missing field"
        );
        assert_eq!(
            MeasurementReport::parse("GTPRP1|1|2|3|4|5"),
            None,
            "trailing junk"
        );
        assert_eq!(
            MeasurementReport::parse("GTPRP2|1|2|3|4"),
            None,
            "wrong version"
        );
        assert_eq!(
            MeasurementReport::parse("GTPRP1|x|2|3|4"),
            None,
            "bad number"
        );
        assert_eq!(MeasurementReport::parse(""), None);
        // "-"/None mapping is unambiguous in both directions.
        let r = MeasurementReport::parse("GTPRP1|-|5|-|9").unwrap();
        assert_eq!(r.jitter_us, Some(5));
        assert_eq!(r.owd_var_us, None);
        assert_eq!(r.srtt_us, None);
        assert_eq!(r.samples, 9);
    }

    #[test]
    fn merging_builds_the_bidirectional_picture() {
        let report = MeasurementReport {
            owd_var_us: Some(2_000),
            jitter_us: Some(400),
            srtt_us: Some(50_000),
            samples: 100,
        };
        let local = PathStats {
            rev_owd_var_us: Some(1_500),
            rev_jitter_us: Some(300),
            rtt_us: Some(49_000),
            sample_count: 90,
            ..Default::default()
        };
        let stats = report.into_path_stats(7, &local);
        assert_eq!(stats.fwd_owd_var_us, Some(2_000));
        assert_eq!(stats.rev_jitter_us, Some(300));
        assert_eq!(stats.rtt_us, Some(49_000), "local RTT wins when present");
        assert_eq!(stats.sample_count, 90, "conservative basis: smaller side");
    }
}
