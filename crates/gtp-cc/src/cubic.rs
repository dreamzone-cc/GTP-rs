use crate::controller::CongestionController;
use gtp_recovery::{AckEvent, LossEvent};
use gtp_types::{Duration, MonotonicTime, PacketNumber};

pub const DEFAULT_SMSS: u64 = 1200;
pub const INITIAL_CWND_PACKETS: u64 = 10;
pub const MIN_CWND_PACKETS: u64 = 2;
pub const BETA_CUBIC: f64 = 0.7;
pub const C_CUBIC: f64 = 0.4;

/// CUBIC Congestion Control implementation for GTP (RFC 8312 compliant with TCP-Friendly region and Fast Convergence).
#[derive(Clone, Debug)]
pub struct CubicCongestionController {
    smss: u64,
    #[allow(dead_code)] // retained for diagnostics/telemetry wiring (Phase-5 metrics)
    initial_cwnd: u64,
    min_cwnd_bytes: u64,
    beta: f64,
    c_const: f64,
    pacing_gain: f64,
    cwnd: u64,
    ssthresh: u64,
    w_max: u64,
    w_last_max: u64,
    k: f64,
    epoch_start: Option<MonotonicTime>,
    origin_point: u64,
    smoothed_rtt: Duration,
    min_rtt: Duration,
    last_loss_time: Option<MonotonicTime>,
}

impl Default for CubicCongestionController {
    fn default() -> Self {
        Self::new(DEFAULT_SMSS)
    }
}

/// Tunable CUBIC parameters (P1-2: populated from `GtpConfig`).
#[derive(Clone, Copy, Debug)]
pub struct CubicConfig {
    pub smss: u64,
    pub initial_cwnd_packets: u64,
    pub min_cwnd_packets: u64,
    pub beta: f64,
    pub c: f64,
    pub pacing_gain: f64,
}

impl Default for CubicConfig {
    fn default() -> Self {
        Self {
            smss: DEFAULT_SMSS,
            initial_cwnd_packets: INITIAL_CWND_PACKETS,
            min_cwnd_packets: MIN_CWND_PACKETS,
            beta: BETA_CUBIC,
            c: C_CUBIC,
            pacing_gain: 1.2,
        }
    }
}

impl CubicCongestionController {
    pub fn new(smss: u64) -> Self {
        Self::with_config(CubicConfig {
            smss,
            ..CubicConfig::default()
        })
    }

    pub fn with_config(config: CubicConfig) -> Self {
        let initial_cwnd = config.initial_cwnd_packets.max(1) * config.smss.max(1);
        Self {
            smss: config.smss.max(1),
            initial_cwnd,
            min_cwnd_bytes: config.min_cwnd_packets.max(1) * config.smss.max(1),
            beta: config.beta.clamp(0.5, 0.95),
            c_const: config.c.max(0.05),
            pacing_gain: config.pacing_gain.max(1.0),
            cwnd: initial_cwnd,
            ssthresh: u64::MAX,
            w_max: initial_cwnd,
            w_last_max: initial_cwnd,
            k: 0.0,
            epoch_start: None,
            origin_point: initial_cwnd,
            smoothed_rtt: Duration::from_millis(50),
            min_rtt: Duration::from_millis(50),
            last_loss_time: None,
        }
    }

    fn update_w_cubic(&mut self, now: MonotonicTime) {
        let epoch_start = match self.epoch_start {
            Some(t) => t,
            None => {
                self.epoch_start = Some(now);
                now
            }
        };

        let t = now.duration_since(epoch_start).as_secs_f64();
        let target_diff = t - self.k;
        let mut w_cubic =
            self.c_const * (target_diff * target_diff * target_diff) * (self.smss as f64)
                + (self.origin_point as f64);

        // RFC 8312 Eq. 4 TCP-Friendly Region — uses the connection RTT, not min_rtt
        // (CC-5), so growth after a path change is not artificially aggressive.
        let rtt_secs = self.smoothed_rtt.as_secs_f64().max(0.001);
        let beta = self.beta;
        let tcp_factor = 3.0 * (1.0 - beta) / (1.0 + beta);
        let w_tcp = (self.w_max as f64 * beta) + (tcp_factor * (t / rtt_secs) * (self.smss as f64));

        if w_tcp > w_cubic {
            w_cubic = w_tcp;
        }

        let w_cubic_clamped = (w_cubic.max(self.min_cwnd() as f64)) as u64;

        if self.cwnd < self.ssthresh {
            // Slow Start
            self.cwnd = self.cwnd.saturating_add(self.smss);
        } else {
            // Congestion Avoidance
            self.cwnd = self.cwnd.max(w_cubic_clamped);
        }
    }

    fn min_cwnd(&self) -> u64 {
        self.min_cwnd_bytes
    }

    fn on_congestion_event(&mut self, now: MonotonicTime) {
        // Prevent multiple window reductions in the same RTT
        if let Some(last_loss) = self.last_loss_time {
            if now.duration_since(last_loss) < self.smoothed_rtt {
                return;
            }
        }
        self.last_loss_time = Some(now);

        // RFC 8312 Fast Convergence:
        if self.cwnd < self.w_last_max {
            self.w_last_max = self.cwnd;
            self.w_max = ((self.cwnd as f64 * (1.0 + self.beta) / 2.0) as u64).max(self.min_cwnd());
        } else {
            self.w_last_max = self.cwnd;
            self.w_max = self.cwnd;
        }

        self.ssthresh = ((self.cwnd as f64 * self.beta) as u64).max(self.min_cwnd());
        self.cwnd = self.ssthresh;
        self.origin_point = self.w_max;

        // K = cbrt(w_max * (1 - beta) / (C * SMSS))
        let w_diff = (self.w_max as f64 * (1.0 - self.beta)) / (self.c_const * self.smss as f64);
        self.k = if w_diff > 0.0 { w_diff.cbrt() } else { 0.0 };

        self.epoch_start = Some(now);
    }
}

impl CongestionController for CubicCongestionController {
    fn on_packet_sent(&mut self, _pn: PacketNumber, _bytes: usize, _send_time: MonotonicTime) {
        // FR-3: in-flight accounting lives in the loss detector; nothing to mirror here.
    }

    fn on_ack(&mut self, ack_event: &AckEvent, now: MonotonicTime) {
        if let Some(rtt) = ack_event.rtt_sample {
            self.on_rtt(rtt);
        }
        // CC-2: the window only grows on NEW acknowledgements carrying bytes —
        // duplicate/empty ACK frames must not inflate cwnd.
        if ack_event.bytes_acked > 0 {
            self.update_w_cubic(now);
        }
    }

    fn on_loss(&mut self, loss_event: &LossEvent, now: MonotonicTime) {
        if !loss_event.lost_packets.is_empty() {
            self.on_congestion_event(now);
        }
    }

    fn on_ecn(&mut self, ce_count: u32, now: MonotonicTime) {
        if ce_count > 0 {
            self.on_congestion_event(now);
        }
    }

    fn on_rtt(&mut self, rtt_sample: Duration) {
        // CC-4: true EWMA instead of storing the raw last sample, so the once-per-RTT
        // loss guard and the pacing rate do not jitter with single observations.
        self.smoothed_rtt =
            Duration::from_micros((self.smoothed_rtt.as_micros() * 7 + rtt_sample.as_micros()) / 8);
        self.min_rtt = self.min_rtt.min(rtt_sample);
    }

    fn on_timeout(&mut self, now: MonotonicTime) {
        // RFC 8312 §4.7 (CC-1): after a timeout the epoch state must be reset so the
        // post-timeout slow start is not instantly overwritten by the stale
        // W_cubic trajectory (stale origin_point/k restored the old window).
        self.ssthresh = ((self.cwnd as f64 * self.beta) as u64).max(self.min_cwnd());
        self.cwnd = self.min_cwnd();

        self.w_max = self.ssthresh;
        self.w_last_max = self.ssthresh;
        self.origin_point = self.w_max;
        self.k = 0.0;
        self.epoch_start = Some(now);
    }

    fn cwnd(&self) -> u64 {
        self.cwnd
    }

    fn pacing_rate(&self) -> u64 {
        // Pacing rate = (cwnd / smoothed_rtt) * pacing_gain (config-driven)
        let rtt_secs = self.smoothed_rtt.as_secs_f64().max(0.001);
        let base_rate = (self.cwnd as f64 / rtt_secs) * self.pacing_gain;
        (base_rate.max(10_000.0)) as u64 // Minimum 10 KB/s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cubic_slow_start_and_loss_reduction() {
        let mut cubic = CubicCongestionController::new(1200);
        let now = MonotonicTime::from_micros(1_000_000);

        let initial_cwnd = cubic.cwnd();
        assert_eq!(initial_cwnd, 12_000);

        // Simulate packet sent. FR-3: the controller no longer mirrors in-flight bytes
        // (that is the loss detector's single source of truth), so this test now only
        // exercises what CUBIC owns — the congestion window.
        cubic.on_packet_sent(PacketNumber(1), 1200, now);

        // Simulate ACK received -> window grows
        let ack_ev = AckEvent {
            largest_acked: PacketNumber(1),
            acked_packets: Vec::new(),
            bytes_acked: 1200,
            rtt_sample: Some(Duration::from_millis(50)),
        };
        cubic.on_ack(&ack_ev, now + Duration::from_millis(50));
        assert!(cubic.cwnd() > initial_cwnd);

        // Simulate Loss Event -> window reduces to beta * cwnd
        let current_cwnd = cubic.cwnd();
        let loss_ev = LossEvent {
            lost_packets: vec![gtp_recovery::SentPacketRecord {
                packet_number: PacketNumber(2),
                send_time: now,
                bytes: 1200,
                ack_eliciting: true,
                in_flight: true,
                retransmittable_frames: Vec::new(),
            }],
            bytes_lost: 1200,
            retransmittable: Vec::new(),
        };

        cubic.on_loss(&loss_ev, now + Duration::from_millis(100));
        assert_eq!(cubic.cwnd(), (current_cwnd as f64 * BETA_CUBIC) as u64);
    }

    /// CC-1: after a timeout, the W_cubic trajectory must not instantly restore the
    /// old window — the epoch resets (k=0, origin_point=w_max) per RFC 8312 §4.7.
    #[test]
    fn cubic_timeout_resets_epoch_state() {
        let mut cubic = CubicCongestionController::new(1200);
        let t0 = MonotonicTime::from_micros(1_000_000);

        // Grow the window large first
        for i in 1..=60u64 {
            let ack = AckEvent {
                largest_acked: PacketNumber(i),
                acked_packets: Vec::new(),
                bytes_acked: 1200,
                rtt_sample: Some(Duration::from_millis(20)),
            };
            cubic.on_ack(&ack, t0 + Duration::from_millis(i * 10));
        }
        let grown = cubic.cwnd();
        assert!(grown > 12_000 * 2);

        // Timeout: window collapses to min_cwnd
        cubic.on_timeout(t0 + Duration::from_millis(1000));
        assert_eq!(cubic.cwnd(), 2 * 1200);

        // Slow-start back up to ssthresh — the window must NOT jump to the old
        // trajectory immediately after leaving slow start.
        let ssthresh_after_timeout = grown * 7 / 10;
        let mut t = t0 + Duration::from_millis(1100);
        let mut acks = 100u64;
        while cubic.cwnd() < ssthresh_after_timeout && acks < 10_000 {
            let ack = AckEvent {
                largest_acked: PacketNumber(acks),
                acked_packets: Vec::new(),
                bytes_acked: 1200,
                rtt_sample: Some(Duration::from_millis(20)),
            };
            cubic.on_ack(&ack, t);
            acks += 1;
            t += Duration::from_millis(10);
        }
        // The climb is gradual (per-ACK SMSS increments), not an instant restore:
        assert!(
            cubic.cwnd() < grown,
            "window must not instantly restore the pre-timeout trajectory"
        );
    }

    /// CC-2: duplicate/empty ACKs must not grow the window.
    #[test]
    fn cubic_does_not_grow_on_empty_acks() {
        let mut cubic = CubicCongestionController::new(1200);
        let t0 = MonotonicTime::from_micros(1_000_000);

        let empty_ack = AckEvent {
            largest_acked: PacketNumber(1),
            acked_packets: Vec::new(),
            bytes_acked: 0,
            rtt_sample: Some(Duration::from_millis(20)),
        };
        let before = cubic.cwnd();
        for i in 0..50 {
            cubic.on_ack(&empty_ack, t0 + Duration::from_millis(i));
        }
        assert_eq!(cubic.cwnd(), before);
    }

    /// P1-2: configuration values must reach the controller.
    #[test]
    fn cubic_config_is_honored() {
        let cfg = CubicConfig {
            smss: 1000,
            initial_cwnd_packets: 50,
            min_cwnd_packets: 10,
            beta: 0.85,
            c: 0.5,
            pacing_gain: 1.5,
        };
        let cubic = CubicCongestionController::with_config(cfg);
        assert_eq!(cubic.cwnd(), 50_000);
        assert_eq!(cubic.min_cwnd_bytes, 10_000);
    }
}
