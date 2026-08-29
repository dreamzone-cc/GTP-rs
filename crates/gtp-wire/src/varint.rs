use gtp_types::{Result, TransportError};

/// Variable-length integer encoding (RFC 9000 2-bit prefix format).
/// Supports values from 0 up to 2^62 - 1.
pub struct VarInt;

impl VarInt {
    pub const MAX: u64 = (1 << 62) - 1;

    pub fn encode(val: u64, buf: &mut [u8]) -> Result<usize> {
        if val <= 63 {
            if buf.is_empty() {
                return Err(TransportError::BufferTooShort);
            }
            buf[0] = val as u8;
            Ok(1)
        } else if val <= 16383 {
            if buf.len() < 2 {
                return Err(TransportError::BufferTooShort);
            }
            let v = (val as u16) | 0x4000;
            buf[0..2].copy_from_slice(&v.to_be_bytes());
            Ok(2)
        } else if val <= 1073741823 {
            if buf.len() < 4 {
                return Err(TransportError::BufferTooShort);
            }
            let v = (val as u32) | 0x80000000;
            buf[0..4].copy_from_slice(&v.to_be_bytes());
            Ok(4)
        } else if val <= Self::MAX {
            if buf.len() < 8 {
                return Err(TransportError::BufferTooShort);
            }
            let v = val | 0xC000000000000000;
            buf[0..8].copy_from_slice(&v.to_be_bytes());
            Ok(8)
        } else {
            Err(TransportError::InvalidPacket("VarInt out of bounds"))
        }
    }

    pub fn decode(buf: &[u8]) -> Result<(u64, usize)> {
        if buf.is_empty() {
            return Err(TransportError::BufferTooShort);
        }

        let prefix = buf[0] >> 6;
        match prefix {
            0 => Ok((buf[0] as u64, 1)),
            1 => {
                if buf.len() < 2 {
                    return Err(TransportError::BufferTooShort);
                }
                let v = u16::from_be_bytes(buf[0..2].try_into().unwrap()) & 0x3FFF;
                Ok((v as u64, 2))
            }
            2 => {
                if buf.len() < 4 {
                    return Err(TransportError::BufferTooShort);
                }
                let v = u32::from_be_bytes(buf[0..4].try_into().unwrap()) & 0x3FFFFFFF;
                Ok((v as u64, 4))
            }
            3 => {
                if buf.len() < 8 {
                    return Err(TransportError::BufferTooShort);
                }
                let v = u64::from_be_bytes(buf[0..8].try_into().unwrap()) & 0x3FFFFFFFFFFFFFFF;
                Ok((v, 8))
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_roundtrips() {
        let test_vals = [
            0,
            25,
            63,
            64,
            15000,
            16383,
            16384,
            1_000_000,
            1073741823,
            1073741824,
            100_000_000_000,
        ];
        let mut buf = [0u8; 16];

        for &val in &test_vals {
            let written = VarInt::encode(val, &mut buf).unwrap();
            let (decoded, consumed) = VarInt::decode(&buf[..written]).unwrap();
            assert_eq!(written, consumed);
            assert_eq!(val, decoded);
        }
    }
}
