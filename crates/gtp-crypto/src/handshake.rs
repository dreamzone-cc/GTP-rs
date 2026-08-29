use crate::kdf::derive_session_keys;
use gtp_types::ConnectionId;
use hkdf::Hkdf;
use rand::rngs::OsRng;
use sha2::Sha256;
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
    pub fn compute_shared_secret(self, peer_public_key: &[u8; 32]) -> HandshakeSharedSecret {
        let peer_pk = PublicKey::from(*peer_public_key);
        let shared = self.secret.diffie_hellman(&peer_pk);
        HandshakeSharedSecret::new(*shared.as_bytes())
    }
}

/// Derives cryptographically isolated 256-bit AEAD key and 96-bit base IV from X25519 shared secret and session nonces.
pub fn derive_handshake_session_keys(
    shared_secret: &HandshakeSharedSecret,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
    connection_id: ConnectionId,
) -> ([u8; 32], [u8; 12]) {
    // Combine shared secret with client & server nonces into HKDF input
    let mut ikm = [0u8; 96];
    ikm[..32].copy_from_slice(shared_secret.as_bytes());
    ikm[32..64].copy_from_slice(client_nonce);
    ikm[64..96].copy_from_slice(server_nonce);

    let salt = b"GTP_V1_1_X25519_SESSION_KEY_EXCHANGE";
    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);

    let mut master_key = [0u8; 32];
    let cid_info = connection_id.0.to_be_bytes();
    hk.expand(&cid_info, &mut master_key)
        .expect("32 bytes is valid length for HKDF-SHA256");

    // Derive final AEAD key and Base IV
    let (key, iv) = derive_session_keys(&master_key, connection_id);
    master_key.zeroize();
    ikm.zeroize();

    (key, iv)
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

/// Computes HMAC-SHA256 client key confirmation proof over `b"gtp-handshake-finish"` || `client_pk` || `server_pk`.
pub fn compute_client_proof(
    derived_key: &[u8; 32],
    client_pk: &[u8; 32],
    server_pk: &[u8; 32],
) -> [u8; 32] {
    use hmac::Mac;
    let mut mac = HmacSha256::new_from_slice(derived_key).expect("HMAC can take key of any size");
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

    #[test]
    fn test_x25519_diffie_hellman_handshake_roundtrip() {
        let client_pair = EphemeralKeyPair::generate();
        let server_pair = EphemeralKeyPair::generate();

        let client_pk = client_pair.public_key;
        let client_nonce = client_pair.nonce;

        let server_pk = server_pair.public_key;
        let server_nonce = server_pair.nonce;

        // Both parties compute the shared secret
        let client_shared = client_pair.compute_shared_secret(&server_pk);
        let server_shared = server_pair.compute_shared_secret(&client_pk);

        // Shared secrets MUST be identical
        assert_eq!(client_shared.as_bytes(), server_shared.as_bytes());

        // Derive session keys
        let cid = ConnectionId(0x1122_3344_5566_7788);
        let (client_key, client_iv) =
            derive_handshake_session_keys(&client_shared, &client_nonce, &server_nonce, cid);
        let (server_key, server_iv) =
            derive_handshake_session_keys(&server_shared, &client_nonce, &server_nonce, cid);

        assert_eq!(client_key, server_key);
        assert_eq!(client_iv, server_iv);

        // Test key ratcheting produces new key
        let ratcheted = ratchet_key(&client_key, cid);
        assert_ne!(ratcheted, client_key);

        // Test client proof HMAC verification
        let proof = compute_client_proof(&client_key, &client_pk, &server_pk);
        assert!(verify_client_proof(
            &server_key,
            &client_pk,
            &server_pk,
            &proof
        ));

        let mut tampered_proof = proof;
        tampered_proof[0] ^= 0xFF;
        assert!(!verify_client_proof(
            &server_key,
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
}
