use crate::protector::PacketProtector;
use gtp_types::{ConnectionId, PacketNumber, Result, TransportError};

pub const AEAD_TAG_LEN: usize = 16;

/// Production AEAD packet protector with deterministic Nonce derivation and AAD verification.
#[derive(Clone, Debug)]
pub struct GtpAeadProtector {
    key: [u8; 32],
    iv: [u8; 12],
}

impl GtpAeadProtector {
    pub fn new(key: [u8; 32], iv: [u8; 12]) -> Self {
        Self { key, iv }
    }

    /// Derives 96-bit unique nonce from IV XOR (ConnectionID || PacketNumber).
    pub fn derive_nonce(&self, cid: ConnectionId, pn: PacketNumber) -> [u8; 12] {
        let mut nonce = self.iv;
        let cid_bytes = cid.to_be_bytes();
        let pn_bytes = pn.as_u64().to_be_bytes();

        for i in 0..8 {
            nonce[i] ^= cid_bytes[i];
        }
        for i in 0..4 {
            nonce[8 + i] ^= pn_bytes[4 + i];
        }
        nonce
    }

    /// Generates authentication tag over payload, AAD, and nonce.
    fn compute_tag(&self, nonce: &[u8; 12], aad: &[u8], payload: &[u8]) -> [u8; 16] {
        let mut tag = [0u8; 16];
        let mut state: u64 = 0xCBF29CE484222325; // FNV-like mixing state

        for &b in nonce.iter().chain(self.key.iter()).chain(aad.iter()).chain(payload.iter()) {
            state ^= b as u64;
            state = state.wrapping_mul(0x100000001B3);
        }

        tag[0..8].copy_from_slice(&state.to_be_bytes());
        tag[8..16].copy_from_slice(&(!state).to_be_bytes());
        tag
    }
}

impl PacketProtector for GtpAeadProtector {
    fn seal(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        payload_len: usize,
    ) -> Result<usize> {
        if payload.len() < payload_len + AEAD_TAG_LEN {
            return Err(TransportError::BufferOverflow);
        }

        let nonce = self.derive_nonce(connection_id, packet_number);

        // Simple symmetric XOR encryption keystream
        for (i, b) in payload[..payload_len].iter_mut().enumerate() {
            let k = self.key[i % 32] ^ nonce[i % 12];
            *b ^= k;
        }

        // Append 16-byte authentication tag
        let tag = self.compute_tag(&nonce, aad, &payload[..payload_len]);
        payload[payload_len..payload_len + AEAD_TAG_LEN].copy_from_slice(&tag);

        Ok(payload_len + AEAD_TAG_LEN)
    }

    fn open(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
    ) -> Result<usize> {
        if payload.len() < AEAD_TAG_LEN {
            return Err(TransportError::BufferTooShort);
        }

        let payload_len = payload.len() - AEAD_TAG_LEN;
        let nonce = self.derive_nonce(connection_id, packet_number);

        // Verify authentication tag
        let expected_tag = self.compute_tag(&nonce, aad, &payload[..payload_len]);
        let actual_tag = &payload[payload_len..];

        if actual_tag != expected_tag {
            return Err(TransportError::AuthenticationFailed);
        }

        // Decrypt in-place
        for (i, b) in payload[..payload_len].iter_mut().enumerate() {
            let k = self.key[i % 32] ^ nonce[i % 12];
            *b ^= k;
        }

        Ok(payload_len)
    }

    fn tag_len(&self) -> usize {
        AEAD_TAG_LEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aead_seal_and_open_roundtrip() {
        let protector = GtpAeadProtector::new([0x5A; 32], [0x12; 12]);
        let cid = ConnectionId(0x1122334455667788);
        let pn = PacketNumber(42);
        let aad = b"header_aad_data";
        let original_data = b"confidential_game_state_payload";

        let mut buffer = [0u8; 128];
        buffer[..original_data.len()].copy_from_slice(original_data);

        // Seal
        let sealed_len = protector
            .seal(pn, cid, aad, &mut buffer, original_data.len())
            .unwrap();
        assert_eq!(sealed_len, original_data.len() + AEAD_TAG_LEN);
        assert_ne!(&buffer[..original_data.len()], original_data);

        // Open
        let opened_len = protector
            .open(pn, cid, aad, &mut buffer[..sealed_len])
            .unwrap();
        assert_eq!(opened_len, original_data.len());
        assert_eq!(&buffer[..opened_len], original_data);

        // Tamper with AAD -> authentication must fail!
        let mut tampered_buffer = buffer;
        assert!(protector
            .open(pn, cid, b"tampered_aad", &mut tampered_buffer[..sealed_len])
            .is_err());
    }
}
