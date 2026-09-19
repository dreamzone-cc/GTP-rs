use gtp_types::{FragmentId, MessageClass, MessageId, MonotonicTime, PriorityTier};

/// Represents an enqueued message item ready for scheduling and packetization.
#[derive(Clone, Debug)]
pub struct SchedulableItem {
    pub message_id: MessageId,
    pub class: MessageClass,
    pub priority: PriorityTier,
    pub created_at: MonotonicTime,
    pub deadline: Option<MonotonicTime>,
    pub supersedable: bool,
    pub payload: Vec<u8>,
    /// CORE-2 fragmentation: which fragment of the logical message this item
    /// carries, and how many there are (1 = not fragmented).
    pub fragment_id: FragmentId,
    pub total_fragments: u16,
}

impl SchedulableItem {
    pub fn is_expired(&self, now: MonotonicTime) -> bool {
        if let Some(dl) = self.deadline {
            now >= dl
        } else {
            false
        }
    }

    pub fn size_bytes(&self) -> usize {
        self.payload.len()
    }
}
