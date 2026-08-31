use gtp_types::{Duration, MonotonicTime};
use std::net::SocketAddr;

pub const PATH_CHALLENGE_TIMEOUT: Duration = Duration::from_secs(3);

/// Validates network paths and manages NAT rebinding via PATH_CHALLENGE/PATH_RESPONSE frames.
///
/// FR-4: the validator does NOT track the active path. The active path is owned solely
/// by the connection (`ConnectionHot::active_path`); the validator only verifies a
/// challenge/response and reports success, and the connection performs the migration.
/// This keeps a single source of truth for the active address (the validator previously
/// held a duplicate copy that nothing read and that could silently diverge).
#[derive(Clone, Debug, Default)]
pub struct PathValidator {
    pending_challenge: Option<(SocketAddr, [u8; 8], MonotonicTime)>,
}

impl PathValidator {
    pub fn new() -> Self {
        Self {
            pending_challenge: None,
        }
    }

    pub fn start_challenge(&mut self, new_addr: SocketAddr, nonce: [u8; 8], now: MonotonicTime) {
        self.pending_challenge = Some((new_addr, nonce, now));
    }

    /// Verifies a PATH_RESPONSE against the pending challenge. Returns `true` when the
    /// response is valid (correct source, matching nonce, within the timeout); the
    /// caller then migrates its own active path. The pending challenge is cleared on a
    /// successful match so a nonce cannot be replayed.
    pub fn validate_response(
        &mut self,
        addr: SocketAddr,
        response_data: &[u8; 8],
        now: MonotonicTime,
    ) -> bool {
        if let Some((pending_addr, expected_nonce, start_time)) = self.pending_challenge {
            if addr == pending_addr
                && response_data == &expected_nonce
                && now.duration_since(start_time) <= PATH_CHALLENGE_TIMEOUT
            {
                self.pending_challenge = None;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_validation_and_nat_rebinding() {
        let new_addr: SocketAddr = "192.168.1.100:6000".parse().unwrap(); // NAT port changed!
        let mut validator = PathValidator::new();

        let now = MonotonicTime::from_micros(1_000_000);
        let challenge_nonce = [1, 2, 3, 4, 5, 6, 7, 8];
        validator.start_challenge(new_addr, challenge_nonce, now);

        // Wrong nonce -> rejected, and the challenge stays pending for a real response.
        assert!(!validator.validate_response(new_addr, &[0; 8], now + Duration::from_millis(50)));

        // Wrong source address -> rejected even with the correct nonce.
        let attacker: SocketAddr = "10.0.0.9:6000".parse().unwrap();
        assert!(!validator.validate_response(attacker, &challenge_nonce, now));

        // Correct source + nonce within the timeout -> validated (the caller migrates).
        assert!(validator.validate_response(
            new_addr,
            &challenge_nonce,
            now + Duration::from_millis(50)
        ));

        // The pending challenge was consumed: the same nonce cannot be replayed.
        assert!(!validator.validate_response(new_addr, &challenge_nonce, now));
    }
}
