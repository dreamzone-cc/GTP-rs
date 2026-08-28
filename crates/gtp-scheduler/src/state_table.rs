use std::collections::HashMap;
use gtp_types::{GenerationId, MessageId, StateKey, StateSequence};

/// Tracks latest state generation and sequence per StateKey to manage automatic supersession.
#[derive(Clone, Debug, Default)]
pub struct StateTable {
    entries: HashMap<u64, (GenerationId, StateSequence, MessageId)>,
}

impl StateTable {
    pub fn new() -> Self {
        Self::default()
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
            let is_newer = gen.is_newer_than(curr_gen) || (gen == curr_gen && seq.is_newer_than(curr_seq));
            if is_newer {
                self.entries.insert(key_u48, (gen, seq, msg_id));
                Some(old_msg_id)
            } else {
                None
            }
        } else {
            self.entries.insert(key_u48, (gen, seq, msg_id));
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
}
