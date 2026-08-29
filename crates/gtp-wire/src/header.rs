use gtp_types::{ConnectionId, PacketNumber, Result, TransportError};

pub const GTP_V1_1: u32 = 0x00010001;
pub const MIN_COMMON_HEADER_LEN: usize = 24; // Short header: flags(1) + header_len(1) + cid(8) + pkt_num(8) + ts(4) + payload_len(2)
pub const MIN_LONG_HEADER_LEN: usize = 28; // Long header: flags(1) + version(4) + header_len(1) + cid(8) + pkt_num(8) + ts(4) + payload_len(2)

/// Flags contained in the first byte of every GTP packet.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct HeaderFlags(pub u8);

impl HeaderFlags {
    pub const LONG_HEADER: u8 = 0b1000_0000;
    pub const KEY_PHASE: u8 = 0b0100_0000;
    pub const ACK_PRESENT: u8 = 0b0010_0000;
    pub const ECN_MASK: u8 = 0b0001_1000;

    pub fn is_long_header(self) -> bool {
        (self.0 & Self::LONG_HEADER) != 0
    }

    pub fn set_long_header(&mut self, is_long: bool) {
        if is_long {
            self.0 |= Self::LONG_HEADER;
        } else {
            self.0 &= !Self::LONG_HEADER;
        }
    }

    pub fn key_phase(self) -> bool {
        (self.0 & Self::KEY_PHASE) != 0
    }

    pub fn set_key_phase(&mut self, phase: bool) {
        if phase {
            self.0 |= Self::KEY_PHASE;
        } else {
            self.0 &= !Self::KEY_PHASE;
        }
    }

    pub fn has_ack(self) -> bool {
        (self.0 & Self::ACK_PRESENT) != 0
    }

    pub fn set_ack_present(&mut self, ack_present: bool) {
        if ack_present {
            self.0 |= Self::ACK_PRESENT;
        } else {
            self.0 &= !Self::ACK_PRESENT;
        }
    }

    pub fn ecn_bits(self) -> u8 {
        (self.0 & Self::ECN_MASK) >> 3
    }

    pub fn set_ecn_bits(&mut self, ecn: u8) {
        self.0 = (self.0 & !Self::ECN_MASK) | ((ecn & 0x03) << 3);
    }
}

/// Common header representing either Long or Short GTP headers.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct PacketHeader {
    pub flags: HeaderFlags,
    pub version: Option<u32>,
    pub header_len: u8,
    pub connection_id: ConnectionId,
    pub packet_number: PacketNumber,
    pub timestamp_micros: u32,
    pub payload_len: u16,
}

impl PacketHeader {
    pub fn new_short(
        connection_id: ConnectionId,
        packet_number: PacketNumber,
        timestamp_micros: u32,
        payload_len: u16,
    ) -> Self {
        let mut flags = HeaderFlags::default();
        flags.set_long_header(false);
        Self {
            flags,
            version: None,
            header_len: MIN_COMMON_HEADER_LEN as u8,
            connection_id,
            packet_number,
            timestamp_micros,
            payload_len,
        }
    }

    pub fn new_long(
        version: u32,
        connection_id: ConnectionId,
        packet_number: PacketNumber,
        timestamp_micros: u32,
        payload_len: u16,
    ) -> Self {
        let mut flags = HeaderFlags::default();
        flags.set_long_header(true);
        Self {
            flags,
            version: Some(version),
            header_len: MIN_LONG_HEADER_LEN as u8,
            connection_id,
            packet_number,
            timestamp_micros,
            payload_len,
        }
    }

    pub fn encode(&self, buf: &mut [u8]) -> Result<usize> {
        let is_long = self.flags.is_long_header();
        let min_len = if is_long {
            MIN_LONG_HEADER_LEN
        } else {
            MIN_COMMON_HEADER_LEN
        };

        if buf.len() < min_len {
            return Err(TransportError::BufferTooShort);
        }

        let mut offset = 0;
        buf[offset] = self.flags.0;
        offset += 1;

        if is_long {
            let ver = self.version.unwrap_or(GTP_V1_1);
            buf[offset..offset + 4].copy_from_slice(&ver.to_be_bytes());
            offset += 4;
        }

        buf[offset] = self.header_len;
        offset += 1;

        buf[offset..offset + 8].copy_from_slice(&self.connection_id.to_be_bytes());
        offset += 8;

        buf[offset..offset + 8].copy_from_slice(&self.packet_number.as_u64().to_be_bytes());
        offset += 8;

        buf[offset..offset + 4].copy_from_slice(&self.timestamp_micros.to_be_bytes());
        offset += 4;

        buf[offset..offset + 2].copy_from_slice(&self.payload_len.to_be_bytes());
        offset += 2;

        Ok(offset)
    }

    pub fn decode(buf: &[u8]) -> Result<(Self, usize)> {
        if buf.is_empty() {
            return Err(TransportError::BufferTooShort);
        }

        let flags = HeaderFlags(buf[0]);
        let is_long = flags.is_long_header();
        let min_len = if is_long {
            MIN_LONG_HEADER_LEN
        } else {
            MIN_COMMON_HEADER_LEN
        };

        if buf.len() < min_len {
            return Err(TransportError::BufferTooShort);
        }

        let mut offset = 1;
        let version = if is_long {
            let mut ver_bytes = [0u8; 4];
            ver_bytes.copy_from_slice(&buf[offset..offset + 4]);
            offset += 4;
            Some(u32::from_be_bytes(ver_bytes))
        } else {
            None
        };

        let header_len = buf[offset];
        offset += 1;

        if (header_len as usize) < min_len || (header_len as usize) > buf.len() {
            return Err(TransportError::InvalidPacket("Invalid header length"));
        }

        let mut cid_bytes = [0u8; 8];
        cid_bytes.copy_from_slice(&buf[offset..offset + 8]);
        let cid = ConnectionId::from_be_bytes(cid_bytes);
        offset += 8;

        let mut pn_bytes = [0u8; 8];
        pn_bytes.copy_from_slice(&buf[offset..offset + 8]);
        let pkt_num = PacketNumber::from_u64(u64::from_be_bytes(pn_bytes));
        offset += 8;

        let mut ts_bytes = [0u8; 4];
        ts_bytes.copy_from_slice(&buf[offset..offset + 4]);
        let ts = u32::from_be_bytes(ts_bytes);
        offset += 4;

        let mut len_bytes = [0u8; 2];
        len_bytes.copy_from_slice(&buf[offset..offset + 2]);
        let payload_len = u16::from_be_bytes(len_bytes);
        let _ = offset; // suppress unused assignment

        // Skip any future extension bytes up to header_len
        let total_header_consumed = header_len as usize;

        Ok((
            Self {
                flags,
                version,
                header_len,
                connection_id: cid,
                packet_number: pkt_num,
                timestamp_micros: ts,
                payload_len,
            },
            total_header_consumed,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_header_roundtrip() {
        let hdr = PacketHeader::new_short(
            ConnectionId(0x1122334455667788),
            PacketNumber(42),
            1234567,
            250,
        );

        let mut buf = [0u8; 64];
        let written = hdr.encode(&mut buf).unwrap();
        assert_eq!(written, MIN_COMMON_HEADER_LEN);

        let (decoded, consumed) = PacketHeader::decode(&buf[..written]).unwrap();
        assert_eq!(consumed, MIN_COMMON_HEADER_LEN);
        assert_eq!(hdr, decoded);
        assert!(!decoded.flags.is_long_header());
    }

    #[test]
    fn test_long_header_roundtrip() {
        let hdr = PacketHeader::new_long(
            GTP_V1_1,
            ConnectionId(0xAABBCCDDEEFF0011),
            PacketNumber(1),
            987654,
            120,
        );

        let mut buf = [0u8; 64];
        let written = hdr.encode(&mut buf).unwrap();
        assert_eq!(written, MIN_LONG_HEADER_LEN);

        let (decoded, consumed) = PacketHeader::decode(&buf[..written]).unwrap();
        assert_eq!(consumed, MIN_LONG_HEADER_LEN);
        assert_eq!(hdr, decoded);
        assert!(decoded.flags.is_long_header());
        assert_eq!(decoded.version, Some(GTP_V1_1));
    }
}
