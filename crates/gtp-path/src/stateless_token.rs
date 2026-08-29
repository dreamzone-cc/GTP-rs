use gtp_types::{Duration, MonotonicTime};
use std::net::SocketAddr;

pub const TOKEN_LIFETIME: Duration = Duration::from_secs(10);

/// Generates and validates HMAC-like stateless cookies for anti-DoS validation.
#[derive(Clone, Debug)]
pub struct StatelessTokenManager {
    secret: [u8; 32],
}

impl StatelessTokenManager {
    pub fn new(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    pub fn generate_cookie(&self, addr: SocketAddr, now: MonotonicTime) -> [u8; 32] {
        let mut cookie = [0u8; 32];
        let addr_str = addr.to_string();
        let addr_bytes = addr_str.as_bytes();
        let ts_bytes = now.as_micros().to_be_bytes();

        // Lightweight hash mixing secret + addr + timestamp
        for (i, b) in cookie.iter_mut().enumerate() {
            let secret_byte = self.secret[i % 32];
            let addr_byte = addr_bytes
                .get(i % addr_bytes.len().max(1))
                .copied()
                .unwrap_or(0);
            let ts_byte = ts_bytes[i % 8];
            *b = secret_byte ^ addr_byte.wrapping_add(ts_byte).wrapping_add(i as u8);
        }

        // Embed timestamp in the first 8 bytes
        cookie[0..8].copy_from_slice(&ts_bytes);
        cookie
    }

    pub fn verify_cookie(&self, addr: SocketAddr, cookie: &[u8; 32], now: MonotonicTime) -> bool {
        let mut ts_bytes = [0u8; 8];
        ts_bytes.copy_from_slice(&cookie[0..8]);
        let token_time = MonotonicTime::from_micros(u64::from_be_bytes(ts_bytes));

        if now.duration_since(token_time) > TOKEN_LIFETIME {
            return false; // Expired cookie
        }

        let expected = self.generate_cookie(addr, token_time);
        use subtle::ConstantTimeEq;
        cookie.ct_eq(&expected).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stateless_cookie_generation_and_verification() {
        let manager = StatelessTokenManager::new([0x42; 32]);
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let now = MonotonicTime::from_micros(1_000_000);

        let cookie = manager.generate_cookie(addr, now);
        assert!(manager.verify_cookie(addr, &cookie, now + Duration::from_secs(2)));

        // Different address -> rejected
        let other_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
        assert!(!manager.verify_cookie(other_addr, &cookie, now + Duration::from_secs(2)));

        // Expired cookie -> rejected
        assert!(!manager.verify_cookie(addr, &cookie, now + Duration::from_secs(15)));
    }
}
