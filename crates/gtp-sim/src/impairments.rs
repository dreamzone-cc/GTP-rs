use gtp_types::Duration;

/// Configuration defining artificial network impairments for deterministic simulation.
#[derive(Clone, Debug)]
pub struct NetworkProfile {
    pub one_way_delay: Duration,
    pub jitter: Duration,
    pub loss_rate: f64,        // 0.0 to 1.0
    pub duplicate_rate: f64,   // 0.0 to 1.0
    pub reorder_rate: f64,     // 0.0 to 1.0
    pub bandwidth_bytes_per_sec: u64,
}

impl Default for NetworkProfile {
    fn default() -> Self {
        Self::lan()
    }
}

impl NetworkProfile {
    pub fn lan() -> Self {
        Self {
            one_way_delay: Duration::from_millis(1),
            jitter: Duration::from_micros(200),
            loss_rate: 0.0,
            duplicate_rate: 0.0,
            reorder_rate: 0.0,
            bandwidth_bytes_per_sec: 100_000_000, // 100 MB/s
        }
    }

    pub fn good_internet() -> Self {
        Self {
            one_way_delay: Duration::from_millis(20),
            jitter: Duration::from_millis(2),
            loss_rate: 0.005, // 0.5% loss
            duplicate_rate: 0.001,
            reorder_rate: 0.001,
            bandwidth_bytes_per_sec: 10_000_000,
        }
    }

    pub fn bad_cellular_wifi() -> Self {
        Self {
            one_way_delay: Duration::from_millis(60),
            jitter: Duration::from_millis(25),
            loss_rate: 0.08, // 8% loss
            duplicate_rate: 0.02,
            reorder_rate: 0.05,
            bandwidth_bytes_per_sec: 1_000_000,
        }
    }

    pub fn extreme_loss() -> Self {
        Self {
            one_way_delay: Duration::from_millis(40),
            jitter: Duration::from_millis(10),
            loss_rate: 0.20, // 20% loss
            duplicate_rate: 0.05,
            reorder_rate: 0.10,
            bandwidth_bytes_per_sec: 500_000,
        }
    }
}
