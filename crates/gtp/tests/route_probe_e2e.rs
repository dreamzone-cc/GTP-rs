//! Route-probe end-to-end over a real in-process endpoint pair (loopback).
//!
//! Exercises the exact bidirectional-exchange contract the CLI uses:
//! the server side drains its events (RT-2), reads its receiver's
//! measurements (the client→server direction), encodes a
//! `MeasurementReport`, and sends it as a `ReliableOrdered` app message; the
//! client side keeps its own receiver's measurements (server→client), merges
//! the report into one `PathStats`, and produces a shadow verdict — which
//! actuates nothing (INV-15).

use gtp::prelude::*;
use gtp::route::MeasurementReport;
use std::time::Duration;

#[tokio::test]
async fn bidirectional_report_exchange_produces_a_shadow_verdict() {
    let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let s_addr = server_ep.local_addr().unwrap();

    // Server side: mirror of the CLI's per-connection task — drain events on
    // a fast tick (200 ms for test speed) and report what its receiver sees.
    let server_task = tokio::spawn(async move {
        let mut conn = server_ep.accept().await.expect("client connects");
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        loop {
            tokio::select! {
                msg = conn.recv() => {
                    if msg.is_none() { break; }
                }
                _ = tick.tick() => {
                    // RT-2: events must be drained; the aggregates below
                    // carry the substance, the event stream does not.
                    let _ = conn.drain_events().await;
                    let m = conn.query_metrics().await;
                    let report = MeasurementReport {
                        owd_var_us: m.owd_var.map(|d| d.as_micros() as u32),
                        jitter_us: m.jitter.map(|d| d.as_micros() as u32),
                        srtt_us: Some(m.smoothed_rtt.as_micros() as u32),
                        // Per-packet basis, matching the CLI contract.
                        samples: m.total_rx_packets.min(u32::MAX as u64) as u32,
                    };
                    let _ = conn
                        .send_reliable_ordered(
                            OrderedGroupId(gtp::route::REPORT_GROUP_ID),
                            report.encode().into_bytes(),
                            PriorityTier::P1Input,
                        )
                        .await;
                }
            }
        }
    });

    // Client side: 60 FPS traffic for ~1.5 s, collecting reports.
    let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let local = client_ep.local_addr().unwrap();
    let cid = ConnectionId(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
            ^ ((local.port() as u64) << 48),
    );
    let mut conn = client_ep.connect(cid, s_addr, true).await.unwrap();

    let start = std::time::Instant::now();
    let mut frame_tick = tokio::time::interval(Duration::from_millis(16));
    let mut frames: u32 = 0;
    let mut reports = 0u32;
    let mut latest: Option<MeasurementReport> = None;
    loop {
        if start.elapsed() >= Duration::from_millis(1500) {
            break;
        }
        tokio::select! {
            _ = frame_tick.tick() => {
                frames += 1;
                conn.send_unreliable(
                    format!("e2e_frame_{frames}").into_bytes(),
                    PriorityTier::P1Input,
                ).await.unwrap();
            }
            msg = conn.recv() => {
                let Some(msg) = msg else { break };
                if let MessageClass::ReliableOrdered { group_id, .. } = msg.class {
                    if group_id.as_u16() == gtp::route::REPORT_GROUP_ID {
                        if let Some(r) =
                            MeasurementReport::parse(&String::from_utf8_lossy(&msg.payload))
                        {
                            latest = Some(r);
                            reports += 1;
                        }
                    }
                }
            }
        }
    }

    // The exchange itself is proven: reports arrived and parsed.
    assert!(reports >= 3, "expected several reports, got {reports}");
    let report = latest.expect("at least one parsed report");
    // Loopback: the server measured a real (tiny) forward direction.
    assert!(report.srtt_us.is_some(), "server RTT present");

    // Merge into the bidirectional picture and produce the shadow verdict.
    let m = conn.query_metrics().await;
    let local_stats = gtp::route::PathStats {
        path_id: 0,
        rev_owd_var_us: m.owd_var.map(|d| d.as_micros() as u32),
        rev_jitter_us: m.jitter.map(|d| d.as_micros() as u32),
        rtt_us: Some(m.smoothed_rtt.as_micros() as u32),
        sample_count: frames.max(1),
        ..Default::default()
    };
    let stats = report.into_path_stats(0, &local_stats);
    let selection = gtp::route::select(&[stats]);
    let health = gtp::route::health(&stats);

    assert_eq!(selection.chosen, Some(0), "single candidate with data");
    assert!(
        matches!(health, gtp::route::HealthVerdict::Healthy)
            || matches!(health, gtp::route::HealthVerdict::Degraded(_))
    );
    // INV-15 shadow discipline: the verdict is a record — nothing to assert
    // on the data plane because nothing was allowed to move.

    server_task.abort();
}
