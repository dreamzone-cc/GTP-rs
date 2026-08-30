use crate::item::SchedulableItem;
use crate::state_table::StateTable;
use gtp_types::{MessageClass, MonotonicTime, PriorityTier, Result, TransportError};
use std::collections::VecDeque;

pub const NUM_PRIORITY_TIERS: usize = 5;

/// Multi-tier game traffic scheduler with deadline awareness, supersession, and starvation prevention.
#[derive(Clone, Debug)]
pub struct GameScheduler {
    queues: [VecDeque<SchedulableItem>; NUM_PRIORITY_TIERS],
    deficits: [usize; NUM_PRIORITY_TIERS],
    state_table: StateTable,
    max_queue_bytes_per_tier: usize,
}

impl Default for GameScheduler {
    fn default() -> Self {
        Self::new(512 * 1024) // 512 KB per tier limit
    }
}

impl GameScheduler {
    pub fn new(max_queue_bytes_per_tier: usize) -> Self {
        Self {
            queues: [
                VecDeque::with_capacity(32),
                VecDeque::with_capacity(64),
                VecDeque::with_capacity(64),
                VecDeque::with_capacity(32),
                VecDeque::with_capacity(32),
            ],
            deficits: [0; NUM_PRIORITY_TIERS],
            state_table: StateTable::new(),
            max_queue_bytes_per_tier,
        }
    }

    pub fn enqueue(&mut self, item: SchedulableItem, now: MonotonicTime) -> Result<()> {
        if item.is_expired(now) {
            return Err(TransportError::MessageExpired);
        }

        let tier_idx = item.priority as usize;

        // Check buffer limits
        let current_tier_bytes: usize = self.queues[tier_idx].iter().map(|i| i.size_bytes()).sum();
        if current_tier_bytes + item.size_bytes() > self.max_queue_bytes_per_tier {
            return Err(TransportError::ResourceLimitExceeded(
                "Scheduler queue tier capacity reached",
            ));
        }

        // Automatic state supersession
        if let MessageClass::UnreliableSequenced {
            state_key,
            sequence,
            generation,
        } = item.class
        {
            if !self
                .state_table
                .should_admit(state_key, generation, sequence)
            {
                // Outdated state packet, drop early
                return Ok(());
            }

            if let Some(superseded_id) =
                self.state_table
                    .update(state_key, generation, sequence, item.message_id)
            {
                // Evict the older superseded message from queue
                self.queues[tier_idx].retain(|i| i.message_id != superseded_id);
            }
        }

        self.queues[tier_idx].push_back(item);
        Ok(())
    }

    /// Re-admits an item that was already popped for transmission but could not be sent.
    ///
    /// R-5: `enqueue` re-runs the supersession gate, and a requeued
    /// `UnreliableSequenced` item carries the exact `(state_key, generation,
    /// sequence)` already recorded in the state table. `should_admit` uses strict
    /// "strictly newer" semantics, so the item would be silently discarded by the
    /// very path meant to preserve it. Requeue skips that gate — the table already
    /// reflects this message — and restores the item at the head of its tier so the
    /// original transmission order is preserved.
    pub fn requeue(&mut self, item: SchedulableItem, now: MonotonicTime) -> Result<()> {
        if item.is_expired(now) {
            return Err(TransportError::MessageExpired);
        }

        let tier_idx = item.priority as usize;
        let current_tier_bytes: usize = self.queues[tier_idx].iter().map(|i| i.size_bytes()).sum();
        if current_tier_bytes + item.size_bytes() > self.max_queue_bytes_per_tier {
            return Err(TransportError::ResourceLimitExceeded(
                "Scheduler queue tier capacity reached",
            ));
        }

        self.queues[tier_idx].push_front(item);
        Ok(())
    }

    pub fn pop_next(&mut self, send_budget: usize, now: MonotonicTime) -> Option<SchedulableItem> {
        if send_budget == 0 {
            return None;
        }

        // 1. P0 Control has strict reserved priority
        while let Some(item) = self.queues[0].pop_front() {
            if item.is_expired(now) {
                continue;
            }
            if item.size_bytes() <= send_budget {
                return Some(item);
            } else {
                // Doesn't fit in current budget, push back to front
                self.queues[0].push_front(item);
                break;
            }
        }

        // 2. Weighted Deficit Round Robin across P1, P2, P3, P4
        let tiers = [
            PriorityTier::P1Input,
            PriorityTier::P2WorldState,
            PriorityTier::P3ReliableGameplay,
            PriorityTier::P4BulkCosmetic,
        ];

        for _ in 0..tiers.len() {
            for &tier in &tiers {
                let idx = tier as usize;
                if self.queues[idx].is_empty() {
                    self.deficits[idx] = 0;
                    continue;
                }

                self.deficits[idx] += tier.default_weight() as usize * 100;

                while let Some(front) = self.queues[idx].pop_front() {
                    if front.is_expired(now) {
                        continue;
                    }

                    let size = front.size_bytes();
                    if size <= send_budget && self.deficits[idx] >= size {
                        self.deficits[idx] -= size;
                        return Some(front);
                    } else {
                        self.queues[idx].push_front(front);
                        break;
                    }
                }
            }
        }

        None
    }

    pub fn prune_stale(&mut self, now: MonotonicTime) -> usize {
        let mut pruned = 0;
        for q in &mut self.queues {
            let before = q.len();
            q.retain(|item| !item.is_expired(now));
            pruned += before - q.len();
        }
        pruned
    }

    pub fn effective_queue_bytes(&self, now: MonotonicTime) -> usize {
        self.queues
            .iter()
            .flat_map(|q| q.iter())
            .filter(|i| !i.is_expired(now))
            .map(|i| i.size_bytes())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtp_types::{Duration, GenerationId, MessageId, StateKey, StateSequence};

    #[test]
    fn test_scheduler_stale_drop_and_deadline() {
        let mut sched = GameScheduler::new(64 * 1024);
        let now = MonotonicTime::from_micros(1_000_000);

        // Enqueue item with 20ms deadline
        let item_expiring_soon = SchedulableItem {
            message_id: MessageId(1),
            class: MessageClass::Unreliable,
            priority: PriorityTier::P1Input,
            created_at: now,
            deadline: Some(now + Duration::from_millis(20)),
            supersedable: true,
            payload: b"input_move".to_vec(),
        };

        sched.enqueue(item_expiring_soon, now).unwrap();
        assert_eq!(sched.effective_queue_bytes(now), 10);

        // Advance time past deadline -> item should be dropped during pop or prune
        let later = now + Duration::from_millis(30);
        assert_eq!(sched.effective_queue_bytes(later), 0);
        let popped = sched.pop_next(1000, later);
        assert!(popped.is_none());
    }

    #[test]
    fn test_scheduler_state_supersession_eviction() {
        let mut sched = GameScheduler::new(64 * 1024);
        let now = MonotonicTime::from_micros(1_000_000);
        let key = StateKey::new(1, 0);

        // Enqueue transform sequence 1
        let state1 = SchedulableItem {
            message_id: MessageId(10),
            class: MessageClass::UnreliableSequenced {
                state_key: key,
                sequence: StateSequence(1),
                generation: GenerationId(1),
            },
            priority: PriorityTier::P2WorldState,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: b"pos_x10".to_vec(),
        };
        sched.enqueue(state1, now).unwrap();

        // Enqueue transform sequence 2 -> should evict sequence 1!
        let state2 = SchedulableItem {
            message_id: MessageId(11),
            class: MessageClass::UnreliableSequenced {
                state_key: key,
                sequence: StateSequence(2),
                generation: GenerationId(1),
            },
            priority: PriorityTier::P2WorldState,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: b"pos_x20".to_vec(),
        };
        sched.enqueue(state2, now).unwrap();

        // Pop should return ONLY sequence 2 (Msg#11)
        let popped = sched.pop_next(1000, now).unwrap();
        assert_eq!(popped.message_id, MessageId(11));
        assert_eq!(popped.payload, b"pos_x20".to_vec());

        // Queue should now be empty
        assert!(sched.pop_next(1000, now).is_none());
    }
}
