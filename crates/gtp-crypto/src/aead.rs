use crate::protector::PacketProtector;
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use core::fmt;
use gtp_types::{ConnectionId, PacketNumber, Result, TransportError};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const AEAD_TAG_LEN: usize = 16;

/// Production ChaCha20-Poly1305 AEAD packet protector with deterministic 96-bit Nonce derivation.
///
/// The nonce is `IV ⊕ (CID_be[0..4] ‖ PN_be[0..8])`: the full 64-bit packet number is
/// mixed into nonce bytes 4..12, so packet numbers 1 and 2^32+1 can never collide.
/// The key itself is already CID-scoped via HKDF info, and direction-scoped by the
/// handshake key schedule (see `derive_directional_handshake_session_keys`).
/// R-6: the key material is wiped when the protector is dropped, so a retired
/// pre-ratchet key does not linger in freed memory and undercut forward secrecy.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct GtpAeadProtector {
    key: [u8; 32],
    iv: [u8; 12],
}

impl GtpAeadProtector {
    pub fn new(key: [u8; 32], iv: [u8; 12]) -> Self {
        Self { key, iv }
    }

    /// Derives 96-bit unique nonce from base IV XOR (CID ‖ full 64-bit PacketNumber).
    pub fn derive_nonce(&self, cid: ConnectionId, pn: PacketNumber) -> [u8; 12] {
        let mut nonce = self.iv;
        let cid_bytes = cid.to_be_bytes();
        let pn_bytes = pn.as_u64().to_be_bytes();

        for i in 0..4 {
            nonce[i] ^= cid_bytes[i];
        }
        for i in 0..8 {
            nonce[4 + i] ^= pn_bytes[i];
        }
        nonce
    }
}

impl fmt::Debug for GtpAeadProtector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redacted: never expose live key material through Debug formatting.
        f.debug_struct("GtpAeadProtector")
            .field("key", &"[REDACTED; 32 bytes]")
            .field("iv", &"[REDACTED; 12 bytes]")
            .finish()
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
        let needed = payload_len
            .checked_add(AEAD_TAG_LEN)
            .ok_or(TransportError::BufferOverflow)?;
        if payload.len() < needed {
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

    /// SEC-2: packet numbers 1 and 2^32+1 must produce distinct nonces.
    #[test]
    fn nonce_uses_full_packet_number() {
        let protector = GtpAeadProtector::new([7u8; 32], [9u8; 12]);
        let cid = ConnectionId(0xDEAD_BEEF_1234_5678);

        let n1 = protector.derive_nonce(cid, PacketNumber(1));
        let n_hi = protector.derive_nonce(cid, PacketNumber(1 + (1u64 << 32)));
        assert_ne!(n1, n_hi);

        // Injectivity across the full 64-bit space for a couple of wrap edges
        assert_ne!(
            protector.derive_nonce(cid, PacketNumber(0)),
            protector.derive_nonce(cid, PacketNumber(u64::MAX))
        );
    }

    /// SEC-10: oversized payload_len must return an error, never panic.
    #[test]
    fn seal_rejects_overflowing_payload_len() {
        let protector = GtpAeadProtector::new([1u8; 32], [2u8; 12]);
        let mut buf = [0u8; 64];
        let res = protector.seal(
            PacketNumber(1),
            ConnectionId(1),
            b"aad",
            &mut buf,
            usize::MAX,
        );
        assert!(res.is_err());
    }

    /// R-8: the key-phase fallback in the receive path tries a second protector on
    /// the SAME buffer after the first attempt failed. That is only sound because a
    /// failed `open` verifies the Poly1305 tag before touching the ciphertext and
    /// therefore leaves the buffer byte-identical. This test pins that dependency on
    /// `chacha20poly1305` down so a crate upgrade that changed it would fail here
    /// instead of silently corrupting every packet that takes the fallback path.
    #[test]
    fn failed_open_leaves_buffer_intact() {
        let protector_a = GtpAeadProtector::new([0x11u8; 32], [0x22u8; 12]);
        let protector_b = GtpAeadProtector::new([0x33u8; 32], [0x22u8; 12]);

        let cid = ConnectionId(0x0BAD_0BAD_0BAD_0BAD);
        let pn = PacketNumber(7);
        let aad = b"short_header_aad";
        let plaintext = b"state_update_seq=91_hp=68";

        let mut buffer = [0u8; 128];
        buffer[..plaintext.len()].copy_from_slice(plaintext);
        let sealed_len = protector_a
            .seal(pn, cid, aad, &mut buffer, plaintext.len())
            .unwrap();

        let sealed_snapshot = buffer;

        // Wrong key: must fail WITHOUT mutating a single byte of the buffer.
        assert!(protector_b
            .open(pn, cid, aad, &mut buffer, sealed_len)
            .is_err());
        assert_eq!(
            buffer, sealed_snapshot,
            "a failed open must not apply the keystream to the buffer"
        );

        // Same buffer, correct key: the fallback attempt still recovers the plaintext.
        let opened_len = protector_a
            .open(pn, cid, aad, &mut buffer, sealed_len)
            .unwrap();
        assert_eq!(&buffer[..opened_len], plaintext);
    }

    /// SEC-14: Debug formatting must not leak key material.
    #[test]
    fn debug_does_not_leak_keys() {
        let protector = GtpAeadProtector::new([0xABu8; 32], [0xCDu8; 12]);
        let rendered = format!("{:?}", protector);
        assert!(!rendered.contains("171")); // 0xAB decimal
        assert!(rendered.contains("REDACTED"));
    }
}
