// Simulation-only crate: intentionally drives the legacy static-secret
#![allow(deprecated)] // constructors (see gtp_core::state::OFFLINE_SIM_MASTER_SECRET).
use crate::impairments::NetworkProfile;
use crate::simulated_network::SimulatedNetwork;
use gtp_core::state::OFFLINE_SIM_MASTER_SECRET;
use gtp_core::{GtpConnection, ReceivedMessage};
use gtp_types::{ConnectionId, Duration, MonotonicTime};
use std::net::SocketAddr;

/// Deterministic stepping simulation testbed for GTP protocol verification.
pub struct SimulationRunner {
    pub client: GtpConnection,
    pub server: GtpConnection,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub network: SimulatedNetwork,
    pub profile: NetworkProfile,
    pub current_time: MonotonicTime,
}

impl SimulationRunner {
    pub fn new(seed: u64, profile: NetworkProfile) -> Self {
        let client_addr: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        let server_addr: SocketAddr = "10.0.0.2:6000".parse().unwrap();
        let cid = ConnectionId(0x1020304050607080);

        // SEC-1: the two peers must hold opposite directional roles — with the same
        // role both would seal with the same key and every packet number would
        // collide on one (key, nonce) pair across directions.
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
            network: SimulatedNetwork::new(seed),
            profile,
            current_time: MonotonicTime::from_micros(1_000_000),
        }
    }

    /// Advance simulation clock by `step_delta` and execute transmit/receive steps.
    pub fn step(&mut self, step_delta: Duration) -> (Vec<ReceivedMessage>, Vec<ReceivedMessage>) {
        self.current_time += step_delta;
        let now = self.current_time;

        let mut client_received = Vec::new();
        let mut server_received = Vec::new();

        // 1. Client produce outgoing datagrams -> send to simulated network
        let mut out_buf = [0u8; 1500];
        while let Ok(Some((dest, len))) = self.client.produce_outgoing_datagram(now, &mut out_buf) {
            let data = out_buf[..len].to_vec();
            self.network
                .transmit(self.client_addr, dest, data, now, &self.profile);
        }

        // 2. Server produce outgoing datagrams -> send to simulated network
        while let Ok(Some((dest, len))) = self.server.produce_outgoing_datagram(now, &mut out_buf) {
            let data = out_buf[..len].to_vec();
            self.network
                .transmit(self.server_addr, dest, data, now, &self.profile);
        }

        // 3. Drain and deliver arrived packets from simulated network
        let delivered_packets = self.network.drain_ready(now);
        for mut pkt in delivered_packets {
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

    /// Run simulation loop for specified duration.
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
    use gtp_types::{GenerationId, OrderedGroupId, PriorityTier, StateKey, StateSequence};

    #[test]
    fn test_simulation_reliable_ordered_recovery_under_high_loss() {
        let mut runner = SimulationRunner::new(12345, NetworkProfile::extreme_loss()); // 20% packet loss!

        // Send 10 reliable ordered messages from client to server
        for i in 0..10 {
            let payload = format!("ordered_event_{}", i).into_bytes();
            runner
                .client
                .send_reliable_ordered(
                    OrderedGroupId(1),
                    payload,
                    PriorityTier::P3ReliableGameplay,
                    None,
                    runner.current_time,
                )
                .unwrap();
        }

        // Run simulation for 3 seconds with 1ms time slices
        let (_client_msgs, server_msgs) =
            runner.run_for(Duration::from_secs(3), Duration::from_millis(1));

        // Server MUST receive all 10 messages in exact in-order sequence despite 20% loss!
        assert_eq!(server_msgs.len(), 10);
        for (i, msg) in server_msgs.iter().enumerate() {
            let expected_payload = format!("ordered_event_{}", i).into_bytes();
            assert_eq!(msg.payload, expected_payload);
        }
    }

    #[test]
    fn test_simulation_state_sequenced_streaming_at_60fps() {
        let mut runner = SimulationRunner::new(54321, NetworkProfile::bad_cellular_wifi());
        let key = StateKey::new(100, 1);
        let mut all_received = Vec::new();

        // Client generates 30 consecutive entity updates at 60 FPS (16ms per frame)
        for seq in 1..=30 {
            let payload = format!("entity_pos_{}", seq).into_bytes();
            runner
                .client
                .send_sequenced(
                    key,
                    StateSequence(seq),
                    GenerationId(1),
                    None,
                    payload,
                    runner.current_time,
                )
                .unwrap();

            let (_c, s) = runner.step(Duration::from_millis(16));
            all_received.extend(s);
        }

        // Drain remaining in-flight packets
        let (_c, s) = runner.run_for(Duration::from_millis(500), Duration::from_millis(1));
        all_received.extend(s);

        // Ensure server received updates and the final state is latest
        assert!(!all_received.is_empty());
        let last = all_received.last().unwrap();
        assert_eq!(last.payload, b"entity_pos_30");
    }
}
