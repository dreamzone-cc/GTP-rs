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
        let ack_delay_clamped = ack_delay.min(self.max_ack_delay);
        let adjusted_rtt = if send_to_ack_duration >= ack_delay_clamped {
            send_to_ack_duration - ack_delay_clamped
        } else {
            send_to_ack_duration
        };

        self.latest_rtt = adjusted_rtt;
        self.min_rtt = self.min_rtt.min(adjusted_rtt);

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
        self.smoothed_rtt + Duration::from_micros(self.rttvar.as_micros() * 4) + self.max_ack_delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtt_stats_update() {
        let mut stats = RttStats::new();
        assert!(stats.first_sample);

        // First sample: 50ms with 5ms ack_delay -> adjusted = 45ms
        stats.update(Duration::from_millis(50), Duration::from_millis(5));
        assert_eq!(stats.latest_rtt, Duration::from_millis(45));
        assert_eq!(stats.smoothed_rtt, Duration::from_millis(45));
        assert_eq!(stats.min_rtt, Duration::from_millis(45));
        assert_eq!(stats.rttvar, Duration::from_micros(22_500));

        // Second sample: 60ms with 5ms ack_delay -> adjusted = 55ms
        stats.update(Duration::from_millis(60), Duration::from_millis(5));
        assert_eq!(stats.latest_rtt, Duration::from_millis(55));
        assert!(stats.smoothed_rtt > Duration::from_millis(45));
        assert_eq!(stats.min_rtt, Duration::from_millis(45));
    }
}
