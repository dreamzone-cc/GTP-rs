//! Cross-layer integration scenarios (master remediation plan §3.2).
//!
//! These tests drive the FULL public seam — application API → scheduler →
//! wire encoding → AEAD → datagram exchange → decryption → frame dispatch →
//! delivery — with deterministic injected time, so they fail if any pair of
//! layers stops agreeing on the contract between them.

use gtp::prelude::*;
use gtp_core::state::OFFLINE_SIM_MASTER_SECRET;
use std::net::SocketAddr;

fn pair(cid: ConnectionId, client_port: u16, server_port: u16) -> (GtpConnection, GtpConnection) {
    let client_addr: SocketAddr = format!("127.0.0.1:{client_port}").parse().unwrap();
    let server_addr: SocketAddr = format!("127.0.0.1:{server_port}").parse().unwrap();
    let client = GtpConnection::new_with_role(
        cid,
        server_addr,
        true,
        true,
        OFFLINE_SIM_MASTER_SECRET,
        GtpConfig::default(),
    );
    let server = GtpConnection::new_with_role(
        cid,
        client_addr,
        true,
        false,
        OFFLINE_SIM_MASTER_SECRET,
        GtpConfig::default(),
    );
    (client, server)
}

/// Scenario 1 — the full recovery seam: a ReliableOrdered flow loses the
/// datagram carrying order sequence 0; sequences 1..5 arrive and are held by
/// the reorder store; the PTO probe re-injects the lost frame through the
/// scheduler and the wire; the receiver delivers [0..5] complete, in order,
/// each message labelled with its OWN sequence (FR-8).
#[test]
fn full_recovery_path_delivers_ordered_stream_after_loss_and_pto() {
    let cid = ConnectionId(0x5501_0000_0000_0001);
    let (mut client, mut server) = pair(cid, 9101, 9102);
    let client_addr = server.hot.active_path;
    let t0 = MonotonicTime::from_micros(20_000_000);
    let group = OrderedGroupId(3);

    // One datagram per message so the loss injection is precise.
    let mut datagrams = Vec::new();
    let mut now = t0;
    for i in 0..6u32 {
        client
            .send_reliable_ordered(
                group,
                format!("ordered-{i}").into_bytes(),
                PriorityTier::P3ReliableGameplay,
                None,
                now,
            )
            .unwrap();
        let mut buf = [0u8; 1500];
        let (_, len) = client
            .produce_outgoing_datagram(now, &mut buf)
            .unwrap()
            .unwrap();
        datagrams.push(buf[..len].to_vec());
        now += Duration::from_millis(1);
    }
    let retransmissions_before = client.control().query_metrics(now).total_retransmissions;

    // The wire drops D0 (order sequence 0). D1..D5 arrive and must all be
    // held by the reorder store — nothing can deliver ahead of the hole.
    for dgram in &datagrams[1..] {
        let mut d = dgram.clone();
        let msgs = server
            .handle_incoming_datagram(client_addr, &mut d, now)
            .unwrap();
        assert!(
            msgs.is_empty(),
            "nothing may deliver ahead of the hole at seq 0"
        );
    }

    // No ACK ever returns, so the probe timer fires: advance well past the
    // worst-case PTO (initial estimate ~325ms, capped by pto_max_duration).
    now += Duration::from_millis(2_000);
    let mut buf = [0u8; 1500];
    let (_, retx_len) = client
        .produce_outgoing_datagram(now, &mut buf)
        .unwrap()
        .expect("the PTO probe must produce a retransmission datagram");
    let mut retx = buf[..retx_len].to_vec();

    let metrics = client.control().query_metrics(now);
    assert!(
        metrics.total_retransmissions > retransmissions_before,
        "the PTO sweep must re-enqueue the drained frames for retransmission"
    );

    // The retransmission closes the hole; the reorder store drains 1..5.
    let delivered = server
        .handle_incoming_datagram(client_addr, &mut retx, now)
        .unwrap();

    let seqs: Vec<u32> = delivered
        .iter()
        .map(|m| match m.class {
            MessageClass::ReliableOrdered { order_seq, .. } => order_seq,
            _ => panic!("only ordered messages were sent"),
        })
        .collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4, 5], "complete, in order, own seqs");
    for (i, msg) in delivered.iter().enumerate() {
        assert_eq!(msg.payload, format!("ordered-{i}").into_bytes());
    }
}

/// Scenario 4 — scheduling fairness under saturation: with P1 continuously
/// saturated, P3 and P4 traffic must still flow through the whole engine
/// (scheduler → wire → peer). Under the pre-N-3 fixed-order scan they stayed
/// at exactly zero until the entire P1 queue drained.
#[test]
fn p3_and_p4_traffic_flows_while_p1_is_saturated() {
    let cid = ConnectionId(0x5502_0000_0000_0002);
    let (mut client, mut server) = pair(cid, 9103, 9104);
    let client_addr = server.hot.active_path;
    let server_addr = client.hot.active_path;
    let mut now = MonotonicTime::from_micros(21_000_000);

    // Byte-weighted provisioning matched to the DRR weights 35:15:5 —
    // equal-size payloads so item shares equal byte shares. The first byte
    // of each payload marks its tier.
    let mk = |tier: u8| vec![tier; 100];
    for _ in 0..350 {
        client
            .send_unreliable(mk(1), PriorityTier::P1Input, None, now)
            .unwrap();
    }
    for _ in 0..150 {
        client
            .send_unreliable(mk(3), PriorityTier::P3ReliableGameplay, None, now)
            .unwrap();
    }
    for _ in 0..50 {
        client
            .send_unreliable(mk(4), PriorityTier::P4BulkCosmetic, None, now)
            .unwrap();
    }

    let mut counts = [0usize; 5];
    let mut saturation_point: Option<(usize, usize, usize)> = None;
    let mut out = [0u8; 2048];
    for _ in 0..2000 {
        now += Duration::from_millis(1);
        let (_, len) = match client.produce_outgoing_datagram(now, &mut out) {
            Ok(Some(produced)) => produced,
            Ok(None) => {
                if client.hot.scheduler.is_empty() {
                    break;
                }
                continue; // pacing-limited: wait for the next token refill
            }
            Err(e) => panic!("produce failed: {e:?}"),
        };

        let mut dgram = out[..len].to_vec();
        for msg in server
            .handle_incoming_datagram(client_addr, &mut dgram, now)
            .unwrap()
        {
            counts[msg.payload[0] as usize] += 1;
        }

        // Return the peer's ACK so in-flight debt drains and the PTO never
        // interferes with the fairness measurement.
        if let Ok(Some((_, ack_len))) = server.produce_outgoing_datagram(now, &mut out) {
            let mut ack = out[..ack_len].to_vec();
            let _ = client.handle_incoming_datagram(server_addr, &mut ack, now);
        }

        if saturation_point.is_none() && counts[1] >= 100 {
            saturation_point = Some((counts[1], counts[3], counts[4]));
        }
    }

    let (p1, p3, p4) =
        saturation_point.expect("the loop must pass through a window with P1 still saturated");
    assert!(p1 < 350, "test premise: P1 must not be fully drained yet");
    assert!(p3 > 0, "P3 must flow while P1 is still saturated (N-3)");
    assert!(p4 > 0, "P4 must flow while P1 is still saturated (N-3)");

    // Everything provisioned is eventually delivered — exactly once each.
    assert_eq!(counts[1], 350);
    assert_eq!(counts[3], 150);
    assert_eq!(counts[4], 50);
}
