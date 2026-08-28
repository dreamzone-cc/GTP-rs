use std::net::SocketAddr;
use gtp_cc::BackpressureLevel;
use gtp_path::ConnectionState;
use gtp_types::{FragmentId, MessageId};

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
    PtoTriggered {
        pto_count: u32,
        inflight_bytes: u64,
    },
    /// Path MTU discovery updated the effective packet size.
    MtuUpdated {
        new_mtu: usize,
    },
    /// Cryptographic key phase rotated for forward secrecy.
    KeyPhaseRotated {
        new_phase: bool,
    },
    /// Explicit Congestion Notification (ECN CE) received from network routers.
    EcnExperienced {
        ce_count: u32,
    },
    /// Extensible custom hook for future experimental or custom protocol features.
    CustomExtensionEvent {
        extension_id: u16,
        payload: Vec<u8>,
    },
}
