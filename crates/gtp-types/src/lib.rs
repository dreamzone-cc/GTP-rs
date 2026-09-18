#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
extern crate std;

/// Largest datagram the transport layer will allocate receive buffers for.
///
/// Single source of truth shared by the I/O batching layer and the wire
/// limits (a hostile datagram larger than this is truncated by the OS and
/// then rejected by AEAD authentication — safe, but every layer should read
/// the same number).
pub const MAX_DATAGRAM_SIZE: usize = 2048;

pub mod error;
pub mod identifiers;
pub mod semantics;
pub mod time;

pub use error::{Result, TransportError};
pub use identifiers::{
    ConnectionId, FragmentId, GenerationId, MessageId, OrderedGroupId, PacketNumber, StateKey,
    StateSequence, TransmissionId,
};
pub use semantics::{MessageClass, MessageOptions, PriorityTier};
pub use time::{Duration, MonotonicTime};
