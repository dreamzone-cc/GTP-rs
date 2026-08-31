use crate::control::config::GtpConfig;
use crate::control::events::ControlEvent;
use crate::control::metrics::DetailedMetrics;
use crate::state::{ConnectionCold, ConnectionHot, OutgoingControlFrame};
use gtp_cc::calculate_backpressure;
use gtp_path::ConnectionState;
use gtp_types::{MonotonicTime, Result};
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
        _now: MonotonicTime,
    ) -> Result<()> {
        // REC-7: the negotiated policy takes effect locally as well as remotely.
        self.config.ack_frequency_packets = ack_frequency_packets;
        self.config.max_ack_delay = gtp_types::Duration::from_millis(max_ack_delay_ms as u64);
        self.config.ack_reorder_threshold = reorder_threshold;
        self.hot.ack_tracker.set_policy(
            ack_frequency_packets,
            gtp_types::Duration::from_millis(max_ack_delay_ms as u64),
        );

        self.hot
            .control_queue
            .push_back(OutgoingControlFrame::AckFrequency {
                ack_frequency_packets,
                max_ack_delay_ms,
                reorder_threshold,
            });
        Ok(())
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

        self.hot
            .control_queue
            .push_back(OutgoingControlFrame::PathChallenge {
                data: nonce,
                dest: new_addr,
            });
        Ok(())
    }

    /// Enqueue an MTU probe frame to test Path MTU expansion.
    pub fn trigger_mtu_probe(
        &mut self,
        probe_id: u32,
        target_size: usize,
        _now: MonotonicTime,
    ) -> Result<()> {
        self.hot
            .control_queue
            .push_back(OutgoingControlFrame::MtuProbe {
                probe_id,
                padding_len: target_size.saturating_sub(32),
            });
        Ok(())
    }

    /// Send a keepalive ping frame.
    pub fn send_ping(&mut self, nonce: u64, _now: MonotonicTime) -> Result<()> {
        self.hot
            .control_queue
            .push_back(OutgoingControlFrame::Ping { nonce });
        Ok(())
    }

    /// Initiate graceful session closing by emitting a CLOSE frame and transitioning to Draining.
    ///
    /// Core-C1: the CLOSE frame is queued BEFORE the state transition so the TX
    /// pipeline can still produce a datagram carrying it.
    pub fn graceful_close(
        &mut self,
        error_code: u16,
        reason: &'static str,
        _now: MonotonicTime,
    ) -> Result<()> {
        self.hot
            .control_queue
            .push_back(OutgoingControlFrame::Close {
                error_code,
                reason: reason.to_string(),
            });

        let old_state = self.hot.state;
        self.hot.state.transition_to(ConnectionState::Draining)?;
        self.event_queue.push(ControlEvent::StateChanged {
            old_state,
            new_state: ConnectionState::Draining,
        });
        Ok(())
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

    /// Rotates the active session AEAD encryption keys in both directions (Key Ratchet).
    pub fn ratchet_key(&mut self) {
        self.hot.ratchet_session_key();
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
            // FR-3: single source of truth for in-flight bytes is the loss detector.
            inflight_bytes: self.hot.loss_detector.inflight_bytes(),
            pacing_rate_bps: gtp_cc::CongestionController::pacing_rate(&self.hot.cc),
            pacing_tokens_remaining: self.hot.pacing.tokens_bytes(),
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
