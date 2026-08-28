use gtp_types::{
    ConnectionId, FragmentId, GenerationId, MessageId, OrderedGroupId, PacketNumber, Result,
    StateKey, StateSequence, TransmissionId, TransportError,
};

pub const FRAME_TYPE_ACK: u8 = 0x01;
pub const FRAME_TYPE_DATA: u8 = 0x02;
pub const FRAME_TYPE_RELIABLE_DATA: u8 = 0x03;
pub const FRAME_TYPE_RETX: u8 = 0x04;
pub const FRAME_TYPE_PING: u8 = 0x05;
pub const FRAME_TYPE_PATH_CHALLENGE: u8 = 0x06;
pub const FRAME_TYPE_PATH_RESPONSE: u8 = 0x07;
pub const FRAME_TYPE_MTU_PROBE: u8 = 0x08;
pub const FRAME_TYPE_CLOSE: u8 = 0x09;
pub const FRAME_TYPE_ACK_FREQUENCY: u8 = 0x0A;
pub const FRAME_TYPE_HANDSHAKE_INIT: u8 = 0x0B;
pub const FRAME_TYPE_HANDSHAKE_RESPONSE: u8 = 0x0C;
pub const FRAME_TYPE_HANDSHAKE_FINISH: u8 = 0x0D;
pub const FRAME_TYPE_PADDING: u8 = 0x0E;

pub const MAX_ACK_RANGES: usize = 32;

/// A contiguous range of acknowledged packets: [start, end].
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct AckRange {
    pub gap: u32,
    pub length: u32,
}

/// Strongly typed frames for the GTP wire format.
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum Frame<'a> {
    Ack {
        largest_acked: PacketNumber,
        ack_delay_us: u32,
        ranges: [AckRange; MAX_ACK_RANGES],
        range_count: u8,
        ect0_count: u32,
        ect1_count: u32,
        ce_count: u32,
    },
    Data {
        message_id: MessageId,
        state_key: StateKey,
        sequence: StateSequence,
        generation: GenerationId,
        deadline_ms: u16,
        payload: &'a [u8],
    },
    ReliableData {
        message_id: MessageId,
        fragment_id: FragmentId,
        total_fragments: u16,
        group_id: OrderedGroupId,
        order_seq: u32,
        payload: &'a [u8],
    },
    Retx {
        message_id: MessageId,
        fragment_id: FragmentId,
        transmission_id: TransmissionId,
        payload: &'a [u8],
    },
    Ping {
        nonce: u64,
    },
    PathChallenge {
        data: [u8; 8],
    },
    PathResponse {
        data: [u8; 8],
    },
    MtuProbe {
        probe_id: u32,
        padding_len: usize,
    },
    Close {
        error_code: u16,
        reason: &'a str,
    },
    AckFrequency {
        ack_frequency_packets: u8,
        max_ack_delay_ms: u16,
        reorder_threshold: u8,
    },
    HandshakeInit {
        client_nonce: [u8; 16],
        version: u32,
    },
    HandshakeResponse {
        server_nonce: [u8; 16],
        stateless_cookie: [u8; 32],
        assigned_cid: ConnectionId,
    },
    HandshakeFinish {
        cookie_echo: [u8; 32],
        client_proof: [u8; 16],
    },
    Padding {
        len: usize,
    },
}

impl<'a> Frame<'a> {
    pub fn frame_type(&self) -> u8 {
        match self {
            Self::Ack { .. } => FRAME_TYPE_ACK,
            Self::Data { .. } => FRAME_TYPE_DATA,
            Self::ReliableData { .. } => FRAME_TYPE_RELIABLE_DATA,
            Self::Retx { .. } => FRAME_TYPE_RETX,
            Self::Ping { .. } => FRAME_TYPE_PING,
            Self::PathChallenge { .. } => FRAME_TYPE_PATH_CHALLENGE,
            Self::PathResponse { .. } => FRAME_TYPE_PATH_RESPONSE,
            Self::MtuProbe { .. } => FRAME_TYPE_MTU_PROBE,
            Self::Close { .. } => FRAME_TYPE_CLOSE,
            Self::AckFrequency { .. } => FRAME_TYPE_ACK_FREQUENCY,
            Self::HandshakeInit { .. } => FRAME_TYPE_HANDSHAKE_INIT,
            Self::HandshakeResponse { .. } => FRAME_TYPE_HANDSHAKE_RESPONSE,
            Self::HandshakeFinish { .. } => FRAME_TYPE_HANDSHAKE_FINISH,
            Self::Padding { .. } => FRAME_TYPE_PADDING,
        }
    }

    pub fn is_ack_eliciting(&self) -> bool {
        !matches!(self, Self::Ack { .. } | Self::Padding { .. })
    }

    pub fn encode(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Err(TransportError::BufferTooShort);
        }

        buf[0] = self.frame_type();
        let mut offset = 1;

        match self {
            Self::Ack {
                largest_acked,
                ack_delay_us,
                ranges,
                range_count,
                ect0_count,
                ect1_count,
                ce_count,
            } => {
                let required = 1 + 8 + 4 + 1 + (*range_count as usize * 8) + 12;
                if buf.len() < required {
                    return Err(TransportError::BufferTooShort);
                }

                buf[offset..offset + 8].copy_from_slice(&largest_acked.as_u64().to_be_bytes());
                offset += 8;

                buf[offset..offset + 4].copy_from_slice(&ack_delay_us.to_be_bytes());
                offset += 4;

                buf[offset] = *range_count;
                offset += 1;

                let count = (*range_count as usize).min(MAX_ACK_RANGES);
                for i in 0..count {
                    buf[offset..offset + 4].copy_from_slice(&ranges[i].gap.to_be_bytes());
                    offset += 4;
                    buf[offset..offset + 4].copy_from_slice(&ranges[i].length.to_be_bytes());
                    offset += 4;
                }

                buf[offset..offset + 4].copy_from_slice(&ect0_count.to_be_bytes());
                offset += 4;
                buf[offset..offset + 4].copy_from_slice(&ect1_count.to_be_bytes());
                offset += 4;
                buf[offset..offset + 4].copy_from_slice(&ce_count.to_be_bytes());
                offset += 4;
            }

            Self::Data {
                message_id,
                state_key,
                sequence,
                generation,
                deadline_ms,
                payload,
            } => {
                let required = 1 + 8 + 6 + 4 + 4 + 2 + 2 + payload.len();
                if buf.len() < required {
                    return Err(TransportError::BufferTooShort);
                }

                buf[offset..offset + 8].copy_from_slice(&message_id.as_u64().to_be_bytes());
                offset += 8;

                buf[offset..offset + 6].copy_from_slice(&state_key.to_u48().to_be_bytes()[2..8]);
                offset += 6;

                buf[offset..offset + 4].copy_from_slice(&sequence.as_u32().to_be_bytes());
                offset += 4;

                buf[offset..offset + 4].copy_from_slice(&generation.as_u32().to_be_bytes());
                offset += 4;

                buf[offset..offset + 2].copy_from_slice(&deadline_ms.to_be_bytes());
                offset += 2;

                let len_u16 = payload.len() as u16;
                buf[offset..offset + 2].copy_from_slice(&len_u16.to_be_bytes());
                offset += 2;

                buf[offset..offset + payload.len()].copy_from_slice(payload);
                offset += payload.len();
            }

            Self::ReliableData {
                message_id,
                fragment_id,
                total_fragments,
                group_id,
                order_seq,
                payload,
            } => {
                let required = 1 + 8 + 2 + 2 + 2 + 4 + 2 + payload.len();
                if buf.len() < required {
                    return Err(TransportError::BufferTooShort);
                }

                buf[offset..offset + 8].copy_from_slice(&message_id.as_u64().to_be_bytes());
                offset += 8;

                buf[offset..offset + 2].copy_from_slice(&fragment_id.as_u16().to_be_bytes());
                offset += 2;

                buf[offset..offset + 2].copy_from_slice(&total_fragments.to_be_bytes());
                offset += 2;

                buf[offset..offset + 2].copy_from_slice(&group_id.as_u16().to_be_bytes());
                offset += 2;

                buf[offset..offset + 4].copy_from_slice(&order_seq.to_be_bytes());
                offset += 4;

                let len_u16 = payload.len() as u16;
                buf[offset..offset + 2].copy_from_slice(&len_u16.to_be_bytes());
                offset += 2;

                buf[offset..offset + payload.len()].copy_from_slice(payload);
                offset += payload.len();
            }

            Self::Retx {
                message_id,
                fragment_id,
                transmission_id,
                payload,
            } => {
                let required = 1 + 8 + 2 + 1 + 2 + payload.len();
                if buf.len() < required {
                    return Err(TransportError::BufferTooShort);
                }

                buf[offset..offset + 8].copy_from_slice(&message_id.as_u64().to_be_bytes());
                offset += 8;

                buf[offset..offset + 2].copy_from_slice(&fragment_id.as_u16().to_be_bytes());
                offset += 2;

                buf[offset] = transmission_id.as_u8();
                offset += 1;

                let len_u16 = payload.len() as u16;
                buf[offset..offset + 2].copy_from_slice(&len_u16.to_be_bytes());
                offset += 2;

                buf[offset..offset + payload.len()].copy_from_slice(payload);
                offset += payload.len();
            }

            Self::Ping { nonce } => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + 8].copy_from_slice(&nonce.to_be_bytes());
                offset += 8;
            }

            Self::PathChallenge { data } | Self::PathResponse { data } => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + 8].copy_from_slice(data);
                offset += 8;
            }

            Self::MtuProbe {
                probe_id,
                padding_len,
            } => {
                if buf.len() < offset + 4 + *padding_len {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + 4].copy_from_slice(&probe_id.to_be_bytes());
                offset += 4;
                buf[offset..offset + *padding_len].fill(0);
                offset += *padding_len;
            }

            Self::Close { error_code, reason } => {
                let reason_bytes = reason.as_bytes();
                let reason_len = reason_bytes.len().min(255) as u8;
                if buf.len() < offset + 2 + 1 + (reason_len as usize) {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + 2].copy_from_slice(&error_code.to_be_bytes());
                offset += 2;
                buf[offset] = reason_len;
                offset += 1;
                buf[offset..offset + (reason_len as usize)]
                    .copy_from_slice(&reason_bytes[..reason_len as usize]);
                offset += reason_len as usize;
            }

            Self::AckFrequency {
                ack_frequency_packets,
                max_ack_delay_ms,
                reorder_threshold,
            } => {
                if buf.len() < offset + 1 + 2 + 1 {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset] = *ack_frequency_packets;
                offset += 1;
                buf[offset..offset + 2].copy_from_slice(&max_ack_delay_ms.to_be_bytes());
                offset += 2;
                buf[offset] = *reorder_threshold;
                offset += 1;
            }

            Self::HandshakeInit {
                client_nonce,
                version,
            } => {
                if buf.len() < offset + 16 + 4 {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + 16].copy_from_slice(client_nonce);
                offset += 16;
                buf[offset..offset + 4].copy_from_slice(&version.to_be_bytes());
                offset += 4;
            }

            Self::HandshakeResponse {
                server_nonce,
                stateless_cookie,
                assigned_cid,
            } => {
                if buf.len() < offset + 16 + 32 + 8 {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + 16].copy_from_slice(server_nonce);
                offset += 16;
                buf[offset..offset + 32].copy_from_slice(stateless_cookie);
                offset += 32;
                buf[offset..offset + 8].copy_from_slice(&assigned_cid.to_be_bytes());
                offset += 8;
            }

            Self::HandshakeFinish {
                cookie_echo,
                client_proof,
            } => {
                if buf.len() < offset + 32 + 16 {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + 32].copy_from_slice(cookie_echo);
                offset += 32;
                buf[offset..offset + 16].copy_from_slice(client_proof);
                offset += 16;
            }

            Self::Padding { len } => {
                if buf.len() < offset + *len {
                    return Err(TransportError::BufferTooShort);
                }
                buf[offset..offset + *len].fill(0);
                offset += *len;
            }
        }

        Ok(offset)
    }

    pub fn decode(buf: &'a [u8]) -> Result<(Self, usize)> {
        if buf.is_empty() {
            return Err(TransportError::BufferTooShort);
        }

        let frame_type = buf[0];
        let mut offset = 1;

        match frame_type {
            FRAME_TYPE_ACK => {
                if buf.len() < offset + 8 + 4 + 1 + 12 {
                    return Err(TransportError::BufferTooShort);
                }

                let largest = PacketNumber::from_u64(u64::from_be_bytes(
                    buf[offset..offset + 8].try_into().unwrap(),
                ));
                offset += 8;

                let ack_delay = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;

                let range_count = buf[offset];
                offset += 1;

                if (range_count as usize) > MAX_ACK_RANGES {
                    return Err(TransportError::ResourceLimitExceeded("Too many ACK ranges"));
                }

                let total_ranges_bytes = (range_count as usize) * 8;
                if buf.len() < offset + total_ranges_bytes + 12 {
                    return Err(TransportError::BufferTooShort);
                }

                let mut ranges = [AckRange::default(); MAX_ACK_RANGES];
                for i in 0..(range_count as usize) {
                    let gap = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                    offset += 4;
                    let length = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                    offset += 4;
                    ranges[i] = AckRange { gap, length };
                }

                let ect0 = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;
                let ect1 = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;
                let ce = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;

                Ok((
                    Frame::Ack {
                        largest_acked: largest,
                        ack_delay_us: ack_delay,
                        ranges,
                        range_count,
                        ect0_count: ect0,
                        ect1_count: ect1,
                        ce_count: ce,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_DATA => {
                if buf.len() < offset + 8 + 6 + 4 + 4 + 2 + 2 {
                    return Err(TransportError::BufferTooShort);
                }

                let msg_id = MessageId::from_u64(u64::from_be_bytes(
                    buf[offset..offset + 8].try_into().unwrap(),
                ));
                offset += 8;

                let mut key_bytes = [0u8; 8];
                key_bytes[2..8].copy_from_slice(&buf[offset..offset + 6]);
                let state_key = StateKey::from_u48(u64::from_be_bytes(key_bytes));
                offset += 6;

                let seq = StateSequence::from_u32(u32::from_be_bytes(
                    buf[offset..offset + 4].try_into().unwrap(),
                ));
                offset += 4;

                let gen = GenerationId::from_u32(u32::from_be_bytes(
                    buf[offset..offset + 4].try_into().unwrap(),
                ));
                offset += 4;

                let deadline = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
                offset += 2;

                let payload_len =
                    u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap()) as usize;
                offset += 2;

                if buf.len() < offset + payload_len {
                    return Err(TransportError::BufferTooShort);
                }

                let payload = &buf[offset..offset + payload_len];
                offset += payload_len;

                Ok((
                    Frame::Data {
                        message_id: msg_id,
                        state_key,
                        sequence: seq,
                        generation: gen,
                        deadline_ms: deadline,
                        payload,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_RELIABLE_DATA => {
                if buf.len() < offset + 8 + 2 + 2 + 2 + 4 + 2 {
                    return Err(TransportError::BufferTooShort);
                }

                let msg_id = MessageId::from_u64(u64::from_be_bytes(
                    buf[offset..offset + 8].try_into().unwrap(),
                ));
                offset += 8;

                let frag_id = FragmentId(u16::from_be_bytes(
                    buf[offset..offset + 2].try_into().unwrap(),
                ));
                offset += 2;

                let total_frags = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
                offset += 2;

                let group_id = OrderedGroupId(u16::from_be_bytes(
                    buf[offset..offset + 2].try_into().unwrap(),
                ));
                offset += 2;

                let order_seq = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;

                let payload_len =
                    u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap()) as usize;
                offset += 2;

                if buf.len() < offset + payload_len {
                    return Err(TransportError::BufferTooShort);
                }

                let payload = &buf[offset..offset + payload_len];
                offset += payload_len;

                Ok((
                    Frame::ReliableData {
                        message_id: msg_id,
                        fragment_id: frag_id,
                        total_fragments: total_frags,
                        group_id,
                        order_seq,
                        payload,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_RETX => {
                if buf.len() < offset + 8 + 2 + 1 + 2 {
                    return Err(TransportError::BufferTooShort);
                }

                let msg_id = MessageId::from_u64(u64::from_be_bytes(
                    buf[offset..offset + 8].try_into().unwrap(),
                ));
                offset += 8;

                let frag_id = FragmentId(u16::from_be_bytes(
                    buf[offset..offset + 2].try_into().unwrap(),
                ));
                offset += 2;

                let transmission_id = TransmissionId(buf[offset]);
                offset += 1;

                let payload_len =
                    u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap()) as usize;
                offset += 2;

                if buf.len() < offset + payload_len {
                    return Err(TransportError::BufferTooShort);
                }

                let payload = &buf[offset..offset + payload_len];
                offset += payload_len;

                Ok((
                    Frame::Retx {
                        message_id: msg_id,
                        fragment_id: frag_id,
                        transmission_id,
                        payload,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_PING => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferTooShort);
                }
                let nonce = u64::from_be_bytes(buf[offset..offset + 8].try_into().unwrap());
                offset += 8;
                Ok((Frame::Ping { nonce }, offset))
            }

            FRAME_TYPE_PATH_CHALLENGE => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferTooShort);
                }
                let mut data = [0u8; 8];
                data.copy_from_slice(&buf[offset..offset + 8]);
                offset += 8;
                Ok((Frame::PathChallenge { data }, offset))
            }

            FRAME_TYPE_PATH_RESPONSE => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferTooShort);
                }
                let mut data = [0u8; 8];
                data.copy_from_slice(&buf[offset..offset + 8]);
                offset += 8;
                Ok((Frame::PathResponse { data }, offset))
            }

            FRAME_TYPE_MTU_PROBE => {
                if buf.len() < offset + 4 {
                    return Err(TransportError::BufferTooShort);
                }
                let probe_id = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;
                let padding_len = buf.len() - offset;
                offset = buf.len();
                Ok((
                    Frame::MtuProbe {
                        probe_id,
                        padding_len,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_CLOSE => {
                if buf.len() < offset + 2 + 1 {
                    return Err(TransportError::BufferTooShort);
                }
                let error_code = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
                offset += 2;
                let reason_len = buf[offset] as usize;
                offset += 1;
                if buf.len() < offset + reason_len {
                    return Err(TransportError::BufferTooShort);
                }
                let reason = core::str::from_utf8(&buf[offset..offset + reason_len])
                    .map_err(|_| TransportError::InvalidPacket("Malformed UTF-8 close reason"))?;
                offset += reason_len;
                Ok((Frame::Close { error_code, reason }, offset))
            }

            FRAME_TYPE_ACK_FREQUENCY => {
                if buf.len() < offset + 1 + 2 + 1 {
                    return Err(TransportError::BufferTooShort);
                }
                let ack_freq = buf[offset];
                offset += 1;
                let max_delay = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
                offset += 2;
                let reorder_thresh = buf[offset];
                offset += 1;
                Ok((
                    Frame::AckFrequency {
                        ack_frequency_packets: ack_freq,
                        max_ack_delay_ms: max_delay,
                        reorder_threshold: reorder_thresh,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_HANDSHAKE_INIT => {
                if buf.len() < offset + 16 + 4 {
                    return Err(TransportError::BufferTooShort);
                }
                let mut client_nonce = [0u8; 16];
                client_nonce.copy_from_slice(&buf[offset..offset + 16]);
                offset += 16;
                let version = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;
                Ok((
                    Frame::HandshakeInit {
                        client_nonce,
                        version,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_HANDSHAKE_RESPONSE => {
                if buf.len() < offset + 16 + 32 + 8 {
                    return Err(TransportError::BufferTooShort);
                }
                let mut server_nonce = [0u8; 16];
                server_nonce.copy_from_slice(&buf[offset..offset + 16]);
                offset += 16;
                let mut stateless_cookie = [0u8; 32];
                stateless_cookie.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;
                let cid = ConnectionId::from_be_bytes(buf[offset..offset + 8].try_into().unwrap());
                offset += 8;
                Ok((
                    Frame::HandshakeResponse {
                        server_nonce,
                        stateless_cookie,
                        assigned_cid: cid,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_HANDSHAKE_FINISH => {
                if buf.len() < offset + 32 + 16 {
                    return Err(TransportError::BufferTooShort);
                }
                let mut cookie_echo = [0u8; 32];
                cookie_echo.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;
                let mut client_proof = [0u8; 16];
                client_proof.copy_from_slice(&buf[offset..offset + 16]);
                offset += 16;
                Ok((
                    Frame::HandshakeFinish {
                        cookie_echo,
                        client_proof,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_PADDING => {
                let padding_len = buf.len() - offset;
                offset = buf.len();
                Ok((Frame::Padding { len: padding_len }, offset))
            }

            _ => Err(TransportError::InvalidPacket(
                "Unknown frame type encountered",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ack_frame_roundtrip() {
        let mut ranges = [AckRange::default(); MAX_ACK_RANGES];
        ranges[0] = AckRange { gap: 0, length: 5 };
        ranges[1] = AckRange { gap: 2, length: 10 };

        let frame = Frame::Ack {
            largest_acked: PacketNumber(1050),
            ack_delay_us: 1500,
            ranges,
            range_count: 2,
            ect0_count: 100,
            ect1_count: 0,
            ce_count: 2,
        };

        let mut buf = [0u8; 128];
        let len = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(consumed, len);
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_data_and_reliable_frames_roundtrip() {
        let payload = b"game_snapshot_data_12345";
        let data_frame = Frame::Data {
            message_id: MessageId(99),
            state_key: StateKey::new(101, 1),
            sequence: StateSequence(50),
            generation: GenerationId(4),
            deadline_ms: 100,
            payload,
        };

        let mut buf = [0u8; 128];
        let len = data_frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(consumed, len);
        assert_eq!(data_frame, decoded);

        let reliable_frame = Frame::ReliableData {
            message_id: MessageId(100),
            fragment_id: FragmentId(0),
            total_fragments: 1,
            group_id: OrderedGroupId(3),
            order_seq: 15,
            payload,
        };

        let len2 = reliable_frame.encode(&mut buf).unwrap();
        let (decoded2, consumed2) = Frame::decode(&buf[..len2]).unwrap();
        assert_eq!(consumed2, len2);
        assert_eq!(reliable_frame, decoded2);
    }

    #[test]
    fn test_control_frames_roundtrip() {
        let mut buf = [0u8; 128];

        let ping = Frame::Ping { nonce: 0xDEADBEEFCAFEBABE };
        let len = ping.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(ping, decoded);

        let challenge = Frame::PathChallenge { data: [1, 2, 3, 4, 5, 6, 7, 8] };
        let len = challenge.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(challenge, decoded);

        let close = Frame::Close { error_code: 0x0100, reason: "Session timeout" };
        let len = close.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(close, decoded);
    }
}
