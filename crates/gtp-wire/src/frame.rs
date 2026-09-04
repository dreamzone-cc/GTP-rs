use gtp_types::{
    ConnectionId, FragmentId, GenerationId, MessageId, OrderedGroupId, PacketNumber, Result,
    StateKey, StateSequence, TransmissionId, TransportError,
};

pub const MAX_ACK_RANGES: usize = 32;

pub const FRAME_TYPE_PADDING: u8 = 0x00;
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
pub const FRAME_TYPE_CLIENT_HELLO: u8 = 0x0B;
pub const FRAME_TYPE_HANDSHAKE_RESPONSE: u8 = 0x0C;
pub const FRAME_TYPE_SERVER_HELLO: u8 = 0x0C;
pub const FRAME_TYPE_HANDSHAKE_FINISH: u8 = 0x0D;

/// Compact ACK Range representation.
#[derive(Clone, Copy, Eq, PartialEq, Debug, Default)]
pub struct AckRange {
    pub gap: u32,
    pub length: u32,
}

/// Strongly typed frames for the GTP wire format.
#[derive(Clone, Eq, PartialEq, Debug)]
#[allow(clippy::large_enum_variant)]
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
    ClientHello {
        client_public_key: [u8; 32],
        client_nonce: [u8; 32],
        version: u32,
    },
    ServerHello {
        server_public_key: [u8; 32],
        server_nonce: [u8; 32],
        stateless_cookie: [u8; 32],
        assigned_cid: ConnectionId,
    },
    HandshakeFinish {
        cookie_echo: [u8; 32],
        client_proof: [u8; 32],
    },
    Padding {
        len: usize,
    },
}

#[inline(always)]
fn read_u8(buf: &[u8], offset: &mut usize) -> Result<u8> {
    if buf.len() < *offset + 1 {
        return Err(TransportError::TruncatedFrame {
            needed: 1,
            available: buf.len().saturating_sub(*offset),
        });
    }
    let b = buf[*offset];
    *offset += 1;
    Ok(b)
}

#[inline(always)]
fn read_u16(buf: &[u8], offset: &mut usize) -> Result<u16> {
    if buf.len() < *offset + 2 {
        return Err(TransportError::TruncatedFrame {
            needed: 2,
            available: buf.len().saturating_sub(*offset),
        });
    }
    let mut bytes = [0u8; 2];
    bytes.copy_from_slice(&buf[*offset..*offset + 2]);
    *offset += 2;
    Ok(u16::from_be_bytes(bytes))
}

#[inline(always)]
fn read_u32(buf: &[u8], offset: &mut usize) -> Result<u32> {
    if buf.len() < *offset + 4 {
        return Err(TransportError::TruncatedFrame {
            needed: 4,
            available: buf.len().saturating_sub(*offset),
        });
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&buf[*offset..*offset + 4]);
    *offset += 4;
    Ok(u32::from_be_bytes(bytes))
}

#[inline(always)]
fn read_u64(buf: &[u8], offset: &mut usize) -> Result<u64> {
    if buf.len() < *offset + 8 {
        return Err(TransportError::TruncatedFrame {
            needed: 8,
            available: buf.len().saturating_sub(*offset),
        });
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&buf[*offset..*offset + 8]);
    *offset += 8;
    Ok(u64::from_be_bytes(bytes))
}

#[inline(always)]
fn read_bytes<'a>(buf: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8]> {
    if buf.len() < *offset + len {
        return Err(TransportError::TruncatedFrame {
            needed: len,
            available: buf.len().saturating_sub(*offset),
        });
    }
    let slice = &buf[*offset..*offset + len];
    *offset += len;
    Ok(slice)
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
            Self::ClientHello { .. } => FRAME_TYPE_CLIENT_HELLO,
            Self::ServerHello { .. } => FRAME_TYPE_SERVER_HELLO,
            Self::HandshakeFinish { .. } => FRAME_TYPE_HANDSHAKE_FINISH,
            Self::Padding { .. } => FRAME_TYPE_PADDING,
        }
    }

    pub fn is_ack_eliciting(&self) -> bool {
        !matches!(self, Self::Ack { .. } | Self::Padding { .. })
    }

    pub fn encode(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Err(TransportError::BufferOverflow);
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
                let count = (*range_count as usize).min(MAX_ACK_RANGES);
                let needed = 1 + 8 + 4 + 1 + (count * 8) + 4 + 4 + 4;
                if buf.len() < needed {
                    return Err(TransportError::BufferOverflow);
                }

                buf[offset..offset + 8].copy_from_slice(&largest_acked.as_u64().to_be_bytes());
                offset += 8;

                buf[offset..offset + 4].copy_from_slice(&ack_delay_us.to_be_bytes());
                offset += 4;

                // WIR-2: write the clamped count so the header always matches
                // the number of ranges actually serialized below — the raw
                // count could exceed MAX_ACK_RANGES and the encoder would
                // emit a frame its own decoder rejects.
                buf[offset] = count as u8;
                offset += 1;

                for range in ranges.iter().take(count) {
                    buf[offset..offset + 4].copy_from_slice(&range.gap.to_be_bytes());
                    offset += 4;
                    buf[offset..offset + 4].copy_from_slice(&range.length.to_be_bytes());
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
                let needed = 1 + 8 + 6 + 4 + 4 + 2 + 2 + payload.len();
                if buf.len() < needed || payload.len() > u16::MAX as usize {
                    return Err(TransportError::BufferOverflow);
                }

                buf[offset..offset + 8].copy_from_slice(&message_id.as_u64().to_be_bytes());
                offset += 8;

                let key_bytes = state_key.to_u48().to_be_bytes();
                buf[offset..offset + 6].copy_from_slice(&key_bytes[2..8]);
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
                let needed = 1 + 8 + 2 + 2 + 2 + 4 + 2 + payload.len();
                if buf.len() < needed || payload.len() > u16::MAX as usize {
                    return Err(TransportError::BufferOverflow);
                }

                buf[offset..offset + 8].copy_from_slice(&message_id.as_u64().to_be_bytes());
                offset += 8;

                buf[offset..offset + 2].copy_from_slice(&fragment_id.0.to_be_bytes());
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
                let needed = 1 + 8 + 2 + 1 + 2 + payload.len();
                if buf.len() < needed || payload.len() > u16::MAX as usize {
                    return Err(TransportError::BufferOverflow);
                }

                buf[offset..offset + 8].copy_from_slice(&message_id.as_u64().to_be_bytes());
                offset += 8;

                buf[offset..offset + 2].copy_from_slice(&fragment_id.0.to_be_bytes());
                offset += 2;

                buf[offset] = transmission_id.0;
                offset += 1;

                let len_u16 = payload.len() as u16;
                buf[offset..offset + 2].copy_from_slice(&len_u16.to_be_bytes());
                offset += 2;

                buf[offset..offset + payload.len()].copy_from_slice(payload);
                offset += payload.len();
            }

            Self::Ping { nonce } => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + 8].copy_from_slice(&nonce.to_be_bytes());
                offset += 8;
            }

            Self::PathChallenge { data } => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + 8].copy_from_slice(data);
                offset += 8;
            }

            Self::PathResponse { data } => {
                if buf.len() < offset + 8 {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + 8].copy_from_slice(data);
                offset += 8;
            }

            Self::MtuProbe {
                probe_id,
                padding_len,
            } => {
                if buf.len() < offset + 4 + *padding_len {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + 4].copy_from_slice(&probe_id.to_be_bytes());
                offset += 4;
                buf[offset..offset + *padding_len].fill(0);
                offset += *padding_len;
            }

            Self::Close { error_code, reason } => {
                let reason_bytes = reason.as_bytes();
                // WIR-4: trim at a UTF-8 character boundary — a raw byte cut
                // can split a multi-byte character, and the peer's
                // `str::from_utf8` on the decoded reason would then reject
                // the whole frame. A cut position is a character boundary
                // iff the byte that follows it is not a UTF-8 continuation
                // byte (10xxxxxx).
                let mut reason_len = reason_bytes.len().min(255);
                while reason_len < reason_bytes.len() && (reason_bytes[reason_len] & 0xC0) == 0x80 {
                    reason_len -= 1;
                }
                let reason_len = reason_len as u8;
                if buf.len() < offset + 2 + 1 + (reason_len as usize) {
                    return Err(TransportError::BufferOverflow);
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
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset] = *ack_frequency_packets;
                offset += 1;
                buf[offset..offset + 2].copy_from_slice(&max_ack_delay_ms.to_be_bytes());
                offset += 2;
                buf[offset] = *reorder_threshold;
                offset += 1;
            }

            Self::ClientHello {
                client_public_key,
                client_nonce,
                version,
            } => {
                if buf.len() < offset + 32 + 32 + 4 {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + 32].copy_from_slice(client_public_key);
                offset += 32;
                buf[offset..offset + 32].copy_from_slice(client_nonce);
                offset += 32;
                buf[offset..offset + 4].copy_from_slice(&version.to_be_bytes());
                offset += 4;
            }

            Self::ServerHello {
                server_public_key,
                server_nonce,
                stateless_cookie,
                assigned_cid,
            } => {
                if buf.len() < offset + 32 + 32 + 32 + 8 {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + 32].copy_from_slice(server_public_key);
                offset += 32;
                buf[offset..offset + 32].copy_from_slice(server_nonce);
                offset += 32;
                buf[offset..offset + 32].copy_from_slice(stateless_cookie);
                offset += 32;
                buf[offset..offset + 8].copy_from_slice(&assigned_cid.to_be_bytes());
                offset += 8;
            }

            Self::HandshakeFinish {
                cookie_echo,
                client_proof,
            } => {
                if buf.len() < offset + 32 + 32 {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + 32].copy_from_slice(cookie_echo);
                offset += 32;
                buf[offset..offset + 32].copy_from_slice(client_proof);
                offset += 32;
            }

            Self::Padding { len } => {
                if buf.len() < offset + *len {
                    return Err(TransportError::BufferOverflow);
                }
                buf[offset..offset + *len].fill(0);
                offset += *len;
            }
        }

        Ok(offset)
    }

    pub fn decode(buf: &'a [u8]) -> Result<(Self, usize)> {
        let mut offset = 0;
        let frame_type = read_u8(buf, &mut offset)?;

        match frame_type {
            FRAME_TYPE_ACK => {
                let largest = PacketNumber::from_u64(read_u64(buf, &mut offset)?);
                let ack_delay = read_u32(buf, &mut offset)?;
                let range_count = read_u8(buf, &mut offset)?;

                if (range_count as usize) > MAX_ACK_RANGES {
                    return Err(TransportError::ResourceLimitExceeded("Too many ACK ranges"));
                }

                let mut ranges = [AckRange::default(); MAX_ACK_RANGES];
                for range in ranges.iter_mut().take(range_count as usize) {
                    let gap = read_u32(buf, &mut offset)?;
                    let length = read_u32(buf, &mut offset)?;
                    *range = AckRange { gap, length };
                }

                let ect0 = read_u32(buf, &mut offset)?;
                let ect1 = read_u32(buf, &mut offset)?;
                let ce = read_u32(buf, &mut offset)?;

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
                let msg_id = MessageId::from_u64(read_u64(buf, &mut offset)?);
                let key_slice = read_bytes(buf, &mut offset, 6)?;
                let mut key_bytes = [0u8; 8];
                key_bytes[2..8].copy_from_slice(key_slice);
                let state_key = StateKey::from_u48(u64::from_be_bytes(key_bytes));

                let seq = StateSequence::from_u32(read_u32(buf, &mut offset)?);
                let gen = GenerationId::from_u32(read_u32(buf, &mut offset)?);
                let deadline = read_u16(buf, &mut offset)?;
                let payload_len = read_u16(buf, &mut offset)? as usize;
                let payload = read_bytes(buf, &mut offset, payload_len)?;

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
                let msg_id = MessageId::from_u64(read_u64(buf, &mut offset)?);
                let frag_id = FragmentId(read_u16(buf, &mut offset)?);
                let total_frags = read_u16(buf, &mut offset)?;
                let group_id = OrderedGroupId(read_u16(buf, &mut offset)?);
                let order_seq = read_u32(buf, &mut offset)?;
                let payload_len = read_u16(buf, &mut offset)? as usize;
                let payload = read_bytes(buf, &mut offset, payload_len)?;

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
                let msg_id = MessageId::from_u64(read_u64(buf, &mut offset)?);
                let frag_id = FragmentId(read_u16(buf, &mut offset)?);
                let transmission_id = TransmissionId(read_u8(buf, &mut offset)?);
                let payload_len = read_u16(buf, &mut offset)? as usize;
                let payload = read_bytes(buf, &mut offset, payload_len)?;

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
                let nonce = read_u64(buf, &mut offset)?;
                Ok((Frame::Ping { nonce }, offset))
            }

            FRAME_TYPE_PATH_CHALLENGE => {
                let data_slice = read_bytes(buf, &mut offset, 8)?;
                let mut data = [0u8; 8];
                data.copy_from_slice(data_slice);
                Ok((Frame::PathChallenge { data }, offset))
            }

            FRAME_TYPE_PATH_RESPONSE => {
                let data_slice = read_bytes(buf, &mut offset, 8)?;
                let mut data = [0u8; 8];
                data.copy_from_slice(data_slice);
                Ok((Frame::PathResponse { data }, offset))
            }

            FRAME_TYPE_MTU_PROBE => {
                let probe_id = read_u32(buf, &mut offset)?;
                let padding_len = buf.len().saturating_sub(offset);
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
                let error_code = read_u16(buf, &mut offset)?;
                let reason_len = read_u8(buf, &mut offset)? as usize;
                let reason_bytes = read_bytes(buf, &mut offset, reason_len)?;
                let reason = core::str::from_utf8(reason_bytes)
                    .map_err(|_| TransportError::MalformedFrame("Malformed UTF-8 close reason"))?;
                Ok((Frame::Close { error_code, reason }, offset))
            }

            FRAME_TYPE_ACK_FREQUENCY => {
                let ack_freq = read_u8(buf, &mut offset)?;
                let max_delay = read_u16(buf, &mut offset)?;
                let reorder_thresh = read_u8(buf, &mut offset)?;
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
                let pk_slice = read_bytes(buf, &mut offset, 32)?;
                let mut client_public_key = [0u8; 32];
                client_public_key.copy_from_slice(pk_slice);

                let nonce_slice = read_bytes(buf, &mut offset, 32)?;
                let mut client_nonce = [0u8; 32];
                client_nonce.copy_from_slice(nonce_slice);

                let version = read_u32(buf, &mut offset)?;
                Ok((
                    Frame::ClientHello {
                        client_public_key,
                        client_nonce,
                        version,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_HANDSHAKE_RESPONSE => {
                let pk_slice = read_bytes(buf, &mut offset, 32)?;
                let mut server_public_key = [0u8; 32];
                server_public_key.copy_from_slice(pk_slice);

                let s_nonce = read_bytes(buf, &mut offset, 32)?;
                let mut server_nonce = [0u8; 32];
                server_nonce.copy_from_slice(s_nonce);

                let cookie = read_bytes(buf, &mut offset, 32)?;
                let mut stateless_cookie = [0u8; 32];
                stateless_cookie.copy_from_slice(cookie);

                let cid_bytes = read_u64(buf, &mut offset)?;
                let cid = ConnectionId(cid_bytes);

                Ok((
                    Frame::ServerHello {
                        server_public_key,
                        server_nonce,
                        stateless_cookie,
                        assigned_cid: cid,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_HANDSHAKE_FINISH => {
                let cookie = read_bytes(buf, &mut offset, 32)?;
                let mut cookie_echo = [0u8; 32];
                cookie_echo.copy_from_slice(cookie);

                let proof = read_bytes(buf, &mut offset, 32)?;
                let mut client_proof = [0u8; 32];
                client_proof.copy_from_slice(proof);

                Ok((
                    Frame::HandshakeFinish {
                        cookie_echo,
                        client_proof,
                    },
                    offset,
                ))
            }

            FRAME_TYPE_PADDING => {
                let padding_len = buf.len().saturating_sub(offset);
                offset = buf.len();
                Ok((Frame::Padding { len: padding_len }, offset))
            }

            _unknown => Err(TransportError::MalformedFrame("Unknown TLV frame type")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ack_frame_roundtrip() {
        let mut ranges = [AckRange::default(); MAX_ACK_RANGES];
        ranges[0] = AckRange { gap: 0, length: 10 };
        ranges[1] = AckRange { gap: 2, length: 5 };

        let frame = Frame::Ack {
            largest_acked: PacketNumber(100),
            ack_delay_us: 1500,
            ranges,
            range_count: 2,
            ect0_count: 50,
            ect1_count: 0,
            ce_count: 2,
        };

        let mut buf = [0u8; 256];
        let len = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = Frame::decode(&buf[..len]).unwrap();

        assert_eq!(len, consumed);
        assert_eq!(frame, decoded);
    }

    /// WIR-2: an ACK frame whose `range_count` exceeds the wire cap must
    /// encode self-consistently — the written count matches the ranges
    /// actually serialized, so the encoder can never emit a frame its own
    /// decoder rejects.
    #[test]
    fn ack_frame_with_uncapped_range_count_encodes_self_consistently() {
        let mut ranges = [AckRange::default(); MAX_ACK_RANGES];
        for (i, range) in ranges.iter_mut().enumerate() {
            *range = AckRange {
                gap: 1,
                length: (i + 1) as u32,
            };
        }
        let frame = Frame::Ack {
            largest_acked: PacketNumber(1000),
            ack_delay_us: 0,
            ranges,
            range_count: 40, // exceeds MAX_ACK_RANGES (32)
            ect0_count: 0,
            ect1_count: 0,
            ce_count: 0,
        };

        let mut buf = [0u8; 512];
        let len = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(len, consumed);
        match decoded {
            Frame::Ack { range_count, .. } => {
                assert_eq!(range_count, MAX_ACK_RANGES as u8);
            }
            other => panic!("expected an Ack frame, got {:?}", other.frame_type()),
        }
    }

    /// WIR-4: trimming the close reason at 255 bytes must land on a UTF-8
    /// character boundary so the peer's `from_utf8` accepts the frame.
    #[test]
    fn close_reason_truncates_at_a_utf8_char_boundary() {
        // 200 two-byte characters: the byte cap of 255 would split the 128th.
        let reason = "é".repeat(200);
        let frame = Frame::Close {
            error_code: 7,
            reason: &reason,
        };

        let mut buf = [0u8; 512];
        let len = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(len, consumed);
        match decoded {
            Frame::Close { reason, .. } => {
                assert_eq!(
                    reason.chars().count(),
                    127,
                    "127 whole characters fit in 254 bytes"
                );
                assert!(reason.chars().all(|c| c == 'é'));
            }
            other => panic!("expected a Close frame, got {:?}", other.frame_type()),
        }
    }

    #[test]
    fn test_data_and_reliable_frames_roundtrip() {
        let payload = b"hello_gameplay_state";
        let data_frame = Frame::Data {
            message_id: MessageId(1234),
            state_key: StateKey::new(10, 2),
            sequence: StateSequence(50),
            generation: GenerationId(1),
            deadline_ms: 100,
            payload,
        };

        let mut buf = [0u8; 256];
        let len = data_frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(len, consumed);
        assert_eq!(data_frame, decoded);

        let reliable_frame = Frame::ReliableData {
            message_id: MessageId(9999),
            fragment_id: FragmentId(0),
            total_fragments: 1,
            group_id: OrderedGroupId(5),
            order_seq: 42,
            payload,
        };

        let len2 = reliable_frame.encode(&mut buf).unwrap();
        let (decoded2, consumed2) = Frame::decode(&buf[..len2]).unwrap();
        assert_eq!(len2, consumed2);
        assert_eq!(reliable_frame, decoded2);
    }

    #[test]
    fn test_control_frames_roundtrip() {
        let mut buf = [0u8; 256];

        let ping = Frame::Ping { nonce: 0xCAFEBABE };
        let len = ping.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(ping, decoded);

        let challenge = Frame::PathChallenge {
            data: [1, 2, 3, 4, 5, 6, 7, 8],
        };
        let len = challenge.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(challenge, decoded);

        let close = Frame::Close {
            error_code: 0x01,
            reason: "Server shutting down",
        };
        let len = close.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(close, decoded);

        let client_hello = Frame::ClientHello {
            client_public_key: [0x42; 32],
            client_nonce: [0x77; 32],
            version: 0x0101,
        };
        let len = client_hello.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(client_hello, decoded);

        let server_hello = Frame::ServerHello {
            server_public_key: [0x55; 32],
            server_nonce: [0x88; 32],
            stateless_cookie: [0x99; 32],
            assigned_cid: ConnectionId(0xDEADBEEFCAFE),
        };
        let len = server_hello.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(server_hello, decoded);

        let finish = Frame::HandshakeFinish {
            cookie_echo: [0x99; 32],
            client_proof: [0xAA; 32],
        };
        let len = finish.encode(&mut buf).unwrap();
        let (decoded, _) = Frame::decode(&buf[..len]).unwrap();
        assert_eq!(finish, decoded);
    }
}
