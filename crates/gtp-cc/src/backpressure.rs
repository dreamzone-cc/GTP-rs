use gtp_types::Duration;

/// Congestion-induced backpressure state signaled to the game engine.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Default)]
pub enum BackpressureLevel {
    /// Normal operation: schedule all message tiers normally.
    #[default]
    Low = 0,
    /// Mild congestion: accelerate expiration of obsolete state snapshots.
    Medium = 1,
    /// High congestion: aggressively drop stale world state and cosmetic P4 traffic.
    High = 2,
    /// Critical: only admit P0 Control, P1 Input, and critical reliable gameplay events.
    Critical = 3,
}

pub fn calculate_backpressure(
    queued_bytes: usize,
    cwnd: u64,
    current_rtt: Duration,
    min_rtt: Duration,
) -> BackpressureLevel {
    let rtt_inflation = if min_rtt.as_micros() > 0 {
        current_rtt.as_micros() as f64 / min_rtt.as_micros() as f64
    } else {
        1.0
    };

    let queue_ratio = (queued_bytes as f64) / (cwnd.max(1200) as f64);

    if queue_ratio > 3.0 || rtt_inflation > 2.5 {
        BackpressureLevel::Critical
    } else if queue_ratio > 1.5 || rtt_inflation > 1.8 {
        BackpressureLevel::High
    } else if queue_ratio > 0.8 || rtt_inflation > 1.3 {
        BackpressureLevel::Medium
    } else {
        BackpressureLevel::Low
    }
}
