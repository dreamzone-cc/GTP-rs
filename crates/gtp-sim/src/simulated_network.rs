use std::collections::BinaryHeap;
use std::net::SocketAddr;
use crate::impairments::NetworkProfile;
use gtp_types::{Duration, MonotonicTime};

#[derive(Clone, Debug)]
pub struct SimulatedPacket {
    pub deliver_at: MonotonicTime,
    pub src: SocketAddr,
    pub dest: SocketAddr,
    pub data: Vec<u8>,
}

impl Ord for SimulatedPacket {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse for min-heap by deliver_at
        other.deliver_at.cmp(&self.deliver_at)
    }
}

impl PartialOrd for SimulatedPacket {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SimulatedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.deliver_at == other.deliver_at
    }
}

impl Eq for SimulatedPacket {}

/// Deterministic simulated network pipe.
pub struct SimulatedNetwork {
    queue: BinaryHeap<SimulatedPacket>,
    rng_state: u64,
}

impl SimulatedNetwork {
    pub fn new(seed: u64) -> Self {
        Self {
            queue: BinaryHeap::new(),
            rng_state: seed.max(1),
        }
    }

    /// Fast deterministic PRNG (XorShift64)
    fn next_random_f64(&mut self) -> f64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        (x as f64) / (u64::MAX as f64)
    }

    pub fn transmit(
        &mut self,
        src: SocketAddr,
        dest: SocketAddr,
        data: Vec<u8>,
        now: MonotonicTime,
        profile: &NetworkProfile,
    ) {
        // 1. Packet Loss Check
        if self.next_random_f64() < profile.loss_rate {
            return; // Dropped
        }

        // 2. Jitter calculation
        let jitter_factor = (self.next_random_f64() * 2.0) - 1.0; // [-1.0, 1.0]
        let jitter_micros = (profile.jitter.as_micros() as f64 * jitter_factor) as i64;
        let base_delay_micros = profile.one_way_delay.as_micros() as i64;
        let effective_delay = (base_delay_micros + jitter_micros).max(100) as u64;

        // 3. Serialization delay based on bandwidth
        let serialization_micros = ((data.len() as f64 / profile.bandwidth_bytes_per_sec as f64)
            * 1_000_000.0) as u64;

        let total_delay = Duration::from_micros(effective_delay + serialization_micros);
        let deliver_at = now + total_delay;

        // 4. Reorder Check
        let final_deliver_at = if self.next_random_f64() < profile.reorder_rate {
            deliver_at + Duration::from_millis(15)
        } else {
            deliver_at
        };

        self.queue.push(SimulatedPacket {
            deliver_at: final_deliver_at,
            src,
            dest,
            data: data.clone(),
        });

        // 5. Duplication Check
        if self.next_random_f64() < profile.duplicate_rate {
            self.queue.push(SimulatedPacket {
                deliver_at: final_deliver_at + Duration::from_micros(500),
                src,
                dest,
                data,
            });
        }
    }

    pub fn drain_ready(&mut self, now: MonotonicTime) -> Vec<SimulatedPacket> {
        let mut ready = Vec::new();
        while let Some(top) = self.queue.peek() {
            if top.deliver_at <= now {
                ready.push(self.queue.pop().unwrap());
            } else {
                break;
            }
        }
        ready
    }
}
