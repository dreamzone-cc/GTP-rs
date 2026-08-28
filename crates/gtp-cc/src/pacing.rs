use gtp_types::{Duration, MonotonicTime};

/// Token-bucket based pacing engine for smooth inter-packet transmit timing.
#[derive(Clone, Debug)]
pub struct PacingEngine {
    tokens_bytes: u64,
    last_update_time: MonotonicTime,
    max_burst_bytes: u64,
}

impl Default for PacingEngine {
    fn default() -> Self {
        Self::new(12_000)
    }
}

impl PacingEngine {
    pub fn new(max_burst_bytes: u64) -> Self {
        Self {
            tokens_bytes: max_burst_bytes,
            last_update_time: MonotonicTime::ZERO,
            max_burst_bytes,
        }
    }

    pub fn update_tokens(&mut self, pacing_rate_bps: u64, now: MonotonicTime) {
        if self.last_update_time == MonotonicTime::ZERO {
            self.last_update_time = now;
            self.tokens_bytes = self.max_burst_bytes;
            return;
        }

        let elapsed = now.duration_since(self.last_update_time);
        if elapsed > Duration::ZERO {
            let added_tokens = (pacing_rate_bps as f64 * elapsed.as_secs_f64()) as u64;
            self.tokens_bytes = (self.tokens_bytes + added_tokens).min(self.max_burst_bytes);
            self.last_update_time = now;
        }
    }

    pub fn can_send(&self, bytes: usize, cwnd: u64, inflight: u64) -> bool {
        let cwnd_allows = inflight + (bytes as u64) <= cwnd;
        let tokens_allow = self.tokens_bytes >= (bytes as u64);
        cwnd_allows && tokens_allow
    }

    pub fn consume(&mut self, bytes: usize) {
        self.tokens_bytes = self.tokens_bytes.saturating_sub(bytes as u64);
    }

    pub fn send_budget(&self, cwnd: u64, inflight: u64) -> usize {
        let cwnd_budget = cwnd.saturating_sub(inflight);
        let pacing_budget = self.tokens_bytes;
        cwnd_budget.min(pacing_budget) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pacing_tokens_accumulation_and_consumption() {
        let mut pacing = PacingEngine::new(2400);
        let now = MonotonicTime::from_micros(1_000_000);

        pacing.update_tokens(100_000, now); // 100 KB/s
        assert!(pacing.can_send(1200, 10_000, 0));

        pacing.consume(1200);
        assert_eq!(pacing.tokens_bytes, 1200);

        pacing.consume(1200);
        assert_eq!(pacing.tokens_bytes, 0);
        assert!(!pacing.can_send(1200, 10_000, 0)); // No tokens remaining

        // Advance time by 20ms -> adds 100,000 * 0.02 = 2000 tokens
        let now_plus_20ms = now + Duration::from_millis(20);
        pacing.update_tokens(100_000, now_plus_20ms);
        assert!(pacing.tokens_bytes >= 2000);
        assert!(pacing.can_send(1200, 10_000, 0));
    }
}
