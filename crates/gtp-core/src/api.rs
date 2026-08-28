use gtp_cc::BackpressureLevel;
use gtp_types::{Duration, MessageClass};

/// High-level message delivered to the game application.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct ReceivedMessage {
    pub class: MessageClass,
    pub payload: Vec<u8>,
}

/// Telemetry and adaptation feedback exposed to the game engine.
#[derive(Copy, Clone, Debug)]
pub struct NetworkFeedback {
    pub rtt: Duration,
    pub smoothed_rtt: Duration,
    pub min_rtt: Duration,
    pub cwnd_bytes: u64,
    pub inflight_bytes: u64,
    pub pacing_rate_bps: u64,
    pub backpressure: BackpressureLevel,
    pub effective_queue_bytes: usize,
}
