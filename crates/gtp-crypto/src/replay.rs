use gtp_types::{PacketNumber, Result, TransportError};

pub const REPLAY_WINDOW_SIZE: u64 = 128;

/// Sliding 128-packet bitmap window for replay attack prevention.
#[derive(Clone, Debug, Default)]
pub struct ReplayWindow {
    largest_seen: u64,
    bitmap: [u64; 2], // 128 bits
    initialized: bool,
}

impl ReplayWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks if packet number is new and updates sliding window upon acceptance.
    pub fn check_and_update(&mut self, packet_number: PacketNumber) -> Result<()> {
        let pn = packet_number.as_u64();

        if !self.initialized {
            self.initialized = true;
            self.largest_seen = pn;
            self.bitmap[0] = 1;
            return Ok(());
        }

        if pn > self.largest_seen {
            let shift = pn - self.largest_seen;
            if shift >= REPLAY_WINDOW_SIZE {
                self.bitmap = [0, 0];
            } else {
                self.shift_bitmap(shift as usize);
            }
            self.largest_seen = pn;
            self.set_bit(0);
            Ok(())
        } else {
            let diff = self.largest_seen - pn;
            if diff >= REPLAY_WINDOW_SIZE {
                // Packet too old, outside sliding window
                return Err(TransportError::ReplayDetected);
            }

            if self.is_bit_set(diff as usize) {
                // Duplicate packet number detected!
                return Err(TransportError::ReplayDetected);
            }

            self.set_bit(diff as usize);
            Ok(())
        }
    }

    fn shift_bitmap(&mut self, shift: usize) {
        if shift >= 128 {
            self.bitmap = [0, 0];
        } else if shift >= 64 {
            let s = shift - 64;
            self.bitmap[1] = self.bitmap[0] << s;
            self.bitmap[0] = 0;
        } else {
            self.bitmap[1] = (self.bitmap[1] << shift) | (self.bitmap[0] >> (64 - shift));
            self.bitmap[0] <<= shift;
        }
    }

    fn set_bit(&mut self, index: usize) {
        if index < 64 {
            self.bitmap[0] |= 1 << index;
        } else if index < 128 {
            self.bitmap[1] |= 1 << (index - 64);
        }
    }

    fn is_bit_set(&self, index: usize) -> bool {
        if index < 64 {
            (self.bitmap[0] & (1 << index)) != 0
        } else if index < 128 {
            (self.bitmap[1] & (1 << (index - 64))) != 0
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replay_window_duplicate_and_out_of_order() {
        let mut window = ReplayWindow::new();

        // 1. Initial packet 100
        assert!(window.check_and_update(PacketNumber(100)).is_ok());

        // 2. Duplicate packet 100 -> rejected
        assert!(window.check_and_update(PacketNumber(100)).is_err());

        // 3. Out of order packet 95 -> accepted
        assert!(window.check_and_update(PacketNumber(95)).is_ok());
        // Duplicate 95 -> rejected
        assert!(window.check_and_update(PacketNumber(95)).is_err());

        // 4. Newer packet 150 -> accepted (shifts window)
        assert!(window.check_and_update(PacketNumber(150)).is_ok());
        assert!(window.check_and_update(PacketNumber(140)).is_ok());

        // 5. Very old packet (< 150 - 128) -> rejected
        assert!(window.check_and_update(PacketNumber(10)).is_err());
    }
}
