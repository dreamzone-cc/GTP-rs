//! N-1 diagnostic: exercises the handshake-routing gate through the **real RX loop**,
//! not through `is_handshake_candidate` in isolation.
//!
//! The unit tests next to the gate prove the predicate. They do not prove the RX loop
//! consults it — delete the call site and they still pass. This test closes that gap by
//! driving real traffic through `start_rx_loop` and watching for the signature of the
//! bug: without the gate, 3 of 256 datagrams have a first ciphertext byte equal to a
//! handshake frame type, enter a handshake branch, and are discarded before decryption.
//! Reliable traffic turns that silent 1.172% loss into retransmissions.
//!
//! `#[ignore]` on purpose: it is a volume test over loopback, and genuine socket-buffer
//! loss on a loaded machine would make a strict assertion flaky in ordinary CI. It is a
//! diagnostic to run on demand, while the deterministic unit tests stay the CI guard:
//!
//! ```text
//! cargo test -p gtp-runtime-tokio --test n1_routing_gate_test -- --ignored --nocapture
//! ```

use gtp_runtime_tokio::GtpEndpoint;
use gtp_types::{ConnectionId, PriorityTier};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Expected retransmissions without the gate: 1200 * 3/256 ≈ 14. With it: 0.
const MESSAGES: usize = 1200;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "volume test over loopback; run on demand with --ignored"]
async fn rx_loop_does_not_discard_ciphertext_colliding_with_handshake_frame_types() {
    let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind server");
    let server_addr = server_ep.local_addr().unwrap();

    let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind client");
    let conn = timeout(
        Duration::from_secs(5),
        client_ep.connect(ConnectionId(0x0000_0000_0000_00B1), server_addr, true),
    )
    .await
    .expect("handshake timed out")
    .expect("handshake failed");

    let mut srv = timeout(Duration::from_secs(5), server_ep.accept())
        .await
        .expect("accept timed out")
        .expect("accept");

    let received = Arc::new(AtomicUsize::new(0));
    let r = Arc::clone(&received);
    tokio::spawn(async move {
        while srv.recv().await.is_some() {
            r.fetch_add(1, Ordering::Relaxed);
        }
    });

    for i in 0..MESSAGES {
        conn.send_reliable_unordered(
            format!("n1-loop-{i}").into_bytes(),
            PriorityTier::P3ReliableGameplay,
        )
        .await
        .expect("send");
        if i % 300 == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    // Reliable traffic always arrives eventually; the discriminator is whether it had to
    // be retransmitted to get there.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while received.load(Ordering::Relaxed) < MESSAGES && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let metrics = conn.query_metrics().await;
    println!(
        "delivered {}/{MESSAGES}  retransmissions {}  pto {}",
        received.load(Ordering::Relaxed),
        metrics.total_retransmissions,
        metrics.pto_count
    );

    assert_eq!(
        received.load(Ordering::Relaxed),
        MESSAGES,
        "reliable stream did not complete"
    );
    assert_eq!(
        metrics.total_retransmissions,
        0,
        "the RX loop discarded datagrams on a lossless loopback path — {} \
         retransmissions over {MESSAGES} messages is the N-1 signature (expected ~{} \
         without the gate)",
        metrics.total_retransmissions,
        MESSAGES * 3 / 256
    );
}
