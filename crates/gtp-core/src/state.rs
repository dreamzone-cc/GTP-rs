use std::collections::HashMap;
use std::net::SocketAddr;
use gtp_cc::{CubicCongestionController, PacingEngine};
use gtp_crypto::{GtpAeadProtector, PacketProtector, PlaintextProtector, ReplayWindow};
use gtp_path::{AntiAmplificationLimiter, ConnectionState, PathValidator};
use gtp_recovery::{AckTracker, LossDetector};
use gtp_scheduler::{GameScheduler, OrderedGroupReceiver};
use gtp_types::{ConnectionId, MonotonicTime, PacketNumber};

/// Hot connection state: cache-line optimized for inner send/receive loops.
pub struct ConnectionHot {
    pub connection_id: ConnectionId,
    pub next_packet_number: PacketNumber,
    pub state: ConnectionState,
    pub active_path: SocketAddr,
    pub next_send_time: MonotonicTime,
    pub loss_detector: LossDetector,
    pub ack_tracker: AckTracker,
    pub cc: CubicCongestionController,
    pub pacing: PacingEngine,
    pub scheduler: GameScheduler,
    pub ordered_groups: HashMap<u16, OrderedGroupReceiver>,
    pub replay_window: ReplayWindow,
    pub protector: Box<dyn PacketProtector>,
    pub anti_amplification: AntiAmplificationLimiter,
    pub path_validator: PathValidator,
    pub next_message_id: u64,
    pub next_order_seqs: HashMap<u16, u32>,
}

impl ConnectionHot {
    pub fn new(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
    ) -> Self {
        let protector: Box<dyn PacketProtector> = if secure {
            Box::new(GtpAeadProtector::new([0x3C; 32], [0x7E; 12]))
        } else {
            Box::new(PlaintextProtector)
        };

        let mut anti_amp = AntiAmplificationLimiter::new();
        anti_amp.mark_validated(); // Default validated for established sessions

        Self {
            connection_id: cid,
            next_packet_number: PacketNumber(1),
            state: ConnectionState::Established,
            active_path: peer_addr,
            next_send_time: MonotonicTime::ZERO,
            loss_detector: LossDetector::new(),
            ack_tracker: AckTracker::new(),
            cc: CubicCongestionController::default(),
            pacing: PacingEngine::default(),
            scheduler: GameScheduler::default(),
            ordered_groups: HashMap::new(),
            replay_window: ReplayWindow::new(),
            protector,
            anti_amplification: anti_amp,
            path_validator: PathValidator::new(peer_addr),
            next_message_id: 1,
            next_order_seqs: HashMap::new(),
        }
    }
}

/// Cold connection state: metrics, telemetry, and debugging records.
#[derive(Clone, Debug, Default)]
pub struct ConnectionCold {
    pub total_rx_packets: u64,
    pub total_tx_packets: u64,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub total_stale_drops: u64,
    pub total_retransmissions: u64,
    pub total_spurious_losses: u64,
}
