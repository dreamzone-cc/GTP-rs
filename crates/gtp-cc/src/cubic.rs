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
    cwnd: u64,
    ssthresh: u64,
    w_max: u64,
    w_last_max: u64,
    k: f64,
    epoch_start: Option<MonotonicTime>,
    origin_point: u64,
    smoothed_rtt: Duration,
    min_rtt: Duration,
    inflight: u64,
    last_loss_time: Option<MonotonicTime>,
}

impl Default for CubicCongestionController {
    fn default() -> Self {
        Self::new(DEFAULT_SMSS)
    }
}

impl CubicCongestionController {
    pub fn new(smss: u64) -> Self {
        let initial_cwnd = INITIAL_CWND_PACKETS * smss;
        Self {
            smss,
            cwnd: initial_cwnd,
            ssthresh: u64::MAX,
            w_max: initial_cwnd,
            w_last_max: initial_cwnd,
            k: 0.0,
            epoch_start: None,
            origin_point: initial_cwnd,
            smoothed_rtt: Duration::from_millis(50),
            min_rtt: Duration::from_millis(50),
            inflight: 0,
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
        let mut w_cubic = C_CUBIC * (target_diff * target_diff * target_diff) * (self.smss as f64)
            + (self.origin_point as f64);

        // RFC 8312 TCP-Friendly Region: W_tcp(t) = W_max*beta + 3*(1-beta)/(1+beta)*(t/RTT)*SMSS
        let rtt_secs = self.min_rtt.as_secs_f64().max(0.001);
        let beta = BETA_CUBIC;
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
        MIN_CWND_PACKETS * self.smss
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
            self.w_max =
                ((self.cwnd as f64 * (1.0 + BETA_CUBIC) / 2.0) as u64).max(self.min_cwnd());
        } else {
            self.w_last_max = self.cwnd;
            self.w_max = self.cwnd;
        }

        self.ssthresh = ((self.cwnd as f64 * BETA_CUBIC) as u64).max(self.min_cwnd());
        self.cwnd = self.ssthresh;
        self.origin_point = self.w_max;

        // K = cbrt(w_max * (1 - beta) / (C * SMSS))
        let w_diff = (self.w_max as f64 * (1.0 - BETA_CUBIC)) / (C_CUBIC * self.smss as f64);
        self.k = if w_diff > 0.0 { w_diff.cbrt() } else { 0.0 };

        self.epoch_start = Some(now);
    }
}

impl CongestionController for CubicCongestionController {
    fn on_packet_sent(&mut self, _pn: PacketNumber, bytes: usize, _send_time: MonotonicTime) {
        self.inflight = self.inflight.saturating_add(bytes as u64);
    }

    fn on_ack(&mut self, ack_event: &AckEvent, now: MonotonicTime) {
        self.inflight = self.inflight.saturating_sub(ack_event.bytes_acked as u64);
        if let Some(rtt) = ack_event.rtt_sample {
            self.on_rtt(rtt);
        }
        self.update_w_cubic(now);
    }

    fn on_loss(&mut self, loss_event: &LossEvent, now: MonotonicTime) {
        self.inflight = self.inflight.saturating_sub(loss_event.bytes_lost as u64);
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
        self.smoothed_rtt = rtt_sample;
        self.min_rtt = self.min_rtt.min(rtt_sample);
    }

    fn on_timeout(&mut self, now: MonotonicTime) {
        self.ssthresh = ((self.cwnd as f64 * BETA_CUBIC) as u64).max(self.min_cwnd());
        self.cwnd = self.min_cwnd();
        self.epoch_start = Some(now);
    }

    fn cwnd(&self) -> u64 {
        self.cwnd
    }

    fn pacing_rate(&self) -> u64 {
        // Pacing rate = (cwnd / smoothed_rtt) * 1.2 (pacing gain)
        let rtt_secs = self.smoothed_rtt.as_secs_f64().max(0.001);
        let base_rate = (self.cwnd as f64 / rtt_secs) * 1.2;
        (base_rate.max(10_000.0)) as u64 // Minimum 10 KB/s
    }

    fn inflight(&self) -> u64 {
        self.inflight
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

        // Simulate packet sent
        cubic.on_packet_sent(PacketNumber(1), 1200, now);
        assert_eq!(cubic.inflight(), 1200);

        // Simulate ACK received -> window grows
        let ack_ev = AckEvent {
            largest_acked: PacketNumber(1),
            acked_packets: Vec::new(),
            bytes_acked: 1200,
            rtt_sample: Some(Duration::from_millis(50)),
        };
        cubic.on_ack(&ack_ev, now + Duration::from_millis(50));
        assert!(cubic.cwnd() > initial_cwnd);
        assert_eq!(cubic.inflight(), 0);

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
}
