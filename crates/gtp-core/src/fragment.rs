//! Fragment reassembly for oversized reliable messages (CORE-2, F1).
//!
//! The wire `ReliableData` frame has carried `fragment_id`/`total_fragments`
//! since the original specification; this module is the receiver half:
//! bounded, adversarially-safe reassembly below the delivery-semantic layer
//! (ORD-4 dedup and ordered-group handling see only COMPLETE messages).
//!
//! Bounds (all DoS guards — `group_id`s and message ids arrive off the wire):
//! - at most [`MAX_REASSEMBLY_MESSAGES`] concurrently incomplete messages
//!   (LRU eviction of the oldest incomplete);
//! - at most [`MAX_REASSEMBLY_BYTES`] buffered fragment bytes in total
//!   (new fragment BYTES are refused past the cap, never silently trusted);
//! - an incomplete message older than [`REASSEMBLY_TTL`] is swept;
//! - `total_fragments` is capped at 256 (the fragment index is a u8).

use gtp_types::{Duration, FragmentId, MonotonicTime, TransportError};
use rustc_hash::FxHashMap;
use std::collections::VecDeque;

pub const MAX_REASSEMBLY_MESSAGES: usize = 64;
pub const MAX_REASSEMBLY_BYTES: usize = 4 * 1024 * 1024;
pub const REASSEMBLY_TTL: Duration = Duration::from_secs(5);
/// FragmentId is a u16, so a message cannot have more than 65535 fragments.
/// The wire field is u16 — this constant documents the hard ceiling (an
/// index cannot overflow past it, so only `total_fragments == 0` is invalid).
pub const MAX_TOTAL_FRAGMENTS: u16 = u16::MAX;

/// How many fragments a payload of `len` bytes splits into at `budget`
/// bytes per fragment. `Err` mirrors the FR-1 contract for payloads that
/// cannot be represented (empty, or exceeding the fragment-index space).
pub fn message_fragment_count(len: usize, budget: usize) -> gtp_types::Result<u16> {
    let budget = budget.max(1);
    let total = len.div_ceil(budget);
    if total == 0 || total > u16::MAX as usize {
        return Err(TransportError::PayloadTooLarge {
            max: budget * u16::MAX as usize,
        });
    }
    Ok(total as u16)
}

/// What one pushed fragment resulted in.
#[derive(Debug, PartialEq, Eq)]
pub enum PushOutcome {
    /// The final missing fragment arrived: the complete payload.
    Delivered(Vec<u8>),
    /// Stored (or a duplicate that changed nothing); not yet complete.
    Partial,
    /// Refused: orphan, poisoned metadata, out-of-range index, or a bound was
    /// hit. The caller counts the drop.
    Dropped,
}

#[derive(Debug)]
struct Partial {
    total: u16,
    parts: Vec<Option<Vec<u8>>>,
    received: u16,
    bytes: usize,
    last_touch: MonotonicTime,
}

/// Bounded fragment reassembler. All state is private; the only inputs are
/// the wire-supplied identity fields, so a hostile peer cannot grow memory
/// past the two documented caps.
#[derive(Debug, Default)]
pub struct MessageReassembler {
    entries: FxHashMap<u64, Partial>,
    /// Insertion/LRU order of the incomplete message ids.
    order: VecDeque<u64>,
    total_bytes: usize,
}

impl MessageReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of concurrently incomplete messages (test observability).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Buffered fragment bytes (test observability).
    pub fn buffered_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Feeds one fragment. `now` drives the TTL sweep.
    pub fn push(
        &mut self,
        message_id: u64,
        fragment_id: FragmentId,
        total_fragments: u16,
        payload: &[u8],
        now: MonotonicTime,
    ) -> PushOutcome {
        // Structural validation before any state changes: a total outside the
        // representable range is hostile or broken, never trusted.
        if total_fragments == 0 {
            return PushOutcome::Dropped;
        }
        let idx = fragment_id.as_u16();
        if idx >= total_fragments {
            return PushOutcome::Dropped;
        }

        // TTL + LRU housekeeping on every push (bounded: entries ≤ cap).
        self.sweep(now);

        match self.entries.get_mut(&message_id) {
            Some(entry) => {
                // Metadata must be consistent across fragments of one message;
                // a peer changing `total` mid-flight is poisoning, not progress.
                if entry.total != total_fragments {
                    return PushOutcome::Dropped;
                }
                entry.last_touch = now;
                if entry.parts[idx as usize].is_some() {
                    // Duplicate fragment: idempotent no-op.
                    return PushOutcome::Partial;
                }
                if entry.bytes + payload.len() > MAX_REASSEMBLY_BYTES
                    || self.total_bytes + payload.len() > MAX_REASSEMBLY_BYTES
                {
                    return PushOutcome::Dropped;
                }
                entry.parts[idx as usize] = Some(payload.to_vec());
                entry.bytes += payload.len();
                entry.received += 1;
                self.total_bytes += payload.len();
                if entry.received == entry.total {
                    let entry = self.entries.remove(&message_id).expect("present");
                    self.order.retain(|id| *id != message_id);
                    self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
                    let full = entry
                        .parts
                        .into_iter()
                        .flat_map(|slot| slot.expect("received == total guarantees every slot"))
                        .collect();
                    return PushOutcome::Delivered(full);
                }
                PushOutcome::Partial
            }
            None => {
                // New message: only fragment 0 may open one — an orphan index
                // arriving before its head has nothing to attach to.
                if idx != 0 {
                    return PushOutcome::Dropped;
                }
                if payload.len() > MAX_REASSEMBLY_BYTES {
                    return PushOutcome::Dropped;
                }
                // Make room: evict the oldest incomplete message first.
                while self.entries.len() >= MAX_REASSEMBLY_MESSAGES {
                    let Some(&oldest) = self.order.front() else {
                        break;
                    };
                    if let Some(evicted) = self.entries.remove(&oldest) {
                        self.total_bytes = self.total_bytes.saturating_sub(evicted.bytes);
                    }
                    self.order.pop_front();
                }
                if self.total_bytes + payload.len() > MAX_REASSEMBLY_BYTES {
                    return PushOutcome::Dropped;
                }
                let mut parts = vec![None; total_fragments as usize];
                parts[0] = Some(payload.to_vec());
                let bytes = payload.len();
                self.entries.insert(
                    message_id,
                    Partial {
                        total: total_fragments,
                        parts,
                        received: 1,
                        bytes,
                        last_touch: now,
                    },
                );
                self.order.push_back(message_id);
                self.total_bytes += bytes;
                if total_fragments == 1 {
                    // Degenerate "fragmented into one" — deliver immediately.
                    let entry = self.entries.remove(&message_id).expect("present");
                    self.order.retain(|id| *id != message_id);
                    self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
                    let full = entry
                        .parts
                        .into_iter()
                        .flat_map(|slot| slot.expect("slot 0 is set"))
                        .collect();
                    return PushOutcome::Delivered(full);
                }
                PushOutcome::Partial
            }
        }
    }

    /// Drops incomplete messages older than the TTL and fixes the order twin.
    fn sweep(&mut self, now: MonotonicTime) {
        let stale: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, e)| now.duration_since(e.last_touch) > REASSEMBLY_TTL)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(e) = self.entries.remove(&id) {
                self.total_bytes = self.total_bytes.saturating_sub(e.bytes);
            }
            self.order.retain(|x| *x != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(micros: u64) -> MonotonicTime {
        MonotonicTime::from_micros(micros)
    }

    fn full_message(total: u16) -> (MessageReassembler, Vec<u8>) {
        let mut r = MessageReassembler::new();
        let mut assembled = Vec::new();
        for i in 0..total {
            let chunk = vec![i as u8; 100];
            let out = r.push(7, FragmentId(i as u16), total, &chunk, t(1_000_000));
            match out {
                PushOutcome::Delivered(v) => assembled = v,
                PushOutcome::Partial => {}
                PushOutcome::Dropped => panic!("fragment {i} dropped"),
            }
        }
        (r, assembled)
    }

    #[test]
    fn completes_in_any_order_with_duplicates() {
        let mut r = MessageReassembler::new();
        let now = t(1_000_000);
        assert_eq!(
            r.push(1, FragmentId(1), 3, b"bb", now),
            PushOutcome::Dropped,
            "orphan index before fragment 0"
        );
        assert_eq!(
            r.push(1, FragmentId(0), 3, b"aa", now),
            PushOutcome::Partial
        );
        // duplicate fragment 0 is idempotent
        assert_eq!(
            r.push(1, FragmentId(0), 3, b"aa", now),
            PushOutcome::Partial
        );
        assert_eq!(
            r.push(1, FragmentId(2), 3, b"cc", now),
            PushOutcome::Partial
        );
        assert_eq!(
            r.push(1, FragmentId(1), 3, b"bb", now),
            PushOutcome::Delivered(b"aabbcc".to_vec())
        );
        assert!(r.is_empty());
    }

    #[test]
    fn poisoned_total_and_out_of_range_are_dropped() {
        let mut r = MessageReassembler::new();
        let now = t(1_000_000);
        assert_eq!(r.push(2, FragmentId(0), 4, b"a", now), PushOutcome::Partial);
        assert_eq!(
            r.push(2, FragmentId(1), 5, b"b", now),
            PushOutcome::Dropped,
            "total changed mid-message"
        );
        assert_eq!(r.push(2, FragmentId(4), 4, b"b", now), PushOutcome::Dropped);
        assert_eq!(
            r.push(2, FragmentId(0), 0, b"b", now),
            PushOutcome::Dropped,
            "zero total"
        );
        // u16 total: idx past it is still dropped
        assert_eq!(r.push(2, FragmentId(5), 4, b"b", now), PushOutcome::Dropped);
        assert_eq!(r.len(), 1, "the honest entry survives");
    }

    #[test]
    fn message_cap_evicts_oldest_incomplete() {
        let mut r = MessageReassembler::new();
        let now = t(1_000_000);
        for id in 0..MAX_REASSEMBLY_MESSAGES as u64 {
            r.push(id, FragmentId(0), 2, b"x", now);
        }
        assert_eq!(r.len(), MAX_REASSEMBLY_MESSAGES);
        // One more head evicts the oldest (id 0).
        r.push(MAX_REASSEMBLY_MESSAGES as u64, FragmentId(0), 2, b"x", now);
        assert_eq!(r.len(), MAX_REASSEMBLY_MESSAGES);
        // The evicted message's tail is now an orphan → dropped.
        assert_eq!(r.push(0, FragmentId(1), 2, b"y", now), PushOutcome::Dropped);
    }

    #[test]
    fn byte_cap_refuses_new_bytes_and_ttl_sweeps() {
        let mut r = MessageReassembler::new();
        let now = t(1_000_000);
        // Fill nearly the whole byte budget with one message's first fragment.
        let big = vec![0u8; MAX_REASSEMBLY_BYTES - 1024];
        assert_eq!(r.push(9, FragmentId(0), 3, &big, now), PushOutcome::Partial);
        // A second message cannot allocate past the global cap.
        assert_eq!(
            r.push(10, FragmentId(0), 2, &vec![0u8; 2048], now),
            PushOutcome::Dropped
        );
        // TTL sweep frees the stuck message.
        let later = now + REASSEMBLY_TTL + Duration::from_micros(1);
        r.push(11, FragmentId(0), 2, b"z", later);
        assert_eq!(r.len(), 1, "only the fresh message remains");
        assert!(r.buffered_bytes() <= 1);
    }
}
