//! # GTP: Game Transport Protocol (GTP/1.1) Rust SDK
//!
//! `gtp` is the primary entry point and high-level SDK for integrating the
//! Game Transport Protocol into game clients, game servers, simulations, and real-time network backends.
//!
//! ## Core Delivery Semantics:
//! - **`Unreliable`**: Fire-and-forget for high-frequency input and transient states.
//! - **`UnreliableSequenced`**: RFC 1982 modulo-safe state updates with automatic predecessor supersession.
//! - **`ReliableUnordered`**: Guaranteed delivery without cross-message Head-of-Line blocking.
//! - **`ReliableOrdered`**: Scoped stream channels (`OrderedGroupId`) preserving strict sequence.
//!
//! ## Quick Start (Synchronous Game Loop):
//! ```rust
//! use gtp::prelude::*;
//! use std::net::SocketAddr;
//!
//! let server_addr: SocketAddr = "127.0.0.1:7777".parse().unwrap();
//! let mut conn = GtpConnection::new_with_config(
//!     ConnectionId(0x1020304050607080),
//!     server_addr,
//!     true, // AEAD protection
//!     GtpConfig::competitive_fps(),
//! );
//!
//! let now = MonotonicTime::now();
//! let _ = conn.send_unreliable(b"player_move_forward".to_vec(), PriorityTier::P1Input, None, now);
//! ```
//!
//! ## Quick Start (Asynchronous Tokio Server):
//! ```rust,ignore
//! use gtp::prelude::*;
//! use std::net::SocketAddr;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let endpoint = GtpEndpoint::bind("0.0.0.0:7777".parse()?).await?;
//!     let client_addr: SocketAddr = "192.168.1.50:5000".parse()?;
//!     let mut conn = endpoint.connect(ConnectionId(0x1122334455667788), client_addr, true).await?;
//!     
//!     tokio::spawn(async move {
//!         while let Some(msg) = conn.recv().await {
//!             println!("Received: {:?}", msg.payload);
//!         }
//!     });
//!     Ok(())
//! }
//! ```

pub use gtp_cc as cc;
pub use gtp_core as core;
pub use gtp_crypto as crypto;
pub use gtp_io as io;
pub use gtp_path as path;
pub use gtp_recovery as recovery;
pub use gtp_scheduler as scheduler;
pub use gtp_types as types;
pub use gtp_wire as wire;

#[cfg(feature = "tokio")]
pub use gtp_runtime_tokio as runtime;

#[cfg(feature = "sim")]
pub use gtp_sim as sim;

pub use gtp_crypto::{
    compute_client_proof, derive_directional_handshake_session_keys, ratchet_key,
    verify_client_proof, EphemeralKeyPair, HandshakeSharedSecret,
};

pub use gtp_core::{
    ConnectionControl, ControlEvent, DetailedMetrics, GtpConfig, GtpConfigBuilder, GtpConnection,
    NetworkFeedback, ReceivedMessage,
};

pub use gtp_types::{
    ConnectionId, Duration, FragmentId, GenerationId, MessageClass, MessageId, MonotonicTime,
    OrderedGroupId, PacketNumber, PriorityTier, Result, StateKey, StateSequence, TransmissionId,
    TransportError,
};

#[cfg(feature = "tokio")]
pub use gtp_runtime_tokio::{AsyncGtpConnection, GtpEndpoint};

#[cfg(feature = "sim")]
pub use gtp_sim::{NetworkProfile, SimulationRunner};

/// Common imports and traits for game engine integration.
pub mod prelude {
    pub use gtp_core::{
        ConnectionControl, ControlEvent, DetailedMetrics, GtpConfig, GtpConfigBuilder,
        GtpConnection, NetworkFeedback, ReceivedMessage,
    };
    pub use gtp_crypto::{
        compute_client_proof, derive_directional_handshake_session_keys, ratchet_key,
        verify_client_proof, EphemeralKeyPair, HandshakeSharedSecret,
    };
    pub use gtp_types::{
        ConnectionId, Duration, FragmentId, GenerationId, MessageClass, MessageId, MonotonicTime,
        OrderedGroupId, PacketNumber, PriorityTier, Result, StateKey, StateSequence,
        TransmissionId, TransportError,
    };

    #[cfg(feature = "tokio")]
    pub use gtp_runtime_tokio::{AsyncGtpConnection, GtpEndpoint};

    #[cfg(feature = "sim")]
    pub use gtp_sim::{NetworkProfile, SimulationRunner};
}

#[cfg(test)]
mod tests {
    use super::prelude::*;
    use std::net::SocketAddr;

    #[test]
    fn test_sdk_facade_roundtrip() {
        let server_addr: SocketAddr = "127.0.0.1:8888".parse().unwrap();
        let mut conn = GtpConnection::new_with_config(
            ConnectionId(0x1111_2222_3333_4444),
            server_addr,
            true,
            GtpConfig::competitive_fps(),
        );

        let now = MonotonicTime::from_micros(1_000_000);
        let msg_id = conn
            .send_unreliable(b"test_payload".to_vec(), PriorityTier::P1Input, None, now)
            .unwrap();

        assert_eq!(msg_id, MessageId(1));
        let metrics = conn.control().query_metrics(now);
        assert_eq!(metrics.total_tx_packets, 0);
    }
}
