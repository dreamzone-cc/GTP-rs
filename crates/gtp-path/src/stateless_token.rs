use gtp_types::{Duration, MonotonicTime};
use hmac::Mac;
use sha2::Sha256;
use std::net::SocketAddr;
use subtle::ConstantTimeEq;

pub const TOKEN_LIFETIME: Duration = Duration::from_secs(10);

type HmacSha256 = hmac::Hmac<Sha256>;

/// Generates and validates stateless cookies for anti-DoS source-address validation.
///
/// SEC-4: the cookie is a real HMAC-SHA256 over the source address and an explicit
/// timestamp — the first 8 bytes carry the timestamp in the clear, the remaining 24
/// bytes are the truncated MAC. Observing cookies reveals nothing about the secret
/// and timestamps cannot be refreshed by an attacker (they are authenticated).
#[derive(Clone)]
pub struct StatelessTokenManager {
    secret: [u8; 32],
}

impl core::fmt::Debug for StatelessTokenManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StatelessTokenManager")
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl StatelessTokenManager {
    pub fn new(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    fn mac(&self, addr: &SocketAddr, ts_bytes: &[u8; 8]) -> [u8; 32] {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(b"GTP-COOKIE-V1");
        mac.update(addr.to_string().as_bytes());
        mac.update(ts_bytes);
        let out = mac.finalize().into_bytes();
        let mut block = [0u8; 32];
        block.copy_from_slice(&out);
        block
    }

    pub fn generate_cookie(&self, addr: SocketAddr, now: MonotonicTime) -> [u8; 32] {
        let ts_bytes = now.as_micros().to_be_bytes();
        let mac_block = self.mac(&addr, &ts_bytes);

        let mut cookie = [0u8; 32];
        cookie[0..8].copy_from_slice(&ts_bytes);
        cookie[8..32].copy_from_slice(&mac_block[..24]);
        cookie
    }

    pub fn verify_cookie(&self, addr: SocketAddr, cookie: &[u8; 32], now: MonotonicTime) -> bool {
        let mut ts_bytes = [0u8; 8];
        ts_bytes.copy_from_slice(&cookie[0..8]);
        let token_time = MonotonicTime::from_micros(u64::from_be_bytes(ts_bytes));

        // Reject future-dated tokens explicitly (duration_since saturates to zero).
        if token_time > now {
            return false;
        }

        if now.duration_since(token_time) > TOKEN_LIFETIME {
            return false; // Expired cookie
        }

        let expected_block = self.mac(&addr, &ts_bytes);
        let expected = &expected_block[..24];
        let candidate = &cookie[8..32];
        candidate.ct_eq(expected).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cookie_verify_rejects_wrong_addr_and_expiry() {
        let mgr = StatelessTokenManager::new([42u8; 32]);
        let addr: SocketAddr = "203.0.113.10:40000".parse().unwrap();
        let now = MonotonicTime::from_micros(1_000_000);

        let cookie = mgr.generate_cookie(addr, now);
        assert!(mgr.verify_cookie(addr, &cookie, now));

        let other: SocketAddr = "203.0.113.10:40001".parse().unwrap();
        assert!(!mgr.verify_cookie(other, &cookie, now));

        let later = now + Duration::from_secs(11);
        assert!(!mgr.verify_cookie(addr, &cookie, later));
    }

    /// SEC-4: refreshing the timestamp of an observed cookie must fail —
    /// the timestamp is covered by the MAC.
    #[test]
    fn cookie_timestamp_refresh_is_rejected() {
        let mgr = StatelessTokenManager::new([7u8; 32]);
        let addr: SocketAddr = "198.51.100.9:51000".parse().unwrap();
        let t0 = MonotonicTime::from_micros(2_000_000);
        let t_late = t0 + Duration::from_secs(9); // within lifetime when stamped at t_late

        let stale_cookie = mgr.generate_cookie(addr, t0);

        // Attacker rewrites the timestamp bytes to a fresh time but cannot re-MAC.
        let mut forged = stale_cookie;
        forged[0..8].copy_from_slice(&t_late.as_micros().to_be_bytes());
        assert!(!mgr.verify_cookie(addr, &forged, t_late));

        // And a legitimately re-issued cookie for the fresh timestamp verifies.
        let fresh = mgr.generate_cookie(addr, t_late);
        assert!(mgr.verify_cookie(addr, &fresh, t_late));
    }

    /// SEC-4: future-dated cookies are rejected outright.
    #[test]
    fn future_dated_cookie_is_rejected() {
        let mgr = StatelessTokenManager::new([9u8; 32]);
        let addr: SocketAddr = "192.0.2.77:53000".parse().unwrap();
        let now = MonotonicTime::from_micros(5_000_000);
        let future = now + Duration::from_secs(120);

        let cookie = mgr.generate_cookie(addr, future);
        assert!(!mgr.verify_cookie(addr, &cookie, now));
    }

    /// SEC-4: observing valid cookies must not enable forging for another address
    /// (the old XOR construction leaked 24/32 secret bytes from a single cookie).
    #[test]
    fn cookie_secret_not_recoverable() {
        let mgr = StatelessTokenManager::new([0x5Au8; 32]);
        let addr: SocketAddr = "203.0.113.50:55000".parse().unwrap();
        let t = MonotonicTime::from_micros(3_000_000);
        let cookie = mgr.generate_cookie(addr, t);

        // With the old construction, cookie[i] for i in 8..32 XOR the known mixing
        // input revealed secret[i]. With HMAC there is no algebraic relation between
        // the cookie bytes and the secret: a different secret must change the cookie
        // completely, and a forged guess must not verify.
        let mgr2 = StatelessTokenManager::new([0xA5u8; 32]);
        let cookie2 = mgr2.generate_cookie(addr, t);
        assert_ne!(&cookie[8..32], &cookie2[8..32]);

        let mut guessed = cookie;
        guessed[8..32].copy_from_slice(&[0u8; 24]);
        assert!(!mgr.verify_cookie(addr, &guessed, t));
    }
}
