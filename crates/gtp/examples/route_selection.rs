//! Runnable example (ARDP §12.2): feed real loopback connection measurements
//! into the pure route scorer and print the shadow verdict.
//!
//! Run with: `cargo run -p gtp --example route_selection`

use gtp::prelude::*;
use gtp::route::{health, select, PathStats};
use std::net::SocketAddr;

fn main() {
    // A real loopback connection pair with injected time — its metrics are
    // genuine engine output, not hand-made numbers.
    let cid = ConnectionId(0x5E11_0000_0000_0001);
    let peer: SocketAddr = "127.0.0.1:6000".parse().unwrap();
    let mut client = GtpConnection::new_with_role(cid, peer, true, true, GtpConfig::default());
    let mut server = GtpConnection::new_with_role(
        cid,
        "127.0.0.1:5000".parse().unwrap(),
        true,
        false,
        GtpConfig::default(),
    );

    let t0 = MonotonicTime::from_micros(50_000_000);
    let mut now = t0;
    let mut buf = [0u8; 1500];
    let mut samples = 0u32;
    for i in 0..120u32 {
        client
            .send_unreliable(
                format!("route_example_{i}").into_bytes(),
                PriorityTier::P1Input,
                None,
                now,
            )
            .unwrap();
        while let Ok(Some((_, len))) = client.produce_outgoing_datagram(now, &mut buf) {
            let mut dgram = [0u8; 1500];
            dgram[..len].copy_from_slice(&buf[..len]);
            let _ = server.handle_incoming_datagram(peer, &mut dgram[..len], now);
        }
        // Return the ACKs so the client's receiver measures too.
        while let Ok(Some((_, len))) = server.produce_outgoing_datagram(now, &mut buf) {
            let mut dgram = [0u8; 1500];
            dgram[..len].copy_from_slice(&buf[..len]);
            let _ = client.handle_incoming_datagram(peer, &mut dgram[..len], now);
        }
        samples += 1;
        now += Duration::from_millis(16);
    }

    let client_metrics = client.control().query_metrics(now);
    let server_metrics = server.control().query_metrics(now);

    // Forward = what the SERVER receiver measured (client→server);
    // reverse  = what the CLIENT receiver measured (server→client).
    let stats = PathStats {
        path_id: 0,
        fwd_owd_var_us: server_metrics.owd_var.map(|d| d.as_micros() as u32),
        fwd_jitter_us: server_metrics.jitter.map(|d| d.as_micros() as u32),
        rev_owd_var_us: client_metrics.owd_var.map(|d| d.as_micros() as u32),
        rev_jitter_us: client_metrics.jitter.map(|d| d.as_micros() as u32),
        rtt_us: Some(client_metrics.smoothed_rtt.as_micros() as u32),
        sample_count: samples,
    };

    let selection = select(&[stats]);
    println!("GTP route selection example (loopback, both directions measured)");
    println!(
        "  forward  owd_var/jitter: {:?} / {:?} µs",
        stats.fwd_owd_var_us, stats.fwd_jitter_us
    );
    println!(
        "  reverse  owd_var/jitter: {:?} / {:?} µs",
        stats.rev_owd_var_us, stats.rev_jitter_us
    );
    println!(
        "  rtt: {:?} µs over {} samples",
        stats.rtt_us, stats.sample_count
    );
    println!("  selection: {}", selection.summary());
    println!("  health:    {:?}", health(&stats));
}
