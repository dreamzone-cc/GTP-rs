use gtp_cc::BackpressureLevel;
use gtp_types::Duration;

/// Comprehensive snapshot of all internal protocol diagnostics and runtime telemetry.
#[derive(Clone, Debug, Default)]
pub struct DetailedMetrics {
    // --- RTT & Latency ---
    pub latest_rtt: Duration,
    pub smoothed_rtt: Duration,
    pub rttvar: Duration,
    pub min_rtt: Duration,
    pub pto_duration: Duration,

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
        format!(
            "RTT: {:?} (min {:?}) | CWND: {} KB | Inflight: {} KB | Rate: {} KB/s | Loss: {:.2}% | Backpressure: {:?}",
            self.smoothed_rtt,
            self.min_rtt,
            self.cwnd_bytes / 1024,
            self.inflight_bytes / 1024,
            self.pacing_rate_bps / 1024,
            self.loss_ratio() * 100.0,
            self.backpressure
        )
    }
}
