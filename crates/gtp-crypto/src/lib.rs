pub mod aead;
pub mod handshake;
pub mod kdf;
#[cfg(any(test, feature = "insecure-plaintext"))]
pub mod plaintext;
pub mod protector;
pub mod replay;

pub use aead::{GtpAeadProtector, AEAD_TAG_LEN};
pub use handshake::{
    compute_client_proof, compute_server_proof, derive_directional_handshake_session_keys,
    ratchet_key, verify_client_proof, verify_server_proof, DirectionalKeys, EphemeralKeyPair,
    HandshakeSharedSecret, HandshakeTranscript, StaticIdentity,
};
/// Legacy single-pair derivation — see the deprecation note for the SEC-1 hazard.
#[allow(deprecated)]
pub use kdf::derive_session_keys;
pub use kdf::{
    derive_directional_session_keys, HandshakeSecret, SessionDirectionalKeys, IV_LEN, KEY_LEN,
};
#[cfg(any(test, feature = "insecure-plaintext"))]
pub use plaintext::PlaintextProtector;
pub use protector::{PacketProtector, Protector};
pub use replay::ReplayWindow;
