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
