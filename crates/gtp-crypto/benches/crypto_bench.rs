use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use gtp_crypto::{
    compute_client_proof, derive_directional_handshake_session_keys, DirectionalKeys,
    EphemeralKeyPair, GtpAeadProtector, PacketProtector,
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

    // C-20: the buffer clone moved OUT of the measured loop so the benchmark
    // measures AEAD open only, not heap allocation + memcpy.
    c.bench_function("chacha20_poly1305_open_1200b", |b| {
        b.iter_batched(
            || {
                let mut buf = vec![0u8; 1500];
                buf[..payload.len()].copy_from_slice(&payload);
                let sealed_len = protector
                    .seal(pn, cid, aad, &mut buf, payload.len())
                    .unwrap();
                (buf, sealed_len)
            },
            |(mut open_buf, sealed_len)| {
                protector
                    .open(pn, cid, aad, &mut open_buf, black_box(sealed_len))
                    .unwrap()
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_handshake_crypto(c: &mut Criterion) {
    let server_pair = EphemeralKeyPair::generate();
    let server_pk = server_pair.public_key;
    let client_nonce = [0x11u8; 32];
    let server_nonce = server_pair.nonce;

    c.bench_function("x25519_key_generation", |b| {
        b.iter(|| black_box(EphemeralKeyPair::generate()))
    });

    // C-21: key generation moved out of the measured loop — this benchmark
    // measures the DH scalar multiplication alone.
    c.bench_function("x25519_diffie_hellman", |b| {
        b.iter_batched(
            EphemeralKeyPair::generate,
            |pair| black_box(pair.compute_shared_secret(&server_pk).is_ok()),
            BatchSize::SmallInput,
        )
    });

    let cid = ConnectionId(0x10203040);
    let client_pair = EphemeralKeyPair::generate();
    let client_pk = client_pair.public_key;
    let client_shared = client_pair
        .compute_shared_secret(&server_pk)
        .expect("contributory DH");
    let keys: DirectionalKeys = derive_directional_handshake_session_keys(
        &client_shared,
        &client_nonce,
        &server_nonce,
        cid,
    );
    let key = keys.client_tx_key;

    c.bench_function("handshake_client_proof_hmac", |b| {
        b.iter(|| {
            black_box(compute_client_proof(
                black_box(&key),
                black_box(&client_pk),
                black_box(&server_pk),
            ))
        })
    });

    c.bench_function("handshake_directional_key_derivation", |b| {
        b.iter(|| {
            black_box(derive_directional_handshake_session_keys(
                &client_shared,
                &client_nonce,
                &server_nonce,
                cid,
            ))
        })
    });
}

criterion_group!(benches, bench_chacha20_poly1305, bench_handshake_crypto);
criterion_main!(benches);
