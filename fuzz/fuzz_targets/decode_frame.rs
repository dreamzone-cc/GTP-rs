#![no_main]

use gtp_wire::frame::Frame;
use gtp_wire::header::PacketHeader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1. Fuzz PacketHeader decoding
    let _ = PacketHeader::decode(data);

    // 2. Fuzz Frame decoding over full MTU buffer slices
    let _ = Frame::decode(data);
});
