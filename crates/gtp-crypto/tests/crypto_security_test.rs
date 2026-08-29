use gtp_crypto::{derive_session_keys, GtpAeadProtector, PlaintextProtector, Protector};
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
