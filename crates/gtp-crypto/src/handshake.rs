use gtp_types::ConnectionId;
use hkdf::Hkdf;
use rand::rngs::OsRng;
use sha2::Sha256;
use std::fmt;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secure container for ephemeral Diffie-Hellman shared secret with guaranteed zeroization on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct HandshakeSharedSecret {
    secret: [u8; 32],
}

impl HandshakeSharedSecret {
    pub fn new(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.secret
    }
}

/// Ephemeral X25519 Diffie-Hellman Key Pair for initial connection handshake.
pub struct EphemeralKeyPair {
    secret: StaticSecret,
    pub public_key: [u8; 32],
    pub nonce: [u8; 32],
}

impl EphemeralKeyPair {
    /// Generate a fresh, cryptographically secure X25519 ephemeral key pair and random 32-byte nonce.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);

        let mut nonce = [0u8; 32];
        let mut rng = OsRng;
        rand::RngCore::fill_bytes(&mut rng, &mut nonce);

        Self {
            secret,
            public_key: *public.as_bytes(),
            nonce,
        }
    }

    /// Perform Diffie-Hellman scalar multiplication against the peer's public key.
    ///
    /// Returns `Err` for non-contributory (low-order / all-zero) peer public keys
    /// so that key material is never derived from a degenerate shared secret.
    pub fn compute_shared_secret(
        self,
        peer_public_key: &[u8; 32],
    ) -> Result<HandshakeSharedSecret, &'static str> {
        let peer_pk = PublicKey::from(*peer_public_key);
        let shared = self.secret.diffie_hellman(&peer_pk);
        if !shared.was_contributory() {
            return Err("non-contributory X25519 public key rejected");
        }
        Ok(HandshakeSharedSecret::new(*shared.as_bytes()))
    }
}

/// Per-direction AEAD traffic keys derived from the handshake.
///
/// SEC-1: the client encrypts with `client_tx` (and opens with `server_tx`) while the
/// server does the inverse. Sharing one key/IV across both directions would replay the
/// same ChaCha20 keystream and Poly1305 one-time key for colliding packet numbers.
#[derive(Clone, Copy)]
pub struct DirectionalKeys {
    pub client_tx_key: [u8; 32],
    pub client_tx_iv: [u8; 12],
    pub server_tx_key: [u8; 32],
    pub server_tx_iv: [u8; 12],
}

// SEC-14: raw key material must never render in logs or panic messages —
// a derived Debug would print every key byte verbatim.
impl fmt::Debug for DirectionalKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectionalKeys")
            .field("client_tx_key", &"[REDACTED]")
            .field("client_tx_iv", &"[REDACTED]")
            .field("server_tx_key", &"[REDACTED]")
            .field("server_tx_iv", &"[REDACTED]")
            .finish()
    }
}

/// (key, base IV) pair for one traffic direction.
pub type KeyMaterial = ([u8; 32], [u8; 12]);

impl DirectionalKeys {
    /// Returns the (seal, open) key material for an endpoint in the given role.
    pub fn for_role(&self, as_client: bool) -> (KeyMaterial, KeyMaterial) {
        if as_client {
            (
                (self.client_tx_key, self.client_tx_iv),
                (self.server_tx_key, self.server_tx_iv),
            )
        } else {
            (
                (self.server_tx_key, self.server_tx_iv),
                (self.client_tx_key, self.client_tx_iv),
            )
        }
    }
}

/// Derives cryptographically isolated per-direction AEAD keys and base IVs from the
/// X25519 shared secret and the session nonces.
pub fn derive_directional_handshake_session_keys(
    shared_secret: &HandshakeSharedSecret,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
    connection_id: ConnectionId,
) -> DirectionalKeys {
    // Combine shared secret with client & server nonces into HKDF input
    let mut ikm = [0u8; 96];
    ikm[..32].copy_from_slice(shared_secret.as_bytes());
    ikm[32..64].copy_from_slice(client_nonce);
    ikm[64..96].copy_from_slice(server_nonce);

    let salt = b"GTP_V1_1_X25519_SESSION_KEY_EXCHANGE";
    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);

    let cid_bytes = connection_id.0.to_be_bytes();

    // info = label(8) ‖ suffix(8) ‖ cid(8) — fixed 24-byte buffer, no concatenation ambiguity
    let expand = |label: &[u8; 8], suffix: &[u8; 8], out: &mut [u8]| {
        let mut info = [0u8; 24];
        info[..8].copy_from_slice(label);
        info[8..16].copy_from_slice(suffix);
        info[16..24].copy_from_slice(&cid_bytes);
        hk.expand(&info, out)
            .expect("fixed output lengths are valid for HKDF-SHA256");
    };

    let mut master_key = [0u8; 32];
    let mut client_tx_key = [0u8; 32];
    let mut client_tx_iv = [0u8; 12];
    let mut server_tx_key = [0u8; 32];
    let mut server_tx_iv = [0u8; 12];

    expand(b"master  ", b"key     ", &mut master_key);
    expand(b"c2s key ", b"gtp/v1  ", &mut client_tx_key);
    expand(b"c2s iv  ", b"gtp/v1  ", &mut client_tx_iv);
    expand(b"s2c key ", b"gtp/v1  ", &mut server_tx_key);
    expand(b"s2c iv  ", b"gtp/v1  ", &mut server_tx_iv);

    master_key.zeroize();
    ikm.zeroize();

    DirectionalKeys {
        client_tx_key,
        client_tx_iv,
        server_tx_key,
        server_tx_iv,
    }
}

/// Rotates session encryption key for long-lived connections (Key Ratchet / Key Phase transition).
pub fn ratchet_key(current_key: &[u8; 32], connection_id: ConnectionId) -> [u8; 32] {
    let salt = b"GTP_V1_1_KEY_RATCHET_SALT";
    let hk = Hkdf::<Sha256>::new(Some(salt), current_key);

    let mut next_key = [0u8; 32];
    let mut info = [0u8; 16];
    info[..8].copy_from_slice(&connection_id.0.to_be_bytes());
    info[8..16].copy_from_slice(b"KEYPHASE");

    hk.expand(&info, &mut next_key)
        .expect("32 bytes is valid length for HKDF-SHA256");

    next_key
}

type HmacSha256 = hmac::Hmac<Sha256>;

/// Derives the dedicated handshake-confirmation key (key-separation from the AEAD keys).
fn confirmation_key(derived_key: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"GTP_V1_1_CONFIRM_SALT"), derived_key);
    let mut finished = [0u8; 32];
    hk.expand(b"gtp/v1 handshake finished key", &mut finished)
        .expect("32 bytes is valid length for HKDF-SHA256");
    finished
}

/// Computes HMAC-SHA256 client key confirmation proof over `b"gtp-handshake-finish"` || `client_pk` || `server_pk`.
///
/// The proof is keyed by a dedicated confirmation key derived from the shared secret,
/// never by the AEAD traffic key itself (SEC-9 key-separation).
pub fn compute_client_proof(
    derived_key: &[u8; 32],
    client_pk: &[u8; 32],
    server_pk: &[u8; 32],
) -> [u8; 32] {
    use hmac::Mac;
    let finished_key = confirmation_key(derived_key);
    let mut mac = HmacSha256::new_from_slice(&finished_key).expect("HMAC can take key of any size");
    mac.update(b"gtp-handshake-finish");
    mac.update(client_pk);
    mac.update(server_pk);
    let result = mac.finalize();
    let mut proof = [0u8; 32];
    proof.copy_from_slice(&result.into_bytes());
    proof
}

/// Verifies client key confirmation proof in constant time.
pub fn verify_client_proof(
    derived_key: &[u8; 32],
    client_pk: &[u8; 32],
    server_pk: &[u8; 32],
    candidate_proof: &[u8; 32],
) -> bool {
    use subtle::ConstantTimeEq;
    let expected = compute_client_proof(derived_key, client_pk, server_pk);
    expected.ct_eq(candidate_proof).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aead::GtpAeadProtector;
    use crate::protector::Protector;
    use gtp_types::PacketNumber;

    /// SEC-14: handshake-derived directional keys must not render raw bytes.
    #[test]
    fn debug_does_not_leak_directional_keys() {
        let keys = DirectionalKeys {
            client_tx_key: [0xA1; 32],
            client_tx_iv: [0xB2; 12],
            server_tx_key: [0xC3; 32],
            server_tx_iv: [0xD4; 12],
        };
        let rendered = format!("{:?}", keys);
        assert!(rendered.contains("DirectionalKeys"));
        assert_eq!(rendered.matches("[REDACTED]").count(), 4);
        assert!(!rendered.contains("161")); // 0xA1
        assert!(!rendered.contains("178")); // 0xB2
        assert!(!rendered.contains(&format!("{:?}", keys.client_tx_key)));
    }

    #[test]
    fn test_x25519_diffie_hellman_handshake_roundtrip() {
        let client_pair = EphemeralKeyPair::generate();
        let server_pair = EphemeralKeyPair::generate();

        let client_pk = client_pair.public_key;
        let client_nonce = client_pair.nonce;

        let server_pk = server_pair.public_key;
        let server_nonce = server_pair.nonce;

        // Both parties compute the shared secret
        let client_shared = client_pair
            .compute_shared_secret(&server_pk)
            .expect("contributory DH");
        let server_shared = server_pair
            .compute_shared_secret(&client_pk)
            .expect("contributory DH");

        // Shared secrets MUST be identical
        assert_eq!(client_shared.as_bytes(), server_shared.as_bytes());

        // Derive directional session keys
        let cid = ConnectionId(0x1122_3344_5566_7788);
        let keys_client = derive_directional_handshake_session_keys(
            &client_shared,
            &client_nonce,
            &server_nonce,
            cid,
        );
        let keys_server = derive_directional_handshake_session_keys(
            &server_shared,
            &client_nonce,
            &server_nonce,
            cid,
        );

        assert_eq!(keys_client.client_tx_key, keys_server.client_tx_key);
        assert_eq!(keys_client.server_tx_key, keys_server.server_tx_key);

        // SEC-1: directions must NEVER share key material
        assert_ne!(keys_client.client_tx_key, keys_client.server_tx_key);
        assert_ne!(keys_client.client_tx_iv, keys_client.server_tx_iv);

        // SEC-1: directional mapping — the server's RX key pair must equal the
        // client's TX key pair (and vice versa), and the two directions must differ.
        let cid2 = ConnectionId(0xAABB_CCDD_EEFF_0011);
        let keys = derive_directional_handshake_session_keys(
            &client_shared,
            &client_nonce,
            &server_nonce,
            cid2,
        );
        let (client_seal, client_open) = keys.for_role(true);
        let (server_seal, server_open) = keys.for_role(false);
        assert_eq!(client_seal, server_open); // server opens client traffic
        assert_eq!(client_open, server_seal); // client opens server traffic
        assert_ne!(client_seal.0, client_open.0);
        assert_ne!(client_seal.1, client_open.1);

        // Seal with the client TX key and open as the server (its RX = client TX): succeeds
        let client_tx = Protector::Aead(GtpAeadProtector::new(client_seal.0, client_seal.1));
        let mut seal_buf = [0u8; 64];
        seal_buf[..4].copy_from_slice(b"data");
        let sealed = client_tx
            .seal(PacketNumber(1), cid2, b"aad", &mut seal_buf, 4)
            .unwrap();

        let server_rx = Protector::Aead(GtpAeadProtector::new(server_open.0, server_open.1));
        let mut open_buf = seal_buf;
        let opened = server_rx.open(PacketNumber(1), cid2, b"aad", &mut open_buf, sealed);
        assert_eq!(opened.unwrap(), 4);
        assert_eq!(&open_buf[..4], b"data");

        // Opening client traffic with the opposite direction key (server TX) must fail
        let wrong = Protector::Aead(GtpAeadProtector::new(server_seal.0, server_seal.1));
        let mut bad_buf = seal_buf;
        assert!(wrong
            .open(PacketNumber(1), cid2, b"aad", &mut bad_buf, sealed)
            .is_err());

        // Test key ratcheting produces new key
        let ratcheted = ratchet_key(&keys_client.client_tx_key, cid);
        assert_ne!(ratcheted, keys_client.client_tx_key);

        // Test client proof HMAC verification (confirmation key, not AEAD key)
        let proof = compute_client_proof(&keys_client.client_tx_key, &client_pk, &server_pk);
        assert!(verify_client_proof(
            &keys_server.client_tx_key,
            &client_pk,
            &server_pk,
            &proof
        ));

        let mut tampered_proof = proof;
        tampered_proof[0] ^= 0xFF;
        assert!(!verify_client_proof(
            &keys_server.client_tx_key,
            &client_pk,
            &server_pk,
            &tampered_proof
        ));

        // Mismatched keys must fail
        let wrong_key = [0x55u8; 32];
        assert!(!verify_client_proof(
            &wrong_key, &client_pk, &server_pk, &proof
        ));
    }

    /// SEC-12: non-contributory (low-order) peer public keys must be rejected.
    #[test]
    fn low_order_public_key_rejected() {
        let pair = EphemeralKeyPair::generate();
        // RFC 8439 §6.1 / X25519 low-order point: all-zero public key
        let zero_pk = [0u8; 32];
        assert!(pair.compute_shared_secret(&zero_pk).is_err());

        // Order-1 point: p = 2^255 - 19 in little-endian
        let mut order1_pk = [0u8; 32];
        order1_pk[0] = 0xED;
        for b in order1_pk.iter_mut().skip(1) {
            *b = 0xFF;
        }
        order1_pk[31] = 0x7F;
        let pair2 = EphemeralKeyPair::generate();
        assert!(pair2.compute_shared_secret(&order1_pk).is_err());
    }
}
