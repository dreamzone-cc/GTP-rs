/// Anti-amplification defense limiter for unvalidated peers (RFC 9000 3x limit).
#[derive(Clone, Debug, Default)]
pub struct AntiAmplificationLimiter {
    bytes_received: u64,
    bytes_sent: u64,
    validated: bool,
}

impl AntiAmplificationLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_bytes_received(&mut self, bytes: usize) {
        self.bytes_received = self.bytes_received.saturating_add(bytes as u64);
    }

    /// Bytes received from the peer that count toward the 3x budget (A-5).
    ///
    /// Only datagrams the connection actually authenticated are counted: a forged or
    /// undecryptable datagram must not buy an attacker send budget toward an address
    /// that was never validated (RFC 9000 §8.1 — discarded datagrams are not counted).
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    pub fn can_send(&self, bytes: usize) -> bool {
        if self.validated {
            return true;
        }
        let max_allowed = self.bytes_received.saturating_mul(3);
        self.bytes_sent.saturating_add(bytes as u64) <= max_allowed
    }

    pub fn on_bytes_sent(&mut self, bytes: usize) {
        if !self.validated {
            self.bytes_sent = self.bytes_sent.saturating_add(bytes as u64);
        }
    }

    pub fn mark_validated(&mut self) {
        self.validated = true;
    }

    pub fn is_validated(&self) -> bool {
        self.validated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anti_amplification_3x_boundary() {
        let mut limiter = AntiAmplificationLimiter::new();
        limiter.on_bytes_received(100); // 100 bytes received -> max 300 allowed

        assert!(limiter.can_send(300));
        assert!(!limiter.can_send(301));

        limiter.on_bytes_sent(200);
        assert!(limiter.can_send(100));
        assert!(!limiter.can_send(101));

        limiter.mark_validated();
        assert!(limiter.can_send(10_000)); // Uncapped after validation
    }
}
