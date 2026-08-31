use crate::api::{NetworkFeedback, ReceivedMessage};
use crate::control::{ConnectionControl, ControlEvent, GtpConfig};
use crate::state::{ConnectionCold, ConnectionHot, OutgoingControlFrame};
use gtp_cc::{calculate_backpressure, BackpressureLevel, CongestionController};
use gtp_crypto::DirectionalKeys;
use gtp_recovery::{RetransmissionRecord, SentPacketRecord};
use gtp_scheduler::{OrderedGroupReceiver, SchedulableItem};
use gtp_types::{
    ConnectionId, FragmentId, GenerationId, MessageClass, MessageId, MonotonicTime, OrderedGroupId,
    PriorityTier, Result, StateKey, StateSequence, TransmissionId, TransportError,
};
use gtp_wire::{Frame, FrameIterator, PacketBuilder, PacketHeader, MIN_COMMON_HEADER_LEN};
use std::net::SocketAddr;

/// High-level Game Transport Protocol Connection Engine with dedicated Control API.
pub struct GtpConnection {
    pub hot: ConnectionHot,
    pub cold: ConnectionCold,
    pub config: GtpConfig,
    pub event_queue: Vec<ControlEvent>,
    pub last_backpressure: BackpressureLevel,
}

impl GtpConnection {
    pub fn new(cid: ConnectionId, peer_addr: SocketAddr, secure: bool) -> Self {
        Self::new_with_role(cid, peer_addr, secure, true, GtpConfig::default())
    }

    #[allow(deprecated)]
    pub fn new_with_config(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
        config: GtpConfig,
    ) -> Self {
        Self::new_with_role(cid, peer_addr, secure, true, config)
    }

    /// Legacy master-secret constructor with an explicit endpoint role (SEC-1: the
    /// role selects which derived direction this endpoint seals with).
    #[allow(deprecated)]
    pub fn new_with_role(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
        as_client: bool,
        config: GtpConfig,
    ) -> Self {
        let mut hot = ConnectionHot::new_with_master_secret(
            cid,
            peer_addr,
            secure,
            b"gtp_default_session_master_secret_2026",
            as_client,
        );
        // Re-apply the configured tuning on the legacy path (P1-2).
        hot.ack_tracker
            .set_policy(config.ack_frequency_packets, config.max_ack_delay);
        hot.anti_amplification.mark_validated();
        Self {
            hot,
            cold: ConnectionCold::default(),
            config,
            event_queue: Vec::with_capacity(32),
            last_backpressure: BackpressureLevel::Low,
        }
    }

    /// Builds a verified connection from directional handshake keys.
    pub fn new_with_directional_keys(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        keys: &DirectionalKeys,
        as_client: bool,
        pre_validated: bool,
        config: GtpConfig,
    ) -> Self {
        Self {
            hot: ConnectionHot::new_with_directional_keys(
                cid,
                peer_addr,
                keys,
                as_client,
                pre_validated,
            ),
            cold: ConnectionCold::default(),
            config,
            event_queue: Vec::with_capacity(32),
            last_backpressure: BackpressureLevel::Low,
        }
    }

    pub fn connection_id(&self) -> ConnectionId {
        self.hot.connection_id
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.hot.active_path
    }

    pub fn is_active(&self) -> bool {
        self.hot.state.is_active()
    }

    /// Access the dedicated runtime control interface.
    pub fn control(&mut self) -> ConnectionControl<'_> {
        ConnectionControl::new(
            &mut self.hot,
            &mut self.cold,
            &mut self.config,
            &mut self.event_queue,
        )
    }

    /// Drain all queued protocol control events.
    pub fn drain_events(&mut self) -> Vec<ControlEvent> {
        std::mem::take(&mut self.event_queue)
    }

    // ==========================================
    // Semantic Game Send API
    // ==========================================

    pub fn send_unreliable(
        &mut self,
        payload: Vec<u8>,
        priority: PriorityTier,
        deadline: Option<MonotonicTime>,
        now: MonotonicTime,
    ) -> Result<MessageId> {
        let msg_id = MessageId(self.hot.next_message_id);
        self.hot.next_message_id += 1;

        let item = SchedulableItem {
            message_id: msg_id,
            class: MessageClass::Unreliable,
            priority,
            created_at: now,
            deadline,
            supersedable: true,
            payload,
        };

        self.hot.scheduler.enqueue(item, now)?;
        Ok(msg_id)
    }

    pub fn send_sequenced(
        &mut self,
        state_key: StateKey,
        sequence: StateSequence,
        generation: GenerationId,
        deadline: Option<MonotonicTime>,
        payload: Vec<u8>,
        now: MonotonicTime,
    ) -> Result<MessageId> {
        let msg_id = MessageId(self.hot.next_message_id);
        self.hot.next_message_id += 1;

        let item = SchedulableItem {
            message_id: msg_id,
            class: MessageClass::UnreliableSequenced {
                state_key,
                sequence,
                generation,
            },
            priority: PriorityTier::P2WorldState,
            created_at: now,
            deadline,
            supersedable: true,
            payload,
        };

        self.hot.scheduler.enqueue(item, now)?;
        Ok(msg_id)
    }

    pub fn send_reliable_unordered(
        &mut self,
        payload: Vec<u8>,
        priority: PriorityTier,
        deadline: Option<MonotonicTime>,
        now: MonotonicTime,
    ) -> Result<MessageId> {
        let msg_id = MessageId(self.hot.next_message_id);
        self.hot.next_message_id += 1;

        let item = SchedulableItem {
            message_id: msg_id,
            class: MessageClass::ReliableUnordered,
            priority,
            created_at: now,
            deadline,
            supersedable: false,
            payload,
        };

        self.hot.scheduler.enqueue(item, now)?;
        Ok(msg_id)
    }

    pub fn send_reliable_ordered(
        &mut self,
        group_id: OrderedGroupId,
        payload: Vec<u8>,
        priority: PriorityTier,
        deadline: Option<MonotonicTime>,
        now: MonotonicTime,
    ) -> Result<MessageId> {
        let order_seq = self
            .hot
            .next_order_seqs
            .entry(group_id.as_u16())
            .or_insert(0);
        let current_order_seq = *order_seq;
        *order_seq = order_seq.wrapping_add(1);

        let msg_id = MessageId(self.hot.next_message_id);
        self.hot.next_message_id += 1;

        let item = SchedulableItem {
            message_id: msg_id,
            class: MessageClass::ReliableOrdered {
                group_id,
                order_seq: current_order_seq,
            },
            priority,
            created_at: now,
            deadline,
            supersedable: false,
            payload,
        };

        self.hot.scheduler.enqueue(item, now)?;
        Ok(msg_id)
    }

    // ==========================================
    // RX Pipeline
    // ==========================================

    pub fn handle_incoming_datagram(
        &mut self,
        src_addr: SocketAddr,
        datagram: &mut [u8],
        now: MonotonicTime,
    ) -> Result<Vec<ReceivedMessage>> {
        self.cold.total_rx_packets += 1;
        self.cold.total_rx_bytes += datagram.len() as u64;
        self.hot
            .anti_amplification
            .on_bytes_received(datagram.len());

        // 1. Decode Packet Header
        let (header, header_consumed) = PacketHeader::decode(datagram)?;

        // 2. Validate Connection ID
        if header.connection_id != self.hot.connection_id {
            return Err(TransportError::InvalidPacket("Mismatched Connection ID"));
        }

        // 3. Replay Protection Window — CHECK ONLY (SEC-3): the window is committed
        // only after successful authentication so a forged packet can never burn
        // packet-number space or evict legitimate entries.
        self.hot.replay_window.check(header.packet_number)?;

        // 4. Decrypt & Authenticate Payload — try the current RX key first, then the
        // previous key within the post-ratchet grace window (P2-5).
        let (aad_slice, encrypted_payload) = datagram.split_at_mut(header_consumed);
        let ciphertext_len = encrypted_payload.len();

        let mut open_result = self.hot.rx_protector.open(
            header.packet_number,
            header.connection_id,
            aad_slice,
            encrypted_payload,
            ciphertext_len,
        );
        if open_result.is_err() {
            if let Some(prev) = self.hot.rx_protector_prev.as_ref() {
                open_result = prev.open(
                    header.packet_number,
                    header.connection_id,
                    aad_slice,
                    encrypted_payload,
                    ciphertext_len,
                );
            }
        }

        let decrypted_len = match open_result {
            Ok(len) => {
                // Commit replay state only after successful authentication (SEC-3).
                self.hot.replay_window.commit(header.packet_number);
                // A legitimate authenticated packet proves peer reachability.
                self.hot.anti_amplification.mark_validated();
                len
            }
            Err(e) => {
                self.cold.total_corrupted_packets += 1;
                return Err(e);
            }
        };

        let decrypted_slice = &encrypted_payload[..decrypted_len];
        let mut delivered_messages: Vec<ReceivedMessage> = Vec::new();
        let mut is_ack_eliciting = false;
        let mut close_received: Option<u16> = None;

        // 5. Frame Dispatch Loop
        for frame_res in FrameIterator::new(decrypted_slice) {
            let frame = match frame_res {
                Ok(f) => f,
                Err(_) => {
                    self.cold.total_corrupted_packets += 1;
                    break;
                }
            };
            if frame.is_ack_eliciting() {
                is_ack_eliciting = true;
            }

            match frame {
                Frame::Ack {
                    largest_acked,
                    ack_delay_us,
                    ranges,
                    range_count,
                    ..
                } => {
                    let (ack_ev, loss_ev, _) = self.hot.loss_detector.on_ack_received(
                        largest_acked,
                        ack_delay_us,
                        &ranges,
                        range_count as usize,
                        now,
                    );

                    // FR-4: sync the controller's RTT from the single source of truth
                    // (RttStats, just updated above) BEFORE it runs its window update and
                    // once-per-RTT loss guard, so pacing/CC and PTO share one RTT.
                    self.hot
                        .cc
                        .on_rtt(self.hot.loss_detector.rtt_stats.smoothed_rtt);
                    self.hot.cc.on_ack(&ack_ev, now);
                    self.hot.cc.on_loss(&loss_ev, now);

                    if !loss_ev.lost_packets.is_empty() {
                        self.event_queue.push(ControlEvent::PacketLossDetected {
                            lost_count: loss_ev.lost_packets.len(),
                            lost_bytes: loss_ev.bytes_lost,
                        });
                    }

                    // Re-enqueue lost reliable frames for retransmission. A scheduler
                    // rejection is counted — never silently swallowed (ORD-3).
                    for retrans in loss_ev.retransmittable {
                        self.cold.total_retransmissions += 1;
                        self.event_queue
                            .push(ControlEvent::RetransmissionTriggered {
                                message_id: retrans.message_id,
                                fragment_id: retrans.fragment_id,
                            });

                        let item = Self::retransmission_item(retrans, now);
                        if self.hot.scheduler.enqueue(item, now).is_err() {
                            self.cold.total_dropped_frames += 1;
                        }
                    }
                }

                Frame::Data {
                    message_id,
                    state_key,
                    sequence,
                    generation,
                    payload,
                    ..
                } => {
                    // SEM-2: receiver-side drop-late for genuinely sequenced state.
                    // Plain unreliable sends travel with default key/seq and bypass
                    // the freshness check entirely (CORE-8 keeps them always-on).
                    let is_plain_unreliable = state_key == StateKey::default()
                        && sequence == StateSequence::default()
                        && generation == GenerationId::default();
                    if !is_plain_unreliable
                        && !self
                            .hot
                            .rx_state_table
                            .should_admit(state_key, generation, sequence)
                    {
                        self.cold.total_stale_drops += 1;
                        continue;
                    }
                    if !is_plain_unreliable {
                        self.hot
                            .rx_state_table
                            .update(state_key, generation, sequence, message_id);
                    }

                    delivered_messages.push(ReceivedMessage {
                        class: MessageClass::UnreliableSequenced {
                            state_key,
                            sequence,
                            generation,
                        },
                        payload: payload.to_vec(),
                    });
                }

                Frame::ReliableData {
                    message_id,
                    group_id,
                    order_seq,
                    payload,
                    ..
                } => {
                    if group_id.as_u16() == 0 {
                        // ORD-4: unordered reliable delivery is deduplicated by
                        // message id — retransmissions of an already-delivered
                        // message must not reach the application twice.
                        if !self.hot.delivered_index.insert_if_new(message_id.as_u64()) {
                            self.cold.total_duplicate_drops += 1;
                            continue;
                        }
                        delivered_messages.push(ReceivedMessage {
                            class: MessageClass::ReliableUnordered,
                            payload: payload.to_vec(),
                        });
                    } else {
                        // Ordered reliable delivery
                        let group = self
                            .hot
                            .ordered_groups
                            .entry(group_id.as_u16())
                            .or_insert_with(|| OrderedGroupReceiver::new(group_id));

                        // ORD-2: a full reorder buffer isolates THIS frame only —
                        // the datagram (and its ACK bookkeeping) survives.
                        match group.on_incoming(order_seq, payload) {
                            Ok(ready_items) => {
                                for item in ready_items {
                                    delivered_messages.push(ReceivedMessage {
                                        class: MessageClass::ReliableOrdered {
                                            group_id,
                                            order_seq,
                                        },
                                        payload: item,
                                    });
                                }
                            }
                            Err(_) => {
                                self.cold.total_dropped_frames += 1;
                            }
                        }
                    }
                }

                Frame::Retx {
                    message_id,
                    payload,
                    ..
                } => {
                    if self.hot.delivered_index.insert_if_new(message_id.as_u64()) {
                        delivered_messages.push(ReceivedMessage {
                            class: MessageClass::ReliableUnordered,
                            payload: payload.to_vec(),
                        });
                    } else {
                        self.cold.total_duplicate_drops += 1;
                    }
                }

                Frame::Ping { .. } => {}

                Frame::PathChallenge { data } => {
                    // Core-C1: the echo is queued as a REAL PathResponse frame,
                    // directed at the challenger's source address (PATH-5).
                    self.hot
                        .control_queue
                        .push_back(OutgoingControlFrame::PathResponse {
                            data,
                            dest: src_addr,
                        });
                }

                Frame::PathResponse { data } => {
                    if self
                        .hot
                        .path_validator
                        .validate_response(src_addr, &data, now)
                    {
                        let old_addr = self.hot.active_path;
                        self.hot.active_path = src_addr;
                        self.event_queue.push(ControlEvent::PathMigrated {
                            old_addr,
                            new_addr: src_addr,
                        });
                    }
                }

                Frame::AckFrequency {
                    ack_frequency_packets,
                    max_ack_delay_ms,
                    reorder_threshold,
                } => {
                    // REC-7: the negotiation actually reaches the ACK engine now.
                    self.config.ack_frequency_packets = ack_frequency_packets;
                    self.config.max_ack_delay =
                        gtp_types::Duration::from_millis(max_ack_delay_ms as u64);
                    self.config.ack_reorder_threshold = reorder_threshold;
                    self.hot.ack_tracker.set_policy(
                        ack_frequency_packets,
                        gtp_types::Duration::from_millis(max_ack_delay_ms as u64),
                    );
                }

                Frame::Close { error_code, .. } => {
                    close_received = Some(error_code);
                }

                _ => {}
            }
        }

        // 6. Update ACK Tracker — runs even for datagrams containing corrupt frames
        // after a valid prefix, and always before Close handling so the peer's final
        // packets are acknowledged.
        self.hot.ack_tracker.on_packet_received(
            header.packet_number,
            is_ack_eliciting,
            header.flags.ecn_bits(),
            now,
        );

        // 7. Check Backpressure level changes
        let feedback = self.feedback(now);
        if feedback.backpressure != self.last_backpressure {
            self.event_queue.push(ControlEvent::BackpressureChanged {
                old_level: self.last_backpressure,
                new_level: feedback.backpressure,
                effective_queue_bytes: feedback.effective_queue_bytes,
            });
            self.last_backpressure = feedback.backpressure;
        }

        // 8. Close handling completes only after the packet has been fully
        // acknowledged and accounted for.
        if let Some(error_code) = close_received {
            let old_state = self.hot.state;
            let _ = self
                .hot
                .state
                .transition_to(gtp_path::ConnectionState::Closed);
            self.event_queue.push(ControlEvent::StateChanged {
                old_state,
                new_state: gtp_path::ConnectionState::Closed,
            });
            return Err(TransportError::ConnectionClosed(error_code));
        }

        Ok(delivered_messages)
    }

    fn retransmission_item(retrans: RetransmissionRecord, now: MonotonicTime) -> SchedulableItem {
        SchedulableItem {
            message_id: retrans.message_id,
            class: if retrans.group_id == 0 {
                MessageClass::ReliableUnordered
            } else {
                MessageClass::ReliableOrdered {
                    group_id: OrderedGroupId(retrans.group_id),
                    order_seq: retrans.order_seq,
                }
            },
            priority: PriorityTier::P3ReliableGameplay,
            created_at: now,
            deadline: None,
            supersedable: false,
            payload: retrans.payload,
        }
    }

    // ==========================================
    // TX Pipeline
    // ==========================================

    pub fn produce_outgoing_datagram(
        &mut self,
        now: MonotonicTime,
        out_buf: &mut [u8],
    ) -> Result<Option<(SocketAddr, usize)>> {
        use gtp_path::ConnectionState;

        match self.hot.state {
            ConnectionState::Closed => return Ok(None),
            ConnectionState::Draining => {
                if self.hot.control_queue.is_empty() {
                    // Draining complete: the CLOSE frame went out, nothing left to send.
                    let old_state = self.hot.state;
                    let _ = self.hot.state.transition_to(ConnectionState::Closed);
                    self.event_queue.push(ControlEvent::StateChanged {
                        old_state,
                        new_state: ConnectionState::Closed,
                    });
                    return Ok(None);
                }
                // Draining with pending control frames (the CLOSE itself) — fall
                // through so the frame is transmitted.
            }
            _ => {}
        }

        // 0. Check for Probe Timeout (PTO) with exponential backoff (REC-11): the
        // sweep re-enqueues at most a bounded burst and DRAINS those records so
        // successive PTOs cannot re-inject the whole window.
        if self.hot.loss_detector.inflight_bytes() > 0 {
            let pto = self
                .hot
                .loss_detector
                .pto_duration_with_backoff(self.config.pto_max_duration);
            if now.duration_since(self.hot.loss_detector.time_of_last_ack_eliciting_packet) >= pto {
                let loss_ev = self.hot.loss_detector.on_timeout(now);
                self.hot.cc.on_timeout(now);

                self.event_queue.push(ControlEvent::PtoTriggered {
                    pto_count: self.hot.loss_detector.pto_count,
                    inflight_bytes: self.hot.loss_detector.inflight_bytes(),
                });

                for retrans in loss_ev.retransmittable {
                    self.cold.total_retransmissions += 1;
                    self.event_queue
                        .push(ControlEvent::RetransmissionTriggered {
                            message_id: retrans.message_id,
                            fragment_id: retrans.fragment_id,
                        });

                    let item = Self::retransmission_item(retrans, now);
                    if self.hot.scheduler.enqueue(item, now).is_err() {
                        self.cold.total_dropped_frames += 1;
                    }
                }
                self.hot.loss_detector.time_of_last_ack_eliciting_packet = now;
            }
        }

        // 1. Prune expired messages from scheduler
        let pruned = self.hot.scheduler.prune_stale(now);
        self.cold.total_stale_drops += pruned as u64;

        // 2. Update Pacing Token Bucket
        let pacing_rate = self.hot.cc.pacing_rate();
        self.hot.pacing.update_tokens(pacing_rate, now);

        let send_budget = self
            .hot
            .pacing
            .send_budget(self.hot.cc.cwnd(), self.hot.cc.inflight());

        let should_ack = self.hot.ack_tracker.should_send_ack(now);
        let has_queued_data =
            !self.hot.scheduler.is_empty() && self.hot.state == ConnectionState::Established;
        let has_control = !self.hot.control_queue.is_empty();

        if !should_ack && !has_queued_data && !has_control {
            return Ok(None);
        }

        if !should_ack && !has_control && send_budget < 64 {
            return Ok(None); // Constrained by congestion window or pacing
        }

        let pn = self.hot.next_packet_number;
        let ts = now.as_micros() as u32;

        let mut header = PacketHeader::new_short(self.hot.connection_id, pn, ts, 0);
        // P2-5: advertise the key phase so a ratcheting peer can select keys.
        header.flags.set_key_phase(self.hot.key_phase);
        let mut builder = PacketBuilder::new(out_buf, header)?;

        let mut retransmittables: Vec<RetransmissionRecord> = Vec::new();
        let mut ack_eliciting = false;
        // PATH-5: a control frame directed at a specific address sends alone.
        let mut override_dest: Option<SocketAddr> = None;

        // 3. Attach ACK Frame if needed
        if should_ack {
            if let Some(ack_frame) = self.hot.ack_tracker.generate_ack_frame(now) {
                let _ = builder.append_frame(&ack_frame);
            }
        }

        // 3b. Drain protocol control frames as REAL frames (Core-C1 fix)
        while let Some(ctrl) = self.hot.control_queue.pop_front() {
            if override_dest.is_some() {
                // A directed frame owns this datagram; push the rest back.
                self.hot.control_queue.push_front(ctrl);
                break;
            }
            if let OutgoingControlFrame::PathResponse { dest, .. }
            | OutgoingControlFrame::PathChallenge { dest, .. } = &ctrl
            {
                if *dest != self.hot.active_path {
                    override_dest = Some(*dest);
                }
            }
            let is_close = matches!(ctrl, OutgoingControlFrame::Close { .. });
            let frame = Self::build_control_frame(&ctrl);
            match builder.append_frame(&frame) {
                Ok(_) => {
                    ack_eliciting = true;
                    if is_close {
                        self.hot.close_frame_sent = true;
                    }
                }
                Err(_) => {
                    // Does not fit this datagram — retry next time at the front.
                    self.hot.control_queue.push_front(ctrl);
                    break;
                }
            }
        }

        // 4. Pop and encode data frames within send budget (Established only)
        let mut popped: Vec<SchedulableItem> = Vec::new();
        if self.hot.state == ConnectionState::Established {
            while builder.remaining_capacity() > 64 && override_dest.is_none() {
                let remaining_budget = send_budget.min(builder.remaining_capacity());
                match self.hot.scheduler.pop_next(remaining_budget, now) {
                    Some(item) => {
                        let encoded =
                            Self::encode_scheduler_item(&item, &mut builder, &mut retransmittables);
                        if encoded.is_err() {
                            // D-2: never silently drop a frame that was popped —
                            // requeue it for the next datagram and stop filling.
                            let _ = self.hot.scheduler.enqueue(item, now);
                            break;
                        }
                        ack_eliciting = true;
                        popped.push(item);
                    }
                    None => break,
                }
            }
        }

        // 5. Finalize unencrypted header & calculate final payload length including tag
        let unencrypted_len = builder.finish()?;
        let header_len = MIN_COMMON_HEADER_LEN;
        let unsealed_payload_len = unencrypted_len - header_len;
        let final_payload_len = unsealed_payload_len + self.hot.tx_protector.tag_len();
        if final_payload_len > u16::MAX as usize {
            // WIR-3: reject instead of truncating the declared length.
            for item in popped.into_iter().rev() {
                let _ = self.hot.scheduler.enqueue(item, now);
            }
            return Err(TransportError::BufferOverflow);
        }

        // Set the final payload_len in the header slice before sealing so AAD matches exactly
        out_buf[header_len - 2..header_len]
            .copy_from_slice(&(final_payload_len as u16).to_be_bytes());

        // 6. Seal Payload with AEAD (TX direction protector — SEC-1)
        let (aad_slice, payload_slice) = out_buf.split_at_mut(header_len);
        let sealed_payload_len = self.hot.tx_protector.seal(
            pn,
            self.hot.connection_id,
            &aad_slice[..header_len],
            payload_slice,
            unsealed_payload_len,
        )?;

        let total_datagram_len = header_len + sealed_payload_len;

        // Check Anti-Amplification Limiter — on rejection, every popped item is
        // re-queued; nothing reliable is lost to the gate.
        if !self.hot.anti_amplification.can_send(total_datagram_len) {
            for item in popped.into_iter().rev() {
                let _ = self.hot.scheduler.enqueue(item, now);
            }
            return Ok(None);
        }
        self.hot
            .anti_amplification
            .on_bytes_sent(total_datagram_len);

        // 7. Record In-Flight and CC Telemetry. P1-3: ACK-only datagrams carry no
        // congestion debt — cc accounting counts ack-eliciting traffic only.
        let sent_record = SentPacketRecord {
            packet_number: pn,
            send_time: now,
            bytes: total_datagram_len,
            ack_eliciting,
            in_flight: ack_eliciting,
            retransmittable_frames: retransmittables,
        };

        self.hot.loss_detector.on_packet_sent(sent_record);
        if ack_eliciting {
            self.hot.cc.on_packet_sent(pn, total_datagram_len, now);
        }
        self.hot.pacing.consume(total_datagram_len);

        self.hot.next_packet_number = pn.next();
        self.hot.packets_since_ratchet += 1;
        self.cold.total_tx_packets += 1;
        self.cold.total_tx_bytes += total_datagram_len as u64;

        let dest = override_dest.unwrap_or(self.hot.active_path);
        Ok(Some((dest, total_datagram_len)))
    }
    fn build_control_frame(ctrl: &OutgoingControlFrame) -> Frame<'_> {
        match ctrl {
            OutgoingControlFrame::Ping { nonce } => Frame::Ping { nonce: *nonce },
            OutgoingControlFrame::PathChallenge { data, .. } => {
                Frame::PathChallenge { data: *data }
            }
            OutgoingControlFrame::PathResponse { data, .. } => Frame::PathResponse { data: *data },
            OutgoingControlFrame::MtuProbe {
                probe_id,
                padding_len,
            } => Frame::MtuProbe {
                probe_id: *probe_id,
                padding_len: *padding_len,
            },
            OutgoingControlFrame::AckFrequency {
                ack_frequency_packets,
                max_ack_delay_ms,
                reorder_threshold,
            } => Frame::AckFrequency {
                ack_frequency_packets: *ack_frequency_packets,
                max_ack_delay_ms: *max_ack_delay_ms,
                reorder_threshold: *reorder_threshold,
            },
            OutgoingControlFrame::Close { error_code, reason } => Frame::Close {
                error_code: *error_code,
                reason,
            },
        }
    }

    fn encode_scheduler_item(
        item: &SchedulableItem,
        builder: &mut PacketBuilder<'_>,
        retransmittables: &mut Vec<RetransmissionRecord>,
    ) -> Result<()> {
        match item.class {
            MessageClass::Unreliable => {
                let frame = Frame::Data {
                    message_id: item.message_id,
                    state_key: StateKey::default(),
                    sequence: StateSequence::default(),
                    generation: GenerationId::default(),
                    deadline_ms: 0,
                    payload: &item.payload,
                };
                builder
                    .append_frame(&frame)
                    .map_err(|_| TransportError::BufferOverflow)
            }
            MessageClass::UnreliableSequenced {
                state_key,
                sequence,
                generation,
            } => {
                let frame = Frame::Data {
                    message_id: item.message_id,
                    state_key,
                    sequence,
                    generation,
                    deadline_ms: 0,
                    payload: &item.payload,
                };
                builder
                    .append_frame(&frame)
                    .map_err(|_| TransportError::BufferOverflow)
            }
            MessageClass::ReliableUnordered => {
                let frame = Frame::ReliableData {
                    message_id: item.message_id,
                    fragment_id: FragmentId(0),
                    total_fragments: 1,
                    group_id: OrderedGroupId(0),
                    order_seq: 0,
                    payload: &item.payload,
                };
                builder
                    .append_frame(&frame)
                    .map_err(|_| TransportError::BufferOverflow)?;
                retransmittables.push(RetransmissionRecord {
                    message_id: item.message_id,
                    fragment_id: FragmentId(0),
                    transmission_id: TransmissionId(1),
                    group_id: 0,
                    order_seq: 0,
                    payload: item.payload.clone(),
                });
                Ok(())
            }
            MessageClass::ReliableOrdered {
                group_id,
                order_seq,
            } => {
                let frame = Frame::ReliableData {
                    message_id: item.message_id,
                    fragment_id: FragmentId(0),
                    total_fragments: 1,
                    group_id,
                    order_seq,
                    payload: &item.payload,
                };
                builder
                    .append_frame(&frame)
                    .map_err(|_| TransportError::BufferOverflow)?;
                retransmittables.push(RetransmissionRecord {
                    message_id: item.message_id,
                    fragment_id: FragmentId(0),
                    transmission_id: TransmissionId(1),
                    group_id: group_id.as_u16(),
                    order_seq,
                    payload: item.payload.clone(),
                });
                Ok(())
            }
        }
    }

    // ==========================================
    // Telemetry & Feedback
    // ==========================================

    pub fn feedback(&self, now: MonotonicTime) -> NetworkFeedback {
        let rtt = self.hot.loss_detector.rtt_stats;
        let eff_queue = self.hot.scheduler.effective_queue_bytes(now);
        let backpressure =
            calculate_backpressure(eff_queue, self.hot.cc.cwnd(), rtt.smoothed_rtt, rtt.min_rtt);

        NetworkFeedback {
            rtt: rtt.latest_rtt,
            smoothed_rtt: rtt.smoothed_rtt,
            min_rtt: rtt.min_rtt,
            cwnd_bytes: self.hot.cc.cwnd(),
            inflight_bytes: self.hot.cc.inflight(),
            pacing_rate_bps: self.hot.cc.pacing_rate(),
            backpressure,
            effective_queue_bytes: eff_queue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtp_types::Duration;

    fn loopback_pair(cid: ConnectionId) -> (GtpConnection, GtpConnection) {
        let client_addr: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let server_addr: SocketAddr = "127.0.0.1:6000".parse().unwrap();
        let client =
            GtpConnection::new_with_role(cid, server_addr, true, true, GtpConfig::default());
        let server =
            GtpConnection::new_with_role(cid, client_addr, true, false, GtpConfig::default());
        (client, server)
    }

    /// FR-4 / TEST-4: after a real ACK round-trip the congestion controller's RTT is
    /// identical to the loss detector's `RttStats::smoothed_rtt` — a single source of
    /// truth feeds pacing, backpressure and PTO, rather than two independent estimates.
    #[test]
    fn controller_rtt_mirrors_the_single_rtt_source_after_ack() {
        let cid = ConnectionId(0xF14_0000_0000_0004);
        let client_addr: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let (mut client, mut server) = loopback_pair(cid);
        let t0 = MonotonicTime::from_micros(4_000_000);

        // Client sends ack-eliciting data.
        client
            .send_reliable_unordered(
                b"rtt_probe".to_vec(),
                PriorityTier::P3ReliableGameplay,
                None,
                t0,
            )
            .unwrap();
        let mut buf = [0u8; 1500];
        let (_, len) = client
            .produce_outgoing_datagram(t0, &mut buf)
            .unwrap()
            .unwrap();

        // Server receives it (now owes an ACK) one RTT later.
        let t1 = t0 + Duration::from_millis(80); // != cubic init (50ms) so the test distinguishes
        server
            .handle_incoming_datagram(client_addr, &mut buf[..len], t1)
            .unwrap();
        // Server emits the ACK.
        let (_, ack_len) = server
            .produce_outgoing_datagram(t1, &mut buf)
            .unwrap()
            .expect("server must produce an ACK datagram");
        // Client processes the ACK -> RTT sample taken, controller RTT synced (FR-4).
        server_ack_to_client(&mut client, &mut buf[..ack_len], t1);

        let source_rtt = client.hot.loss_detector.rtt_stats.smoothed_rtt;
        assert!(source_rtt > Duration::ZERO);
        assert_eq!(
            client.hot.cc.smoothed_rtt(),
            source_rtt,
            "controller RTT must mirror the single RTT source exactly"
        );
    }

    fn server_ack_to_client(client: &mut GtpConnection, datagram: &mut [u8], now: MonotonicTime) {
        let server_addr: SocketAddr = "127.0.0.1:6000".parse().unwrap();
        let _ = client.handle_incoming_datagram(server_addr, datagram, now);
    }

    #[test]
    fn test_gtp_connection_send_and_receive_pipeline() {
        let cid = ConnectionId(0x1122334455667788);
        let server_addr: SocketAddr = "127.0.0.1:6000".parse().unwrap();
        let (mut client, mut server) = loopback_pair(cid);

        let now = MonotonicTime::from_micros(1_000_000);

        // 1. Client enqueues unreliable and reliable messages
        let _ = client
            .send_unreliable(b"client_input".to_vec(), PriorityTier::P1Input, None, now)
            .unwrap();
        let _ = client
            .send_reliable_unordered(
                b"player_damage".to_vec(),
                PriorityTier::P3ReliableGameplay,
                None,
                now,
            )
            .unwrap();

        // 2. Client produces outgoing datagram
        let mut out_buffer = [0u8; 1500];
        let (dest, len) = client
            .produce_outgoing_datagram(now, &mut out_buffer)
            .unwrap()
            .unwrap();
        assert_eq!(dest, server_addr);
        assert!(len > MIN_COMMON_HEADER_LEN);

        // 3. Server receives and parses datagram
        let mut in_buffer = out_buffer;
        let delivered = server
            .handle_incoming_datagram(
                client_addr_of(&server),
                &mut in_buffer[..len],
                now + Duration::from_millis(10),
            )
            .unwrap();

        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0].payload, b"client_input");
        assert_eq!(delivered[1].payload, b"player_damage");
    }

    fn client_addr_of(server: &GtpConnection) -> SocketAddr {
        // The peer address the server expects traffic from (the client in the pair)
        let _ = server;
        "127.0.0.1:5000".parse().unwrap()
    }

    /// SEC-3: a forged packet that fails authentication must not burn the replay
    /// window — the legitimate packet with the same number still gets through.
    #[test]
    fn spoofed_high_pn_does_not_burn_replay_window() {
        let cid = ConnectionId(0xABCD_0000_0000_0001);
        let (mut client, mut server) = loopback_pair(cid);
        let now = MonotonicTime::from_micros(1_000_000);

        // Legitimate packet from the client
        client
            .send_unreliable(b"real".to_vec(), PriorityTier::P1Input, None, now)
            .unwrap();
        let mut buf = [0u8; 1500];
        let (_, len) = client
            .produce_outgoing_datagram(now, &mut buf)
            .unwrap()
            .unwrap();

        // "Attacker" flips ciphertext bytes (auth fails) — window must not commit.
        let mut forged = buf;
        forged[len - 1] ^= 0xFF;
        assert!(server
            .handle_incoming_datagram("127.0.0.1:9999".parse().unwrap(), &mut forged[..len], now)
            .is_err());

        // The genuine packet still opens fine afterwards
        let mut genuine = buf;
        let delivered = server
            .handle_incoming_datagram("127.0.0.1:5000".parse().unwrap(), &mut genuine[..len], now)
            .unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].payload, b"real");
    }

    /// SEM-2: a late sequenced update (older than one already delivered) is dropped
    /// at the receiver instead of overwriting fresh state.
    #[test]
    fn rx_drop_late_sequenced() {
        let cid = ConnectionId(0x57A7_0000_0000_0004);
        let (mut client, mut server) = loopback_pair(cid);
        let now = MonotonicTime::from_micros(4_000_000);

        let key = StateKey::new(7, 1);
        let mut out = [0u8; 1500];

        // Fresh state: seq 2 arrives first
        client
            .send_sequenced(
                key,
                StateSequence(2),
                GenerationId(1),
                None,
                b"new".to_vec(),
                now,
            )
            .unwrap();
        let (_, len) = client
            .produce_outgoing_datagram(now, &mut out)
            .unwrap()
            .unwrap();
        let mut buf = out;
        let d = server
            .handle_incoming_datagram("127.0.0.1:5000".parse().unwrap(), &mut buf[..len], now)
            .unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].payload, b"new");

        // Late state: seq 1 (older) — the TX-side supersession already refuses to
        // enqueue it (supersedable), so produce has nothing to send.
        client
            .send_sequenced(
                key,
                StateSequence(1),
                GenerationId(1),
                None,
                b"old".to_vec(),
                now,
            )
            .unwrap();
        assert!(client
            .produce_outgoing_datagram(now, &mut out)
            .unwrap()
            .is_none());

        // RX-side defense (SEM-2): craft the late frame manually — as a reordered
        // retransmission or a tier-crossing send (T4) would deliver it — and prove
        // the receiver drops it instead of overwriting fresh state.
        let late_frame = Frame::Data {
            message_id: MessageId(999),
            state_key: key,
            sequence: StateSequence(1),
            generation: GenerationId(1),
            deadline_ms: 0,
            payload: b"old".as_slice(),
        };
        let mut late_buf = [0u8; 1500];
        let pn = gtp_types::PacketNumber(42);
        let mut header = PacketHeader::new_short(cid, pn, now.as_micros() as u32, 0);
        header.flags.set_key_phase(client.hot.key_phase);
        let mut builder = PacketBuilder::new(&mut late_buf, header).unwrap();
        builder.append_frame(&late_frame).unwrap();
        let unsealed = builder.finish().unwrap();
        let tag_len = 16usize;
        late_buf[22..24].copy_from_slice(&((unsealed - 24 + tag_len) as u16).to_be_bytes());
        let tx_iv = client.hot.tx_iv;
        let aad = late_buf[..24].to_vec();
        let seal = gtp_crypto::GtpAeadProtector::new(client.hot.tx_key, tx_iv);
        let sealed_len = gtp_crypto::PacketProtector::seal(
            &seal,
            pn,
            cid,
            &aad,
            &mut late_buf[24..],
            unsealed - 24,
        )
        .unwrap();
        let total = 24 + sealed_len;

        let d = server
            .handle_incoming_datagram(
                "127.0.0.1:5000".parse().unwrap(),
                &mut late_buf[..total],
                now,
            )
            .unwrap();
        assert!(
            d.is_empty(),
            "late sequenced state must be dropped at the receiver"
        );
        assert!(server.cold.total_stale_drops >= 1);
    }

    /// PATH-1: closing a connection whose handshake never completed must succeed.
    #[test]
    fn force_close_fresh_connection_succeeds() {
        use gtp_path::ConnectionState;
        let cid = ConnectionId(0xF0CE_0000_0000_0005);
        let mut conn = GtpConnection::new_with_role(
            cid,
            "127.0.0.1:7000".parse().unwrap(),
            false,
            true,
            GtpConfig::default(),
        );
        // Simulate a pre-establishment state to prove Initial -> Closed is legal
        conn.hot.state = ConnectionState::Initial;
        assert!(conn.control().force_close(0).is_ok());
        assert!(conn.hot.state == ConnectionState::Closed);
        assert!(!conn.is_active());
    }

    /// SEC-1: client and server must seal with DIFFERENT keys.
    #[test]
    fn directional_keys_distinct_per_role() {
        let cid = ConnectionId(0x1234_5678_9ABC_DEF0);
        let client_addr: SocketAddr = "127.0.0.1:5001".parse().unwrap();
        let server_addr: SocketAddr = "127.0.0.1:6001".parse().unwrap();

        let client = GtpConnection::new_with_directional_keys(
            cid,
            server_addr,
            &make_keys(cid),
            true,
            true,
            GtpConfig::default(),
        );
        let server = GtpConnection::new_with_directional_keys(
            cid,
            client_addr,
            &make_keys(cid),
            false,
            true,
            GtpConfig::default(),
        );

        // The TX key on one side equals the RX key on the other (matching halves),
        // but the two TX keys differ.
        assert_ne!(client.hot.tx_key, server.hot.tx_key);
        assert_eq!(client.hot.tx_key, server.hot.rx_key);
        assert_eq!(server.hot.tx_key, client.hot.rx_key);
    }

    fn make_keys(cid: ConnectionId) -> gtp_crypto::DirectionalKeys {
        let shared = gtp_crypto::HandshakeSharedSecret::new([7u8; 32]);
        gtp_crypto::derive_directional_handshake_session_keys(&shared, &[1u8; 32], &[2u8; 32], cid)
    }

    /// SEC-6 / P2-5: after BOTH peers ratchet in lockstep, traffic keeps flowing and
    /// the old RX key still admits in-flight packets from before the rotation.
    #[test]
    fn coordinated_ratchet_keeps_link_alive() {
        let cid = ConnectionId(0x2A7C_0000_0000_0006);
        let (mut client, mut server) = loopback_pair(cid);
        let now = MonotonicTime::from_micros(6_000_000);

        // Pre-ratchet datagram (still in flight when the peers rotate)
        client
            .send_unreliable(b"pre".to_vec(), PriorityTier::P1Input, None, now)
            .unwrap();
        let mut pre_buf = [0u8; 1500];
        let (_, pre_len) = client
            .produce_outgoing_datagram(now, &mut pre_buf)
            .unwrap()
            .unwrap();

        // Both peers rotate simultaneously (documented coordination requirement)
        client.control().ratchet_key();
        server.control().ratchet_key();
        assert!(client.hot.key_phase && server.hot.key_phase);
        // Directions remain distinct after rotation
        assert_ne!(client.hot.tx_key, server.hot.tx_key);
        assert_eq!(client.hot.tx_key, server.hot.rx_key);
        assert_eq!(server.hot.tx_key, client.hot.rx_key);

        // Post-ratchet data flows both ways
        client
            .send_unreliable(b"post".to_vec(), PriorityTier::P1Input, None, now)
            .unwrap();
        let mut post_buf = [0u8; 1500];
        let (_, post_len) = client
            .produce_outgoing_datagram(now, &mut post_buf)
            .unwrap()
            .unwrap();

        // KEY_PHASE advertised on the sealed header
        let header = gtp_wire::PacketHeader::decode(&post_buf[..post_len])
            .unwrap()
            .0;
        assert!(header.flags.key_phase());

        let delivered = server
            .handle_incoming_datagram(
                "127.0.0.1:5000".parse().unwrap(),
                &mut post_buf[..post_len],
                now,
            )
            .unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].payload, b"post");

        // Grace window: the pre-ratchet datagram still opens with the retained key
        let pre_delivered = server
            .handle_incoming_datagram(
                "127.0.0.1:5000".parse().unwrap(),
                &mut pre_buf[..pre_len],
                now,
            )
            .unwrap();
        assert_eq!(pre_delivered.len(), 1);
        assert_eq!(pre_delivered[0].payload, b"pre");
    }

    /// Core-C1 + PATH-5: path migration works through the PROTOCOL — a challenge
    /// reaches the peer as a real PathChallenge, the echo is a real PathResponse
    /// directed at the challenger's source address, and the challenger migrates.
    #[test]
    fn path_migration_via_protocol() {
        use crate::control::ControlEvent;
        let cid = ConnectionId(0x1A7B_0000_0000_0007);
        let (mut client, mut server) = loopback_pair(cid);
        let now = MonotonicTime::from_micros(7_000_000);

        let new_server_addr: SocketAddr = "127.0.0.1:6100".parse().unwrap();

        // 1. Client challenges the server's NEW address
        client
            .control()
            .trigger_path_challenge(new_server_addr, [0xAB; 8], now)
            .unwrap();
        let mut out = [0u8; 1500];
        let (dest, len) = client
            .produce_outgoing_datagram(now, &mut out)
            .unwrap()
            .unwrap();
        assert_eq!(
            dest, new_server_addr,
            "challenge must target the new address"
        );

        let client_active_addr: SocketAddr = "127.0.0.1:5000".parse().unwrap();

        // 2. The challenge arrives FROM the client's address — the server echoes a
        //    real PathResponse directed at that source.
        let mut challenge_buf = out;
        let resp = server
            .handle_incoming_datagram(client_active_addr, &mut challenge_buf[..len], now)
            .unwrap();
        assert!(resp.is_empty(), "challenge is not an application message");
        let (resp_dest, resp_len) = server
            .produce_outgoing_datagram(now, &mut out)
            .unwrap()
            .unwrap();
        assert_eq!(
            resp_dest, client_active_addr,
            "echo must be directed at the challenger's source address (PATH-5)"
        );

        // 3. The response reaches the client from the new address: migration fires.
        let mut resp_buf = out;
        let r = client
            .handle_incoming_datagram(new_server_addr, &mut resp_buf[..resp_len], now)
            .unwrap();
        assert!(r.is_empty());
        assert_eq!(
            client.hot.active_path, new_server_addr,
            "client must migrate to the validated address"
        );
        assert!(client.drain_events().iter().any(|e| matches!(
            e,
            ControlEvent::PathMigrated { new_addr, .. } if *new_addr == new_server_addr
        )));
    }

    /// ORD-4: the delivered index blocks duplicate reliable delivery, and a
    /// retransmitted datagram under the same packet number is rejected as replay.
    #[test]
    fn reliable_unordered_dedup() {
        let cid = ConnectionId(0xDDBA_0000_0000_0002);
        let (mut client, mut server) = loopback_pair(cid);
        let now = MonotonicTime::from_micros(2_000_000);

        client
            .send_reliable_unordered(
                b"once".to_vec(),
                PriorityTier::P3ReliableGameplay,
                None,
                now,
            )
            .unwrap();
        let mut buf = [0u8; 1500];
        let (_, len) = client
            .produce_outgoing_datagram(now, &mut buf)
            .unwrap()
            .unwrap();

        // First delivery arrives
        let mut first = buf;
        let d1 = server
            .handle_incoming_datagram("127.0.0.1:5000".parse().unwrap(), &mut first[..len], now)
            .unwrap();
        assert_eq!(d1.len(), 1);
        assert_eq!(d1[0].payload, b"once");

        // The same datagram replayed under the same PN is rejected by the replay window
        let mut replay = buf;
        let r = server.handle_incoming_datagram(
            "127.0.0.1:5000".parse().unwrap(),
            &mut replay[..len],
            now,
        );
        assert!(r.is_err());

        // A retransmission under a NEW packet number carrying the same message_id
        // is dropped by the delivered index, not delivered twice.
        // (message ids start at 1: this reliable message was id 1)
        assert!(!server.hot.delivered_index.insert_if_new(1));
    }

    /// Core-C1: control frames reach the peer as real frames and close transitions.
    #[test]
    fn control_frames_reach_peer_as_frames() {
        use crate::control::ControlEvent;
        let cid = ConnectionId(0xC0DE_0000_0000_0003);
        let (mut client, mut server) = loopback_pair(cid);
        let now = MonotonicTime::from_micros(3_000_000);

        // Client pings the server: the frame must be transmitted and parsed as Ping.
        client.control().send_ping(0xBEEF, now).unwrap();
        let mut buf = [0u8; 1500];
        let (_, len) = client
            .produce_outgoing_datagram(now, &mut buf)
            .unwrap()
            .unwrap();

        // Before the fix, the peer saw a Data frame wrapping raw frame bytes.
        let delivered = server
            .handle_incoming_datagram("127.0.0.1:5000".parse().unwrap(), &mut buf[..len], now)
            .unwrap();
        assert!(
            delivered.is_empty(),
            "ping must not be delivered as an application message"
        );

        // Graceful close: the CLOSE frame must actually go out and close the peer.
        client.control().graceful_close(7, "bye", now).unwrap();
        let (_, len) = client
            .produce_outgoing_datagram(now, &mut buf)
            .unwrap()
            .unwrap();
        let r = server.handle_incoming_datagram(
            "127.0.0.1:5000".parse().unwrap(),
            &mut buf[..len],
            now,
        );
        assert!(matches!(r, Err(TransportError::ConnectionClosed(7))));
        assert!(server.hot.state == gtp_path::ConnectionState::Closed);
        let events = client.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            ControlEvent::StateChanged {
                new_state: gtp_path::ConnectionState::Draining,
                ..
            }
        )));
    }
}
