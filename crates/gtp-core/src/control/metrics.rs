use gtp_cc::BackpressureLevel;
use gtp_types::Duration;

/// Comprehensive snapshot of all internal protocol diagnostics and runtime telemetry.
#[derive(Clone, Debug, Default)]
pub struct DetailedMetrics {
    // --- RTT & Latency ---
    pub latest_rtt: Duration,
    pub smoothed_rtt: Duration,
    pub rttvar: Duration,
    /// N-5: `None` until the first RTT sample is observed — the internal
    /// `u64::MAX` sentinel never leaks into public telemetry.
    pub min_rtt: Option<Duration>,
    pub pto_duration: Duration,
    /// RE-1 (G1): one-way-delay variance above the sliding floor, from the
    /// authenticated header timestamp. `None` until the first authenticated
    /// packet — same no-sentinel discipline as `min_rtt`.
    pub owd_var: Option<Duration>,
    /// RE-1 (G1): RFC 3550 §6.4.1 inter-arrival jitter, µs resolution.
    /// `None` until the first authenticated packet.
    pub jitter: Option<Duration>,

    // --- Congestion Control & Pacing ---
    pub cwnd_bytes: u64,
    pub inflight_bytes: u64,
    pub pacing_rate_bps: u64,
    pub pacing_tokens_remaining: u64,
    pub backpressure: BackpressureLevel,

    // --- Scheduler & Traffic Tiers ---
    pub queue_bytes_per_tier: [usize; 5],
    pub effective_queue_bytes: usize,
    pub total_stale_drops: u64,

    // --- Packet & Byte Counters ---
    pub total_tx_packets: u64,
    pub total_rx_packets: u64,
    pub total_tx_bytes: u64,
    pub total_rx_bytes: u64,
    pub total_retransmissions: u64,
    pub total_corrupted_packets: u64,
    /// RT-2: control events shed by the bounded event queue.
    pub total_dropped_events: u64,
    pub pto_count: u32,

    // --- ECN Signals ---
    pub ecn_ect0_count: u32,
    pub ecn_ect1_count: u32,
    pub ecn_ce_count: u32,
}

impl DetailedMetrics {
    /// Calculated packet loss ratio based on retransmissions vs total transmissions.
    pub fn loss_ratio(&self) -> f64 {
        if self.total_tx_packets == 0 {
            0.0
        } else {
            self.total_retransmissions as f64 / self.total_tx_packets as f64
        }
    }

    /// Formatted human-readable single-line summary for logging.
    pub fn summary_line(&self) -> String {
        // N-5: an unsampled minimum renders as `n/a`, not as 18446744073709s.
        let min_rtt = match self.min_rtt {
            Some(min) => format!("{:?}", min),
            None => "n/a".to_string(),
        };
        format!(
            "RTT: {:?} (min {}) | CWND: {} KB | Inflight: {} KB | Rate: {} KB/s | Loss: {:.2}% | Backpressure: {:?}",
            self.smoothed_rtt,
            min_rtt,
            self.cwnd_bytes / 1024,
            self.inflight_bytes / 1024,
            self.pacing_rate_bps / 1024,
            self.loss_ratio() * 100.0,
            self.backpressure
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N-5: the sentinel must not render in human-facing output.
    #[test]
    fn summary_line_marks_an_unsampled_min_rtt_as_na() {
        let mut metrics = DetailedMetrics {
            smoothed_rtt: Duration::from_millis(100),
            ..DetailedMetrics::default()
        };
        assert!(metrics.min_rtt.is_none());
        assert!(metrics.summary_line().contains("min n/a"));

        metrics.min_rtt = Some(Duration::from_millis(40));
        assert!(!metrics.summary_line().contains("n/a"));
        assert!(metrics.summary_line().contains("min 40.000ms"));
    }
}
