#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
extern crate std;

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
