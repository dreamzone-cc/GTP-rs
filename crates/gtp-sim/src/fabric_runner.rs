//! Connection-driven multi-link fabric runner (D-2, gate G2).
//!
//! `SimulationRunner` drives one connection pair through one symmetric pipe.
//! This runner drives the same pair through the multi-link `SimulatedFabric`,
//! where every link carries **independent forward/reverse impairment
//! profiles** and a time-scripted impairment schedule — the substrate the
//! adaptive-routing engine needs to be testable at all (ARDP §5 D-1/D-2).
//!
//! Determinism ledger: the fabric's `event_log` plus the delivered
//! application-message sequences at both endpoints. Two runs with the same
//! master seed, the same script, and the same traffic produce byte-identical
//! ledgers — the G2 exit criterion, pinned by test below.

// Simulation-only crate: intentionally drives the legacy static-secret
#![allow(deprecated)] // constructors (see gtp_core::state::OFFLINE_SIM_MASTER_SECRET).
use crate::fabric::{FabricDirection, ScriptedImpairment, SimulatedFabric};
use crate::impairments::NetworkProfile;
use gtp_core::state::OFFLINE_SIM_MASTER_SECRET;
use gtp_core::{GtpConnection, ReceivedMessage};
use gtp_types::{ConnectionId, Duration, MonotonicTime};
use std::net::SocketAddr;

/// Drives a full client/server `GtpConnection` pair through the multi-link
/// fabric on a shared virtual clock. Data flows on `active_link`; the caller
/// can move it (or open further runners) per candidate — the single-path data
/// plane invariant (one active path per connection) is preserved by design.
pub struct FabricRunner {
    pub client: GtpConnection,
    pub server: GtpConnection,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub fabric: SimulatedFabric,
    pub current_time: MonotonicTime,
    /// Link index used for all transmissions in [`step`].
    pub active_link: usize,
}

impl FabricRunner {
    /// Same constructor conventions as `SimulationRunner` (addresses, CID,
    /// opposite SEC-1 roles, clock origin) so determinism baselines stay
    /// comparable across the two runners.
    pub fn new(master_seed: u64, link_profiles: &[(NetworkProfile, NetworkProfile)]) -> Self {
        let client_addr: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        let server_addr: SocketAddr = "10.0.0.2:6000".parse().unwrap();
        let cid = ConnectionId(0x1020304050607080);

        // SEC-1: opposite directional roles — same-role peers would collide
        // on one (key, nonce) pair across directions.
        let client = GtpConnection::new_with_role(
            cid,
            server_addr,
            true,
            true,
            OFFLINE_SIM_MASTER_SECRET,
            Default::default(),
        );
        let server = GtpConnection::new_with_role(
            cid,
            client_addr,
            true,
            false,
            OFFLINE_SIM_MASTER_SECRET,
            Default::default(),
        );

        Self {
            client,
            server,
            client_addr,
            server_addr,
            fabric: SimulatedFabric::new(master_seed, link_profiles),
            current_time: MonotonicTime::from_micros(1_000_000),
            active_link: 0,
        }
    }

    /// Attach a time-scripted impairment schedule (see `SimulatedFabric`).
    pub fn with_script(mut self, script: Vec<ScriptedImpairment>) -> Self {
        self.fabric = self.fabric.with_script(script);
        self
    }

    /// Advance the virtual clock by `step_delta`, apply any due scripted
    /// impairments, move both endpoints' outgoing datagrams through the
    /// active link, and dispatch everything the fabric delivered this tick.
    /// Returns `(client_delivered, server_delivered)` for this step.
    pub fn step(&mut self, step_delta: Duration) -> (Vec<ReceivedMessage>, Vec<ReceivedMessage>) {
        self.current_time += step_delta;
        let now = self.current_time;

        // D-2: scripted impairments are cursor-idempotent — ticking every
        // step applies each entry exactly once, at its scheduled time.
        self.fabric.tick_script(now);

        let mut client_received = Vec::new();
        let mut server_received = Vec::new();
        let mut out_buf = [0u8; 1500];

        // 1. Client produces → forward direction (unless a directed control
        //    frame overrides the destination, which flips the direction).
        while let Ok(Some((dest, len))) = self.client.produce_outgoing_datagram(now, &mut out_buf) {
            let direction = if dest == self.server_addr {
                FabricDirection::Forward
            } else {
                FabricDirection::Reverse
            };
            self.fabric.transmit(
                self.active_link,
                direction,
                self.client_addr,
                dest,
                out_buf[..len].to_vec(),
                now,
            );
        }

        // 2. Server produces → reverse direction by default.
        while let Ok(Some((dest, len))) = self.server.produce_outgoing_datagram(now, &mut out_buf) {
            let direction = if dest == self.client_addr {
                FabricDirection::Reverse
            } else {
                FabricDirection::Forward
            };
            self.fabric.transmit(
                self.active_link,
                direction,
                self.server_addr,
                dest,
                out_buf[..len].to_vec(),
                now,
            );
        }

        // 3. Deliver everything the fabric released this tick.
        for mut pkt in self.fabric.drain_ready(now) {
            if pkt.dest == self.server_addr {
                if let Ok(msgs) = self
                    .server
                    .handle_incoming_datagram(pkt.src, &mut pkt.data, now)
                {
                    server_received.extend(msgs);
                }
            } else if pkt.dest == self.client_addr {
                if let Ok(msgs) = self
                    .client
                    .handle_incoming_datagram(pkt.src, &mut pkt.data, now)
                {
                    client_received.extend(msgs);
                }
            }
        }

        (client_received, server_received)
    }

    /// Run the loop for `duration` at `step_size` granularity.
    pub fn run_for(
        &mut self,
        duration: Duration,
        step_size: Duration,
    ) -> (Vec<ReceivedMessage>, Vec<ReceivedMessage>) {
        let mut all_client = Vec::new();
        let mut all_server = Vec::new();
        let end_time = self.current_time + duration;
        while self.current_time < end_time {
            let (c, s) = self.step(step_size);
            all_client.extend(c);
            all_server.extend(s);
        }
        (all_client, all_server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::{ImpairDirection, ProfileDelta};
    use crate::impairments::NetworkProfile;
    use gtp_types::PriorityTier;

    /// 20 ms symmetric base profile with light loss/jitter so the RNG streams
    /// actually matter for the determinism ledger.
    fn live_profile() -> NetworkProfile {
        NetworkProfile {
            one_way_delay: Duration::from_millis(20),
            jitter: Duration::from_millis(2),
            loss_rate: 0.02,
            reorder_rate: 0.01,
            duplicate_rate: 0.01,
            bandwidth_bytes_per_sec: 10_000_000,
        }
    }

    /// D-2 / G2 exit criterion: two CONNECTION-DRIVEN runs with the same
    /// master seed, script, and traffic produce byte-identical ledgers —
    /// the fabric event log AND both delivered-message sequences — while a
    /// different seed diverges.
    #[test]
    fn same_seed_connection_driven_runs_are_byte_identical() {
        let script = |t_secs: u64| {
            vec![ScriptedImpairment {
                at: MonotonicTime::from_micros(1_000_000 + t_secs * 1_000_000),
                link: 0,
                direction: ImpairDirection::Forward,
                delta: ProfileDelta {
                    one_way_delay_add: Duration::from_millis(15),
                    ..Default::default()
                },
            }]
        };

        let drive = |seed: u64| {
            let mut runner = FabricRunner::new(
                seed,
                &[
                    (live_profile(), live_profile()),
                    (live_profile(), live_profile()),
                ],
            )
            .with_script(script(1));
            runner.active_link = 0;
            // 60 FPS ordered traffic for ~2.5 s of virtual time.
            let tick = Duration::from_micros(16_667);
            let end = runner.current_time + Duration::from_millis(2_500);
            let mut server_payloads: Vec<Vec<u8>> = Vec::new();
            let mut i = 0u32;
            while runner.current_time < end {
                runner
                    .client
                    .send_reliable_ordered(
                        gtp_types::OrderedGroupId(1),
                        format!("g2_det_{i:04}").into_bytes(),
                        PriorityTier::P3ReliableGameplay,
                        None,
                        runner.current_time,
                    )
                    .unwrap();
                i += 1;
                let (_, s) = runner.step(tick);
                server_payloads.extend(s.into_iter().map(|m| m.payload));
            }
            (runner.fabric.event_log.clone(), server_payloads)
        };

        let (log_a, payloads_a) = drive(42);
        let (log_b, payloads_b) = drive(42);
        assert_eq!(log_a, log_b, "same seed ⟹ identical fabric event log");
        assert_eq!(
            payloads_a, payloads_b,
            "same seed ⟹ identical delivered-message sequence"
        );
        assert!(!payloads_a.is_empty(), "test premise: traffic must flow");
        assert!(
            payloads_a.len() >= 100,
            "test premise: substantial ordered delivery, got {}",
            payloads_a.len()
        );

        let (log_c, _) = drive(43);
        assert_ne!(log_a, log_c, "different seed must diverge");
    }

    /// G2 exit criterion: per-direction impairment is demonstrably
    /// independent — a FORWARD-only delay step (scripted mid-run) spikes the
    /// SERVER's one-way-delay variance (it receives the impaired direction)
    /// while the CLIENT's stays quiet, and the mirror case behaves
    /// symmetrically. This is the substrate A-1/B-5 directional diagnosis
    /// builds on: RTT alone can never separate the two.
    #[test]
    fn per_direction_impairment_is_independently_visible() {
        let run =
            |direction: ImpairDirection| -> (u64, u64) {
                let step_at = MonotonicTime::from_micros(1_000_000 + 1_000_000);
                let mut runner = FabricRunner::new(7, &[(live_profile(), live_profile())])
                    .with_script(vec![ScriptedImpairment {
                        at: step_at,
                        link: 0,
                        direction,
                        delta: ProfileDelta {
                            one_way_delay_add: Duration::from_millis(40),
                            ..Default::default()
                        },
                    }]);
                let tick = Duration::from_micros(16_667);
                let end = runner.current_time + Duration::from_millis(2_000);
                while runner.current_time < end {
                    runner
                        .client
                        .send_unreliable(
                            b"dir_probe".to_vec(),
                            PriorityTier::P1Input,
                            None,
                            runner.current_time,
                        )
                        .unwrap();
                    runner.step(tick);
                }
                let server_owd = runner
                    .server
                    .control()
                    .query_metrics(runner.current_time)
                    .owd_var
                    .map(|d| d.as_micros())
                    .unwrap_or(0);
                let client_owd = runner
                    .client
                    .control()
                    .query_metrics(runner.current_time)
                    .owd_var
                    .map(|d| d.as_micros())
                    .unwrap_or(0);
                (server_owd, client_owd)
            };

        // Forward (+40 ms client→server only): the server sees the step, the
        // client does not.
        let (server_owd, client_owd) = run(ImpairDirection::Forward);
        assert!(
            server_owd >= 30_000,
            "forward impairment must surface at the server (got {server_owd} µs)"
        );
        assert!(
            client_owd <= 5_000,
            "forward impairment must NOT surface at the client (got {client_owd} µs)"
        );

        // Reverse (+40 ms server→client only): the mirror image.
        let (server_owd, client_owd) = run(ImpairDirection::Reverse);
        assert!(
            client_owd >= 30_000,
            "reverse impairment must surface at the client (got {client_owd} µs)"
        );
        assert!(
            server_owd <= 5_000,
            "reverse impairment must NOT surface at the server (got {server_owd} µs)"
        );
    }
}
