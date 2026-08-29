use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gtp_types::{ConnectionId, GenerationId, MessageId, PacketNumber, StateKey, StateSequence};
use gtp_wire::frame::Frame;
use gtp_wire::header::PacketHeader;

fn bench_frame_encoding_decoding(c: &mut Criterion) {
    let state_frame = Frame::Data {
        message_id: MessageId(42),
        state_key: StateKey::new(100, 2),
        sequence: StateSequence(500),
        generation: GenerationId(1),
        deadline_ms: 50,
        payload: b"entity_position_x=12.5_y=45.2_z=-10.0",
    };

    c.bench_function("frame_data_encode", |b| {
        let mut buf = [0u8; 128];
        b.iter(|| state_frame.encode(black_box(&mut buf)).unwrap())
    });

    let mut encoded_buf = [0u8; 128];
    let encoded_len = state_frame.encode(&mut encoded_buf).unwrap();

    c.bench_function("frame_data_decode", |b| {
        b.iter(|| Frame::decode(black_box(&encoded_buf[..encoded_len])).unwrap())
    });

    let handshake_frame = Frame::ServerHello {
        server_public_key: [0x5Au8; 32],
        server_nonce: [0x3Cu8; 32],
        stateless_cookie: [0x88u8; 32],
        assigned_cid: ConnectionId(0x1020304050607080),
    };

    c.bench_function("frame_server_hello_encode", |b| {
        let mut buf = [0u8; 256];
        b.iter(|| handshake_frame.encode(black_box(&mut buf)).unwrap())
    });
}

fn bench_header_encoding_decoding(c: &mut Criterion) {
    let header = PacketHeader::new_short(ConnectionId(0xCAFE_BABE), PacketNumber(1000), 54321, 0);

    c.bench_function("header_short_encode", |b| {
        let mut buf = [0u8; 64];
        b.iter(|| header.encode(black_box(&mut buf)).unwrap())
    });

    let mut buf = [0u8; 64];
    header.encode(&mut buf).unwrap();

    c.bench_function("header_short_decode", |b| {
        b.iter(|| PacketHeader::decode(black_box(&buf)).unwrap())
    });
}

criterion_group!(
    benches,
    bench_frame_encoding_decoding,
    bench_header_encoding_decoding
);
criterion_main!(benches);
