use gtp_cc::{CubicConfig, CubicCongestionController, PacingEngine, PacingEngineConfig};
use gtp_crypto::{
    derive_directional_session_keys, ratchet_key, DirectionalKeys, GtpAeadProtector,
    PlaintextProtector, Protector, ReplayWindow,
};
use gtp_path::{AntiAmplificationLimiter, ConnectionState, PathValidator};
use gtp_recovery::{AckTracker, LossDetector, OwdEstimator};
use gtp_scheduler::{GameScheduler, OrderedGroupReceiver, StateTable};
use gtp_types::{ConnectionId, MonotonicTime, OrderedGroupId, PacketNumber};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::net::SocketAddr;

/// A protocol control frame queued for transmission as a real frame — never wrapped
/// inside `Frame::Data` (Core-C1 fix). `dest` overrides the active path when the
/// frame must answer a specific source address (e.g. PathResponse to a challenger).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutgoingControlFrame {
    Ping {
        nonce: u64,
    },
    PathChallenge {
        data: [u8; 8],
        /// Routing target: a challenge probes the NEW address, so it must be sent
        /// there rather than along the current active path.
        dest: SocketAddr,
    },
    PathResponse {
        data: [u8; 8],
        dest: SocketAddr,
    },
    MtuProbe {
        probe_id: u32,
        padding_len: usize,
    },
    AckFrequency {
        ack_frequency_packets: u8,
        max_ack_delay_ms: u16,
        reorder_threshold: u8,
    },
    Close {
        error_code: u16,
        reason: String,
    },
}

/// Bounded FIFO index of recently delivered reliable message ids (ORD-4): blocks
/// duplicate delivery of `ReliableUnordered` payloads when loss detection or PTO
/// re-enqueues the same message.
#[derive(Debug, Default)]
pub struct DeliveredIndex {
    seen: FxHashSet<u64>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl DeliveredIndex {
    pub fn new(capacity: usize) -> Self {
        Self {
            seen: FxHashSet::default(),
            order: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(16),
        }
    }

    /// Returns `true` when the key was NOT seen before and is now recorded;
    /// `false` when it is a duplicate.
    pub fn insert_if_new(&mut self, key: u64) -> bool {
        if self.seen.contains(&key) {
            return false;
        }
        self.seen.insert(key);
        self.order.push_back(key);
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

/// Number of datagrams for which the pre-ratchet RX key stays acceptable (R-6).
///
/// Wide enough to absorb reordering and in-flight packets around a key rotation,
/// short enough that a leaked old key stops being useful almost immediately.
pub const RX_PREV_KEY_GRACE_PACKETS: u32 = 256;

/// Hot connection state: protocol engine fields for the inner send/receive loops.
/// Maximum number of distinct receive-side ordered groups tracked at once (FR-2).
///
/// The `group_id` is chosen by the peer and read straight off the wire, so the map
/// must be bounded or a peer can force unbounded allocation. A legitimate game uses a
/// handful of ordered channels; 256 is far above any honest load. Eviction is
/// insertion-order FIFO and costs only the freshness of the oldest group, never the
/// correctness of an active one (the same trade-off as the `StateTable` bound).
pub const MAX_ORDERED_GROUPS: usize = 256;

/// New-12: the most `PathResponse` frames one INBOUND datagram may enqueue.
///
/// The value is not a tuning knob; it is what a conforming peer can produce. The
/// outgoing path builds one `PathChallenge` per `trigger_path_challenge` call, and a
/// directed control frame takes an entire datagram to itself (the drain stops at the
/// first one), so a datagram carrying two challenges is not something this protocol's
/// own send path emits. Answering only the first bounds the packet fan-out a single
/// datagram can provoke, and costs a conforming peer nothing.
pub const MAX_PATH_RESPONSES_PER_DATAGRAM: usize = 1;

/// New-12: safety bound on `PathResponse` frames sitting in `control_queue` at once.
///
/// The answer policy replies only to the active path, so one queued response is the
/// steady state; two leaves headroom for a response left over from a path promotion
/// that has not drained yet. This is a backstop for challenges arriving faster than
/// the queue drains, not the primary defence (that is the per-datagram cap).
///
/// Overflow drops the NEW response and keeps the queued ones: the older entry may be
/// the legitimate exchange that arrived first, and a response is never retransmitted
/// anyway — `retransmittable_frames` carries data frames only — so a dropped response
/// is indistinguishable to the peer from ordinary wire loss, which the challenger
/// already recovers from by issuing a fresh `PathChallenge`.
pub const MAX_PENDING_PATH_RESPONSES: usize = 2;

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
    /// Insertion order of the live receive-side ordered groups (FR-2). Bounds the map
    /// so a peer choosing many distinct `group_id`s off the wire cannot force unbounded
    /// allocation (up to 65536 groups × 256 KB each).
    pub ordered_group_order: VecDeque<u16>,
    /// RX-side freshness table (SEM-2): drops late sequenced state at the receiver.
    pub rx_state_table: StateTable,
    /// ReliableUnordered duplicate-delivery guard (ORD-4).
    pub delivered_index: DeliveredIndex,
    pub replay_window: ReplayWindow,
    /// Seals outgoing packets (this endpoint's TX direction only).
    pub tx_protector: Protector,
    /// Opens incoming packets (the peer's TX direction).
    pub rx_protector: Protector,
    /// Previous RX key retained for a grace window across a key ratchet (P2-5).
    pub rx_protector_prev: Option<Protector>,
    /// Remaining datagrams for which `rx_protector_prev` stays acceptable.
    ///
    /// R-6: the retained key used to live for the rest of the connection, which
    /// meant a compromised pre-ratchet key could inject packets forever and the
    /// ratchet delivered no forward secrecy at all. The grace window is now finite;
    /// when it expires the protector is dropped (and its key zeroized).
    pub rx_prev_grace_packets: u32,
    pub tx_key: [u8; 32],
    pub rx_key: [u8; 32],
    pub tx_iv: [u8; 12],
    pub rx_iv: [u8; 12],
    pub key_phase: bool,
    /// Protocol control frames awaiting transmission (Core-C1 fix).
    pub control_queue: VecDeque<OutgoingControlFrame>,
    /// Set once the CLOSE frame has been encoded into an outgoing datagram.
    pub close_frame_sent: bool,
    /// Amplification budget for the **active** path only (New-8).
    pub anti_amplification: AntiAmplificationLimiter,
    /// Amplification budget for the one address currently under path challenge
    /// (New-8). `PathValidator` holds at most one pending challenge, so this is a
    /// single slot rather than a map: an unbounded `Address -> state` map would let
    /// a peer grow connection state by naming addresses, and would need an eviction
    /// policy of its own. `None` whenever no challenge is outstanding.
    ///
    /// The address is only honoured while it matches
    /// `path_validator.pending_addr()`, so probe state cannot outlive its challenge.
    pub anti_amplification_probe: Option<(SocketAddr, AntiAmplificationLimiter)>,
    pub path_validator: PathValidator,
    /// RE-1 (G1): one-way-delay variance + RFC 3550 jitter derived from the
    /// authenticated `timestamp_micros` of every received packet. `Copy`,
    /// integer-only — zero allocation on the RX path.
    pub owd: OwdEstimator,
    /// Time of the last `ControlEvent::OwdSample` emission. Bounded-rate
    /// emission (G1 design note D6): `event_queue` is an unbounded Vec, so
    /// per-packet events at 60–144 Hz would flood it. `ZERO` = never emitted.
    pub last_owd_emit: MonotonicTime,
    pub next_message_id: u64,
    pub next_order_seqs: FxHashMap<u16, u32>,
    pub packets_since_ratchet: u64,
}

impl ConnectionHot {
    #[deprecated(
        note = "Uses a hardcoded shared secret; use the handshake-driven GtpEndpoint::connect which derives real per-session keys via X25519. Only safe for offline gtp-sim testing."
    )]
    pub fn new(cid: ConnectionId, peer_addr: SocketAddr, secure: bool) -> Self {
        #[allow(deprecated)]
        Self::new_with_master_secret(
            cid,
            peer_addr,
            secure,
            b"gtp_default_session_master_secret_2026",
            true,
        )
    }

    #[deprecated(
        note = "Uses a hardcoded shared secret; use the handshake-driven GtpEndpoint::connect which derives real per-session keys via X25519. Only safe for offline gtp-sim testing."
    )]
    pub fn new_with_master_secret(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
        master_secret: &[u8],
        as_client: bool,
    ) -> Self {
        let (tx, rx, tx_key, rx_key, tx_iv, rx_iv) = if secure {
            // SEC-1: even the legacy path derives per-direction keys so packet
            // number N never collides on the same (key, nonce) in both directions.
            let dirs = derive_directional_session_keys(master_secret, cid);
            let (tx, rx) = if as_client {
                (
                    (dirs.client_tx_key, dirs.client_tx_iv),
                    (dirs.server_tx_key, dirs.server_tx_iv),
                )
            } else {
                (
                    (dirs.server_tx_key, dirs.server_tx_iv),
                    (dirs.client_tx_key, dirs.client_tx_iv),
                )
            };
            (
                Protector::Aead(GtpAeadProtector::new(tx.0, tx.1)),
                Protector::Aead(GtpAeadProtector::new(rx.0, rx.1)),
                tx.0,
                rx.0,
                tx.1,
                rx.1,
            )
        } else {
            (
                Protector::Plaintext(PlaintextProtector),
                Protector::Plaintext(PlaintextProtector),
                [0u8; 32],
                [0u8; 32],
                [0u8; 12],
                [0u8; 12],
            )
        };

        Self::build(
            cid, peer_addr, tx, rx, None, tx_key, rx_key, tx_iv, rx_iv, false, true,
        )
    }

    /// Builds a connection from directional handshake keys (SEC-1): `as_client`
    /// selects which direction this endpoint seals with.
    pub fn new_with_directional_keys(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        keys: &DirectionalKeys,
        as_client: bool,
        pre_validated: bool,
    ) -> Self {
        let ((tx_key, tx_iv), (rx_key, rx_iv)) = keys.for_role(as_client);
        Self::build(
            cid,
            peer_addr,
            Protector::Aead(GtpAeadProtector::new(tx_key, tx_iv)),
            Protector::Aead(GtpAeadProtector::new(rx_key, rx_iv)),
            None,
            tx_key,
            rx_key,
            tx_iv,
            rx_iv,
            false,
            pre_validated,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        tx_protector: Protector,
        rx_protector: Protector,
        rx_protector_prev: Option<Protector>,
        tx_key: [u8; 32],
        rx_key: [u8; 32],
        tx_iv: [u8; 12],
        rx_iv: [u8; 12],
        key_phase: bool,
        pre_validated: bool,
    ) -> Self {
        let config = crate::control::config::GtpConfig::default();
        Self::build_with_config(
            cid,
            peer_addr,
            tx_protector,
            rx_protector,
            rx_protector_prev,
            tx_key,
            rx_key,
            tx_iv,
            rx_iv,
            key_phase,
            pre_validated,
            &config,
        )
    }

    /// Full constructor honoring `GtpConfig` (P1-2: the tuning surface is live).
    #[allow(clippy::too_many_arguments)]
    pub fn build_with_config(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        tx_protector: Protector,
        rx_protector: Protector,
        rx_protector_prev: Option<Protector>,
        tx_key: [u8; 32],
        rx_key: [u8; 32],
        tx_iv: [u8; 12],
        rx_iv: [u8; 12],
        key_phase: bool,
        pre_validated: bool,
        config: &crate::control::config::GtpConfig,
    ) -> Self {
        let mut anti_amp = AntiAmplificationLimiter::with_factor(config.anti_amplification_factor);
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
            ack_tracker: AckTracker::with_policy(
                config.ack_frequency_packets,
                config.max_ack_delay,
            ),
            cc: CubicCongestionController::with_config(CubicConfig {
                smss: config.smss,
                initial_cwnd_packets: config.initial_cwnd_packets,
                min_cwnd_packets: config.min_cwnd_packets,
                beta: config.cubic_beta,
                c: config.cubic_c,
                pacing_gain: config.pacing_gain,
            }),
            pacing: PacingEngine::with_config(PacingEngineConfig {
                max_burst_bytes: config.max_pacing_burst_bytes,
            }),
            // Per-tier caps are enforced individually in a later phase; the scheduler
            // currently takes one cap per tier array position via its constructor.
            scheduler: GameScheduler::new(
                *config
                    .max_queue_bytes_per_tier
                    .iter()
                    .max()
                    .unwrap_or(&(512 * 1024)),
            ),
            ordered_groups: FxHashMap::default(),
            ordered_group_order: VecDeque::new(),
            rx_state_table: StateTable::new(),
            delivered_index: DeliveredIndex::new(4096),
            replay_window: ReplayWindow::new(),
            tx_protector,
            rx_protector,
            rx_protector_prev,
            rx_prev_grace_packets: 0,
            tx_key,
            rx_key,
            tx_iv,
            rx_iv,
            key_phase,
            control_queue: VecDeque::new(),
            close_frame_sent: false,
            anti_amplification: anti_amp,
            anti_amplification_probe: None,
            path_validator: PathValidator::new(),
            owd: OwdEstimator::new(),
            last_owd_emit: MonotonicTime::ZERO,
            next_message_id: 1,
            next_order_seqs: FxHashMap::default(),
            packets_since_ratchet: 0,
        }
    }

    /// Returns the receive-side ordered group for `group_id`, creating it if new
    /// and evicting the least recently used group once `MAX_ORDERED_GROUPS` is
    /// reached (FR-2 bounded; FU-5 upgrades the policy from FIFO-by-creation
    /// to LRU).
    ///
    /// Every access — not just creation — refreshes the group's position in
    /// the eviction order, so a long-lived but active group stays resident
    /// while an idle one is trimmed first. Eviction drops that group's reorder
    /// buffer; if the evicted group later receives more traffic it is
    /// recreated fresh.
    pub fn ordered_group_mut(&mut self, group_id: OrderedGroupId) -> &mut OrderedGroupReceiver {
        let key = group_id.as_u16();
        if self.ordered_groups.contains_key(&key) {
            // FU-5: touch — move to the most-recently-used end so eviction
            // trims the least recently USED group, not the oldest created one.
            self.ordered_group_order.retain(|k| *k != key);
            self.ordered_group_order.push_back(key);
        } else {
            while self.ordered_group_order.len() >= MAX_ORDERED_GROUPS {
                match self.ordered_group_order.pop_front() {
                    Some(lru) => {
                        self.ordered_groups.remove(&lru);
                    }
                    None => break,
                }
            }
            self.ordered_groups
                .insert(key, OrderedGroupReceiver::new(group_id));
            self.ordered_group_order.push_back(key);
        }
        self.ordered_groups
            .get_mut(&key)
            .expect("group is present: just inserted or already tracked")
    }

    /// Rotates BOTH direction keys in lockstep (SEC-6 / P2-5) and retains the old RX
    /// key for a grace window. Both peers must invoke this at the same logical point
    /// (documented limitation until a wire-level key update frame exists).
    pub fn ratchet_session_key(&mut self) {
        if matches!(self.tx_protector, Protector::Plaintext(_)) {
            return;
        }
        let new_tx_key = ratchet_key(&self.tx_key, self.connection_id);
        let new_rx_key = ratchet_key(&self.rx_key, self.connection_id);
        let new_tx_iv = gtp_crypto::derive_session_keys(&new_tx_key, self.connection_id).1;
        let new_rx_iv = gtp_crypto::derive_session_keys(&new_rx_key, self.connection_id).1;

        let old_rx = std::mem::replace(
            &mut self.rx_protector,
            Protector::Aead(GtpAeadProtector::new(new_rx_key, new_rx_iv)),
        );
        self.rx_protector_prev = Some(old_rx);
        self.rx_prev_grace_packets = RX_PREV_KEY_GRACE_PACKETS;
        self.tx_protector = Protector::Aead(GtpAeadProtector::new(new_tx_key, new_tx_iv));
        self.tx_key = new_tx_key;
        self.rx_key = new_rx_key;
        self.tx_iv = new_tx_iv;
        self.rx_iv = new_rx_iv;
        self.key_phase = !self.key_phase;
        self.packets_since_ratchet = 0;
    }

    /// Consumes one unit of the post-ratchet grace window and retires the previous
    /// RX key when it runs out (R-6). Called once per processed datagram in each
    /// direction so the window covers reordering across roughly one RTT of traffic.
    pub fn tick_rx_key_grace(&mut self) {
        if self.rx_protector_prev.is_none() {
            return;
        }
        self.rx_prev_grace_packets = self.rx_prev_grace_packets.saturating_sub(1);
        if self.rx_prev_grace_packets == 0 {
            // Dropping the protector zeroizes the retired key material.
            self.rx_protector_prev = None;
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
    pub total_dropped_frames: u64,
    pub total_duplicate_drops: u64,
}
