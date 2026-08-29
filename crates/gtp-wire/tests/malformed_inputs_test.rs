use gtp_types::TransportError;
use gtp_wire::frame::Frame;
use gtp_wire::header::PacketHeader;

#[test]
fn test_decode_truncated_frames_does_not_panic() {
    // 1. Truncated ACK frame (frame type 0x01 with only 3 bytes)
    let buf_truncated_ack = [0x01, 0x00, 0x01];
    let res = Frame::decode(&buf_truncated_ack);
    assert!(matches!(res, Err(TransportError::TruncatedFrame { .. })));

    // 2. Truncated DATA frame (frame type 0x02 with only 5 bytes)
    let buf_truncated_data = [0x02, 0x00, 0x00, 0x00, 0x01];
    let res = Frame::decode(&buf_truncated_data);
    assert!(matches!(res, Err(TransportError::TruncatedFrame { .. })));

    // 3. Truncated RELIABLE DATA frame (frame type 0x03)
    let buf_truncated_reliable = [0x03, 0x00, 0x00];
    let res = Frame::decode(&buf_truncated_reliable);
    assert!(matches!(res, Err(TransportError::TruncatedFrame { .. })));

    // 4. Truncated CLOSE frame (frame type 0x09)
    let buf_truncated_close = [0x09, 0x00, 0x01];
    let res = Frame::decode(&buf_truncated_close);
    assert!(matches!(res, Err(TransportError::TruncatedFrame { .. })));

    // 5. Unknown frame type
    let buf_unknown = [0xFF, 0x01, 0x02, 0x03];
    let res = Frame::decode(&buf_unknown);
    assert!(matches!(res, Err(TransportError::MalformedFrame(_))));

    // 6. Completely empty buffer
    let buf_empty: [u8; 0] = [];
    let res = Frame::decode(&buf_empty);
    assert!(matches!(res, Err(TransportError::TruncatedFrame { .. })));
}

#[test]
fn test_decode_truncated_header_does_not_panic() {
    // 1. Empty buffer
    let res = PacketHeader::decode(&[]);
    assert!(matches!(res, Err(TransportError::BufferTooShort)));

    // 2. Too short for common header
    let res = PacketHeader::decode(&[0x00, 0x01, 0x02]);
    assert!(matches!(res, Err(TransportError::BufferTooShort)));

    // 3. Invalid header length field (less than minimum)
    let mut bad_header = [0u8; 32];
    bad_header[1] = 5; // header_len = 5 (less than MIN_COMMON_HEADER_LEN = 24)
    let res = PacketHeader::decode(&bad_header);
    assert!(matches!(res, Err(TransportError::InvalidPacket(_))));
}

#[test]
fn test_fuzz_mutated_buffers_resilience() {
    // Deterministic pseudo-random fuzzing over 1,000 corrupted buffers
    let mut state: u32 = 0x12345678;
    for _ in 0..1000 {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let len = ((state >> 16) % 64) as usize;
        let mut fuzz_buf = vec![0u8; len];
        for b in &mut fuzz_buf {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (state >> 24) as u8;
        }

        // Must never panic, always return clean Ok or Err
        let _ = Frame::decode(&fuzz_buf);
        let _ = PacketHeader::decode(&fuzz_buf);
    }
}
