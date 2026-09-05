use gtp_types::{Duration, PriorityTier};

/// Comprehensive protocol configuration parameters for GTP/1.1 connections.
/// Designed for extensibility with backward-compatible defaults and specialized game presets.
#[derive(Clone, Debug)]
pub struct GtpConfig {
    // --- MTU & Framing ---
    pub initial_mtu: usize,
    pub min_mtu: usize,
    pub max_mtu: usize,
    pub mtu_probe_interval: Duration,

    // --- Adaptive ACK & Timing ---
    pub ack_frequency_packets: u8,
    pub max_ack_delay: Duration,
    pub ack_reorder_threshold: u8,
    pub immediate_ack_on_gap: bool,

    // --- Congestion Control & Pacing ---
    pub initial_cwnd_packets: u64,
    pub min_cwnd_packets: u64,
    pub smss: u64,
    pub cubic_beta: f64,
    pub cubic_c: f64,
    pub pacing_gain: f64,
    pub max_pacing_burst_bytes: u64,

    // --- Scheduler & Multi-tier Buffers ---
    pub max_queue_bytes_per_tier: [usize; 5],
    pub tier_weights: [u32; 5],
    pub auto_state_supersession: bool,
    pub enable_deadline_pruning: bool,

    // --- Liveness & Probe Timeout (PTO) ---
    pub idle_timeout: Duration,
    pub keepalive_ping_interval: Duration,
    pub pto_max_duration: Duration,

    // --- Measurement (RE-1, G1) ---
    /// Minimum interval between `ControlEvent::OwdSample` emissions.
    /// Bounded-rate emission: the event queue is unbounded and game traffic
    /// runs at 60–144 Hz, so per-packet events would flood it.
    pub owd_sample_interval: Duration,

    // --- Telemetry hygiene (RT-2) ---
    /// Maximum control events retained before the oldest is dropped.
    /// A runtime that never drains the queue must not accumulate memory
    /// forever; overflow sheds the OLDEST event (newest information
    /// survives) and counts it in `total_dropped_events`. `0` drops all
    /// events. INV-18-safe: this bounds storage, not measurement.
    pub event_queue_capacity: usize,

    // --- Security & Anti-DoS ---
    pub anti_amplification_factor: u64,
    pub stateless_token_lifetime: Duration,
    pub replay_window_size: usize,
    pub key_rotation_interval_packets: u64,
}

impl Default for GtpConfig {
    fn default() -> Self {
        Self::competitive_fps()
    }
}

impl GtpConfig {
    /// Preset: Competitive fast-paced action FPS (128-tick, ultra-low latency, aggressive pruning).
    pub fn competitive_fps() -> Self {
        Self {
            initial_mtu: 1200,
            min_mtu: 1200,
            max_mtu: 1450,
            mtu_probe_interval: Duration::from_secs(10),

            ack_frequency_packets: 1, // Immediate ACKs for instant RTT updates
            max_ack_delay: Duration::from_millis(5),
            ack_reorder_threshold: 1,
            immediate_ack_on_gap: true,

            initial_cwnd_packets: 20,
            min_cwnd_packets: 4,
            smss: 1200,
            // N-6: deliberate preset tuning, now documented — a milder window
            // reduction than RFC 8312's recommended 0.7 keeps the competitive
            // FPS default latency-friendly after single-loss events.
            cubic_beta: 0.75,
            cubic_c: 0.4,
            pacing_gain: 1.25,
            max_pacing_burst_bytes: 24_000,

            max_queue_bytes_per_tier: [
                128 * 1024,  // P0 Control
                512 * 1024,  // P1 Input
                1024 * 1024, // P2 World State
                512 * 1024,  // P3 Reliable Gameplay
                256 * 1024,  // P4 Bulk Cosmetic
            ],
            tier_weights: [15, 40, 30, 10, 5],
            auto_state_supersession: true,
            enable_deadline_pruning: true,

            idle_timeout: Duration::from_secs(15),
            keepalive_ping_interval: Duration::from_secs(1),
            pto_max_duration: Duration::from_millis(500),
            owd_sample_interval: Duration::from_millis(100),
            event_queue_capacity: 1024,

            anti_amplification_factor: 3,
            stateless_token_lifetime: Duration::from_secs(10),
            replay_window_size: 128,
            key_rotation_interval_packets: 1_000_000,
        }
    }

    /// Preset: MMO & Large Scale World (high throughput, multi-client scaling).
    pub fn mmo_world() -> Self {
        Self {
            initial_mtu: 1200,
            min_mtu: 1200,
            max_mtu: 1400,
            mtu_probe_interval: Duration::from_secs(30),

            ack_frequency_packets: 2,
            max_ack_delay: Duration::from_millis(25),
            ack_reorder_threshold: 2,
            immediate_ack_on_gap: true,

            initial_cwnd_packets: 10,
            min_cwnd_packets: 2,
            smss: 1200,
            cubic_beta: 0.70,
            cubic_c: 0.4,
            pacing_gain: 1.20,
            max_pacing_burst_bytes: 12_000,

            max_queue_bytes_per_tier: [64 * 1024, 256 * 1024, 1024 * 1024, 1024 * 1024, 512 * 1024],
            tier_weights: [15, 30, 30, 20, 5],
            auto_state_supersession: true,
            enable_deadline_pruning: true,

            idle_timeout: Duration::from_secs(30),
            keepalive_ping_interval: Duration::from_secs(3),
            pto_max_duration: Duration::from_secs(2),
            owd_sample_interval: Duration::from_millis(100),
            event_queue_capacity: 1024,

            anti_amplification_factor: 3,
            stateless_token_lifetime: Duration::from_secs(10),
            replay_window_size: 128,
            key_rotation_interval_packets: 2_000_000,
        }
    }

    /// Preset: Resilient Mobile / Wireless (handles high packet loss, jitter, and frequent NAT shifts).
    pub fn mobile_wireless() -> Self {
        Self {
            initial_mtu: 1200,
            min_mtu: 1200,
            max_mtu: 1350,
            mtu_probe_interval: Duration::from_secs(15),

            ack_frequency_packets: 1,
            max_ack_delay: Duration::from_millis(15),
            ack_reorder_threshold: 3,
            immediate_ack_on_gap: true,

            initial_cwnd_packets: 8,
            min_cwnd_packets: 2,
            smss: 1200,
            cubic_beta: 0.65,
            cubic_c: 0.35,
            pacing_gain: 1.15,
            max_pacing_burst_bytes: 8_000,

            max_queue_bytes_per_tier: [64 * 1024, 256 * 1024, 512 * 1024, 512 * 1024, 128 * 1024],
            tier_weights: [20, 40, 25, 10, 5],
            auto_state_supersession: true,
            enable_deadline_pruning: true,

            idle_timeout: Duration::from_secs(20),
            keepalive_ping_interval: Duration::from_secs(2),
            pto_max_duration: Duration::from_secs(1),
            owd_sample_interval: Duration::from_millis(100),
            event_queue_capacity: 1024,

            anti_amplification_factor: 3,
            stateless_token_lifetime: Duration::from_secs(15),
            replay_window_size: 128,
            key_rotation_interval_packets: 500_000,
        }
    }

    /// Preset: Local LAN / Dedicated Server cluster testing (maximum throughput, near-zero RTT).
    pub fn lan_cluster() -> Self {
        Self {
            initial_mtu: 1450,
            min_mtu: 1200,
            max_mtu: 1450,
            mtu_probe_interval: Duration::from_secs(60),

            ack_frequency_packets: 8,
            max_ack_delay: Duration::from_millis(2),
            ack_reorder_threshold: 1,
            immediate_ack_on_gap: true,

            initial_cwnd_packets: 50,
            min_cwnd_packets: 10,
            smss: 1450,
            cubic_beta: 0.85,
            cubic_c: 0.5,
            pacing_gain: 1.5,
            max_pacing_burst_bytes: 100_000,

            max_queue_bytes_per_tier: [
                256 * 1024,
                1024 * 1024,
                2048 * 1024,
                2048 * 1024,
                1024 * 1024,
            ],
            tier_weights: [10, 35, 35, 15, 5],
            auto_state_supersession: true,
            enable_deadline_pruning: true,

            idle_timeout: Duration::from_secs(60),
            keepalive_ping_interval: Duration::from_secs(5),
            pto_max_duration: Duration::from_millis(100),
            owd_sample_interval: Duration::from_millis(100),
            event_queue_capacity: 1024,

            anti_amplification_factor: 10,
            stateless_token_lifetime: Duration::from_secs(30),
            replay_window_size: 256,
            key_rotation_interval_packets: 10_000_000,
        }
    }
}

/// Fluent builder for constructing customized `GtpConfig` profiles.
#[derive(Clone, Debug, Default)]
pub struct GtpConfigBuilder {
    config: GtpConfig,
}

impl GtpConfigBuilder {
    pub fn new() -> Self {
        Self {
            config: GtpConfig::default(),
        }
    }

    pub fn with_preset(mut self, preset: GtpConfig) -> Self {
        self.config = preset;
        self
    }

    pub fn ack_frequency(mut self, packets: u8, max_delay: Duration) -> Self {
        self.config.ack_frequency_packets = packets;
        self.config.max_ack_delay = max_delay;
        self
    }

    pub fn initial_cwnd_packets(mut self, packets: u64) -> Self {
        self.config.initial_cwnd_packets = packets;
        self
    }

    pub fn tier_capacity(mut self, tier: PriorityTier, max_bytes: usize) -> Self {
        self.config.max_queue_bytes_per_tier[tier as usize] = max_bytes;
        self
    }

    pub fn tier_weight(mut self, tier: PriorityTier, weight: u32) -> Self {
        self.config.tier_weights[tier as usize] = weight;
        self
    }

    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.config.idle_timeout = timeout;
        self
    }

    /// RT-2: bound the retained control events (drop-oldest on overflow).
    pub fn event_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.event_queue_capacity = capacity;
        self
    }

    pub fn build(self) -> GtpConfig {
        self.config
    }
}
