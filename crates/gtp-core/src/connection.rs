use crate::api::{NetworkFeedback, ReceivedMessage};
use crate::control::{ConnectionControl, ControlEvent, GtpConfig};
use crate::state::{ConnectionCold, ConnectionHot};
use gtp_cc::{calculate_backpressure, BackpressureLevel, CongestionController};
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
        Self::new_with_config(cid, peer_addr, secure, GtpConfig::default())
    }

    pub fn new_with_config(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
        config: GtpConfig,
    ) -> Self {
        Self {
            hot: ConnectionHot::new(cid, peer_addr, secure),
            cold: ConnectionCold::default(),
            config,
            event_queue: Vec::with_capacity(32),
            last_backpressure: BackpressureLevel::Low,
        }
    }

    pub fn new_with_session_keys(
        cid: ConnectionId,
        peer_addr: SocketAddr,
        key: [u8; 32],
        iv: [u8; 12],
        pre_validated: bool,
        config: GtpConfig,
    ) -> Self {
        Self {
            hot: ConnectionHot::new_with_session_keys(cid, peer_addr, key, iv, pre_validated),
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
        let msg_id = MessageId(self.hot.next_message_id);
        self.hot.next_message_id += 1;

        let order_seq = self
            .hot
            .next_order_seqs
            .entry(group_id.as_u16())
            .or_insert(0);
        let current_order_seq = *order_seq;
        *order_seq += 1;

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

        // 3. Replay Protection Window
        self.hot
            .replay_window
            .check_and_update(header.packet_number)?;

        // 4. Decrypt & Authenticate Payload
        let (aad_slice, encrypted_payload) = datagram.split_at_mut(header_consumed);
        let ciphertext_len = encrypted_payload.len();
        let decrypted_len = match self.hot.protector.open(
            header.packet_number,
            header.connection_id,
            aad_slice,
            encrypted_payload,
            ciphertext_len,
        ) {
            Ok(len) => {
                // Legitimate authenticated packet proves peer address ownership
                self.hot.anti_amplification.mark_validated();
                len
            }
            Err(e) => {
                self.cold.total_corrupted_packets += 1;
                return Err(e);
            }
        };

        let decrypted_slice = &encrypted_payload[..decrypted_len];
        let mut delivered_messages = Vec::new();
        let mut is_ack_eliciting = false;

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

                    self.hot.cc.on_ack(&ack_ev, now);
                    self.hot.cc.on_loss(&loss_ev, now);

                    if !loss_ev.lost_packets.is_empty() {
                        self.event_queue.push(ControlEvent::PacketLossDetected {
                            lost_count: loss_ev.lost_packets.len(),
                            lost_bytes: loss_ev.bytes_lost,
                        });
                    }

                    // Re-enqueue lost reliable frames for retransmission
                    for retrans in loss_ev.retransmittable {
                        self.cold.total_retransmissions += 1;
                        self.event_queue
                            .push(ControlEvent::RetransmissionTriggered {
                                message_id: retrans.message_id,
                                fragment_id: retrans.fragment_id,
                            });

                        let item = SchedulableItem {
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
                        };
                        let _ = self.hot.scheduler.enqueue(item, now);
                    }
                }

                Frame::Data {
                    state_key,
                    sequence,
                    generation,
                    payload,
                    ..
                } => {
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
                    group_id,
                    order_seq,
                    payload,
                    ..
                } => {
                    if group_id.as_u16() == 0 {
                        // Unordered reliable delivery
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

                        let ready_items = group.on_incoming(order_seq, payload)?;
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
                }

                Frame::Retx { payload, .. } => {
                    delivered_messages.push(ReceivedMessage {
                        class: MessageClass::ReliableUnordered,
                        payload: payload.to_vec(),
                    });
                }

                Frame::Ping { .. } => {}

                Frame::PathChallenge { data } => {
                    let resp = Frame::PathResponse { data };
                    let item = SchedulableItem {
                        message_id: MessageId(self.hot.next_message_id),
                        class: MessageClass::Unreliable,
                        priority: PriorityTier::P0Control,
                        created_at: now,
                        deadline: None,
                        supersedable: true,
                        payload: {
                            let mut b = [0u8; 16];
                            let _ = resp.encode(&mut b);
                            b[..9].to_vec()
                        },
                    };
                    self.hot.next_message_id += 1;
                    let _ = self.hot.scheduler.enqueue(item, now);
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
                    self.config.ack_frequency_packets = ack_frequency_packets;
                    self.config.max_ack_delay =
                        gtp_types::Duration::from_millis(max_ack_delay_ms as u64);
                    self.config.ack_reorder_threshold = reorder_threshold;
                }

                Frame::Close { error_code, .. } => {
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

                _ => {}
            }
        }

        // 6. Update ACK Tracker
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

        Ok(delivered_messages)
    }

    // ==========================================
    // TX Pipeline
    // ==========================================

    pub fn produce_outgoing_datagram(
        &mut self,
        now: MonotonicTime,
        out_buf: &mut [u8],
    ) -> Result<Option<(SocketAddr, usize)>> {
        if !self.hot.state.is_active()
            && !matches!(self.hot.state, gtp_path::ConnectionState::Handshaking)
        {
            return Ok(None);
        }

        // 0. Check for Probe Timeout (PTO) to trigger retransmissions
        if self.hot.loss_detector.inflight_bytes() > 0 {
            let pto = self.hot.loss_detector.rtt_stats.pto_duration();
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

                    let item = SchedulableItem {
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
                    };
                    let _ = self.hot.scheduler.enqueue(item, now);
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
        let has_queued_data = !self.hot.scheduler.is_empty();

        if !should_ack && !has_queued_data {
            return Ok(None);
        }

        if !should_ack && send_budget < 64 {
            return Ok(None); // Constrained by congestion window or pacing
        }

        let pn = self.hot.next_packet_number;
        let ts = now.as_micros() as u32;

        let header = PacketHeader::new_short(self.hot.connection_id, pn, ts, 0);
        let mut builder = PacketBuilder::new(out_buf, header)?;

        let mut retransmittables = Vec::new();
        let mut ack_eliciting = false;

        // 3. Attach ACK Frame if needed
        if should_ack || has_queued_data {
            if let Some(ack_frame) = self.hot.ack_tracker.generate_ack_frame(now) {
                let _ = builder.append_frame(&ack_frame);
            }
        }

        // 4. Pop and encode data frames within send budget
        while builder.remaining_capacity() > 64 {
            let remaining_budget = send_budget.min(builder.remaining_capacity());
            if let Some(item) = self.hot.scheduler.pop_next(remaining_budget, now) {
                ack_eliciting = true;
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
                        let _ = builder.append_frame(&frame);
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
                        let _ = builder.append_frame(&frame);
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
                        let _ = builder.append_frame(&frame);
                        retransmittables.push(RetransmissionRecord {
                            message_id: item.message_id,
                            fragment_id: FragmentId(0),
                            transmission_id: TransmissionId(1),
                            group_id: 0,
                            order_seq: 0,
                            payload: item.payload,
                        });
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
                        let _ = builder.append_frame(&frame);
                        retransmittables.push(RetransmissionRecord {
                            message_id: item.message_id,
                            fragment_id: FragmentId(0),
                            transmission_id: TransmissionId(1),
                            group_id: group_id.as_u16(),
                            order_seq,
                            payload: item.payload,
                        });
                    }
                }
            } else {
                break;
            }
        }

        // 5. Finalize unencrypted header & calculate final payload length including tag
        let unencrypted_len = builder.finish()?;
        let header_len = MIN_COMMON_HEADER_LEN;
        let unsealed_payload_len = unencrypted_len - header_len;
        let final_payload_len = (unsealed_payload_len + self.hot.protector.tag_len()) as u16;

        // Set the final payload_len in the header slice before sealing so AAD matches exactly
        out_buf[header_len - 2..header_len].copy_from_slice(&final_payload_len.to_be_bytes());

        // 6. Seal Payload with AEAD
        let (aad_slice, payload_slice) = out_buf.split_at_mut(header_len);
        let sealed_payload_len = self.hot.protector.seal(
            pn,
            self.hot.connection_id,
            &aad_slice[..header_len],
            payload_slice,
            unsealed_payload_len,
        )?;

        let total_datagram_len = header_len + sealed_payload_len;

        // Check Anti-Amplification Limiter
        if !self.hot.anti_amplification.can_send(total_datagram_len) {
            return Ok(None);
        }
        self.hot
            .anti_amplification
            .on_bytes_sent(total_datagram_len);

        // 7. Record In-Flight and CC Telemetry
        let sent_record = SentPacketRecord {
            packet_number: pn,
            send_time: now,
            bytes: total_datagram_len,
            ack_eliciting,
            in_flight: ack_eliciting,
            retransmittable_frames: retransmittables,
        };

        self.hot.loss_detector.on_packet_sent(sent_record);
        self.hot.cc.on_packet_sent(pn, total_datagram_len, now);
        self.hot.pacing.consume(total_datagram_len);

        self.hot.next_packet_number = pn.next();
        self.cold.total_tx_packets += 1;
        self.cold.total_tx_bytes += total_datagram_len as u64;

        Ok(Some((self.hot.active_path, total_datagram_len)))
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

    #[test]
    fn test_gtp_connection_send_and_receive_pipeline() {
        let client_addr: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let server_addr: SocketAddr = "127.0.0.1:6000".parse().unwrap();
        let cid = ConnectionId(0x1122334455667788);

        let mut client = GtpConnection::new(cid, server_addr, true);
        let mut server = GtpConnection::new(cid, client_addr, true);

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
                client_addr,
                &mut in_buffer[..len],
                now + Duration::from_millis(10),
            )
            .unwrap();

        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0].payload, b"client_input");
        assert_eq!(delivered[1].payload, b"player_damage");
    }

    #[test]
    fn test_control_api_runtime_tuning_and_events() {
        let server_addr: SocketAddr = "127.0.0.1:6000".parse().unwrap();
        let cid = ConnectionId(0x9988776655443322);

        let mut conn =
            GtpConnection::new_with_config(cid, server_addr, true, GtpConfig::competitive_fps());

        let now = MonotonicTime::from_micros(1_000_000);

        // 1. Test Control API ACK frequency adjustment
        assert!(conn.control().set_ack_frequency(4, 10, 2, now).is_ok());

        // 2. Test Control API Ping frame dispatch
        assert!(conn.control().send_ping(0xDEADBEEF, now).is_ok());

        // 3. Test Metrics query
        let metrics = conn.control().query_metrics(now);
        assert_eq!(metrics.total_tx_packets, 0);
        assert_eq!(metrics.backpressure, BackpressureLevel::Low);

        // 4. Test Event Drain on Graceful Close
        assert!(conn
            .control()
            .graceful_close(0, "Game session ended", now)
            .is_ok());
        let events = conn.drain_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            ControlEvent::StateChanged {
                new_state: gtp_path::ConnectionState::Draining,
                ..
            }
        ));
    }
}
