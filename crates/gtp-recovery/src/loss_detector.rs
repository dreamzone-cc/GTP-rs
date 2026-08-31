use crate::rtt::RttStats;
use crate::sent_packet::{RetransmissionRecord, SentPacketRecord};
use gtp_types::{Duration, MonotonicTime, PacketNumber};
use gtp_wire::AckRange;
use std::collections::BTreeMap;

pub const PACKET_THRESHOLD: u64 = 3;
pub const TIME_THRESHOLD_FACTOR_NUM: u64 = 9;
pub const TIME_THRESHOLD_FACTOR_DEN: u64 = 8;
/// Maximum total packets a single ACK frame may claim, defending the loss detector
/// from attacker-controlled range expansion (REC-10) and optimistic-ACK window inflation.
pub const MAX_ACKED_PER_FRAME: u64 = 16_384;
/// Per-PTO retransmission burst cap (RFC 9002 §7.5: at most 2 probe datagrams).
pub const MAX_PTO_RETRANSMIT_BURST: usize = 2;
/// Exponential backoff ceiling exponent for PTO (2^8 = 256x).
pub const MAX_PTO_BACKOFF_EXPONENT: u32 = 8;

#[derive(Clone, Debug)]
pub struct AckEvent {
    pub largest_acked: PacketNumber,
    pub acked_packets: Vec<SentPacketRecord>,
    /// Bytes reclaimed from congestion-window accounting (in-flight packets only).
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
    /// FR-3 / PERF-2: running total of in-flight bytes, maintained incrementally so
    /// `inflight_bytes()` is O(1) instead of an O(N) scan. This is the SINGLE source
    /// of truth for in-flight accounting — the congestion controller no longer keeps
    /// its own mirror counter, which is what let the two diverge (the R-1 regression).
    inflight_bytes: u64,
    largest_acked_packet: Option<PacketNumber>,
    largest_sent_packet: u64,
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
            inflight_bytes: 0,
            largest_acked_packet: None,
            largest_sent_packet: 0,
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
        self.largest_sent_packet = self.largest_sent_packet.max(record.packet_number.as_u64());
        if record.in_flight {
            self.inflight_bytes = self.inflight_bytes.saturating_add(record.bytes as u64);
        }
        self.sent_packets
            .insert(record.packet_number.as_u64(), record);
    }

    /// In-flight bytes — the single source of truth for congestion accounting (FR-3),
    /// maintained incrementally so this is O(1) (PERF-2). In debug builds it is checked
    /// against a full scan of `sent_packets` so any missed update path fails loudly.
    pub fn inflight_bytes(&self) -> u64 {
        debug_assert_eq!(
            self.inflight_bytes,
            self.sent_packets
                .values()
                .filter(|p| p.in_flight)
                .map(|p| p.bytes as u64)
                .sum::<u64>(),
            "inflight_bytes counter drifted from the sent_packets scan"
        );
        self.inflight_bytes
    }

    /// True once at least one authenticated ACK from the peer has been processed.
    ///
    /// The client uses this as a handshake-establishment signal: an ACK can only
    /// be produced by a peer that decrypted our traffic, which in turn means it
    /// accepted and registered the connection. Until it flips true the peer may
    /// never have completed acceptance (a lost HandshakeFinish leaves the session
    /// half-open), so `connect()` retransmits the Finish rather than declaring the
    /// connection established on faith.
    pub fn has_received_ack(&self) -> bool {
        self.largest_acked_packet.is_some()
    }

    /// PTO duration with RFC 9002 §6.2 exponential backoff, capped at `max_pto`.
    pub fn pto_duration_with_backoff(&self, max_pto: Duration) -> Duration {
        let factor = 1u64 << self.pto_count.min(MAX_PTO_BACKOFF_EXPONENT);
        let expanded = Duration::from_micros(
            self.rtt_stats
                .pto_duration()
                .as_micros()
                .saturating_mul(factor),
        );
        expanded.min(max_pto)
    }

    /// Parses acknowledged packet numbers from ACK ranges.
    ///
    /// The wire semantics produced by `AckTracker::generate_ack_frame` are: range 0
    /// covers `largest_acked` down to `largest_acked - length` (length = block size - 1);
    /// every subsequent range first skips `gap` unacknowledged packets, then covers
    /// `length + 1` packets. Malformed or overflowing ranges truncate the walk — they
    /// can never inflate the acknowledged set (REC-10 / optimistic-ACK defense).
    fn parse_ack_ranges(largest_pn: u64, ranges: &[AckRange], range_count: usize) -> Vec<u64> {
        let mut acked = Vec::new();
        let mut total: u64 = 0;
        let mut block_high = largest_pn;
        let mut prev_low = largest_pn;

        for (i, range) in ranges.iter().take(range_count).enumerate() {
            if i > 0 {
                let gap = range.gap as u64;
                // The gap sits between the previous block's low edge and this block's
                // high edge; underflow means the frame is malformed — stop walking.
                if gap + 1 > prev_low {
                    break;
                }
                block_high = prev_low - gap - 1;
            }

            let length = range.length as u64;
            if length > block_high {
                break;
            }
            let block_low = block_high - length;

            let count = length + 1;
            if total + count > MAX_ACKED_PER_FRAME {
                break;
            }
            for pn in (block_low..=block_high).rev() {
                acked.push(pn);
            }
            total += count;
            prev_low = block_low;
        }

        acked
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

        // Optimistic-ACK defense (0.8): a peer cannot acknowledge packets we never sent.
        if largest_pn > self.largest_sent_packet {
            return (
                AckEvent {
                    largest_acked,
                    acked_packets: Vec::new(),
                    bytes_acked: 0,
                    rtt_sample: None,
                },
                LossEvent {
                    lost_packets: Vec::new(),
                    bytes_lost: 0,
                    retransmittable: Vec::new(),
                },
                None,
            );
        }

        let mut newly_acked = Vec::new();
        let mut bytes_acked = 0usize;
        let mut rtt_sample = None;

        let acked_numbers = Self::parse_ack_ranges(largest_pn, ranges, range_count);

        // Process newly acknowledged packets
        for &pn in &acked_numbers {
            if let Some(record) = self.sent_packets.remove(&pn) {
                // P1-3: only in-flight bytes participate in congestion accounting —
                // ACK-only packets would otherwise leak phantom congestion debt.
                if record.in_flight {
                    bytes_acked += record.bytes;
                    self.inflight_bytes = self.inflight_bytes.saturating_sub(record.bytes as u64);
                }
                if pn == largest_pn {
                    let sample = now.duration_since(record.send_time);
                    self.rtt_stats.update(sample, ack_delay);
                    rtt_sample = Some(sample);
                }
                newly_acked.push(record);
            }
        }

        self.total_bytes_acked += bytes_acked as u64;

        if self
            .largest_acked_packet
            .is_none_or(|l| largest_pn > l.as_u64())
        {
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
                let delivered = self
                    .total_bytes_acked
                    .saturating_sub(self.last_delivery_rate_bytes);
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

        // Detect Lost Packets — RFC 9002 §2: loss is declared for in-flight,
        // ack-eliciting packets only. Never for ACK-only packets (the fix that
        // eliminated the per-RTT phantom window reduction).
        let mut lost_packets = Vec::new();
        let mut bytes_lost = 0;
        let mut retransmittable = Vec::new();

        let time_threshold = Duration::from_micros(
            (self
                .rtt_stats
                .smoothed_rtt
                .max(self.rtt_stats.latest_rtt)
                .as_micros()
                * TIME_THRESHOLD_FACTOR_NUM)
                / TIME_THRESHOLD_FACTOR_DEN,
        );

        let mut lost_pns = Vec::new();
        for (&pn, record) in &self.sent_packets {
            if pn > largest_pn || !record.in_flight {
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
                // in_flight is always true here (filtered above), but guard anyway.
                if record.in_flight {
                    self.inflight_bytes = self.inflight_bytes.saturating_sub(record.bytes as u64);
                }
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

    /// PTO timeout sweep (RFC 9002 §7.5): retransmit the oldest in-flight
    /// retransmittable records — at most `MAX_PTO_RETRANSMIT_BURST` of them — and
    /// **remove** them from the outstanding set so each PTO fires a bounded burst
    /// instead of re-enqueueing the entire window every period.
    ///
    /// R-1: the drained records leave `sent_packets`, so they can never later be
    /// acknowledged (`bytes_acked`) nor declared lost (`bytes_lost`). Their bytes
    /// are therefore reported here as `bytes_lost` so the congestion controller can
    /// settle the in-flight debt. `lost_packets` stays empty on purpose: a PTO is a
    /// probe, not a loss declaration, and every controller gates
    /// `on_congestion_event` on `!lost_packets.is_empty()`. Without this the
    /// controller's `inflight` ratchets up permanently and `cwnd - inflight`
    /// collapses to zero for the rest of the connection.
    pub fn on_timeout(&mut self, _now: MonotonicTime) -> LossEvent {
        self.pto_count = self.pto_count.saturating_add(1);

        let mut retransmittable = Vec::new();
        let mut drained: Vec<u64> = Vec::new();

        for (&pn, record) in &self.sent_packets {
            if drained.len() >= MAX_PTO_RETRANSMIT_BURST {
                break;
            }
            if !record.in_flight {
                continue;
            }
            for frame in &record.retransmittable_frames {
                retransmittable.push(frame.clone());
            }
            drained.push(pn);
        }

        let mut bytes_drained = 0u64;
        for pn in drained {
            if let Some(record) = self.sent_packets.remove(&pn) {
                if record.in_flight {
                    bytes_drained = bytes_drained.saturating_add(record.bytes as u64);
                }
            }
        }
        // FR-3: the drained records leave the in-flight set, so the single counter must
        // shed their bytes. (This is exactly the debt that used to strand the CC mirror
        // counter in R-1; with one source of truth it can no longer diverge.)
        self.inflight_bytes = self.inflight_bytes.saturating_sub(bytes_drained);

        LossEvent {
            lost_packets: Vec::new(),
            // R-1: report the drained bytes so the congestion controller settles the
            // debt for records that will never be acked or swept again.
            bytes_lost: bytes_drained as usize,
            retransmittable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtp_types::{FragmentId, MessageId, TransmissionId};

    fn record(pn: u64) -> SentPacketRecord {
        SentPacketRecord {
            packet_number: PacketNumber(pn),
            send_time: MonotonicTime::from_micros(1_000_000),
            bytes: 100,
            ack_eliciting: true,
            in_flight: true,
            retransmittable_frames: vec![RetransmissionRecord {
                message_id: MessageId(pn),
                fragment_id: FragmentId(0),
                transmission_id: TransmissionId(1),
                group_id: 0,
                order_seq: 0,
                payload: b"critical_game_event".to_vec(),
            }],
        }
    }

    #[test]
    fn test_loss_detection_via_packet_threshold() {
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);

        // Send packets 1, 2, 3, 4
        for pn in 1..=4 {
            detector.on_packet_sent(record(pn));
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

    /// P0-6 / reconciliation 0.7: with multiple gaps, the acknowledged set decoded by
    /// the loss detector must EXACTLY equal the received set — the old decoder shifted
    /// gap application by one block and phantom-acknowledged never-sent packets.
    #[test]
    fn ack_ranges_multi_gap_roundtrip_exact() {
        let mut tracker_received: Vec<u64> = Vec::new();
        // Received pattern with gaps: 1..=5, 8..=10, 20..=22 (from the reconciliation doc)
        let mut tracker = crate::ack_tracker::AckTracker::new();
        let t0 = MonotonicTime::from_micros(1_000_000);
        for pn in [1u64, 2, 3, 4, 5, 8, 9, 10, 20, 21, 22] {
            tracker.on_packet_received(PacketNumber(pn), true, 0, t0);
            tracker_received.push(pn);
        }

        let frame = tracker.generate_ack_frame(t0).unwrap();
        let (largest_acked, ranges, range_count) = match &frame {
            gtp_wire::Frame::Ack {
                largest_acked,
                ranges,
                range_count,
                ..
            } => (*largest_acked, *ranges, *range_count as usize),
            _ => panic!("expected ACK frame"),
        };

        let mut detector = LossDetector::new();
        for pn in 1..=22 {
            detector.on_packet_sent(record(pn));
        }

        let (ack_ev, _, _) = detector.on_ack_received(largest_acked, 0, &ranges, range_count, t0);

        let acked_set: std::collections::HashSet<u64> = ack_ev
            .acked_packets
            .iter()
            .map(|r| r.packet_number.as_u64())
            .collect();
        let received_set: std::collections::HashSet<u64> = tracker_received.into_iter().collect();

        assert_eq!(
            acked_set, received_set,
            "acked set must equal received set exactly"
        );
        // The never-sent / never-received packets must not be acknowledged
        for phantom in [6u64, 7, 17, 18, 19] {
            assert!(!acked_set.contains(&phantom), "phantom ack for {}", phantom);
        }
    }

    /// Deterministic property sweep: random receive patterns (seeded LCG) must
    /// round-trip through ACK encode → loss-detector decode losslessly whenever the
    /// pattern fits within the 32-range frame budget, and must NEVER phantom-ack a
    /// packet that was not received.
    #[test]
    fn ack_ranges_property_sweep_seeded() {
        let mut lcg: u64 = 0x9E3779B97F4A7C15;
        let mut next = move || {
            lcg ^= lcg << 13;
            lcg ^= lcg >> 7;
            lcg ^= lcg << 17;
            lcg
        };

        for _case in 0..64 {
            let mut tracker = crate::ack_tracker::AckTracker::new();
            let t0 = MonotonicTime::from_micros(1_000_000);
            let mut detector = LossDetector::new();
            let mut received = Vec::new();

            let total: u64 = 30 + next() % 90;
            for pn in 1..=total {
                detector.on_packet_sent(record(pn));
                let roll = next() % 100;
                let delivered = roll < 82 || next() % 3 == 0;
                if delivered {
                    tracker.on_packet_received(PacketNumber(pn), true, 0, t0);
                    received.push(pn);
                }
            }

            if let Some(frame) = tracker.generate_ack_frame(t0) {
                let (largest_acked, ranges, range_count) = match &frame {
                    gtp_wire::Frame::Ack {
                        largest_acked,
                        ranges,
                        range_count,
                        ..
                    } => (*largest_acked, *ranges, *range_count as usize),
                    _ => unreachable!(),
                };
                let (ack_ev, _, _) =
                    detector.on_ack_received(largest_acked, 0, &ranges, range_count, t0);
                let acked: std::collections::HashSet<u64> = ack_ev
                    .acked_packets
                    .iter()
                    .map(|r| r.packet_number.as_u64())
                    .collect();
                let expect: std::collections::HashSet<u64> = received.iter().copied().collect();

                // Safety (the critical property): no phantom acknowledgements, ever.
                assert!(
                    acked.is_subset(&expect),
                    "phantom ack detected in seeded ACK property case"
                );

                // Completeness: when the pattern fits the 32-range budget the frame
                // must acknowledge every received packet exactly.
                if tracker.interval_count() <= 32 {
                    assert_eq!(
                        acked, expect,
                        "round-trip mismatch in seeded ACK property case"
                    );
                }
            }
        }
    }

    /// 0.8: a peer acknowledging packets we never sent must be ignored entirely.
    #[test]
    fn optimistic_ack_rejected() {
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);
        detector.on_packet_sent(record(1));

        let ranges = [AckRange {
            gap: 0,
            length: 1000,
        }];
        let (ack_ev, loss_ev, _) = detector.on_ack_received(PacketNumber(5000), 0, &ranges, 1, now);

        assert!(ack_ev.acked_packets.is_empty());
        assert!(loss_ev.lost_packets.is_empty());
        assert_eq!(detector.inflight_bytes(), 100); // untouched
    }

    /// REC-10: a malicious ACK frame cannot drive unbounded allocation.
    #[test]
    fn oversized_ack_ranges_capped() {
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);
        detector.on_packet_sent(record(1));

        let ranges = [AckRange {
            gap: 0,
            length: u32::MAX,
        }];
        let (ack_ev, _, _) = detector.on_ack_received(PacketNumber(1), 0, &ranges, 1, now);
        // Everything beyond the sanity cap is refused; nothing panics or explodes.
        assert!(ack_ev.acked_packets.len() as u64 <= MAX_ACKED_PER_FRAME);
    }

    /// P1-3: ACK-only packets are never declared lost and never charged to congestion.
    #[test]
    fn ack_only_packets_never_declared_lost() {
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);

        // ACK-only packet #1 (in_flight = false)
        let mut ack_only = record(1);
        ack_only.in_flight = false;
        ack_only.ack_eliciting = false;
        detector.on_packet_sent(ack_only);

        // Data packets 2..=6, then ACK 6 → PN 1 is far past the packet threshold
        for pn in 2..=6 {
            detector.on_packet_sent(record(pn));
        }

        let ranges = [AckRange { gap: 0, length: 0 }];
        let (ack_ev, loss_ev, _) = detector.on_ack_received(
            PacketNumber(6),
            0,
            &ranges,
            1,
            now + Duration::from_millis(20),
        );

        // PN 1 (ACK-only) must not appear in either event
        assert!(!loss_ev
            .lost_packets
            .iter()
            .any(|r| r.packet_number == PacketNumber(1)));
        assert!(!ack_ev
            .acked_packets
            .iter()
            .any(|r| r.packet_number == PacketNumber(1)));
        // Packet threshold (k=3): PN 2 and 3 are in-flight and >= 3 below largest(6)
        assert_eq!(loss_ev.lost_packets.len(), 2);
        // bytes_acked only counts the in-flight acked packet (not the ACK-only one)
        assert_eq!(ack_ev.bytes_acked, 100);
        assert_eq!(detector.inflight_bytes(), 200); // PNs 4,5 still in flight
    }

    /// P1-4: PTO drains only a bounded burst and removes the swept records.
    #[test]
    fn pto_burst_capped_and_drains_records() {
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);
        for pn in 1..=10 {
            detector.on_packet_sent(record(pn));
        }

        let ev1 = detector.on_timeout(now);
        assert_eq!(ev1.retransmittable.len(), 2); // burst cap
        assert_eq!(detector.sent_packets.len(), 8); // records removed

        let ev2 = detector.on_timeout(now);
        assert_eq!(ev2.retransmittable.len(), 2);
        assert_eq!(detector.sent_packets.len(), 6);

        // Backoff grows then saturates and respects the cap
        let base = detector.rtt_stats.pto_duration();
        let max_pto = Duration::from_millis(10_000);
        let b1 = detector.pto_duration_with_backoff(max_pto);
        assert!(b1 > base);
        for _ in 0..40 {
            detector.on_timeout(now);
        }
        assert!(detector.pto_count >= MAX_PTO_BACKOFF_EXPONENT);
        let capped = detector.pto_duration_with_backoff(Duration::from_millis(500));
        assert_eq!(capped, Duration::from_millis(500));
    }

    #[test]
    fn pto_drain_reports_bytes_for_congestion_settlement() {
        // R-1: `on_timeout` removes the drained records from `sent_packets`, so no
        // later ACK or loss sweep can ever repay their in-flight debt. The event must
        // therefore carry those bytes, while leaving `lost_packets` empty so no extra
        // congestion event fires on top of the timeout.
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);

        for pn in 1..=2u64 {
            detector.on_packet_sent(SentPacketRecord {
                packet_number: PacketNumber(pn),
                send_time: now,
                bytes: 100,
                ack_eliciting: true,
                in_flight: true,
                retransmittable_frames: Vec::new(),
            });
        }
        assert_eq!(detector.inflight_bytes(), 200);

        let ev = detector.on_timeout(now);
        assert_eq!(
            ev.bytes_lost, 200,
            "drained bytes must be reported to the controller"
        );
        assert!(
            ev.lost_packets.is_empty(),
            "a PTO is a probe, not a loss declaration"
        );
        assert_eq!(detector.inflight_bytes(), 0);
    }

    /// FR-3 / TEST-3: in-flight bytes are owned by a single incremental counter, and
    /// every path that removes an outstanding record sheds its bytes from that counter.
    /// A PTO drain is the case that stranded the old CC mirror counter (the R-1
    /// regression); here the one source of truth reflects it immediately. Every
    /// `inflight_bytes()` call also runs the debug-build drift check against the scan.
    #[test]
    fn inflight_is_a_single_source_and_pto_drain_sheds_it() {
        let mut detector = LossDetector::new();
        let now = MonotonicTime::from_micros(1_000_000);

        // 5 in-flight ack-eliciting packets, 100 bytes each.
        for pn in 1..=5 {
            detector.on_packet_sent(record(pn));
        }
        assert_eq!(detector.inflight_bytes(), 500);

        // A PTO sweep drains a bounded burst (MAX_PTO_RETRANSMIT_BURST = 2) and the
        // single counter loses exactly those bytes — no debt is stranded.
        detector.on_timeout(now);
        assert_eq!(
            detector.inflight_bytes(),
            500 - (MAX_PTO_RETRANSMIT_BURST as u64) * 100,
            "drained records must leave the single in-flight source"
        );

        // Acknowledging the rest settles in-flight to zero.
        let ranges = [AckRange { gap: 0, length: 4 }];
        let _ = detector.on_ack_received(PacketNumber(5), 0, &ranges, 1, now);
        assert_eq!(detector.inflight_bytes(), 0);
    }
}
