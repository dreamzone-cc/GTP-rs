use gtp_types::{OrderedGroupId, Result, TransportError};
use std::collections::BTreeMap;

pub const DEFAULT_MAX_GROUP_BUFFER_BYTES: usize = 256 * 1024; // 256 KB per group

/// Manages in-order reassembly for an independent ordered stream group.
#[derive(Clone, Debug)]
pub struct OrderedGroupReceiver {
    pub group_id: OrderedGroupId,
    pub next_expected: u32,
    reorder_buffer: BTreeMap<u32, Vec<u8>>,
    max_buffer_bytes: usize,
    current_buffer_bytes: usize,
}

impl OrderedGroupReceiver {
    pub fn new(group_id: OrderedGroupId) -> Self {
        Self {
            group_id,
            next_expected: 0,
            reorder_buffer: BTreeMap::new(),
            max_buffer_bytes: DEFAULT_MAX_GROUP_BUFFER_BYTES,
            current_buffer_bytes: 0,
        }
    }

    pub fn on_incoming(&mut self, order_seq: u32, payload: &[u8]) -> Result<Vec<Vec<u8>>> {
        // If already delivered or predecessor, ignore duplicate
        if order_seq < self.next_expected {
            return Ok(Vec::new());
        }

        let mut ready = Vec::new();

        if order_seq == self.next_expected {
            // Immediate in-order delivery
            ready.push(payload.to_vec());
            self.next_expected += 1;

            // Drain any contiguous buffered items
            while let Some(buffered) = self.reorder_buffer.remove(&self.next_expected) {
                self.current_buffer_bytes =
                    self.current_buffer_bytes.saturating_sub(buffered.len());
                ready.push(buffered);
                self.next_expected += 1;
            }
        } else {
            // Out of order: buffer it
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
}
