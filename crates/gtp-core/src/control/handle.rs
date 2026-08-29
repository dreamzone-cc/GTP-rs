use crate::control::config::GtpConfig;
use crate::control::events::ControlEvent;
use crate::control::metrics::DetailedMetrics;
use crate::state::{ConnectionCold, ConnectionHot};
use gtp_cc::calculate_backpressure;
use gtp_path::ConnectionState;
use gtp_scheduler::SchedulableItem;
use gtp_types::{MessageClass, MessageId, MonotonicTime, PriorityTier, Result};
use gtp_wire::Frame;
use std::net::SocketAddr;

/// Dedicated control handle providing full administrative and tuning access to an active GTP connection.
pub struct ConnectionControl<'a> {
    pub(crate) hot: &'a mut ConnectionHot,
    pub(crate) cold: &'a mut ConnectionCold,
    pub(crate) config: &'a mut GtpConfig,
    pub(crate) event_queue: &'a mut Vec<ControlEvent>,
}

impl<'a> ConnectionControl<'a> {
    pub fn new(
        hot: &'a mut ConnectionHot,
        cold: &'a mut ConnectionCold,
        config: &'a mut GtpConfig,
        event_queue: &'a mut Vec<ControlEvent>,
    ) -> Self {
        Self {
            hot,
            cold,
            config,
            event_queue,
        }
    }

    /// Dynamically adjust the remote peer's ACK frequency by enqueuing an ACK_FREQUENCY frame.
    pub fn set_ack_frequency(
        &mut self,
        ack_frequency_packets: u8,
        max_ack_delay_ms: u16,
        reorder_threshold: u8,
        now: MonotonicTime,
    ) -> Result<()> {
        self.config.ack_frequency_packets = ack_frequency_packets;
        self.config.max_ack_delay = gtp_types::Duration::from_millis(max_ack_delay_ms as u64);
        self.config.ack_reorder_threshold = reorder_threshold;

        let frame = Frame::AckFrequency {
            ack_frequency_packets,
            max_ack_delay_ms,
            reorder_threshold,
        };

        let mut buf = [0u8; 16];
        let written = frame.encode(&mut buf)?;

        let item = SchedulableItem {
            message_id: MessageId(self.hot.next_message_id),
            class: MessageClass::Unreliable,
            priority: PriorityTier::P0Control,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: buf[..written].to_vec(),
        };
        self.hot.next_message_id += 1;
        self.hot.scheduler.enqueue(item, now)
    }

    /// Trigger path validation and migration to a new remote address.
    pub fn trigger_path_challenge(
        &mut self,
        new_addr: SocketAddr,
        nonce: [u8; 8],
        now: MonotonicTime,
    ) -> Result<()> {
        self.hot
            .path_validator
            .start_challenge(new_addr, nonce, now);

        let frame = Frame::PathChallenge { data: nonce };
        let mut buf = [0u8; 16];
        let written = frame.encode(&mut buf)?;

        let item = SchedulableItem {
            message_id: MessageId(self.hot.next_message_id),
            class: MessageClass::Unreliable,
            priority: PriorityTier::P0Control,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: buf[..written].to_vec(),
        };
        self.hot.next_message_id += 1;
        self.hot.scheduler.enqueue(item, now)
    }

    /// Enqueue an MTU probe frame to test Path MTU expansion.
    pub fn trigger_mtu_probe(
        &mut self,
        probe_id: u32,
        target_size: usize,
        now: MonotonicTime,
    ) -> Result<()> {
        let frame = Frame::MtuProbe {
            probe_id,
            padding_len: target_size.saturating_sub(32),
        };
        let mut buf = vec![0u8; target_size];
        let written = frame.encode(&mut buf)?;

        let item = SchedulableItem {
            message_id: MessageId(self.hot.next_message_id),
            class: MessageClass::Unreliable,
            priority: PriorityTier::P0Control,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: buf[..written].to_vec(),
        };
        self.hot.next_message_id += 1;
        self.hot.scheduler.enqueue(item, now)
    }

    /// Send a keepalive ping frame.
    pub fn send_ping(&mut self, nonce: u64, now: MonotonicTime) -> Result<()> {
        let frame = Frame::Ping { nonce };
        let mut buf = [0u8; 16];
        let written = frame.encode(&mut buf)?;

        let item = SchedulableItem {
            message_id: MessageId(self.hot.next_message_id),
            class: MessageClass::Unreliable,
            priority: PriorityTier::P0Control,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: buf[..written].to_vec(),
        };
        self.hot.next_message_id += 1;
        self.hot.scheduler.enqueue(item, now)
    }

    /// Initiate graceful session closing by emitting a CLOSE frame and transitioning to Draining.
    pub fn graceful_close(
        &mut self,
        error_code: u16,
        reason: &'static str,
        now: MonotonicTime,
    ) -> Result<()> {
        let old_state = self.hot.state;
        self.hot.state.transition_to(ConnectionState::Draining)?;
        self.event_queue.push(ControlEvent::StateChanged {
            old_state,
            new_state: ConnectionState::Draining,
        });

        let frame = Frame::Close { error_code, reason };
        let mut buf = [0u8; 256];
        let written = frame.encode(&mut buf)?;

        let item = SchedulableItem {
            message_id: MessageId(self.hot.next_message_id),
            class: MessageClass::Unreliable,
            priority: PriorityTier::P0Control,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: buf[..written].to_vec(),
        };
        self.hot.next_message_id += 1;
        self.hot.scheduler.enqueue(item, now)
    }

    /// Forcefully terminate connection immediately without draining.
    pub fn force_close(&mut self, _error_code: u16) -> Result<()> {
        let old_state = self.hot.state;
        self.hot.state.transition_to(ConnectionState::Closed)?;
        self.event_queue.push(ControlEvent::StateChanged {
            old_state,
            new_state: ConnectionState::Closed,
        });
        Ok(())
    }

    /// Take a comprehensive telemetry snapshot of all protocol subsystems.
    pub fn query_metrics(&self, now: MonotonicTime) -> DetailedMetrics {
        let rtt = self.hot.loss_detector.rtt_stats;
        let eff_queue = self.hot.scheduler.effective_queue_bytes(now);
        let backpressure = calculate_backpressure(
            eff_queue,
            gtp_cc::CongestionController::cwnd(&self.hot.cc),
            rtt.smoothed_rtt,
            rtt.min_rtt,
        );

        DetailedMetrics {
            latest_rtt: rtt.latest_rtt,
            smoothed_rtt: rtt.smoothed_rtt,
            rttvar: rtt.rttvar,
            min_rtt: rtt.min_rtt,
            pto_duration: rtt.pto_duration(),

            cwnd_bytes: gtp_cc::CongestionController::cwnd(&self.hot.cc),
            inflight_bytes: gtp_cc::CongestionController::inflight(&self.hot.cc),
            pacing_rate_bps: gtp_cc::CongestionController::pacing_rate(&self.hot.cc),
            pacing_tokens_remaining: 0,
            backpressure,

            queue_bytes_per_tier: [0, 0, 0, 0, 0],
            effective_queue_bytes: eff_queue,
            total_stale_drops: self.cold.total_stale_drops,

            total_tx_packets: self.cold.total_tx_packets,
            total_rx_packets: self.cold.total_rx_packets,
            total_tx_bytes: self.cold.total_tx_bytes,
            total_rx_bytes: self.cold.total_rx_bytes,
            total_retransmissions: self.cold.total_retransmissions,
            total_corrupted_packets: self.cold.total_corrupted_packets,
            pto_count: self.hot.loss_detector.pto_count,

            ecn_ect0_count: 0,
            ecn_ect1_count: 0,
            ecn_ce_count: 0,
        }
    }

    /// Drain all queued control events emitted since last drain.
    pub fn drain_events(&mut self) -> Vec<ControlEvent> {
        std::mem::take(self.event_queue)
    }
}
