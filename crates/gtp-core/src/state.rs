use gtp_cc::{CubicCongestionController, PacingEngine};
use gtp_crypto::{
    derive_session_keys, GtpAeadProtector, PlaintextProtector, Protector, ReplayWindow,
};
use gtp_path::{AntiAmplificationLimiter, ConnectionState, PathValidator};
use gtp_recovery::{AckTracker, LossDetector};
use gtp_scheduler::{GameScheduler, OrderedGroupReceiver};
use gtp_types::{ConnectionId, MonotonicTime, PacketNumber};
use rustc_hash::FxHashMap;
use std::net::SocketAddr;

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
    pub ordered_groups: FxHashMap<u16, OrderedGroupReceiver>,
    pub replay_window: ReplayWindow,
    pub protector: Protector,
    pub anti_amplification: AntiAmplificationLimiter,
    pub path_validator: PathValidator,
    pub next_message_id: u64,
    pub next_order_seqs: FxHashMap<u16, u32>,
}

impl ConnectionHot {
    pub fn new(cid: ConnectionId, peer_addr: SocketAddr, secure: bool) -> Self {
        Self::new_with_master_secret(
            cid,
            peer_addr,
            secure,
            b"gtp_default_session_master_secret_2026",
        )
    }

    pub fn new_with_master_secret(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
        master_secret: &[u8],
    ) -> Self {
        let protector = if secure {
            let (key, iv) = derive_session_keys(master_secret, cid);
            Protector::Aead(GtpAeadProtector::new(key, iv))
        } else {
            Protector::Plaintext(PlaintextProtector)
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
            ordered_groups: FxHashMap::default(),
            replay_window: ReplayWindow::new(),
            protector,
            anti_amplification: anti_amp,
            path_validator: PathValidator::new(peer_addr),
            next_message_id: 1,
            next_order_seqs: FxHashMap::default(),
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
    pub total_corrupted_packets: u64,
}
