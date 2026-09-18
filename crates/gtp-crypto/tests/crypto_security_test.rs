use gtp_crypto::{
    derive_directional_handshake_session_keys, derive_session_keys, ratchet_key, EphemeralKeyPair,
    GtpAeadProtector, PacketProtector, PlaintextProtector, Protector,
};
use gtp_types::{ConnectionId, PacketNumber};

#[test]
#[allow(deprecated)] // legacy single-pair derivation: isolation property under test
fn test_hkdf_multi_connection_entropy_isolation() {
    let master_secret = b"championship_final_tournament_master_secret";
    let mut derived_keys = std::collections::HashSet::new();

    for i in 1..=500 {
        let cid = ConnectionId(i);
        let (key, iv) = derive_session_keys(master_secret, cid);
        assert!(
            derived_keys.insert(key),
            "Collision detected for connection ID: {}",
            i
        );
        assert_eq!(iv.len(), 12);
        assert_eq!(key.len(), 32);
    }
}

/// SEC-1: directional derivation must be distinct across connections too.
#[test]
fn test_hkdf_directional_keys_isolation() {
    let secret = b"directional_isolation_master_secret";
    let cid_a = ConnectionId(0x1111);
    let cid_b = ConnectionId(0x2222);

    let a = gtp_crypto::derive_directional_session_keys(secret, cid_a);
    let b = gtp_crypto::derive_directional_session_keys(secret, cid_b);

    assert_ne!(a.client_tx_key, b.client_tx_key);
    assert_ne!(a.client_tx_iv, b.client_tx_iv);
    assert_ne!(a.server_tx_key, a.client_tx_key);
}

#[test]
#[allow(deprecated)] // legacy single-pair derivation feeds the protector under test
fn test_protector_static_dispatch_behavior() {
    let cid = ConnectionId(0xDEADBEEF);
    let pn = PacketNumber(1);
    let aad = b"header_data";
    let original = b"gameplay_input";

    let (key, iv) = derive_session_keys(b"secret", cid);
    let aead_protector = Protector::Aead(GtpAeadProtector::new(key, iv));
    let plaintext_protector = Protector::Plaintext(PlaintextProtector);

    // Test AEAD variant
    let mut aead_buf = [0u8; 128];
    aead_buf[..original.len()].copy_from_slice(original);
    let sealed_len = aead_protector
        .seal(pn, cid, aad, &mut aead_buf, original.len())
        .unwrap();
    assert_eq!(sealed_len, original.len() + 16);
    let opened_len = aead_protector
        .open(pn, cid, aad, &mut aead_buf, sealed_len)
        .unwrap();
    assert_eq!(opened_len, original.len());
    assert_eq!(&aead_buf[..opened_len], original);

    // Test Plaintext variant
    let mut pt_buf = [0u8; 128];
    pt_buf[..original.len()].copy_from_slice(original);
    let pt_sealed = plaintext_protector
        .seal(pn, cid, aad, &mut pt_buf, original.len())
        .unwrap();
    assert_eq!(pt_sealed, original.len());
    let pt_opened = plaintext_protector
        .open(pn, cid, aad, &mut pt_buf, pt_sealed)
        .unwrap();
    assert_eq!(pt_opened, original.len());
    assert_eq!(&pt_buf[..pt_opened], original);
}

#[test]
fn test_x25519_passive_eavesdropper_cannot_decrypt() {
    let client_ephemeral = EphemeralKeyPair::generate();
    let server_ephemeral = EphemeralKeyPair::generate();

    let client_pk = client_ephemeral.public_key;
    let client_nonce = client_ephemeral.nonce;

    let server_pk = server_ephemeral.public_key;
    let server_nonce = server_ephemeral.nonce;

    // Passive eavesdropper creates their own keypair
    let attacker_ephemeral = EphemeralKeyPair::generate();

    // Legitimate parties establish shared secret and directional session keys
    let client_shared = client_ephemeral
        .compute_shared_secret(&server_pk)
        .expect("contributory DH");
    let server_shared = server_ephemeral
        .compute_shared_secret(&client_pk)
        .expect("contributory DH");

    let cid = ConnectionId(0x1234_5678_9ABC_DEF0);
    let client_keys = derive_directional_handshake_session_keys(
        &client_shared,
        &client_nonce,
        &server_nonce,
        cid,
    );
    let server_keys = derive_directional_handshake_session_keys(
        &server_shared,
        &client_nonce,
        &server_nonce,
        cid,
    );

    // Both parties derive identical directional material, but the directions differ.
    assert_eq!(client_keys.client_tx_key, server_keys.client_tx_key);
    assert_ne!(client_keys.client_tx_key, client_keys.server_tx_key);

    // Client seals with client_tx; the server opens with client_tx (its RX direction).
    let client_protector =
        GtpAeadProtector::new(client_keys.client_tx_key, client_keys.client_tx_iv);
    let server_protector =
        GtpAeadProtector::new(client_keys.client_tx_key, client_keys.client_tx_iv);

    // Attacker tries to compute shared secret with client public key
    let attacker_shared = attacker_ephemeral
        .compute_shared_secret(&client_pk)
        .expect("contributory DH");
    let attacker_keys = derive_directional_handshake_session_keys(
        &attacker_shared,
        &client_nonce,
        &server_nonce,
        cid,
    );
    let attacker_protector =
        GtpAeadProtector::new(attacker_keys.client_tx_key, attacker_keys.client_tx_iv);

    // Client seals gameplay packet
    let original_payload = b"super_secret_game_event_position_update";
    let mut wire_buffer = [0u8; 256];
    wire_buffer[..original_payload.len()].copy_from_slice(original_payload);

    let sealed_len = client_protector
        .seal(
            PacketNumber(1),
            cid,
            b"header_aad",
            &mut wire_buffer,
            original_payload.len(),
        )
        .unwrap();

    // 1. Legitimate server can decrypt and authenticate
    let mut server_buf = wire_buffer;
    let decrypted_len = server_protector
        .open(
            PacketNumber(1),
            cid,
            b"header_aad",
            &mut server_buf,
            sealed_len,
        )
        .unwrap();
    assert_eq!(&server_buf[..decrypted_len], original_payload);

    // 2. Attacker attempting to open MUST FAIL with CryptoFailure (Poly1305 authentication rejection)
    let mut attacker_buf = wire_buffer;
    let att_res = attacker_protector.open(
        PacketNumber(1),
        cid,
        b"header_aad",
        &mut attacker_buf,
        sealed_len,
    );
    assert!(
        att_res.is_err(),
        "Attacker must NOT be able to open AEAD payload"
    );
}

#[test]
fn test_key_ratchet_forward_security() {
    let cid = ConnectionId(0x7777_8888_9999);
    let initial_key = [0x42u8; 32];

    let (phase_1_key, phase_1_iv) = ratchet_key(&initial_key, cid, 1);
    let (phase_2_key, phase_2_iv) = ratchet_key(&phase_1_key, cid, 2);

    assert_ne!(initial_key, phase_1_key);
    assert_ne!(phase_1_key, phase_2_key);
    assert_ne!(initial_key, phase_2_key);
    // The base IV rotates together with the key (nonce-pair stays coherent)
    assert_ne!(phase_1_iv, phase_2_iv);
    // Phase binding: re-deriving phase 1 from the same key is deterministic
    assert_eq!(phase_1_key, ratchet_key(&initial_key, cid, 1).0);
    // ...and a different phase never aliases another one's material
    assert_ne!(ratchet_key(&initial_key, cid, 3).0, phase_1_key);
}

#[test]
fn test_active_mitm_key_tamper_rejected() {
    use gtp_crypto::{compute_client_proof, verify_client_proof};

    let client_ephemeral = EphemeralKeyPair::generate();
    let server_ephemeral = EphemeralKeyPair::generate();
    let attacker_ephemeral = EphemeralKeyPair::generate();

    let client_pk = client_ephemeral.public_key;
    let client_nonce = client_ephemeral.nonce;

    let server_pk = server_ephemeral.public_key;
    let server_nonce = server_ephemeral.nonce;

    let attacker_pk = attacker_ephemeral.public_key;

    let cid = ConnectionId(0xAAAA_BBBB_CCCC_DDDD);

    // 1. Client computes shared secret with legitimate server PK
    let client_shared = client_ephemeral
        .compute_shared_secret(&server_pk)
        .expect("contributory DH");

    // 2. Attacker tampers with ClientHello on the wire, substituting client_pk with attacker_pk
    // Server computes shared secret with attacker_pk instead of client_pk
    let server_shared = server_ephemeral
        .compute_shared_secret(&attacker_pk)
        .expect("contributory DH");

    // 3. Client generates the transcript-bound HMAC key confirmation proof
    let client_transcript = gtp_crypto::HandshakeTranscript {
        client_pk,
        server_pk,
        client_nonce,
        server_nonce,
        connection_id: cid,
    };
    let client_proof = compute_client_proof(&client_shared, &client_transcript);

    // 4. Server MUST REJECT the tampered handshake finish proof: it shares a
    // secret with the attacker's leg and sees attacker_pk in the transcript.
    let server_view = gtp_crypto::HandshakeTranscript {
        client_pk: attacker_pk,
        server_pk,
        client_nonce,
        server_nonce,
        connection_id: cid,
    };
    let is_valid = verify_client_proof(&server_shared, &server_view, &client_proof);
    assert!(
        !is_valid,
        "Active MITM key substitution MUST fail cryptographic key confirmation!"
    );
}
