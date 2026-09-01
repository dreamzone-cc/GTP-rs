//! N-2 regression suite: an application that stops draining its own connection must
//! not be able to stall the endpoint's shared RX loop.
//!
//! Every wait here is bounded by a timeout, so the current (broken) behaviour makes
//! these tests FAIL rather than hang.
//!
//! The policy under test, per receive class, when the application channel is full:
//!   Unreliable          -> drop
//!   UnreliableSequenced -> drop (supersession already applied upstream)
//!   ReliableUnordered   -> close the offending connection
//!   ReliableOrdered     -> close the offending connection

use gtp_runtime_tokio::{AsyncGtpConnection, GtpEndpoint};
use gtp_types::{
    ConnectionId, GenerationId, MessageClass, OrderedGroupId, PriorityTier, StateKey, StateSequence,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Application channel capacity in `GtpEndpoint` (`mpsc::channel(1024)`).
const APP_CHANNEL_CAPACITY: usize = 1024;
/// Comfortably past the bound, so the channel is genuinely saturated.
const FLOOD: usize = APP_CHANNEL_CAPACITY + 600;
const PROBE: usize = 50;

struct Bench {
    _server_ep: Arc<GtpEndpoint>,
    _ep_a: GtpEndpoint,
    _ep_b: GtpEndpoint,
    cli_a: AsyncGtpConnection,
    cli_b: AsyncGtpConnection,
    srv_a: AsyncGtpConnection,
    srv_b: AsyncGtpConnection,
}

async fn setup() -> Bench {
    let server_ep = Arc::new(
        GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .expect("bind server"),
    );
    let server_addr = server_ep.local_addr().unwrap();

    let ep_a = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind A");
    let cli_a = timeout(
        Duration::from_secs(5),
        ep_a.connect(ConnectionId(0xAAAA_0000_0000_0001), server_addr, true),
    )
    .await
    .expect("A handshake timed out")
    .expect("A handshake failed");
    let srv_a = timeout(Duration::from_secs(5), server_ep.accept())
        .await
        .expect("accept A timed out")
        .expect("accept A");

    let ep_b = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind B");
    let cli_b = timeout(
        Duration::from_secs(5),
        ep_b.connect(ConnectionId(0xBBBB_0000_0000_0002), server_addr, true),
    )
    .await
    .expect("B handshake timed out")
    .expect("B handshake failed");
    let srv_b = timeout(Duration::from_secs(5), server_ep.accept())
        .await
        .expect("accept B timed out")
        .expect("accept B");

    Bench {
        _server_ep: server_ep,
        _ep_a: ep_a,
        _ep_b: ep_b,
        cli_a,
        cli_b,
        srv_a,
        srv_b,
    }
}

/// Drains a server-side connection in the background, counting what arrives.
fn drain(mut conn: AsyncGtpConnection) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    tokio::spawn(async move {
        while conn.recv().await.is_some() {
            c.fetch_add(1, Ordering::Relaxed);
        }
    });
    count
}

async fn wait_until(target: usize, counter: &AtomicUsize, budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if counter.load(Ordering::Relaxed) >= target {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn flood_unreliable(conn: &AsyncGtpConnection, n: usize) {
    for i in 0..n {
        let _ = conn
            .send_unreliable(format!("flood-{i}").into_bytes(), PriorityTier::P1Input)
            .await;
        if i % 400 == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(1200)).await;
}

// ---------------------------------------------------------------------------
// 1 + 5: a parked consumer on A must not stop B from receiving.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_consumer_on_one_connection_does_not_stall_the_others() {
    let bench = setup().await;
    // A's application never polls: the channel saturates and stays saturated.
    let _parked_a = bench.srv_a;
    let b_count = drain(bench.srv_b);

    for i in 0..PROBE {
        let _ = bench
            .cli_b
            .send_unreliable(format!("b-warmup-{i}").into_bytes(), PriorityTier::P1Input)
            .await;
    }
    assert!(
        wait_until(PROBE, &b_count, Duration::from_secs(5)).await,
        "B could not receive even before the flood: {} of {PROBE}",
        b_count.load(Ordering::Relaxed)
    );

    flood_unreliable(&bench.cli_a, FLOOD).await;

    let before = b_count.load(Ordering::Relaxed);
    for i in 0..PROBE {
        let _ = bench
            .cli_b
            .send_unreliable(format!("b-probe-{i}").into_bytes(), PriorityTier::P1Input)
            .await;
    }
    assert!(
        wait_until(before + PROBE, &b_count, Duration::from_secs(10)).await,
        "B received {} of {PROBE} probes after A saturated its own channel — \
         a slow consumer on A stalled the shared RX loop",
        b_count.load(Ordering::Relaxed) - before
    );
}

// ---------------------------------------------------------------------------
// 6 + 8: the RX loop must not hold the per-connection lock while delivering.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saturated_channel_does_not_hold_the_connection_lock() {
    let bench = setup().await;
    let _drained_b = drain(bench.srv_b);
    let parked_a = bench.srv_a;

    flood_unreliable(&bench.cli_a, FLOOD).await;

    // `feedback()` takes the same per-connection mutex the RX loop holds. If the RX
    // loop parks on a full channel while holding it, this never returns.
    assert!(
        timeout(Duration::from_secs(5), parked_a.feedback())
            .await
            .is_ok(),
        "feedback() on the saturated connection blocked — the RX loop is awaiting \
         message delivery while holding the connection lock"
    );
}

// ---------------------------------------------------------------------------
// 7: a saturated connection must not stop new players from joining.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_client_can_connect_while_another_connection_is_saturated() {
    let bench = setup().await;
    let _drained_b = drain(bench.srv_b);
    let _parked_a = bench.srv_a;
    let server_addr = bench._server_ep.local_addr().unwrap();

    flood_unreliable(&bench.cli_a, FLOOD).await;

    let ep_c = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let joined = timeout(
        Duration::from_secs(10),
        ep_c.connect(ConnectionId(0xCCCC_0000_0000_0003), server_addr, true),
    )
    .await;
    let verdict = match &joined {
        Ok(Ok(_)) => "joined".to_string(),
        Ok(Err(e)) => format!("handshake error: {e:?}"),
        Err(_) => "timed out".to_string(),
    };
    assert!(
        matches!(joined, Ok(Ok(_))),
        "a new client could not join while another connection was saturated: {verdict}"
    );
}

// ---------------------------------------------------------------------------
// 2: sequenced overflow must not deadlock, and must preserve supersession order.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sequenced_overflow_does_not_deadlock_and_keeps_supersession_monotonic() {
    let bench = setup().await;
    let b_count = drain(bench.srv_b);
    let mut parked_a = bench.srv_a;

    // Distinct state keys per message. A single key would never saturate anything:
    // the SEND-side scheduler supersedes the previous queued message for that key
    // (scheduler.rs), so at most one is ever in flight. A large world with many
    // entities is what actually fills the receive channel.
    for i in 0..FLOOD {
        let _ = bench
            .cli_a
            .send_sequenced(
                StateKey::new(i as u32, 1),
                StateSequence(1),
                GenerationId(1),
                format!("state-{i}").into_bytes(),
            )
            .await;
        if i % 400 == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(1200)).await;

    // B must still be alive: no deadlock.
    let before = b_count.load(Ordering::Relaxed);
    for i in 0..PROBE {
        let _ = bench
            .cli_b
            .send_unreliable(format!("b-{i}").into_bytes(), PriorityTier::P1Input)
            .await;
    }
    assert!(
        wait_until(before + PROBE, &b_count, Duration::from_secs(10)).await,
        "sequenced overflow on A stalled B"
    );

    // Whatever A does receive must still be monotonic per state key: the drop policy
    // may shorten the stream but must never reorder or resurrect stale state.
    let mut seen: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    let mut delivered = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(300), parked_a.recv()).await {
            Ok(Some(msg)) => {
                if let MessageClass::UnreliableSequenced {
                    state_key,
                    sequence,
                    ..
                } = msg.class
                {
                    delivered += 1;
                    if let Some(prev) = seen.insert(state_key.to_u48(), sequence.0) {
                        assert!(
                            sequence.0 > prev,
                            "supersession broken for key {:?}: sequence {} after {prev}",
                            state_key,
                            sequence.0
                        );
                    }
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    assert!(delivered > 0, "A received no sequenced state at all");
}

// ---------------------------------------------------------------------------
// 3: ReliableUnordered overflow must close the offending connection, not the endpoint.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reliable_unordered_overflow_closes_only_the_offending_connection() {
    let bench = setup().await;
    let b_count = drain(bench.srv_b);
    let parked_a = bench.srv_a;

    for i in 0..FLOOD {
        let _ = bench
            .cli_a
            .send_reliable_unordered(
                format!("rpc-{i}").into_bytes(),
                PriorityTier::P3ReliableGameplay,
            )
            .await;
        if i % 400 == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The endpoint stays alive for everyone else.
    let before = b_count.load(Ordering::Relaxed);
    for i in 0..PROBE {
        let _ = bench
            .cli_b
            .send_unreliable(format!("b-{i}").into_bytes(), PriorityTier::P1Input)
            .await;
    }
    assert!(
        wait_until(before + PROBE, &b_count, Duration::from_secs(10)).await,
        "reliable overflow on A stalled B — the endpoint went down with one connection"
    );

    // A is closed: draining its backlog ends with the channel closed, never a hang.
    let mut a = parked_a;
    let outcome = timeout(Duration::from_secs(20), async move {
        let mut n = 0usize;
        while a.recv().await.is_some() {
            n += 1;
        }
        n
    })
    .await;
    assert!(
        outcome.is_ok(),
        "A was never closed after a reliable message overflowed its channel — \
         reliable data must not be dropped, so the connection must end"
    );

    // A is gone. B must now be fully normal, not merely alive: a RELIABLE round trip
    // exercises B's ACK processing, which shares the RX loop A just tore down.
    let after_close = b_count.load(Ordering::Relaxed);
    for i in 0..PROBE {
        let _ = bench
            .cli_b
            .send_reliable_unordered(
                format!("b-after-{i}").into_bytes(),
                PriorityTier::P3ReliableGameplay,
            )
            .await;
    }
    assert!(
        wait_until(after_close + PROBE, &b_count, Duration::from_secs(10)).await,
        "B received {} of {PROBE} reliable messages AFTER A was closed — closing one \
         connection damaged the others",
        b_count.load(Ordering::Relaxed) - after_close
    );

    // And the endpoint itself continues: a brand new client can still join.
    let server_addr = bench._server_ep.local_addr().unwrap();
    let ep_d = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    assert!(
        matches!(
            timeout(
                Duration::from_secs(10),
                ep_d.connect(ConnectionId(0xDDDD_0000_0000_0004), server_addr, true),
            )
            .await,
            Ok(Ok(_))
        ),
        "the endpoint stopped accepting new clients after closing a saturated connection"
    );
}

// ---------------------------------------------------------------------------
// 4: ReliableOrdered overflow must close the offending connection, not the endpoint.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reliable_ordered_overflow_closes_only_the_offending_connection() {
    let bench = setup().await;
    let b_count = drain(bench.srv_b);
    let parked_a = bench.srv_a;

    for i in 0..FLOOD {
        let _ = bench
            .cli_a
            .send_reliable_ordered(
                OrderedGroupId(1),
                format!("line-{i}").into_bytes(),
                PriorityTier::P3ReliableGameplay,
            )
            .await;
        if i % 400 == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let before = b_count.load(Ordering::Relaxed);
    for i in 0..PROBE {
        let _ = bench
            .cli_b
            .send_unreliable(format!("b-{i}").into_bytes(), PriorityTier::P1Input)
            .await;
    }
    assert!(
        wait_until(before + PROBE, &b_count, Duration::from_secs(10)).await,
        "ordered overflow on A stalled B — the endpoint went down with one connection"
    );

    let mut a = parked_a;
    let outcome = timeout(Duration::from_secs(20), async move {
        let mut n = 0usize;
        while a.recv().await.is_some() {
            n += 1;
        }
        n
    })
    .await;
    assert!(
        outcome.is_ok(),
        "A was never closed after an ordered message overflowed its channel"
    );

    // B must still deliver an ORDERED stream correctly after A's close: the group's
    // sequencing is per connection, and A's teardown must not perturb it.
    let after_close = b_count.load(Ordering::Relaxed);
    for i in 0..PROBE {
        let _ = bench
            .cli_b
            .send_reliable_ordered(
                OrderedGroupId(2),
                format!("b-after-{i}").into_bytes(),
                PriorityTier::P3ReliableGameplay,
            )
            .await;
    }
    assert!(
        wait_until(after_close + PROBE, &b_count, Duration::from_secs(10)).await,
        "B received {} of {PROBE} ordered messages AFTER A was closed",
        b_count.load(Ordering::Relaxed) - after_close
    );
}
