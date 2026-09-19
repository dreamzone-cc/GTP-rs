use gtp_cc::{CubicConfig, CubicCongestionController, PacingEngine, PacingEngineConfig};
use gtp_crypto::{
    derive_directional_session_keys, ratchet_key, DirectionalKeys, GtpAeadProtector, Protector,
    ReplayWindow,
};
use gtp_path::{AntiAmplificationLimiter, ConnectionState, PathValidator};
use gtp_recovery::{AckTracker, LossDetector, OwdEstimator};
use gtp_scheduler::{GameScheduler, OrderedGroupReceiver, StateTable};
use gtp_types::{ConnectionId, MonotonicTime, OrderedGroupId, PacketNumber};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::net::SocketAddr;
use zeroize::Zeroizing;

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
    /// F2: wire-level key-phase negotiation. The initiator ratchets then
    /// enqueues this frame (sealed under the NEW key).
    KeyUpdate {
        next_phase: u64,
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

/// Explicit master secret for OFFLINE SIMULATION, EXAMPLES, AND TESTS ONLY.
///
/// This is not a hidden default: every caller must name it explicitly at the
/// call site, so a production path cannot silently end up on a compile-time
/// shared secret. Production connections derive per-session keys through the
/// X25519 handshake (`GtpEndpoint::connect` + `new_with_directional_keys`).
pub const OFFLINE_SIM_MASTER_SECRET: &[u8] = b"gtp_offline_sim_master_secret_NOT_FOR_PRODUCTION";

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
    /// Live receive-side ordered groups, keyed by `group_id` (FR-2). PRIVATE:
    /// bounded by `MAX_ORDERED_GROUPS`; mutation happens only through
    /// [`ConnectionHot::ordered_group_mut`].
    ordered_groups: FxHashMap<u16, OrderedGroupReceiver>,
    /// FU-5/O(1) LRU: monotonic access generation per live group. A touch is
    /// one hash insert; eviction (only when a NEW group arrives at the cap)
    /// scans for the minimum generation. This replaces the per-frame
    /// `VecDeque::retain` scan on the RX hot path.
    group_gens: FxHashMap<u16, u64>,
    gen_counter: u64,
    /// RX-side freshness table (SEM-2): drops late sequenced state at the receiver.
    pub rx_state_table: StateTable,
    /// CORE-2/F1: bounded reassembly of fragmented reliable messages.
    pub reassembler: crate::fragment::MessageReassembler,
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
    /// Session key material, wrapped so retired/live keys are zeroized when
    /// overwritten or dropped (the plain-array fields the R-6 comment relied
    /// on were never wiped — only the protector's copies were).
    pub tx_key: Zeroizing<[u8; 32]>,
    pub rx_key: Zeroizing<[u8; 32]>,
    pub tx_iv: Zeroizing<[u8; 12]>,
    pub rx_iv: Zeroizing<[u8; 12]>,
    /// Wire key-phase bit (header flag) — flips on every ratchet.
    pub key_phase: bool,
    /// Monotonic ratchet counter feeding the key derivation (distinct material
    /// per phase; the wire flag above cannot distinguish 2k rotations apart).
    pub key_phase_counter: u64,
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
    /// RT-3 (G3 prelude): local receive time of the last AUTHENTICATED
    /// datagram. Measurement-basis age in this endpoint's own clock —
    /// set post-auth only (INV-3), beside the OWD feed.
    pub last_rx_time: Option<MonotonicTime>,
    pub next_message_id: u64,
    pub next_order_seqs: FxHashMap<u16, u32>,
    pub packets_since_ratchet: u64,
    /// F2: a KeyUpdate frame is queued; ratchet the keys AFTER it leaves.
    pub pending_ratchet: bool,
    /// F3: adaptive route switching controller (None = routing disabled).
    pub route_controller: Option<gtp_route::SwitchController>,
    /// F3: path_id → remote address mapping for the controller's decisions.
    pub route_paths: Vec<(u32, std::net::SocketAddr)>,
    /// F3: latest measured PathStats per candidate (indexed by position in
    /// route_paths — the application feeds these from telemetry/reports).
    pub route_stats: Vec<gtp_route::PathStats>,
}

impl ConnectionHot {
    /// Legacy static-secret constructor. The master secret must be supplied
    /// EXPLICITLY — there is no embedded default (a compile-time shared
    /// secret on a production-reachable path let anyone with the source
    /// derive every connection's keys).
    ///
    /// For offline simulation/examples/tests, pass
    /// [`OFFLINE_SIM_MASTER_SECRET`]; production must use the handshake-driven
    /// [`ConnectionHot::new_with_directional_keys`] instead.
    #[deprecated(
        note = "Static shared secret; use the handshake-driven GtpEndpoint::connect which derives real per-session keys via X25519. Only safe for offline gtp-sim testing."
    )]
    pub fn new_with_master_secret(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
        master_secret: &[u8],
        as_client: bool,
    ) -> Self {
        let (tx, rx, tx_key, rx_key, tx_iv, rx_iv) = {
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
            let (tx_p, rx_p) = if secure {
                // Bound each key to this CID so the nonce-space contract is
                // enforced structurally (see GtpAeadProtector::new_for_cid).
                (
                    Protector::Aead(GtpAeadProtector::new_for_cid(tx.0, tx.1, cid)),
                    Protector::Aead(GtpAeadProtector::new_for_cid(rx.0, rx.1, cid)),
                )
            } else {
                // Insecure plaintext mode exists only in test/sim builds; in
                // production builds (feature off) the connection is sealed
                // anyway — never silently plaintext.
                #[cfg(any(test, feature = "insecure-plaintext"))]
                {
                    (
                        Protector::Plaintext(gtp_crypto::PlaintextProtector),
                        Protector::Plaintext(gtp_crypto::PlaintextProtector),
                    )
                }
                #[cfg(not(any(test, feature = "insecure-plaintext")))]
                {
                    (
                        Protector::Aead(GtpAeadProtector::new_for_cid(tx.0, tx.1, cid)),
                        Protector::Aead(GtpAeadProtector::new_for_cid(rx.0, rx.1, cid)),
                    )
                }
            };
            (tx_p, rx_p, tx.0, rx.0, tx.1, rx.1)
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
            Protector::Aead(GtpAeadProtector::new_for_cid(tx_key, tx_iv, cid)),
            Protector::Aead(GtpAeadProtector::new_for_cid(rx_key, rx_iv, cid)),
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
            // P1-2/QoS: the per-tier byte budgets from `GtpConfig` are now
            // enforced per tier — tier 0 keeps its latency-critical 64 KB bound
            // instead of inheriting the largest tier's 1 MB ceiling.
            scheduler: GameScheduler::with_per_tier_byte_caps(config.max_queue_bytes_per_tier),
            ordered_groups: FxHashMap::default(),
            group_gens: FxHashMap::default(),
            gen_counter: 0,
            rx_state_table: StateTable::new(),
            reassembler: crate::fragment::MessageReassembler::new(),
            delivered_index: DeliveredIndex::new(4096),
            replay_window: ReplayWindow::new(),
            tx_protector,
            rx_protector,
            rx_protector_prev,
            rx_prev_grace_packets: 0,
            tx_key: Zeroizing::new(tx_key),
            rx_key: Zeroizing::new(rx_key),
            tx_iv: Zeroizing::new(tx_iv),
            rx_iv: Zeroizing::new(rx_iv),
            key_phase,
            key_phase_counter: 0,
            control_queue: VecDeque::new(),
            close_frame_sent: false,
            anti_amplification: anti_amp,
            anti_amplification_probe: None,
            path_validator: PathValidator::new(),
            owd: OwdEstimator::new(),
            last_owd_emit: MonotonicTime::ZERO,
            last_rx_time: None,
            next_message_id: 1,
            next_order_seqs: FxHashMap::default(),
            packets_since_ratchet: 0,
            pending_ratchet: false,
            route_controller: None,
            route_paths: Vec::new(),
            route_stats: Vec::new(),
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
        if let Some(gen_slot) = self.group_gens.get_mut(&key) {
            // FU-5 touch: O(1) — bump the access generation.
            self.gen_counter += 1;
            *gen_slot = self.gen_counter;
        } else {
            // New group: evict the least-recently-USED one only when the cap
            // is reached (rare path — once per distinct new group).
            while self.ordered_groups.len() >= MAX_ORDERED_GROUPS {
                let Some((&lru, _)) = self.group_gens.iter().min_by_key(|(_, g)| **g) else {
                    break;
                };
                self.ordered_groups.remove(&lru);
                self.group_gens.remove(&lru);
            }
            self.gen_counter += 1;
            self.group_gens.insert(key, self.gen_counter);
            self.ordered_groups
                .insert(key, OrderedGroupReceiver::new(group_id));
        }
        self.ordered_groups
            .get_mut(&key)
            .expect("group is present: just inserted or already tracked")
    }

    /// Number of live receive-side ordered groups (FR-2 bounded by
    /// `MAX_ORDERED_GROUPS`).
    pub fn ordered_group_count(&self) -> usize {
        self.ordered_groups.len()
    }

    /// Whether `group_id` is currently tracked as a live ordered group.
    pub fn ordered_group_contains(&self, group_id: impl Into<u16>) -> bool {
        self.ordered_groups.contains_key(&group_id.into())
    }

    /// Rotates BOTH direction keys in lockstep (SEC-6 / P2-5) and retains the old RX
    /// key for a grace window. Both peers must invoke this at the same logical point
    /// (documented limitation until a wire-level key update frame exists).
    pub fn ratchet_session_key(&mut self) {
        // R-6/P2-5: rotating again while the previous grace window is still
        // open would retire the pre-ratchet key while its packets may still be
        // in flight (and strand a lockstep peer one ratchet behind — it would
        // fail BOTH of its protectors and drop all traffic with no recovery
        // path). Refuse instead.
        if self.rx_protector_prev.is_some() && self.rx_prev_grace_packets > 0 {
            return;
        }
        #[cfg(any(test, feature = "insecure-plaintext"))]
        if matches!(self.tx_protector, Protector::Plaintext(_)) {
            return;
        }

        // Phase counter is bound into the derivation: distinct material per
        // phase, and the base IV rotates together with the key (distinct label)
        // so the (key, IV) nonce pair stays coherent across phases.
        let next_phase = self.key_phase_counter + 1;
        let (new_tx_key, new_tx_iv) = ratchet_key(&self.tx_key, self.connection_id, next_phase);
        let (new_rx_key, new_rx_iv) = ratchet_key(&self.rx_key, self.connection_id, next_phase);

        let old_rx = std::mem::replace(
            &mut self.rx_protector,
            Protector::Aead(GtpAeadProtector::new_for_cid(
                new_rx_key,
                new_rx_iv,
                self.connection_id,
            )),
        );
        self.rx_protector_prev = Some(old_rx);
        self.rx_prev_grace_packets = RX_PREV_KEY_GRACE_PACKETS;
        self.tx_protector = Protector::Aead(GtpAeadProtector::new_for_cid(
            new_tx_key,
            new_tx_iv,
            self.connection_id,
        ));
        // Assigning through Zeroizing wipes the retired key bytes in place.
        *self.tx_key = new_tx_key;
        *self.rx_key = new_rx_key;
        *self.tx_iv = new_tx_iv;
        *self.rx_iv = new_rx_iv;
        self.key_phase = !self.key_phase;
        self.key_phase_counter = next_phase;
        self.packets_since_ratchet = 0;
    }

    /// F2: wire-negotiated ratchet — enqueues the KeyUpdate frame FIRST
    /// (sealed under the CURRENT key the peer still holds), then ratchets
    /// on the NEXT produce call AFTER the frame has actually left. Returns
    /// the announced phase (0 = refused by the grace-window guard).
    pub fn ratchet_and_announce(&mut self) -> u64 {
        // Guard: same conditions as ratchet_session_key (grace window, plaintext).
        if self.rx_protector_prev.is_some() && self.rx_prev_grace_packets > 0 {
            return 0;
        }
        #[cfg(any(test, feature = "insecure-plaintext"))]
        if matches!(self.tx_protector, Protector::Plaintext(_)) {
            return 0;
        }
        let next_phase = self.key_phase_counter + 1;
        self.control_queue
            .push_back(OutgoingControlFrame::KeyUpdate { next_phase });
        // Deferred: the actual ratchet fires on the produce call AFTER the
        // KeyUpdate datagram has been sealed (under the old key) and sent.
        self.pending_ratchet = true;
        next_phase
    }

    /// F2: fires the deferred ratchet. Called by produce_outgoing_datagram
    /// after the datagram carrying the KeyUpdate control frame was accepted.
    pub fn flush_pending_ratchet(&mut self) {
        if self.pending_ratchet {
            self.pending_ratchet = false;
            self.ratchet_session_key();
        }
    }

    /// F2: responds to a received KeyUpdate — ratchets if the announced
    /// phase is strictly newer than ours; a retransmit/replay (phase <=
    /// ours) is silently ignored (the initiator will see our traffic under
    /// the new key and stop retransmitting).
    pub fn on_key_update(&mut self, announced_phase: u64) {
        if announced_phase > self.key_phase_counter {
            self.ratchet_session_key();
        }
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
    /// RT-2: control events shed by the bounded event queue (drop-oldest).
    pub total_dropped_events: u64,
    /// F1: fragments refused by the bounded reassembler (orphan/poison/cap).
    pub total_fragment_drops: u64,
}
