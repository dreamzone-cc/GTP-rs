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
    pub current_session_key: [u8; 32],
    pub packets_since_ratchet: u64,
}

impl ConnectionHot {
    #[deprecated(
        note = "Uses a hardcoded shared secret; use the handshake-driven GtpEndpoint::connect which derives real per-session keys via X25519. Only safe for offline gtp-sim testing with secure=false."
    )]
    pub fn new(cid: ConnectionId, peer_addr: SocketAddr, secure: bool) -> Self {
        #[allow(deprecated)]
        Self::new_with_master_secret(
            cid,
            peer_addr,
            secure,
            b"gtp_default_session_master_secret_2026",
        )
    }

    pub fn new_with_session_keys(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        key: [u8; 32],
        iv: [u8; 12],
        pre_validated: bool,
    ) -> Self {
        let protector = Protector::Aead(GtpAeadProtector::new(key, iv));
        let mut anti_amp = AntiAmplificationLimiter::new();
        if pre_validated {
            anti_amp.mark_validated();
        }

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
            current_session_key: key,
            packets_since_ratchet: 0,
        }
    }

    /// Rotates the session AEAD encryption key for forward secrecy (Key Phase ratchet).
    pub fn ratchet_session_key(&mut self) {
        let next_key = gtp_crypto::ratchet_key(&self.current_session_key, self.connection_id);
        let (_, iv) = derive_session_keys(&next_key, self.connection_id);
        self.protector = Protector::Aead(GtpAeadProtector::new(next_key, iv));
        self.current_session_key = next_key;
        self.packets_since_ratchet = 0;
    }

    #[deprecated(
        note = "Uses a hardcoded shared secret; use the handshake-driven GtpEndpoint::connect which derives real per-session keys via X25519. Only safe for offline gtp-sim testing with secure=false."
    )]
    pub fn new_with_master_secret(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
        master_secret: &[u8],
    ) -> Self {
        let (protector, key) = if secure {
            let (key, iv) = derive_session_keys(master_secret, cid);
            (Protector::Aead(GtpAeadProtector::new(key, iv)), key)
        } else {
            (Protector::Plaintext(PlaintextProtector), [0u8; 32])
        };

        let anti_amp = AntiAmplificationLimiter::new();

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
            current_session_key: key,
            packets_since_ratchet: 0,
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
