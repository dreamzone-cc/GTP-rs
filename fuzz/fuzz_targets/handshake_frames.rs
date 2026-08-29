#![no_main]

use gtp_crypto::{verify_client_proof, EphemeralKeyPair};
use gtp_wire::frame::Frame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1. Decode Frame
    if let Ok((frame, _)) = Frame::decode(data) {
        match frame {
            Frame::ClientHello {
                client_public_key,
                client_nonce: _,
                version: _,
            } => {
                // Compute DH against arbitrary fuzzed public key bytes
                let server_pair = EphemeralKeyPair::generate();
                let _ = server_pair.compute_shared_secret(&client_public_key);
            }
            Frame::ServerHello { .. } => {}
            Frame::HandshakeFinish {
                cookie_echo: _,
                client_proof,
            } => {
                let dummy_key = [0x55u8; 32];
                let dummy_pk = [0xAAu8; 32];
                let _ = verify_client_proof(&dummy_key, &dummy_pk, &dummy_pk, &client_proof);
            }
            _ => {}
        }
    }
});
