use gtp_cc::BackpressureLevel;
use gtp_path::ConnectionState;
use gtp_types::{FragmentId, MessageId};
use std::net::SocketAddr;

/// Strongly typed protocol events emitted for game engine hooks and telemetry observers.
/// Designed for easy future expansion as new protocol features are added.
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum ControlEvent {
    /// Connection state machine transitioned to a new state.
    StateChanged {
        old_state: ConnectionState,
        new_state: ConnectionState,
    },
    /// Congestion-induced backpressure level shifted.
    BackpressureChanged {
        old_level: BackpressureLevel,
        new_level: BackpressureLevel,
        effective_queue_bytes: usize,
    },
    /// NAT rebinding or path migration succeeded to a new remote address.
    PathMigrated {
        old_addr: SocketAddr,
        new_addr: SocketAddr,
    },
    /// Loss detection subsystem identified dropped packets.
    PacketLossDetected {
        lost_count: usize,
        lost_bytes: usize,
    },
    /// Selective retransmission queued for a reliable message fragment.
    RetransmissionTriggered {
        message_id: MessageId,
        fragment_id: FragmentId,
    },
    /// Probe Timeout (PTO) timer expired, triggering retransmission sweeps.
    PtoTriggered { pto_count: u32, inflight_bytes: u64 },
    /// Path MTU discovery updated the effective packet size.
    MtuUpdated { new_mtu: usize },
    /// Cryptographic key phase rotated for forward secrecy.
    KeyPhaseRotated { new_phase: bool },
    /// Explicit Congestion Notification (ECN CE) received from network routers.
    EcnExperienced { ce_count: u32 },
    /// One-way-delay measurement derived from the authenticated header
    /// timestamp (RE-1, gate G1). Emitted at a bounded rate —
    /// `GtpConfig::owd_sample_interval`, not per packet — because the event
    /// queue is unbounded and game traffic runs at 60–144 Hz. `epoch` is
    /// constant 0 until path-epoch tagging lands (RE-3, G3).
    OwdSample {
        /// Delay variance above the sliding floor, µs.
        owd_var_us: u32,
        /// RFC 3550 §6.4.1 inter-arrival jitter, µs.
        jitter_us: u32,
        /// Measurement epoch (RE-3); 0 until G3.
        epoch: u8,
    },
    /// Path event discriminator signature: ISP reroute vs congestion, with
    /// directional attribution (RE-6). **Defined in G1 (A-2) so later gates
    /// do not break the enum; emission begins in G5.**
    PathEventDetected {
        kind: PathEventKind,
        /// Step magnitude of the one-way-delay jump, µs.
        owd_step_us: u32,
        direction: PathDirection,
    },
    /// Extensible custom hook for future experimental or custom protocol features.
    CustomExtensionEvent { extension_id: u16, payload: Vec<u8> },
}

/// Classification of a detected path event (RE-6, consumed from G5).
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum PathEventKind {
    /// Step jump in one-way delay with no congestion correlation — the
    /// signature of an ISP reroute.
    Reroute,
    /// Ramp-up correlated with inflight/cwnd — ordinary congestion.
    Congestion,
}

/// Direction a degradation was observed in (RE-1 makes this separable:
/// probes alone can never split the two directions, timestamps can).
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum PathDirection {
    Forward,
    Reverse,
}
