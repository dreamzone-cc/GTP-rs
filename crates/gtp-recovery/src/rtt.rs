use gtp_types::Duration;

pub const INITIAL_RTT: Duration = Duration::from_millis(100);
pub const DEFAULT_MAX_ACK_DELAY: Duration = Duration::from_millis(25);

/// RFC-aligned RTT estimation statistics.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct RttStats {
    pub latest_rtt: Duration,
    pub smoothed_rtt: Duration,
    pub rttvar: Duration,
    pub min_rtt: Duration,
    pub max_ack_delay: Duration,
    first_sample: bool,
}

impl Default for RttStats {
    fn default() -> Self {
        Self {
            latest_rtt: INITIAL_RTT,
            smoothed_rtt: INITIAL_RTT,
            rttvar: Duration::from_micros(INITIAL_RTT.as_micros() / 2),
            min_rtt: Duration::from_micros(u64::MAX),
            max_ack_delay: DEFAULT_MAX_ACK_DELAY,
            first_sample: true,
        }
    }
}

impl RttStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// N-5: the observed minimum RTT, or `None` before the first sample.
    ///
    /// Internally `min_rtt` holds a `u64::MAX` sentinel until the first
    /// `update`. That sentinel must never leak into metrics (it formats as
    /// `18446744073709s`) or into backpressure math (it zeroes the
    /// RTT-inflation axis). Consumers that need a plain value should fall
    /// back to `INITIAL_RTT`, never to the sentinel.
    pub fn min_rtt_sample(&self) -> Option<Duration> {
        if self.first_sample {
            None
        } else {
            Some(self.min_rtt)
        }
    }

    /// Resets the estimator when the connection migrates to a **validated** new path
    /// (RFC 9000 §9.4, X-1).
    ///
    /// Samples taken on the old path describe the old path. `min_rtt` in particular
    /// only ever decreases, so without this reset the *old* path's floor keeps
    /// defining `rtt_inflation = smoothed_rtt / min_rtt` for the rest of the session:
    /// after migrating from a 10ms path to a 60ms one the inflation reads 6x forever
    /// and the engine is told to shed level-of-detail on the path it just chose as
    /// better. Silent, permanent, and it punishes a successful migration.
    ///
    /// `max_ack_delay` is a property of the **peer**, not of the path, so it survives
    /// the reset; everything else returns to its initial value and the first sample on
    /// the new path re-seeds the estimator.
    ///
    /// **The `INITIAL_RTT` window is deliberate.** Between the migration and the first
    /// acknowledgement covering a packet actually sent on the new path, `smoothed_rtt`
    /// reads 100 ms, and `CubicCongestionController::pacing_rate` divides the congestion
    /// window by it — so the pacing rate is understated for that window. Three things
    /// bound it: it lasts one round trip plus one send interval; `first_sample` is set
    /// here, so the first new sample *replaces* the estimate outright instead of being
    /// blended into it; and RFC 9000 §9.4 asks for exactly this reset (it asks for the
    /// congestion window too, which is not reset here — leaving the pacing rate more
    /// permissive during the window than full conformance would be, not less).
    /// Measured against a live 50 ms path carrying ~6 KB/s, the understated rate is
    /// still ~1.7 MB/s: real, bounded, and far above the offered load. Keeping the old
    /// path's `smoothed_rtt` instead would close the window but compute PTO from a stale
    /// low RTT right after migrating to a slower path, which is the worse trade.
    pub fn reset_for_new_path(&mut self) {
        let max_ack_delay = self.max_ack_delay;
        *self = Self {
            max_ack_delay,
            ..Self::default()
        };
    }

    pub fn update(&mut self, send_to_ack_duration: Duration, ack_delay: Duration) {
        // RFC 9002 §5.2: min_rtt is tracked from the *unadjusted* latest sample so a
        // peer over-reporting ack_delay cannot drag min_rtt (and every derived
        // quantity) toward zero.
        let raw_rtt = send_to_ack_duration;
        self.min_rtt = self.min_rtt.min(raw_rtt);

        // RFC 9002 §5.3: adjust for ack_delay only when the sample exceeds
        // min_rtt + ack_delay, and never on the first sample (no min_rtt reference yet).
        let adjusted_rtt = if self.first_sample {
            raw_rtt
        } else {
            let ack_delay_clamped = ack_delay.min(self.max_ack_delay);
            if raw_rtt > self.min_rtt + ack_delay_clamped {
                raw_rtt - ack_delay_clamped
            } else {
                raw_rtt
            }
        };

        self.latest_rtt = adjusted_rtt;

        if self.first_sample {
            self.first_sample = false;
            self.smoothed_rtt = adjusted_rtt;
            self.rttvar = Duration::from_micros(adjusted_rtt.as_micros() / 2);
        } else {
            let rtt_diff = if self.smoothed_rtt >= adjusted_rtt {
                self.smoothed_rtt - adjusted_rtt
            } else {
                adjusted_rtt - self.smoothed_rtt
            };

            // rttvar = (3/4)*rttvar + (1/4)*|smoothed_rtt - adjusted_rtt|
            self.rttvar =
                Duration::from_micros((self.rttvar.as_micros() * 3 + rtt_diff.as_micros()) / 4);

            // smoothed_rtt = (7/8)*smoothed_rtt + (1/8)*adjusted_rtt
            self.smoothed_rtt = Duration::from_micros(
                (self.smoothed_rtt.as_micros() * 7 + adjusted_rtt.as_micros()) / 8,
            );
        }
    }

    pub fn pto_duration(&self) -> Duration {
        let pto = self.smoothed_rtt
            + Duration::from_micros(self.rttvar.as_micros() * 4)
            + self.max_ack_delay;
        // RFC 9002 §6.2.1: PTO must never be smaller than the timer granularity.
        pto.max(Duration::from_millis(1))
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn reset_for_new_path_drops_old_path_samples_but_keeps_the_peer_property() {
        let mut stats = RttStats::new();
        stats.max_ack_delay = Duration::from_millis(11);
        for _ in 0..8 {
            stats.update(Duration::from_millis(10), Duration::from_micros(0));
        }
        assert_eq!(stats.min_rtt, Duration::from_millis(10));

        stats.reset_for_new_path();

        // max_ack_delay is negotiated with the peer, not measured on the path.
        assert_eq!(stats.max_ack_delay, Duration::from_millis(11));
        // The old path's floor is gone: the next sample re-seeds the estimator.
        assert_eq!(stats.min_rtt, RttStats::new().min_rtt);
        stats.update(Duration::from_millis(60), Duration::from_micros(0));
        assert_eq!(stats.min_rtt, Duration::from_millis(60));
        assert_eq!(stats.smoothed_rtt, Duration::from_millis(60));
    }
    use super::*;

    #[test]
    fn test_rtt_stats_update() {
        let mut stats = RttStats::new();
        assert!(stats.first_sample);

        // First sample: 50ms — RFC 9002: taken unadjusted (no min_rtt reference yet)
        stats.update(Duration::from_millis(50), Duration::from_millis(5));
        assert_eq!(stats.latest_rtt, Duration::from_millis(50));
        assert_eq!(stats.smoothed_rtt, Duration::from_millis(50));
        assert_eq!(stats.min_rtt, Duration::from_millis(50));
        assert_eq!(stats.rttvar, Duration::from_micros(25_000));

        // Second sample: 60ms raw with 5ms ack_delay: 60 > min_rtt(50) + 5 -> adjusted 55ms
        stats.update(Duration::from_millis(60), Duration::from_millis(5));
        assert_eq!(stats.latest_rtt, Duration::from_millis(55));
        assert!(stats.smoothed_rtt > Duration::from_millis(50));
        // min_rtt tracks the RAW sample, not the ack-delay-adjusted one
        assert_eq!(stats.min_rtt, Duration::from_millis(50));

        // Peer claiming an impossibly large ack_delay cannot drag values below min_rtt
        stats.update(Duration::from_millis(52), Duration::from_millis(10_000));
        assert_eq!(stats.latest_rtt, Duration::from_millis(52));
        assert_eq!(stats.min_rtt, Duration::from_millis(50));

        // PTO has a granularity floor even with a zero-ish RTT
        let mut fast = RttStats::new();
        fast.update(Duration::from_micros(1), Duration::ZERO);
        assert!(fast.pto_duration() >= Duration::from_millis(1));
    }
}
