#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
extern crate std;

pub mod codec;
pub mod frame;
pub mod header;
pub mod varint;

pub use codec::{FrameIterator, PacketBuilder};
pub use frame::{AckRange, Frame, MAX_ACK_RANGES};
pub use header::{HeaderFlags, PacketHeader, GTP_V1_1, MIN_COMMON_HEADER_LEN, MIN_LONG_HEADER_LEN};
pub use varint::VarInt;
