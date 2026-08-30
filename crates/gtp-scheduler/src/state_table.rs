use gtp_types::{GenerationId, MessageId, StateKey, StateSequence};
use std::collections::{HashMap, VecDeque};

/// Maximum number of distinct `StateKey`s tracked at once.
///
/// R-2: on the receive path the keys are chosen by the peer (`Frame::Data` carries a
/// 48-bit `state_key` straight off the wire), so an unbounded map is a remote memory
/// exhaustion vector — a peer packing ~50 minimal Data frames per datagram would add
/// ~50 entries per packet with nothing ever reclaiming them. The bound also fixes the
/// long-standing SEM-5 growth on the send path.
pub const DEFAULT_STATE_TABLE_CAPACITY: usize = 4096;

/// Tracks latest state generation and sequence per StateKey to manage automatic supersession.
///
/// The table is a bounded FIFO: once `capacity` distinct keys are tracked, the
/// oldest-inserted key is evicted. Eviction only costs freshness (a later stale update
/// for an evicted key is re-admitted once), never correctness or memory.
#[derive(Clone, Debug)]
pub struct StateTable {
    entries: HashMap<u64, (GenerationId, StateSequence, MessageId)>,
    /// Insertion order of the keys currently present in `entries`.
    order: VecDeque<u64>,
    capacity: usize,
}

impl Default for StateTable {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_STATE_TABLE_CAPACITY)
    }
}

impl StateTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            entries: HashMap::with_capacity(capacity.min(1024)),
            order: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
        }
    }

    /// Number of distinct state keys currently tracked.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records a brand-new key, evicting the oldest one when over capacity.
    fn insert_new(&mut self, key: u64, value: (GenerationId, StateSequence, MessageId)) {
        self.entries.insert(key, value);
        self.order.push_back(key);
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    /// Checks if a new state update is newer than currently recorded state.
    pub fn should_admit(&self, key: StateKey, gen: GenerationId, seq: StateSequence) -> bool {
        if let Some(&(curr_gen, curr_seq, _)) = self.entries.get(&key.to_u48()) {
            if gen.is_newer_than(curr_gen) {
                return true;
            }
            if gen == curr_gen && seq.is_newer_than(curr_seq) {
                return true;
            }
            false
        } else {
            true
        }
    }

    /// Records new state. If an older message ID was superseded, returns it for eviction.
    pub fn update(
        &mut self,
        key: StateKey,
        gen: GenerationId,
        seq: StateSequence,
        msg_id: MessageId,
    ) -> Option<MessageId> {
        let key_u48 = key.to_u48();
        if let Some(&(curr_gen, curr_seq, old_msg_id)) = self.entries.get(&key_u48) {
            let is_newer =
                gen.is_newer_than(curr_gen) || (gen == curr_gen && seq.is_newer_than(curr_seq));
            if is_newer {
                // Key already present: refresh in place, insertion order is unchanged.
                self.entries.insert(key_u48, (gen, seq, msg_id));
                Some(old_msg_id)
            } else {
                None
            }
        } else {
            self.insert_new(key_u48, (gen, seq, msg_id));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_table_supersession() {
        let mut table = StateTable::new();
        let key = StateKey::new(10, 1);

        // First update
        assert!(table.should_admit(key, GenerationId(1), StateSequence(1)));
        let superseded = table.update(key, GenerationId(1), StateSequence(1), MessageId(100));
        assert_eq!(superseded, None);

        // Older sequence arrives -> rejected
        assert!(!table.should_admit(key, GenerationId(1), StateSequence(0)));

        // Newer sequence arrives -> admitted, supersedes Msg#100
        assert!(table.should_admit(key, GenerationId(1), StateSequence(2)));
        let superseded2 = table.update(key, GenerationId(1), StateSequence(2), MessageId(101));
        assert_eq!(superseded2, Some(MessageId(100)));

        // Newer generation arrives -> admitted, supersedes Msg#101
        assert!(table.should_admit(key, GenerationId(2), StateSequence(1)));
        let superseded3 = table.update(key, GenerationId(2), StateSequence(1), MessageId(102));
        assert_eq!(superseded3, Some(MessageId(101)));
    }

    #[test]
    fn state_table_is_bounded_under_adversarial_keys() {
        // R-2: on the RX path the key comes straight off the wire, so a peer must
        // not be able to grow this map without limit.
        let mut table = StateTable::with_capacity(64);
        for i in 0..10_000u64 {
            let key = StateKey::from_u48(i);
            table.update(key, GenerationId(1), StateSequence(1), MessageId(i));
        }
        assert!(table.len() <= 64, "state table grew past its capacity");

        // The most recent keys survive; the oldest were evicted.
        let recent = StateKey::from_u48(9_999);
        assert!(!table.should_admit(recent, GenerationId(1), StateSequence(1)));
        let evicted = StateKey::from_u48(0);
        assert!(table.should_admit(evicted, GenerationId(1), StateSequence(1)));
    }
}
