//! # Session Key Derivation (HKDF-SHA256)
//!
//! Cryptographically derives unique, session-scoped 256-bit AEAD keys and 96-bit base IVs
//! from a master shared secret and the 64-bit `ConnectionId`.

use gtp_types::ConnectionId;
use hkdf::Hkdf;
use sha2::Sha256;

pub const KEY_LEN: usize = 32;
pub const IV_LEN: usize = 12;

/// Handshake master secret container.
#[derive(Clone, Debug)]
pub struct HandshakeSecret {
    secret: Vec<u8>,
}

impl HandshakeSecret {
    pub fn new(secret: Vec<u8>) -> Self {
        Self { secret }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.secret
    }
}

/// Derives a 32-byte AEAD encryption key and a 12-byte base IV using HKDF-SHA256.
///
/// Uses the 64-bit `ConnectionId` in the info parameter to ensure per-connection isolation.
pub fn derive_session_keys(
    shared_secret: &[u8],
    cid: ConnectionId,
) -> ([u8; KEY_LEN], [u8; IV_LEN]) {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);

    let cid_bytes = cid.to_be_bytes();
    let mut info_key = [0u8; 16];
    info_key[0..8].copy_from_slice(b"gtp_key_");
    info_key[8..16].copy_from_slice(&cid_bytes);

    let mut key = [0u8; KEY_LEN];
    hk.expand(&info_key, &mut key)
        .expect("32 bytes is valid length for HKDF-SHA256");

    let mut info_iv = [0u8; 16];
    info_iv[0..8].copy_from_slice(b"gtp__iv_");
    info_iv[8..16].copy_from_slice(&cid_bytes);

    let mut iv = [0u8; IV_LEN];
    hk.expand(&info_iv, &mut iv)
        .expect("12 bytes is valid length for HKDF-SHA256");

    (key, iv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hkdf_session_key_derivation_isolation() {
        let secret = b"super_secure_game_session_master_secret_2026";
        let cid1 = ConnectionId(0x1122_3344_5566_7788);
        let cid2 = ConnectionId(0x99AA_BBCC_DDEE_FF00);

        let (key1, iv1) = derive_session_keys(secret, cid1);
        let (key2, iv2) = derive_session_keys(secret, cid2);

        // Different connections MUST yield distinct keys and IVs
        assert_ne!(key1, key2);
        assert_ne!(iv1, iv2);

        // Derivation must be deterministic for identical inputs
        let (key1_repeat, iv1_repeat) = derive_session_keys(secret, cid1);
        assert_eq!(key1, key1_repeat);
        assert_eq!(iv1, iv1_repeat);
    }
}
