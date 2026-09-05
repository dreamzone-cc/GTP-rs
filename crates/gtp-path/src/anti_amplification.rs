/// Anti-amplification defense limiter for unvalidated peers (RFC 9000 3x limit).
///
/// The amplification factor defaults to RFC 9000's 3 and is operator-tunable
/// via `GtpConfig::anti_amplification_factor` (A-6). A factor of 0 would
/// deadlock the handshake (nothing could ever be answered before validation),
/// so `with_factor` floors it at 1.
#[derive(Clone, Debug)]
pub struct AntiAmplificationLimiter {
    bytes_received: u64,
    bytes_sent: u64,
    factor: u64,
    validated: bool,
}

impl Default for AntiAmplificationLimiter {
    fn default() -> Self {
        Self {
            bytes_received: 0,
            bytes_sent: 0,
            factor: 3,
            validated: false,
        }
    }
}

impl AntiAmplificationLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// A-6: limiter with the configured amplification factor (floored at 1).
    pub fn with_factor(factor: u64) -> Self {
        Self {
            factor: factor.max(1),
            ..Self::default()
        }
    }

    /// A-6: the active amplification factor (for wiring tests and telemetry).
    pub fn factor(&self) -> u64 {
        self.factor
    }

    /// A-6: re-seed the factor on a limiter whose constructor had no config in
    /// scope (the handshake-driven hot-layer constructors); floored at 1.
    pub fn set_factor(&mut self, factor: u64) {
        self.factor = factor.max(1);
    }

    pub fn on_bytes_received(&mut self, bytes: usize) {
        self.bytes_received = self.bytes_received.saturating_add(bytes as u64);
    }

    /// Bytes received from the peer that count toward the amplification budget (A-5).
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
        let max_allowed = self.bytes_received.saturating_mul(self.factor);
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

    #[test]
    fn configured_factor_is_honored() {
        // A-6: a factor of 10 (as used by the lan_cluster profile) must widen
        // the budget from 3x to 10x — the boundary moves exactly with the config.
        let mut limiter = AntiAmplificationLimiter::with_factor(10);
        limiter.on_bytes_received(100);

        assert!(limiter.can_send(1_000));
        assert!(!limiter.can_send(1_001));
    }

    #[test]
    fn factor_is_floored_at_one() {
        // A factor of 0 must not deadlock the pre-validation exchange.
        let mut limiter = AntiAmplificationLimiter::with_factor(0);
        limiter.on_bytes_received(100);

        assert!(limiter.can_send(100));
        assert!(!limiter.can_send(101));
    }
}
