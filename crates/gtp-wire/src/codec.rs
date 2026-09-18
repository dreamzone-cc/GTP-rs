use crate::frame::Frame;
use crate::header::PacketHeader;
use gtp_types::{Result, TransportError};

/// Zero-allocation frame iterator over a packet's payload bytes.
pub struct FrameIterator<'a> {
    buf: &'a [u8],
    offset: usize,
}

impl<'a> FrameIterator<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, offset: 0 }
    }

    pub fn remaining(&self) -> &'a [u8] {
        &self.buf[self.offset..]
    }
}

impl<'a> Iterator for FrameIterator<'a> {
    type Item = Result<Frame<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.buf.len() {
            return None;
        }

        match Frame::decode(&self.buf[self.offset..]) {
            Ok((frame, consumed)) => {
                // Network-facing parser invariant: a decode that consumes zero
                // bytes would make `next()` never advance — iterating a hostile
                // payload becomes an infinite loop. Fail closed instead.
                if consumed == 0 {
                    self.offset = self.buf.len();
                    return Some(Err(TransportError::MalformedFrame(
                        "decoded frame consumed zero bytes",
                    )));
                }
                self.offset += consumed;
                Some(Ok(frame))
            }
            Err(e) => {
                self.offset = self.buf.len(); // Stop further iteration on error
                Some(Err(e))
            }
        }
    }
}

/// Zero-allocation packet builder for constructing GTP datagrams.
pub struct PacketBuilder<'a> {
    buf: &'a mut [u8],
    header: PacketHeader,
    header_len: usize,
    payload_len: usize,
}

impl<'a> PacketBuilder<'a> {
    pub fn new(buf: &'a mut [u8], header: PacketHeader) -> Result<Self> {
        let is_long = header.flags.is_long_header();
        let header_len = if is_long {
            crate::header::MIN_LONG_HEADER_LEN
        } else {
            crate::header::MIN_COMMON_HEADER_LEN
        };

        if buf.len() < header_len {
            return Err(TransportError::BufferTooShort);
        }
        // The header's own `header_len` field is honored by `PacketHeader::decode`
        // (bytes up to it are skipped as "future extensions"). If a caller hands
        // us a header whose declared length disagrees with the flags-derived size,
        // the peer's decoder would silently skip payload bytes as phantom
        // extensions — reject the disagreement here.
        if header.header_len as usize != header_len {
            return Err(TransportError::InvalidPacket(
                "header_len field disagrees with flags-derived header size",
            ));
        }

        Ok(Self {
            buf,
            header,
            header_len,
            payload_len: 0,
        })
    }

    pub fn remaining_capacity(&self) -> usize {
        self.buf
            .len()
            .saturating_sub(self.header_len + self.payload_len)
    }

    pub fn append_frame(&mut self, frame: &Frame<'_>) -> Result<()> {
        let current_offset = self.header_len + self.payload_len;
        let written = frame.encode(&mut self.buf[current_offset..])?;
        self.payload_len += written;

        if let Frame::Ack { .. } = frame {
            self.header.flags.set_ack_present(true);
        }

        Ok(())
    }

    pub fn finish(mut self) -> Result<usize> {
        // Typed bound, not a truncating cast: a wrapped payload_len would put a
        // spec-violating length on the wire while this returns the true count.
        self.header.payload_len = u16::try_from(self.payload_len).map_err(|_| {
            TransportError::ResourceLimitExceeded("payload length exceeds u16::MAX")
        })?;
        let _ = self.header.encode(&mut self.buf[..self.header_len])?;
        Ok(self.header_len + self.payload_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{AckRange, MAX_ACK_RANGES};
    use gtp_types::{ConnectionId, GenerationId, MessageId, PacketNumber, StateKey, StateSequence};

    #[test]
    fn test_packet_builder_and_frame_iterator_roundtrip() {
        let mut buffer = [0u8; 512];
        let header = PacketHeader::new_short(
            ConnectionId(0x1234567890ABCDEF),
            PacketNumber(500),
            1_000_000,
            0,
        );

        let mut builder = PacketBuilder::new(&mut buffer, header).unwrap();

        // 1. Append ACK Frame
        let mut ranges = [AckRange::default(); MAX_ACK_RANGES];
        ranges[0] = AckRange { gap: 0, length: 1 };
        let ack_frame = Frame::Ack {
            largest_acked: PacketNumber(499),
            ack_delay_us: 200,
            ranges,
            range_count: 1,
            ect0_count: 0,
            ect1_count: 0,
            ce_count: 0,
        };
        builder.append_frame(&ack_frame).unwrap();

        // 2. Append DATA Frame
        let payload = b"player_position_data";
        let data_frame = Frame::Data {
            message_id: MessageId(10),
            state_key: StateKey::new(1, 0),
            sequence: StateSequence(1),
            generation: GenerationId(1),
            deadline_ms: 50,
            payload,
        };
        builder.append_frame(&data_frame).unwrap();

        let total_packet_bytes = builder.finish().unwrap();
        assert!(total_packet_bytes > crate::header::MIN_COMMON_HEADER_LEN);

        // Decode packet
        let (decoded_hdr, consumed_hdr) =
            PacketHeader::decode(&buffer[..total_packet_bytes]).unwrap();
        assert_eq!(decoded_hdr.connection_id, ConnectionId(0x1234567890ABCDEF));
        assert_eq!(decoded_hdr.packet_number, PacketNumber(500));
        assert!(decoded_hdr.flags.has_ack());
        // Header/payload consistency invariant: the encoded payload_len field
        // must account for exactly the payload bytes on the wire (a wrong
        // `finish()` — e.g. a wrapped `as u16` — would fail here).
        assert_eq!(
            consumed_hdr + decoded_hdr.payload_len as usize,
            total_packet_bytes
        );

        let payload_slice = &buffer[consumed_hdr..total_packet_bytes];
        let frames: Vec<Frame<'_>> = FrameIterator::new(payload_slice)
            .collect::<Result<Vec<_>>>()
            .unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], ack_frame);
        assert_eq!(frames[1], data_frame);
    }

    /// A payload that cannot be represented in the header's u16 payload_len
    /// must be REJECTED with a typed error, never silently truncated to a
    /// wrapped length on the wire.
    #[test]
    fn finish_rejects_payload_over_u16_max() {
        let payload_len = 70_000usize;
        let mut buffer = vec![0u8; crate::header::MIN_COMMON_HEADER_LEN + payload_len];
        let header =
            PacketHeader::new_short(ConnectionId(0x1234567890ABCDEF), PacketNumber(1), 0, 0);
        let mut builder = PacketBuilder::new(&mut buffer, header).unwrap();

        let big = Frame::Padding {
            // minus the frame tag + length varint so encode fits the buffer
            len: payload_len - 8,
        };
        builder.append_frame(&big).unwrap();
        let res = builder.finish();
        match res {
            Err(TransportError::ResourceLimitExceeded(msg)) => {
                assert!(msg.contains("u16::MAX"));
            }
            other => panic!(
                "expected ResourceLimitExceeded, got {:?}",
                other.map(|_| ())
            ),
        }
    }

    /// A header whose declared header_len disagrees with its flags-derived size
    /// is rejected at builder construction (the peer would otherwise skip real
    /// payload bytes as phantom "future extensions").
    #[test]
    fn builder_rejects_header_len_disagreement() {
        let mut buffer = [0u8; 512];
        let header = PacketHeader::new_short(ConnectionId(1), PacketNumber(1), 0, 0);
        let mut tampered = header;
        tampered.header_len = (crate::header::MIN_COMMON_HEADER_LEN + 4) as u8;
        let res = PacketBuilder::new(&mut buffer, tampered);
        assert!(matches!(res, Err(TransportError::InvalidPacket(_))));
        // The honest header still builds.
        assert!(PacketBuilder::new(&mut buffer, header).is_ok());
    }
}
