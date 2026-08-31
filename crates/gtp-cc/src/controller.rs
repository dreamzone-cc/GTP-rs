use gtp_recovery::{AckEvent, LossEvent};
use gtp_types::{Duration, MonotonicTime, PacketNumber};

/// Generic interface implemented by all GTP congestion control algorithms.
pub trait CongestionController: Send + Sync {
    fn on_packet_sent(
        &mut self,
        packet_number: PacketNumber,
        bytes: usize,
        send_time: MonotonicTime,
    );
    fn on_ack(&mut self, ack_event: &AckEvent, now: MonotonicTime);
    fn on_loss(&mut self, loss_event: &LossEvent, now: MonotonicTime);
    fn on_ecn(&mut self, ce_count: u32, now: MonotonicTime);
    fn on_rtt(&mut self, rtt_sample: Duration);
    fn on_timeout(&mut self, now: MonotonicTime);
    fn cwnd(&self) -> u64;
    fn pacing_rate(&self) -> u64; // Bytes per second
                                  // FR-3: in-flight accounting is owned solely by the loss detector
                                  // (`LossDetector::inflight_bytes`). The controller no longer mirrors it, so the two
                                  // can never diverge (the R-1 regression). Callers read in-flight from the recovery
                                  // layer, which is the single source of truth derived from the outstanding set.
}
