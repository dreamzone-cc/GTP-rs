use crate::item::SchedulableItem;
use crate::state_table::StateTable;
use gtp_types::{MessageClass, MonotonicTime, PriorityTier, Result, TransportError};
use std::collections::VecDeque;

pub const NUM_PRIORITY_TIERS: usize = 5;

/// N-7: cap on the number of items per tier, independent of payload bytes.
///
/// The byte cap alone cannot bound a flood of zero-payload items (`size_bytes`
/// is payload-only), so each tier also refuses admission past this many items.
pub const DEFAULT_MAX_QUEUE_ITEMS_PER_TIER: usize = 4096;

/// N-3: the DRR service loop covers P1..P4; P0 keeps strict reserved priority.
const DRR_FIRST_TIER: usize = 1;
const DRR_TIER_COUNT: usize = 4;

/// X-19: a tier's deficit never grows beyond this many of its own quanta.
///
/// A tier whose front item is repeatedly unaffordable (budget-constrained, not
/// deficit-constrained) would otherwise accumulate an unbounded burst claim
/// that drains in one go the moment the constraint lifts.
const DEFICIT_CAP_FACTOR: usize = 4;

/// N-3: per-tier DRR quantum in bytes — weight percentage scaled by 100.
const fn tier_quantum(idx: usize) -> usize {
    PriorityTier::ALL[idx].default_weight() as usize * 100
}

/// Multi-tier game traffic scheduler with deadline awareness, supersession, and starvation prevention.
#[derive(Clone, Debug)]
pub struct GameScheduler {
    queues: [VecDeque<SchedulableItem>; NUM_PRIORITY_TIERS],
    deficits: [usize; NUM_PRIORITY_TIERS],
    /// N-3: whether the tier already accrued its quantum since it was last
    /// passed. Accrual happens once per round, not once per visit.
    accrued: [bool; NUM_PRIORITY_TIERS],
    /// N-3: persistent DRR cursor — the tier the next `pop_next` examines
    /// first. Survives across calls; this is what breaks the strict-priority
    /// collapse where every call restarted the scan at P1.
    drr_pointer: usize,
    state_table: StateTable,
    /// FR-7/QoS: byte cap enforced PER TIER, so a latency-critical tier keeps
    /// its own small budget instead of inheriting the largest tier's ceiling.
    max_queue_bytes_per_tier: [usize; NUM_PRIORITY_TIERS],
    max_queue_items_per_tier: usize,
    /// FR-7: O(1) per-tier byte counters, maintained on every queue mutation.
    queue_bytes: [usize; NUM_PRIORITY_TIERS],
    queue_items: [usize; NUM_PRIORITY_TIERS],
}

impl Default for GameScheduler {
    fn default() -> Self {
        Self::new(512 * 1024) // 512 KB per tier limit
    }
}

impl GameScheduler {
    pub fn new(max_queue_bytes_per_tier: usize) -> Self {
        Self::with_item_limit(max_queue_bytes_per_tier, DEFAULT_MAX_QUEUE_ITEMS_PER_TIER)
    }

    /// Per-tier byte caps: each priority tier gets its own budget (mirrors
    /// `GtpConfig::max_queue_bytes_per_tier`), so tier-0's latency-critical
    /// bound is not inflated by a larger tier's ceiling.
    pub fn with_per_tier_byte_caps(caps: [usize; NUM_PRIORITY_TIERS]) -> Self {
        Self::with_per_tier_caps_and_item_limit(caps, DEFAULT_MAX_QUEUE_ITEMS_PER_TIER)
    }

    /// N-7: constructor that also bounds the item count per tier.
    pub fn with_item_limit(
        max_queue_bytes_per_tier: usize,
        max_queue_items_per_tier: usize,
    ) -> Self {
        Self::with_per_tier_caps_and_item_limit(
            [max_queue_bytes_per_tier; NUM_PRIORITY_TIERS],
            max_queue_items_per_tier,
        )
    }

    /// Full constructor: per-tier byte caps plus a per-tier item limit.
    pub fn with_per_tier_caps_and_item_limit(
        max_queue_bytes_per_tier: [usize; NUM_PRIORITY_TIERS],
        max_queue_items_per_tier: usize,
    ) -> Self {
        Self {
            queues: [
                VecDeque::with_capacity(32),
                VecDeque::with_capacity(64),
                VecDeque::with_capacity(64),
                VecDeque::with_capacity(32),
                VecDeque::with_capacity(32),
            ],
            deficits: [0; NUM_PRIORITY_TIERS],
            accrued: [false; NUM_PRIORITY_TIERS],
            drr_pointer: DRR_FIRST_TIER,
            state_table: StateTable::new(),
            max_queue_bytes_per_tier,
            max_queue_items_per_tier,
            queue_bytes: [0; NUM_PRIORITY_TIERS],
            queue_items: [0; NUM_PRIORITY_TIERS],
        }
    }

    pub fn enqueue(&mut self, item: SchedulableItem, now: MonotonicTime) -> Result<()> {
        if item.is_expired(now) {
            return Err(TransportError::MessageExpired);
        }

        let tier_idx = item.priority as usize;

        // Check buffer limits (FR-7: O(1) counters, no per-enqueue tier scan).
        if self.queue_bytes[tier_idx] + item.size_bytes() > self.max_queue_bytes_per_tier[tier_idx]
        {
            return Err(TransportError::ResourceLimitExceeded(
                "Scheduler queue tier capacity reached",
            ));
        }
        // N-7: item-count cap catches zero-payload floods that cost no bytes.
        if self.queue_items[tier_idx] + 1 > self.max_queue_items_per_tier {
            return Err(TransportError::ResourceLimitExceeded(
                "Scheduler queue tier item capacity reached",
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
                let mut bytes_removed = 0;
                let mut items_removed = 0;
                self.queues[tier_idx].retain(|i| {
                    if i.message_id == superseded_id {
                        bytes_removed += i.size_bytes();
                        items_removed += 1;
                        false
                    } else {
                        true
                    }
                });
                self.queue_bytes[tier_idx] =
                    self.queue_bytes[tier_idx].saturating_sub(bytes_removed);
                self.queue_items[tier_idx] =
                    self.queue_items[tier_idx].saturating_sub(items_removed);
            }
        }

        self.queue_bytes[tier_idx] += item.size_bytes();
        self.queue_items[tier_idx] += 1;
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
        if self.queue_bytes[tier_idx] + item.size_bytes() > self.max_queue_bytes_per_tier[tier_idx]
        {
            return Err(TransportError::ResourceLimitExceeded(
                "Scheduler queue tier capacity reached",
            ));
        }
        if self.queue_items[tier_idx] + 1 > self.max_queue_items_per_tier {
            return Err(TransportError::ResourceLimitExceeded(
                "Scheduler queue tier item capacity reached",
            ));
        }

        self.queue_bytes[tier_idx] += item.size_bytes();
        self.queue_items[tier_idx] += 1;
        self.queues[tier_idx].push_front(item);
        Ok(())
    }

    pub fn pop_next(&mut self, send_budget: usize, now: MonotonicTime) -> Option<SchedulableItem> {
        if send_budget == 0 {
            return None;
        }

        // 1. P0 Control has strict reserved priority. Its volume is bounded
        //    upstream (control-queue caps, ACK policy), so strict drain cannot
        //    starve the data tiers in practice.
        while let Some(item) = self.queues[0].pop_front() {
            if item.is_expired(now) {
                self.discard(0, &item);
                continue;
            }
            if item.size_bytes() <= send_budget {
                self.discard(0, &item);
                return Some(item);
            }
            // Doesn't fit in current budget, push back to front
            self.queues[0].push_front(item);
            break;
        }

        // 2. N-3: Deficit Round Robin over P1..P4 with a persistent cursor.
        //
        // Textbook DRR: a tier accrues its quantum exactly once per round
        // (between consecutive passes), keeps serving while its deficit covers
        // the front item, and the cursor stays put across `pop_next` calls.
        // The previous implementation restarted the scan at P1 on every call
        // and returned on the first affordable item, so sustained P1 traffic
        // starved P3/P4 completely (measured 0% under saturation).
        for _ in 0..DRR_TIER_COUNT {
            // Next non-empty tier, scanning cyclically from the cursor.
            let mut idx = None;
            for step in 0..DRR_TIER_COUNT {
                let t =
                    DRR_FIRST_TIER + (self.drr_pointer - DRR_FIRST_TIER + step) % DRR_TIER_COUNT;
                if self.queues[t].is_empty() {
                    self.deficits[t] = 0;
                    self.accrued[t] = false;
                    continue;
                }
                idx = Some(t);
                break;
            }
            let idx = idx?; // every tier empty — nothing to schedule

            if !self.accrued[idx] {
                let quantum = tier_quantum(idx);
                self.deficits[idx] =
                    (self.deficits[idx] + quantum).min(quantum * DEFICIT_CAP_FACTOR);
                self.accrued[idx] = true;
            }

            while let Some(front) = self.queues[idx].pop_front() {
                if front.is_expired(now) {
                    self.discard(idx, &front);
                    continue;
                }

                let size = front.size_bytes();
                if size <= send_budget && self.deficits[idx] >= size {
                    self.deficits[idx] -= size;
                    self.discard(idx, &front);
                    // Stay on this tier: keep serving it while the deficit
                    // still covers the next front item (accrual will not
                    // repeat until the tier has been passed).
                    self.drr_pointer = idx;
                    return Some(front);
                }
                self.queues[idx].push_front(front);
                break;
            }

            // Cannot serve this tier right now — pass it; it re-accrues when
            // the round comes back around.
            self.accrued[idx] = false;
            self.drr_pointer = DRR_FIRST_TIER + (idx - DRR_FIRST_TIER + 1) % DRR_TIER_COUNT;
        }

        None
    }

    /// FR-7 bookkeeping when an item leaves a queue for good (served or expired).
    fn discard(&mut self, tier_idx: usize, item: &SchedulableItem) {
        self.queue_bytes[tier_idx] = self.queue_bytes[tier_idx].saturating_sub(item.size_bytes());
        self.queue_items[tier_idx] = self.queue_items[tier_idx].saturating_sub(1);
    }

    pub fn prune_stale(&mut self, now: MonotonicTime) -> usize {
        let mut pruned = 0;
        for (t, q) in self.queues.iter_mut().enumerate() {
            let mut bytes_removed = 0;
            let before = q.len();
            q.retain(|item| {
                if item.is_expired(now) {
                    bytes_removed += item.size_bytes();
                    false
                } else {
                    true
                }
            });
            pruned += before - q.len();
            self.queue_items[t] = self.queue_items[t].saturating_sub(before - q.len());
            self.queue_bytes[t] = self.queue_bytes[t].saturating_sub(bytes_removed);
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

    /// FR-7: O(1) per-tier byte accounting, maintained incrementally.
    pub fn queue_tier_bytes(&self, tier: PriorityTier) -> usize {
        self.queue_bytes[tier as usize]
    }

    /// N-7: O(1) per-tier item accounting, maintained incrementally.
    pub fn queue_tier_items(&self, tier: PriorityTier) -> usize {
        self.queue_items[tier as usize]
    }

    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtp_types::{Duration, GenerationId, MessageId, StateKey, StateSequence};

    fn item(id: u64, tier: PriorityTier, payload: &[u8]) -> SchedulableItem {
        SchedulableItem {
            message_id: MessageId(id),
            class: MessageClass::Unreliable,
            priority: tier,
            created_at: MonotonicTime::from_micros(1_000_000),
            deadline: None,
            supersedable: true,
            payload: payload.to_vec(),
        }
    }

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

    /// N-3: under continuous P1 saturation the DRR tiers still receive their
    /// weight-proportional shares. Weights are 35/15/5 (denominator 55, X-11),
    /// so provisioning 350/150/50 equal-size items must deliver exactly
    /// 350/150/50 — and, with P1 still saturated at the end, nothing starved.
    #[test]
    fn test_drr_fairness_under_p1_saturation() {
        let mut sched = GameScheduler::new(16 * 1024 * 1024);
        let now = MonotonicTime::from_micros(1_000_000);

        let mut id = 0u64;
        // P1 saturated far beyond its share, P3 and P4 provisioned exactly at theirs.
        for _ in 0..1000 {
            id += 1;
            sched
                .enqueue(item(id, PriorityTier::P1Input, &[7u8; 100]), now)
                .unwrap();
        }
        for _ in 0..150 {
            id += 1;
            sched
                .enqueue(item(id, PriorityTier::P3ReliableGameplay, &[7u8; 100]), now)
                .unwrap();
        }
        for _ in 0..50 {
            id += 1;
            sched
                .enqueue(item(id, PriorityTier::P4BulkCosmetic, &[7u8; 100]), now)
                .unwrap();
        }

        let mut p1 = 0;
        let mut p3 = 0;
        let mut p4 = 0;
        // One DRR round serves 35 P1 + 15 P3 + 5 P4 equal-size items, so after
        // exactly 550 pops (10 rounds) P3 and P4 must be fully delivered —
        // while P1 (provisioned with 1000) still has 650 items waiting. Under
        // the old fixed-order scan P3/P4 stayed at zero until P1 drained.
        for _ in 0..550usize {
            let Some(it) = sched.pop_next(1500, now) else {
                break;
            };
            match it.priority {
                PriorityTier::P1Input => p1 += 1,
                PriorityTier::P3ReliableGameplay => p3 += 1,
                PriorityTier::P4BulkCosmetic => p4 += 1,
                _ => unreachable!("no P0/P2 items were enqueued"),
            }
        }

        assert_eq!(p1, 350, "P1 gets exactly its 35/55 share of the window");
        assert_eq!(p3, 150, "every P3 item is delivered in the window");
        assert_eq!(p4, 50, "every P4 item is delivered in the window");
        assert!(
            !sched.is_empty(),
            "test premise: P1 must still hold items after the window"
        );
        assert_eq!(
            sched.queue_tier_items(PriorityTier::P1Input),
            1000 - 350,
            "the remaining queue is exactly the unserved P1 items"
        );
    }

    /// N-3 regression: the very first P3 item must not wait behind the whole
    /// P1 queue. Under the old fixed-order scan it was pop #1001; under DRR it
    /// lands within the first round (35 P1 pops + 1).
    #[test]
    fn p3_is_served_within_the_first_round_under_p1_saturation() {
        let mut sched = GameScheduler::new(16 * 1024 * 1024);
        let now = MonotonicTime::from_micros(1_000_000);

        for id in 1..=1000 {
            sched
                .enqueue(item(id, PriorityTier::P1Input, &[7u8; 100]), now)
                .unwrap();
        }
        sched
            .enqueue(
                item(1001, PriorityTier::P3ReliableGameplay, &[7u8; 100]),
                now,
            )
            .unwrap();

        let mut p3_at: Option<usize> = None;
        for pop in 1..=100usize {
            if let Some(it) = sched.pop_next(1500, now) {
                if it.priority == PriorityTier::P3ReliableGameplay {
                    p3_at = Some(pop);
                    break;
                }
            }
        }
        assert_eq!(
            p3_at,
            Some(36),
            "P3's first item is served right after P1's first-round burst (35 items)"
        );
    }

    /// FR-7: the O(1) byte/item counters must stay exact across every mutation
    /// path — enqueue, supersession eviction, requeue, pop, prune.
    #[test]
    fn queue_counters_stay_exact_across_all_mutation_paths() {
        let mut sched = GameScheduler::new(64 * 1024);
        let now = MonotonicTime::from_micros(1_000_000);

        sched
            .enqueue(item(1, PriorityTier::P1Input, &[1u8; 100]), now)
            .unwrap();
        sched
            .enqueue(item(2, PriorityTier::P1Input, &[1u8; 200]), now)
            .unwrap();
        assert_eq!(sched.queue_tier_bytes(PriorityTier::P1Input), 300);
        assert_eq!(sched.queue_tier_items(PriorityTier::P1Input), 2);

        // Supersession eviction must shed both bytes and items.
        let key = StateKey::new(7, 0);
        let s1 = SchedulableItem {
            message_id: MessageId(3),
            class: MessageClass::UnreliableSequenced {
                state_key: key,
                sequence: StateSequence(1),
                generation: GenerationId(1),
            },
            priority: PriorityTier::P2WorldState,
            created_at: now,
            deadline: None,
            supersedable: true,
            payload: vec![9u8; 50],
        };
        let mut s2 = s1.clone();
        s2.message_id = MessageId(4);
        s2.class = MessageClass::UnreliableSequenced {
            state_key: key,
            sequence: StateSequence(2),
            generation: GenerationId(1),
        };
        sched.enqueue(s1, now).unwrap();
        sched.enqueue(s2, now).unwrap(); // evicts message 3
        assert_eq!(sched.queue_tier_bytes(PriorityTier::P2WorldState), 50);
        assert_eq!(sched.queue_tier_items(PriorityTier::P2WorldState), 1);

        // Requeue adds back; pop removes.
        let popped = sched.pop_next(1500, now).unwrap();
        assert_eq!(popped.message_id, MessageId(1));
        assert_eq!(sched.queue_tier_bytes(PriorityTier::P1Input), 200);
        sched.requeue(popped, now).unwrap();
        assert_eq!(sched.queue_tier_bytes(PriorityTier::P1Input), 300);

        // Prune must shed the counters of everything it drops.
        let mut expiring = item(9, PriorityTier::P4BulkCosmetic, &[1u8; 80]);
        expiring.deadline = Some(now + Duration::from_millis(10));
        sched.enqueue(expiring, now).unwrap();
        assert_eq!(sched.queue_tier_bytes(PriorityTier::P4BulkCosmetic), 80);
        let later = now + Duration::from_millis(20);
        let pruned = sched.prune_stale(later);
        assert_eq!(pruned, 1);
        assert_eq!(sched.queue_tier_bytes(PriorityTier::P4BulkCosmetic), 0);
        assert_eq!(sched.queue_tier_items(PriorityTier::P4BulkCosmetic), 0);

        // And the counters agree with a full drain of what remains.
        let mut drained_bytes = 0usize;
        while let Some(it) = sched.pop_next(1500, later) {
            drained_bytes += it.size_bytes();
        }
        assert_eq!(drained_bytes, 300 + 50);
    }

    /// N-7: zero-payload items cost no bytes, so only the item cap bounds a
    /// flood of them.
    #[test]
    fn zero_byte_flood_is_bounded_by_item_caps() {
        let mut sched = GameScheduler::with_item_limit(64 * 1024, 8);
        let now = MonotonicTime::from_micros(1_000_000);

        for id in 1..=8u64 {
            sched
                .enqueue(item(id, PriorityTier::P1Input, b""), now)
                .unwrap();
        }
        assert!(matches!(
            sched.enqueue(item(9, PriorityTier::P1Input, b""), now),
            Err(TransportError::ResourceLimitExceeded(_))
        ));
        // Serving frees item slots again.
        assert!(sched.pop_next(1500, now).is_some());
        assert!(sched
            .enqueue(item(10, PriorityTier::P1Input, b""), now)
            .is_ok());
    }
}
