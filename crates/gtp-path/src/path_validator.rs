use std::net::SocketAddr;
use gtp_types::{Duration, MonotonicTime};

pub const PATH_CHALLENGE_TIMEOUT: Duration = Duration::from_secs(3);

/// Validates network paths and manages NAT rebinding via PATH_CHALLENGE/PATH_RESPONSE frames.
#[derive(Clone, Debug)]
pub struct PathValidator {
    active_path: SocketAddr,
    pending_challenge: Option<(SocketAddr, [u8; 8], MonotonicTime)>,
}

impl PathValidator {
    pub fn new(initial_path: SocketAddr) -> Self {
        Self {
            active_path: initial_path,
            pending_challenge: None,
        }
    }

    pub fn active_path(&self) -> SocketAddr {
        self.active_path
    }

    pub fn is_active_path(&self, addr: SocketAddr) -> bool {
        self.active_path == addr
    }

    pub fn start_challenge(&mut self, new_addr: SocketAddr, nonce: [u8; 8], now: MonotonicTime) {
        self.pending_challenge = Some((new_addr, nonce, now));
    }

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
                self.active_path = addr;
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
        let initial_addr: SocketAddr = "192.168.1.100:5000".parse().unwrap();
        let new_addr: SocketAddr = "192.168.1.100:6000".parse().unwrap(); // NAT port changed!
        let mut validator = PathValidator::new(initial_addr);

        assert!(validator.is_active_path(initial_addr));
        assert!(!validator.is_active_path(new_addr));

        let now = MonotonicTime::from_micros(1_000_000);
        let challenge_nonce = [1, 2, 3, 4, 5, 6, 7, 8];
        validator.start_challenge(new_addr, challenge_nonce, now);

        // Incorrect response data -> rejected
        assert!(!validator.validate_response(new_addr, &[0; 8], now + Duration::from_millis(50)));
        assert_eq!(validator.active_path(), initial_addr);

        // Correct response data -> active path switched to new_addr
        assert!(validator.validate_response(new_addr, &challenge_nonce, now + Duration::from_millis(50)));
        assert_eq!(validator.active_path(), new_addr);
        assert!(validator.is_active_path(new_addr));
    }
}
