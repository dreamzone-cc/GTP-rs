use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gtp_crypto::{
    compute_client_proof, derive_handshake_session_keys, EphemeralKeyPair, GtpAeadProtector,
    PacketProtector,
};
use gtp_types::{ConnectionId, PacketNumber};

fn bench_chacha20_poly1305(c: &mut Criterion) {
    let key = [0x42u8; 32];
    let iv = [0x24u8; 12];
    let protector = GtpAeadProtector::new(key, iv);

    let pn = PacketNumber(1);
    let cid = ConnectionId(0x1122_3344_5566_7788);
    let aad = b"gtp_packet_header_aad_bytes";
    let payload = vec![0xABu8; 1200];

    c.bench_function("chacha20_poly1305_seal_1200b", |b| {
        let mut buf = vec![0u8; 1500];
        b.iter(|| {
            buf[..payload.len()].copy_from_slice(&payload);
            protector
                .seal(pn, cid, aad, &mut buf, black_box(payload.len()))
                .unwrap()
        })
    });

    c.bench_function("chacha20_poly1305_open_1200b", |b| {
        let mut buf = vec![0u8; 1500];
        buf[..payload.len()].copy_from_slice(&payload);
        let sealed_len = protector
            .seal(pn, cid, aad, &mut buf, payload.len())
            .unwrap();
        b.iter(|| {
            let mut open_buf = buf.clone();
            protector
                .open(pn, cid, aad, &mut open_buf, black_box(sealed_len))
                .unwrap()
        })
    });
}

fn bench_handshake_crypto(c: &mut Criterion) {
    let client_pair = EphemeralKeyPair::generate();
    let server_pair = EphemeralKeyPair::generate();
    let client_pk = client_pair.public_key;
    let client_nonce = client_pair.nonce;
    let server_pk = server_pair.public_key;
    let server_nonce = server_pair.nonce;

    c.bench_function("x25519_key_generation", |b| {
        b.iter(|| black_box(EphemeralKeyPair::generate()))
    });

    c.bench_function("x25519_diffie_hellman", |b| {
        b.iter(|| {
            let pair = EphemeralKeyPair::generate();
            black_box(pair.compute_shared_secret(&server_pk))
        })
    });

    let cid = ConnectionId(0x10203040);
    let client_shared = client_pair.compute_shared_secret(&server_pk);
    let (key, _) = derive_handshake_session_keys(&client_shared, &client_nonce, &server_nonce, cid);

    c.bench_function("handshake_client_proof_hmac", |b| {
        b.iter(|| {
            black_box(compute_client_proof(
                black_box(&key),
                black_box(&client_pk),
                black_box(&server_pk),
            ))
        })
    });
}

criterion_group!(benches, bench_chacha20_poly1305, bench_handshake_crypto);
criterion_main!(benches);
