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
        self.header.payload_len = self.payload_len as u16;
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

        let payload_slice = &buffer[consumed_hdr..total_packet_bytes];
        let frames: Vec<Frame<'_>> = FrameIterator::new(payload_slice)
            .collect::<Result<Vec<_>>>()
            .unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], ack_frame);
        assert_eq!(frames[1], data_frame);
    }
}
