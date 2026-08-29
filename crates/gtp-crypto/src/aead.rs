use crate::protector::PacketProtector;
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use gtp_types::{ConnectionId, PacketNumber, Result, TransportError};

pub const AEAD_TAG_LEN: usize = 16;

/// Production ChaCha20-Poly1305 AEAD packet protector with deterministic 96-bit Nonce derivation.
#[derive(Clone, Debug)]
pub struct GtpAeadProtector {
    key: [u8; 32],
    iv: [u8; 12],
}

impl GtpAeadProtector {
    pub fn new(key: [u8; 32], iv: [u8; 12]) -> Self {
        Self { key, iv }
    }

    /// Derives 96-bit unique nonce from base IV XOR (ConnectionID || PacketNumber).
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

        let nonce_bytes = self.derive_nonce(connection_id, packet_number);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let nonce = Nonce::from_slice(&nonce_bytes);

        let tag = cipher
            .encrypt_in_place_detached(nonce, aad, &mut payload[..payload_len])
            .map_err(|_| TransportError::CryptoFailure)?;

        payload[payload_len..payload_len + AEAD_TAG_LEN].copy_from_slice(tag.as_slice());
        Ok(payload_len + AEAD_TAG_LEN)
    }

    fn open(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize> {
        if ciphertext_len < AEAD_TAG_LEN || payload.len() < ciphertext_len {
            return Err(TransportError::BufferTooShort);
        }

        let plaintext_len = ciphertext_len - AEAD_TAG_LEN;
        let nonce_bytes = self.derive_nonce(connection_id, packet_number);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let nonce = Nonce::from_slice(&nonce_bytes);

        let (data_slice, tag_slice) = payload[..ciphertext_len].split_at_mut(plaintext_len);
        let tag = Tag::from_slice(tag_slice);

        cipher
            .decrypt_in_place_detached(nonce, aad, data_slice, tag)
            .map_err(|_| TransportError::CryptoFailure)?;

        Ok(plaintext_len)
    }

    fn tag_len(&self) -> usize {
        AEAD_TAG_LEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chacha20_poly1305_seal_and_open_roundtrip() {
        let key = [0x42u8; 32];
        let iv = [0x13u8; 12];
        let protector = GtpAeadProtector::new(key, iv);

        let cid = ConnectionId(0x1020304050607080);
        let pn = PacketNumber(100);
        let aad = b"gtp_short_header_24_bytes_metadata";
        let original_data = b"entity_move_x=123.45_y=678.90_vx=1.2";

        let mut buffer = [0u8; 256];
        buffer[..original_data.len()].copy_from_slice(original_data);

        // 1. Seal
        let sealed_len = protector
            .seal(pn, cid, aad, &mut buffer, original_data.len())
            .unwrap();
        assert_eq!(sealed_len, original_data.len() + AEAD_TAG_LEN);
        assert_ne!(&buffer[..original_data.len()], original_data);

        // 2. Open
        let opened_len = protector
            .open(pn, cid, aad, &mut buffer, sealed_len)
            .unwrap();
        assert_eq!(opened_len, original_data.len());
        assert_eq!(&buffer[..opened_len], original_data);
    }

    #[test]
    fn test_chacha20_poly1305_tamper_detection() {
        let key = [0x99u8; 32];
        let iv = [0x88u8; 12];
        let protector = GtpAeadProtector::new(key, iv);

        let cid = ConnectionId(0x1122334455667788);
        let pn = PacketNumber(42);
        let aad = b"authenticated_header_bytes";
        let original_data = b"critical_gameplay_rpc_payload";

        let mut buffer = [0u8; 128];
        buffer[..original_data.len()].copy_from_slice(original_data);

        let sealed_len = protector
            .seal(pn, cid, aad, &mut buffer, original_data.len())
            .unwrap();

        // Tamper with ciphertext byte
        buffer[2] ^= 0x01;
        let res = protector.open(pn, cid, aad, &mut buffer, sealed_len);
        assert!(res.is_err());

        // Restore and tamper with tag byte
        buffer[2] ^= 0x01;
        buffer[sealed_len - 1] ^= 0x80;
        let res_tag_tamper = protector.open(pn, cid, aad, &mut buffer, sealed_len);
        assert!(res_tag_tamper.is_err());

        // Restore and tamper with AAD
        buffer[sealed_len - 1] ^= 0x80;
        let tampered_aad = b"authenticated_header_tampered";
        let res_aad_tamper = protector.open(pn, cid, tampered_aad, &mut buffer, sealed_len);
        assert!(res_aad_tamper.is_err());
    }
}
