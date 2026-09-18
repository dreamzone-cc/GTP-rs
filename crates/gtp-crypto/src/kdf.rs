//! # Session Key Derivation (HKDF-SHA256)
//!
//! Cryptographically derives unique, session-scoped 256-bit AEAD keys and 96-bit base IVs
//! from a master shared secret and the 64-bit `ConnectionId`.

use gtp_types::ConnectionId;
use hkdf::Hkdf;
use sha2::Sha256;
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const KEY_LEN: usize = 32;
pub const IV_LEN: usize = 12;

/// Handshake master secret container.
///
/// `Debug` is manually implemented and redacted so key material can never leak
/// through formatting paths (logs, panic messages, error reports).
pub struct HandshakeSecret {
    secret: Vec<u8>,
}

impl core::fmt::Debug for HandshakeSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HandshakeSecret")
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for HandshakeSecret {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
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
///
/// # Deprecated (SEC-1 hazard)
/// This returns ONE key + ONE base IV with no direction label. Any caller that
/// uses the returned pair for both sending and receiving makes client packet N
/// and server packet N collide on the same (key, nonce) — catastrophic for
/// ChaCha20-Poly1305 (keystream reuse + tag forgery). Use
/// [`derive_directional_session_keys`] instead. The only legitimate legacy use
/// is deriving the base IV alone for a caller that already holds
/// direction-distinct keys.
#[deprecated(
    note = "bidirectional use reuses one (key, nonce) pair — SEC-1; use derive_directional_session_keys or only the IV half"
)]
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

/// Directional key material for a session (see `derive_directional_session_keys`).
///
/// Raw key material: no `Clone`/`Copy` (duplication would escape zeroization,
/// SEC-14) and wiped on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SessionDirectionalKeys {
    pub client_tx_key: [u8; KEY_LEN],
    pub client_tx_iv: [u8; IV_LEN],
    pub server_tx_key: [u8; KEY_LEN],
    pub server_tx_iv: [u8; IV_LEN],
}

// SEC-14: raw key material must never render in logs or panic messages.
impl fmt::Debug for SessionDirectionalKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionDirectionalKeys")
            .field("client_tx_key", &"[REDACTED]")
            .field("client_tx_iv", &"[REDACTED]")
            .field("server_tx_key", &"[REDACTED]")
            .field("server_tx_iv", &"[REDACTED]")
            .finish()
    }
}

/// Derives per-direction AEAD keys and IVs from a master secret and connection ID.
///
/// Even the legacy static-secret path must never share one key/IV across both
/// directions (SEC-1): client packet N and server packet N would collide on the
/// same (key, nonce) pair.
///
/// # Forward secrecy assumption
/// `master_secret` MUST be the ephemeral handshake output (X25519 shared
/// secret mixed with both parties' random nonces), never a static/PSK secret:
/// with a static secret, per-connection separation comes only from the public
/// 64-bit CID, so compromise of the secret passively yields every session's
/// directional keys (no FS, no post-compromise security). The unsalted HKDF
/// here also provides no extraction strengthening for non-uniform IKM.
pub fn derive_directional_session_keys(
    master_secret: &[u8],
    cid: ConnectionId,
) -> SessionDirectionalKeys {
    let hk = Hkdf::<Sha256>::new(None, master_secret);
    let cid_bytes = cid.to_be_bytes();

    // info = label(8) ‖ suffix(8) ‖ cid(8) — fixed 24-byte buffer
    let expand = |label: &[u8; 8], suffix: &[u8; 8], out: &mut [u8]| {
        let mut info = [0u8; 24];
        info[..8].copy_from_slice(label);
        info[8..16].copy_from_slice(suffix);
        info[16..24].copy_from_slice(&cid_bytes);
        hk.expand(&info, out)
            .expect("fixed output lengths are valid for HKDF-SHA256");
    };

    let mut out = SessionDirectionalKeys {
        client_tx_key: [0u8; KEY_LEN],
        client_tx_iv: [0u8; IV_LEN],
        server_tx_key: [0u8; KEY_LEN],
        server_tx_iv: [0u8; IV_LEN],
    };
    expand(b"c2s key ", b"gtp/v1  ", &mut out.client_tx_key);
    expand(b"c2s iv  ", b"gtp/v1  ", &mut out.client_tx_iv);
    expand(b"s2c key ", b"gtp/v1  ", &mut out.server_tx_key);
    expand(b"s2c iv  ", b"gtp/v1  ", &mut out.server_tx_iv);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
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

    /// SEC-14: directional session keys must not render their raw bytes.
    #[test]
    fn debug_does_not_leak_session_directional_keys() {
        let keys = derive_directional_session_keys(b"audit secret", ConnectionId(9));
        let rendered = format!("{:?}", keys);
        assert!(rendered.contains("SessionDirectionalKeys"));
        for field in [
            "client_tx_key",
            "client_tx_iv",
            "server_tx_key",
            "server_tx_iv",
        ] {
            assert!(rendered.contains(field), "field {} should be named", field);
            assert!(
                rendered.contains("[REDACTED]"),
                "field {} must be redacted",
                field
            );
        }
        assert!(!rendered.contains(&format!("{:?}", keys.client_tx_key)));
        assert!(!rendered.contains(&format!("{:?}", keys.server_tx_iv)));
    }
}
