use crate::protector::PacketProtector;
use gtp_types::{ConnectionId, PacketNumber, Result};

/// Zero-overhead plaintext protector for local benchmarking, fuzzing, and unit simulation.
#[derive(Clone, Debug, Default)]
pub struct PlaintextProtector;

impl PacketProtector for PlaintextProtector {
    fn seal(
        &self,
        _packet_number: PacketNumber,
        _connection_id: ConnectionId,
        _aad: &[u8],
        _payload: &mut [u8],
        payload_len: usize,
    ) -> Result<usize> {
        Ok(payload_len)
    }

    fn open(
        &self,
        _packet_number: PacketNumber,
        _connection_id: ConnectionId,
        _aad: &[u8],
        _payload: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize> {
        Ok(ciphertext_len)
    }

    fn tag_len(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plaintext_protector_passthrough() {
        let protector = PlaintextProtector;
        let mut buf = [1, 2, 3, 4, 5];
        let sealed_len = protector
            .seal(PacketNumber(1), ConnectionId(10), b"aad", &mut buf, 5)
            .unwrap();
        assert_eq!(sealed_len, 5);

        let opened_len = protector
            .open(PacketNumber(1), ConnectionId(10), b"aad", &mut buf, 5)
            .unwrap();
        assert_eq!(opened_len, 5);
        assert_eq!(&buf[..5], &[1, 2, 3, 4, 5]);
    }
}
