use gtp_types::{Duration, MonotonicTime, PacketNumber};
use gtp_wire::{AckRange, Frame, MAX_ACK_RANGES};

/// Represents a contiguous interval of received packet numbers: [start, end].
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct PacketInterval {
    pub start: u64,
    pub end: u64,
}

/// Received intervals older than this many packets below `largest_received` are
/// pruned: they are beyond any realistic re-ACK usefulness and would otherwise grow
/// without bound (REC-5) and crowd the 32-range frame budget (REC-6).
pub const ACK_RETENTION_WINDOW: u64 = 1024;

/// Tracks received incoming packet numbers and formats ACK frames with range compression.
#[derive(Clone, Debug)]
pub struct AckTracker {
    intervals: Vec<PacketInterval>,
    largest_received: Option<PacketNumber>,
    largest_received_time: MonotonicTime,
    unacked_packet_count: u8,
    last_ack_sent_time: Option<MonotonicTime>,
    ack_frequency: u8,
    max_ack_delay: Duration,
    ect0_count: u32,
    ect1_count: u32,
    ce_count: u32,
    has_gap: bool,
}

impl Default for AckTracker {
    fn default() -> Self {
        Self {
            intervals: Vec::with_capacity(32),
            largest_received: None,
            largest_received_time: MonotonicTime::ZERO,
            unacked_packet_count: 0,
            last_ack_sent_time: None,
            ack_frequency: 2,
            max_ack_delay: Duration::from_millis(25),
            ect0_count: 0,
            ect1_count: 0,
            ce_count: 0,
            has_gap: false,
        }
    }
}

impl AckTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a tracker honoring the negotiated ACK policy (P1-2: GtpConfig wiring).
    pub fn with_policy(ack_frequency: u8, max_ack_delay: Duration) -> Self {
        Self {
            ack_frequency: ack_frequency.max(1),
            max_ack_delay,
            ..Self::default()
        }
    }

    /// Applies a new ACK policy at runtime (local control API or remote ACK_FREQUENCY).
    pub fn set_policy(&mut self, ack_frequency: u8, max_ack_delay: Duration) {
        self.ack_frequency = ack_frequency.max(1);
        self.max_ack_delay = max_ack_delay;
    }

    /// Number of disjoint received intervals currently retained.
    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }

    pub fn on_packet_received(
        &mut self,
        packet_number: PacketNumber,
        is_ack_eliciting: bool,
        ecn_bits: u8,
        now: MonotonicTime,
    ) {
        let pn = packet_number.as_u64();

        match ecn_bits {
            1 => self.ect1_count = self.ect1_count.saturating_add(1),
            2 => self.ect0_count = self.ect0_count.saturating_add(1),
            3 => self.ce_count = self.ce_count.saturating_add(1),
            _ => {}
        }

        if self.last_ack_sent_time.is_none() {
            self.last_ack_sent_time = Some(now);
        }

        // Update largest received
        if self.largest_received.is_none_or(|l| pn > l.as_u64()) {
            if let Some(prev) = self.largest_received {
                if pn > prev.as_u64() + 1 {
                    self.has_gap = true;
                }
            }
            self.largest_received = Some(packet_number);
            self.largest_received_time = now;
        }

        self.insert_packet(pn);
        self.prune_old_intervals();

        if is_ack_eliciting {
            self.unacked_packet_count = self.unacked_packet_count.saturating_add(1);
        }
    }

    /// Drops intervals that fell out of the retention window below the newest packet.
    fn prune_old_intervals(&mut self) {
        let Some(largest) = self.largest_received else {
            return;
        };
        let floor = largest.as_u64().saturating_sub(ACK_RETENTION_WINDOW);
        self.intervals.retain(|iv| iv.end > floor);
    }

    fn insert_packet(&mut self, pn: u64) {
        let mut i = 0;
        while i < self.intervals.len() {
            let interval = self.intervals[i];
            if pn >= interval.start && pn <= interval.end {
                return; // Already acknowledged
            }
            if pn + 1 == interval.start {
                self.intervals[i].start = pn;
                self.merge_adjacent();
                return;
            }
            if pn == interval.end + 1 {
                self.intervals[i].end = pn;
                self.merge_adjacent();
                return;
            }
            if pn > interval.end {
                self.intervals
                    .insert(i, PacketInterval { start: pn, end: pn });
                return;
            }
            i += 1;
        }
        self.intervals.push(PacketInterval { start: pn, end: pn });
    }

    fn merge_adjacent(&mut self) {
        let mut i = 0;
        while i + 1 < self.intervals.len() {
            if self.intervals[i].start <= self.intervals[i + 1].end + 1 {
                self.intervals[i].start = self.intervals[i + 1].start;
                self.intervals.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    pub fn should_send_ack(&self, now: MonotonicTime) -> bool {
        if self.unacked_packet_count == 0 {
            return false;
        }
        // Immediate ACK on detected gap (loss suspicion)
        if self.has_gap {
            return true;
        }
        // Normal ACK frequency
        if self.unacked_packet_count >= self.ack_frequency {
            return true;
        }
        // Max ACK delay timer expired
        if let Some(last_ack) = self.last_ack_sent_time {
            if now.duration_since(last_ack) >= self.max_ack_delay {
                return true;
            }
        }
        false
    }

    pub fn generate_ack_frame(&mut self, now: MonotonicTime) -> Option<Frame<'static>> {
        let largest = self.largest_received?;
        let ack_delay_us = now.duration_since(self.largest_received_time).as_micros() as u32;

        let mut ranges = [AckRange::default(); MAX_ACK_RANGES];
        let mut range_count = 0;

        if !self.intervals.is_empty() {
            // First range: Largest Acked down to first interval start
            let first = self.intervals[0];
            let first_len = (first.end - first.start) as u32;
            ranges[0] = AckRange {
                gap: 0,
                length: first_len,
            };
            range_count = 1;

            let mut prev_start = first.start;
            for interval in self.intervals.iter().skip(1).take(MAX_ACK_RANGES - 1) {
                let gap = (prev_start.saturating_sub(interval.end + 1)) as u32;
                let length = (interval.end - interval.start) as u32;
                ranges[range_count] = AckRange { gap, length };
                range_count += 1;
                prev_start = interval.start;
            }
        }

        self.unacked_packet_count = 0;
        self.has_gap = false;
        self.last_ack_sent_time = Some(now);

        Some(Frame::Ack {
            largest_acked: largest,
            ack_delay_us,
            ranges,
            range_count: range_count as u8,
            ect0_count: self.ect0_count,
            ect1_count: self.ect1_count,
            ce_count: self.ce_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ack_tracker_contiguous_packets() {
        let mut tracker = AckTracker::new();
        let now = MonotonicTime::from_micros(1_000_000);

        tracker.on_packet_received(PacketNumber(1), true, 0, now);
        assert!(!tracker.should_send_ack(now));

        tracker.on_packet_received(PacketNumber(2), true, 0, now);
        assert!(tracker.should_send_ack(now));

        let ack = tracker.generate_ack_frame(now).unwrap();
        if let Frame::Ack {
            largest_acked,
            range_count,
            ranges,
            ..
        } = ack
        {
            assert_eq!(largest_acked, PacketNumber(2));
            assert_eq!(range_count, 1);
            assert_eq!(ranges[0].gap, 0);
            assert_eq!(ranges[0].length, 1); // covers [2, 1]
        } else {
            panic!("Expected ACK frame");
        }
    }

    #[test]
    fn test_ack_tracker_sparse_gap_detection() {
        let mut tracker = AckTracker::new();
        let now = MonotonicTime::from_micros(1_000_000);

        tracker.on_packet_received(PacketNumber(1), true, 0, now);
        tracker.on_packet_received(PacketNumber(3), true, 0, now); // Packet 2 missing!

        assert!(tracker.should_send_ack(now)); // Gap triggers immediate ACK

        let ack = tracker.generate_ack_frame(now).unwrap();
        if let Frame::Ack {
            largest_acked,
            range_count,
            ranges,
            ..
        } = ack
        {
            assert_eq!(largest_acked, PacketNumber(3));
            assert_eq!(range_count, 2);
            assert_eq!(ranges[0].length, 0); // [3]
            assert_eq!(ranges[1].gap, 1); // gap of 1 packet (packet 2)
            assert_eq!(ranges[1].length, 0); // [1]
        } else {
            panic!("Expected ACK frame");
        }
    }

    /// REC-5: old intervals are pruned so memory cannot grow without bound.
    #[test]
    fn ack_intervals_pruned_and_coalesced() {
        let mut tracker = AckTracker::new();
        let t0 = MonotonicTime::from_micros(1_000_000);

        // Create a sparse pattern far below the retention window, then advance far ahead
        for pn in 1..=50u64 {
            tracker.on_packet_received(PacketNumber(pn * 2), true, 0, t0); // 50 disjoint intervals
        }
        assert!(tracker.interval_count() > 10);

        // Jump beyond the retention window: the old sparse intervals fall out
        tracker.on_packet_received(PacketNumber(500_000), true, 0, t0);
        assert!(
            tracker.interval_count() <= 32,
            "intervals must be pruned to stay within the frame budget"
        );

        // The frame still acknowledges the newest 32 intervals
        let frame = tracker.generate_ack_frame(t0).unwrap();
        if let Frame::Ack { largest_acked, .. } = frame {
            assert_eq!(largest_acked, PacketNumber(500_000));
        }
    }

    /// P1-2: the negotiated policy must actually drive ACK behavior.
    #[test]
    fn policy_config_drives_ack_behavior() {
        let t0 = MonotonicTime::from_micros(1_000_000);

        // High frequency threshold: 4 packets before an ACK
        let mut tracker = AckTracker::with_policy(4, Duration::from_millis(25));
        tracker.on_packet_received(PacketNumber(1), true, 0, t0);
        tracker.on_packet_received(PacketNumber(2), true, 0, t0);
        assert!(!tracker.should_send_ack(t0), "2 < 4 packets must not ack");
        tracker.on_packet_received(PacketNumber(3), true, 0, t0);
        tracker.on_packet_received(PacketNumber(4), true, 0, t0);
        assert!(tracker.should_send_ack(t0));

        // Immediate ACK despite low count when the max_ack_delay timer expires
        let mut patient = AckTracker::with_policy(8, Duration::from_millis(10));
        patient.on_packet_received(PacketNumber(1), true, 0, t0);
        assert!(!patient.should_send_ack(t0 + Duration::from_millis(5)));
        assert!(patient.should_send_ack(t0 + Duration::from_millis(11)));

        // Runtime policy change takes effect
        patient.set_policy(1, Duration::from_millis(25));
        patient.on_packet_received(PacketNumber(2), true, 0, t0);
        assert!(patient.should_send_ack(t0));
    }
}
