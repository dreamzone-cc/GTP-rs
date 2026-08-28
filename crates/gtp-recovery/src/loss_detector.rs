use std::collections::BTreeMap;
use crate::rtt::RttStats;
use crate::sent_packet::{RetransmissionRecord, SentPacketRecord};
use gtp_types::{Duration, MonotonicTime, PacketNumber};
use gtp_wire::AckRange;

pub const PACKET_THRESHOLD: u64 = 3;
pub const TIME_THRESHOLD_FACTOR_NUM: u64 = 9;
pub const TIME_THRESHOLD_FACTOR_DEN: u64 = 8;

#[derive(Clone, Debug)]
pub struct AckEvent {
    pub largest_acked: PacketNumber,
    pub acked_packets: Vec<SentPacketRecord>,
    pub bytes_acked: usize,
    pub rtt_sample: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct LossEvent {
    pub lost_packets: Vec<SentPacketRecord>,
    pub bytes_lost: usize,
    pub retransmittable: Vec<RetransmissionRecord>,
}

#[derive(Copy, Clone, Debug)]
pub struct DeliveryRateSample {
    pub delivered_bytes: u64,
    pub interval: Duration,
    pub rate_bytes_per_sec: u64,
}

#[derive(Clone, Debug)]
pub struct LossDetector {
    pub rtt_stats: RttStats,
    sent_packets: BTreeMap<u64, SentPacketRecord>,
    largest_acked_packet: Option<PacketNumber>,
    pub time_of_last_ack_eliciting_packet: MonotonicTime,
    pub pto_count: u32,
    total_bytes_sent: u64,
    total_bytes_acked: u64,
    last_delivery_rate_time: MonotonicTime,
    last_delivery_rate_bytes: u64,
}

impl Default for LossDetector {
    fn default() -> Self {
        Self {
            rtt_stats: RttStats::new(),
            sent_packets: BTreeMap::new(),
            largest_acked_packet: None,
            time_of_last_ack_eliciting_packet: MonotonicTime::ZERO,
            pto_count: 0,
            total_bytes_sent: 0,
            total_bytes_acked: 0,
            last_delivery_rate_time: MonotonicTime::ZERO,
            last_delivery_rate_bytes: 0,
        }
    }
}

impl LossDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_packet_sent(&mut self, record: SentPacketRecord) {
        if record.ack_eliciting {
            self.time_of_last_ack_eliciting_packet = record.send_time;
        }
        self.total_bytes_sent += record.bytes as u64;
        self.sent_packets.insert(record.packet_number.as_u64(), record);
    }

    pub fn inflight_bytes(&self) -> u64 {
        self.sent_packets
            .values()
            .filter(|p| p.in_flight)
            .map(|p| p.bytes as u64)
            .sum()
    }

    pub fn on_ack_received(
        &mut self,
        largest_acked: PacketNumber,
        ack_delay_us: u32,
        ranges: &[AckRange],
        range_count: usize,
        now: MonotonicTime,
    ) -> (AckEvent, LossEvent, Option<DeliveryRateSample>) {
        let ack_delay = Duration::from_micros(ack_delay_us as u64);
        let largest_pn = largest_acked.as_u64();

        let mut newly_acked = Vec::new();
        let mut bytes_acked = 0;
        let mut rtt_sample = None;

        // Parse acknowledged packet numbers from ACK ranges
        let mut acked_numbers = Vec::new();
        let mut current_pn = largest_pn;

        for range in ranges.iter().take(range_count) {
            let start = current_pn.saturating_sub(range.length as u64);
            for pn in (start..=current_pn).rev() {
                acked_numbers.push(pn);
            }
            if current_pn >= (range.length as u64 + range.gap as u64 + 1) {
                current_pn = current_pn - (range.length as u64) - (range.gap as u64) - 1;
            } else {
                break;
            }
        }

        // Process newly acknowledged packets
        for &pn in &acked_numbers {
            if let Some(record) = self.sent_packets.remove(&pn) {
                bytes_acked += record.bytes;
                if pn == largest_pn {
                    let sample = now.duration_since(record.send_time);
                    self.rtt_stats.update(sample, ack_delay);
                    rtt_sample = Some(sample);
                }
                newly_acked.push(record);
            }
        }

        self.total_bytes_acked += bytes_acked as u64;

        if self.largest_acked_packet.map_or(true, |l| largest_pn > l.as_u64()) {
            self.largest_acked_packet = Some(largest_acked);
        }

        // Reset PTO on new progress
        if !newly_acked.is_empty() {
            self.pto_count = 0;
        }

        // Delivery rate calculation
        let mut delivery_sample = None;
        if self.last_delivery_rate_time != MonotonicTime::ZERO {
            let interval = now.duration_since(self.last_delivery_rate_time);
            if interval.as_micros() > 1000 {
                let delivered = self.total_bytes_acked.saturating_sub(self.last_delivery_rate_bytes);
                let rate = (delivered as f64 / interval.as_secs_f64()) as u64;
                delivery_sample = Some(DeliveryRateSample {
                    delivered_bytes: delivered,
                    interval,
                    rate_bytes_per_sec: rate,
                });
                self.last_delivery_rate_time = now;
                self.last_delivery_rate_bytes = self.total_bytes_acked;
            }
        } else {
            self.last_delivery_rate_time = now;
            self.last_delivery_rate_bytes = self.total_bytes_acked;
        }

        // Detect Lost Packets
        let mut lost_packets = Vec::new();
        let mut bytes_lost = 0;
        let mut retransmittable = Vec::new();

        let time_threshold = Duration::from_micros(
            (self.rtt_stats.smoothed_rtt.max(self.rtt_stats.latest_rtt).as_micros()
                * TIME_THRESHOLD_FACTOR_NUM)
                / TIME_THRESHOLD_FACTOR_DEN,
        );

        let mut lost_pns = Vec::new();
        for (&pn, record) in &self.sent_packets {
            if pn > largest_pn {
                continue;
            }

            let pkt_threshold_exceeded = largest_pn >= pn + PACKET_THRESHOLD;
            let time_threshold_exceeded = now.duration_since(record.send_time) >= time_threshold;

            if pkt_threshold_exceeded || time_threshold_exceeded {
                lost_pns.push(pn);
            }
        }

        for pn in lost_pns {
            if let Some(record) = self.sent_packets.remove(&pn) {
                bytes_lost += record.bytes;
                for frame in record.retransmittable_frames.clone() {
                    retransmittable.push(frame);
                }
                lost_packets.push(record);
            }
        }

        (
            AckEvent {
                largest_acked,
                acked_packets: newly_acked,
                bytes_acked,
                rtt_sample,
            },
            LossEvent {
                lost_packets,
                bytes_lost,
                retransmittable,
            },
            delivery_sample,
        )
    }

    pub fn on_timeout(&mut self, _now: MonotonicTime) -> LossEvent {
        self.pto_count += 1;
        // On PTO timeout, we return unacknowledged retransmissions
        let mut retransmittable = Vec::new();
        for record in self.sent_packets.values() {
            for frame in &record.retransmittable_frames {
                retransmittable.push(frame.clone());
            }
        }
        LossEvent {
            lost_packets: Vec::new(),
            bytes_lost: 0,
            retransmittable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtp_types::{FragmentId, MessageId, TransmissionId};

    #[test]
    fn test_loss_detection_via_packet_threshold() {
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);

        // Send packets 1, 2, 3, 4
        for pn in 1..=4 {
            let retrans = vec![RetransmissionRecord {
                message_id: MessageId(pn),
                fragment_id: FragmentId(0),
                transmission_id: TransmissionId(1),
                group_id: 0,
                order_seq: 0,
                payload: b"critical_game_event".to_vec(),
            }];

            detector.on_packet_sent(SentPacketRecord {
                packet_number: PacketNumber(pn),
                send_time: now,
                bytes: 100,
                ack_eliciting: true,
                in_flight: true,
                retransmittable_frames: retrans,
            });
        }

        assert_eq!(detector.inflight_bytes(), 400);

        // Receive ACK for Packet 4 only (range length 0) -> Packet 1 is lost (4 >= 1 + 3)
        let ranges = [AckRange { gap: 0, length: 0 }];
        let (ack_ev, loss_ev, _) = detector.on_ack_received(
            PacketNumber(4),
            0,
            &ranges,
            1,
            now + Duration::from_millis(50),
        );

        assert_eq!(ack_ev.acked_packets.len(), 1);
        assert_eq!(ack_ev.acked_packets[0].packet_number, PacketNumber(4));

        assert_eq!(loss_ev.lost_packets.len(), 1);
        assert_eq!(loss_ev.lost_packets[0].packet_number, PacketNumber(1));
        assert_eq!(loss_ev.retransmittable.len(), 1);
        assert_eq!(loss_ev.retransmittable[0].message_id, MessageId(1));
    }
}
