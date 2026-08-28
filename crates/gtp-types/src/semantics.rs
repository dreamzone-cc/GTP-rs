use crate::identifiers::{GenerationId, OrderedGroupId, StateKey, StateSequence};
use crate::time::MonotonicTime;

/// The four core GTP message delivery semantics.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum MessageClass {
    /// Low-latency data dropped immediately on loss (e.g. transforms, aim updates).
    Unreliable,
    /// Latest state only per StateKey; drops obsolete predecessors automatically.
    UnreliableSequenced {
        state_key: StateKey,
        sequence: StateSequence,
        generation: GenerationId,
    },
    /// Guaranteed delivery without ordering constraints (prevents Head-of-Line blocking).
    ReliableUnordered,
    /// Guaranteed delivery ordered strictly within a scoped group.
    ReliableOrdered {
        group_id: OrderedGroupId,
        order_seq: u32,
    },
}

/// Priority classification for traffic scheduling and budget reservation.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[repr(u8)]
pub enum PriorityTier {
    /// P0: ACKs, Handshake, Path Challenge/Response, Close (Reserved bandwidth, never starved).
    P0Control = 0,
    /// P1: Critical player inputs and actions.
    P1Input = 1,
    /// P2: Realtime world snapshots and entity states.
    P2WorldState = 2,
    /// P3: Reliable gameplay events and transactions.
    P3ReliableGameplay = 3,
    /// P4: Bulk, cosmetic, non-essential effects (first to be shed under congestion).
    P4BulkCosmetic = 4,
}

impl PriorityTier {
    pub const ALL: [PriorityTier; 5] = [
        PriorityTier::P0Control,
        PriorityTier::P1Input,
        PriorityTier::P2WorldState,
        PriorityTier::P3ReliableGameplay,
        PriorityTier::P4BulkCosmetic,
    ];

    /// Default scheduler weight percentage.
    pub const fn default_weight(self) -> u32 {
        match self {
            PriorityTier::P0Control => 15,
            PriorityTier::P1Input => 35,
            PriorityTier::P2WorldState => 30,
            PriorityTier::P3ReliableGameplay => 15,
            PriorityTier::P4BulkCosmetic => 5,
        }
    }
}

/// Options associated with enqueueing a message into the transport engine.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct MessageOptions {
    pub priority: PriorityTier,
    pub deadline: Option<MonotonicTime>,
    pub supersedable: bool,
}

impl Default for MessageOptions {
    fn default() -> Self {
        Self {
            priority: PriorityTier::P2WorldState,
            deadline: None,
            supersedable: true,
        }
    }
}

impl MessageOptions {
    pub fn with_priority(mut self, priority: PriorityTier) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_deadline(mut self, deadline: MonotonicTime) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn non_supersedable(mut self) -> Self {
        self.supersedable = false;
        self
    }
}
