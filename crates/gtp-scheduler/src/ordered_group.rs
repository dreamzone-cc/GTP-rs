use gtp_types::{OrderedGroupId, Result, TransportError};
use std::collections::BTreeMap;

pub const DEFAULT_MAX_GROUP_BUFFER_BYTES: usize = 256 * 1024; // 256 KB per group

/// N-7: cap on the number of buffered out-of-order items per group.
///
/// The byte cap alone cannot bound a flood of zero-payload items, so the
/// reorder buffer also refuses past this many entries.
pub const DEFAULT_MAX_GROUP_BUFFER_ITEMS: usize = 1024;

/// Manages in-order reassembly for an independent ordered stream group.
#[derive(Clone, Debug)]
pub struct OrderedGroupReceiver {
    pub group_id: OrderedGroupId,
    pub next_expected: u32,
    reorder_buffer: BTreeMap<u32, Vec<u8>>,
    max_buffer_bytes: usize,
    current_buffer_bytes: usize,
    max_buffer_items: usize,
}

impl OrderedGroupReceiver {
    pub fn new(group_id: OrderedGroupId) -> Self {
        Self {
            group_id,
            next_expected: 0,
            reorder_buffer: BTreeMap::new(),
            max_buffer_bytes: DEFAULT_MAX_GROUP_BUFFER_BYTES,
            current_buffer_bytes: 0,
            max_buffer_items: DEFAULT_MAX_GROUP_BUFFER_ITEMS,
        }
    }

    /// ORD-1: strict "older than" with RFC 1982 serial arithmetic so sequence
    /// wraparound at 2^32 cannot permanently stall the group.
    fn seq_is_stale(seq: u32, next_expected: u32) -> bool {
        let diff = seq.wrapping_sub(next_expected);
        // diff in the upper half of the space means "behind" next_expected
        diff >= 0x8000_0000 && diff != 0
    }

    pub fn on_incoming(&mut self, order_seq: u32, payload: &[u8]) -> Result<Vec<Vec<u8>>> {
        // If already delivered or predecessor, ignore duplicate (wrap-safe)
        if Self::seq_is_stale(order_seq, self.next_expected) {
            return Ok(Vec::new());
        }

        let mut ready = Vec::new();

        if order_seq == self.next_expected {
            // Immediate in-order delivery
            ready.push(payload.to_vec());
            self.next_expected = self.next_expected.wrapping_add(1);

            // Drain any contiguous buffered items
            while let Some(buffered) = self.reorder_buffer.remove(&self.next_expected) {
                self.current_buffer_bytes =
                    self.current_buffer_bytes.saturating_sub(buffered.len());
                ready.push(buffered);
                self.next_expected = self.next_expected.wrapping_add(1);
            }
        } else {
            // Out of order: buffer it
            // N-7: the item cap catches zero-payload floods that cost no bytes.
            if self.reorder_buffer.len() + 1 > self.max_buffer_items {
                return Err(TransportError::ResourceLimitExceeded(
                    "Ordered group buffer item capacity exceeded",
                ));
            }
            if self.current_buffer_bytes + payload.len() > self.max_buffer_bytes {
                return Err(TransportError::ResourceLimitExceeded(
                    "Ordered group buffer capacity exceeded",
                ));
            }

            if let std::collections::btree_map::Entry::Vacant(e) =
                self.reorder_buffer.entry(order_seq)
            {
                self.current_buffer_bytes += payload.len();
                e.insert(payload.to_vec());
            }
        }

        Ok(ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordered_group_in_order_and_out_of_order() {
        let mut group = OrderedGroupReceiver::new(OrderedGroupId(1));

        // 1. Packet 0 arrives in order
        let ready0 = group.on_incoming(0, b"seq_0").unwrap();
        assert_eq!(ready0, vec![b"seq_0".to_vec()]);
        assert_eq!(group.next_expected, 1);

        // 2. Packet 2 arrives out of order -> buffered
        let ready2 = group.on_incoming(2, b"seq_2").unwrap();
        assert!(ready2.is_empty());
        assert_eq!(group.next_expected, 1);

        // 3. Packet 3 arrives out of order -> buffered
        let ready3 = group.on_incoming(3, b"seq_3").unwrap();
        assert!(ready3.is_empty());

        // 4. Missing Packet 1 arrives -> delivers 1, 2, 3 in order!
        let ready1 = group.on_incoming(1, b"seq_1").unwrap();
        assert_eq!(
            ready1,
            vec![b"seq_1".to_vec(), b"seq_2".to_vec(), b"seq_3".to_vec()]
        );
        assert_eq!(group.next_expected, 4);
    }

    /// ORD-1: u32 sequence wraparound must keep the group flowing.
    #[test]
    fn order_seq_wraparound_still_delivers() {
        let mut group = OrderedGroupReceiver::new(OrderedGroupId(9));
        // Fast-forward next_expected near the u32 boundary via in-order delivery
        let near_max = u32::MAX - 1;
        group.next_expected = near_max;

        assert_eq!(
            group.on_incoming(near_max, b"seq_max-1").unwrap(),
            vec![b"seq_max-1".to_vec()]
        );
        assert_eq!(group.next_expected, u32::MAX);
        assert_eq!(
            group.on_incoming(u32::MAX, b"seq_max").unwrap(),
            vec![b"seq_max".to_vec()]
        );
        assert_eq!(group.next_expected, 0); // wrapped
        assert_eq!(
            group.on_incoming(0, b"seq_0").unwrap(),
            vec![b"seq_0".to_vec()]
        );
        assert_eq!(group.next_expected, 1);

        // A pre-wrap duplicate is now recognized as stale (not buffered forever)
        assert!(group.on_incoming(u32::MAX, b"dup").unwrap().is_empty());
    }

    /// N-7: zero-payload out-of-order items cost no bytes, so only the item
    /// cap bounds a flood of them.
    #[test]
    fn zero_byte_flood_is_bounded_by_the_item_cap() {
        let mut group = OrderedGroupReceiver::new(OrderedGroupId(4));
        group.next_expected = 1;

        // Hole at seq 1 keeps everything out of order and buffered.
        // Seqs 2..=1025 fill the cap exactly; seq 1026 must be refused.
        for seq in 2..=(DEFAULT_MAX_GROUP_BUFFER_ITEMS as u32 + 2) {
            let res = group.on_incoming(seq, b"");
            if seq <= DEFAULT_MAX_GROUP_BUFFER_ITEMS as u32 + 1 {
                assert!(res.is_ok(), "item {} fits within the cap", seq);
            } else {
                assert!(
                    matches!(res, Err(TransportError::ResourceLimitExceeded(_))),
                    "item {} must be refused past the cap",
                    seq
                );
                break;
            }
        }

        // Filling the hole drains the buffer and frees item slots again.
        assert!(!group.on_incoming(1, b"head").unwrap().is_empty());
        assert!(group
            .on_incoming(DEFAULT_MAX_GROUP_BUFFER_ITEMS as u32 + 3, b"")
            .is_ok());
    }
}
