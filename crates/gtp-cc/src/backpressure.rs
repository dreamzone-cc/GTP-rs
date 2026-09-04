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

/// N-5: `min_rtt` is optional — `None` means no RTT sample has been observed
/// yet. Without a reference floor the RTT-inflation axis is neutral (1.0) and
/// only the queue ratio can raise the level; the old behaviour divided by a
/// `u64::MAX` sentinel, computing an inflation of ~0 that read as artificially
/// healthy instead of unknown.
pub fn calculate_backpressure(
    queued_bytes: usize,
    cwnd: u64,
    current_rtt: Duration,
    min_rtt: Option<Duration>,
) -> BackpressureLevel {
    let rtt_inflation = match min_rtt {
        Some(min) if min.as_micros() > 0 => current_rtt.as_micros() as f64 / min.as_micros() as f64,
        _ => 1.0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_min_rtt_sample_leaves_the_rtt_axis_neutral() {
        // Unknown floor must read as neutral, not as artificially healthy
        // (the old u64::MAX sentinel produced inflation ≈ 0).
        assert_eq!(
            calculate_backpressure(0, 10_000, Duration::from_millis(100), None),
            BackpressureLevel::Low
        );
        // The queue axis still works with no RTT reference.
        assert_eq!(
            calculate_backpressure(40_000, 10_000, Duration::from_millis(100), None),
            BackpressureLevel::Critical
        );
        // A real 1.5x inflation with the same inputs raises the level.
        assert_eq!(
            calculate_backpressure(
                0,
                10_000,
                Duration::from_millis(60),
                Some(Duration::from_millis(40))
            ),
            BackpressureLevel::Medium
        );
    }
}
