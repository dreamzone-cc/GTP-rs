pub mod aead;
pub mod handshake;
pub mod kdf;
pub mod plaintext;
pub mod protector;
pub mod replay;

pub use aead::{GtpAeadProtector, AEAD_TAG_LEN};
pub use handshake::{
    compute_client_proof, derive_handshake_session_keys, ratchet_key, verify_client_proof,
    EphemeralKeyPair, HandshakeSharedSecret,
};
pub use kdf::{derive_session_keys, HandshakeSecret, IV_LEN, KEY_LEN};
pub use plaintext::PlaintextProtector;
pub use protector::{PacketProtector, Protector};
pub use replay::ReplayWindow;
