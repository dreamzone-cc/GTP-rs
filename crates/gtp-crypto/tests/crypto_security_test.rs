use gtp_crypto::{
    derive_handshake_session_keys, derive_session_keys, ratchet_key, EphemeralKeyPair,
    GtpAeadProtector, PacketProtector, PlaintextProtector, Protector,
};
use gtp_types::{ConnectionId, PacketNumber};

#[test]
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

#[test]
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

    // Legitimate parties establish shared secret and session keys
    let client_shared = client_ephemeral.compute_shared_secret(&server_pk);
    let server_shared = server_ephemeral.compute_shared_secret(&client_pk);

    let cid = ConnectionId(0x1234_5678_9ABC_DEF0);
    let (c_key, c_iv) =
        derive_handshake_session_keys(&client_shared, &client_nonce, &server_nonce, cid);
    let (s_key, s_iv) =
        derive_handshake_session_keys(&server_shared, &client_nonce, &server_nonce, cid);

    assert_eq!(c_key, s_key);
    assert_eq!(c_iv, s_iv);

    let client_protector = GtpAeadProtector::new(c_key, c_iv);
    let server_protector = GtpAeadProtector::new(s_key, s_iv);

    // Attacker tries to compute shared secret with client public key
    let attacker_shared = attacker_ephemeral.compute_shared_secret(&client_pk);
    let (att_key, att_iv) =
        derive_handshake_session_keys(&attacker_shared, &client_nonce, &server_nonce, cid);
    let attacker_protector = GtpAeadProtector::new(att_key, att_iv);

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

    let phase_1_key = ratchet_key(&initial_key, cid);
    let phase_2_key = ratchet_key(&phase_1_key, cid);

    assert_ne!(initial_key, phase_1_key);
    assert_ne!(phase_1_key, phase_2_key);
    assert_ne!(initial_key, phase_2_key);
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
    let client_shared = client_ephemeral.compute_shared_secret(&server_pk);
    let (client_key, _) =
        derive_handshake_session_keys(&client_shared, &client_nonce, &server_nonce, cid);

    // 2. Attacker tampers with ClientHello on the wire, substituting client_pk with attacker_pk
    // Server computes shared secret with attacker_pk instead of client_pk
    let server_shared = server_ephemeral.compute_shared_secret(&attacker_pk);
    let (server_key, _) =
        derive_handshake_session_keys(&server_shared, &client_nonce, &server_nonce, cid);

    // 3. Client generates HMAC Key Confirmation proof based on its key and PKs
    let client_proof = compute_client_proof(&client_key, &client_pk, &server_pk);

    // 4. Server MUST REJECT the tampered handshake finish proof
    let is_valid = verify_client_proof(&server_key, &attacker_pk, &server_pk, &client_proof);
    assert!(
        !is_valid,
        "Active MITM key substitution MUST fail cryptographic key confirmation!"
    );
}
