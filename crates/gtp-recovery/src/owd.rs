//! One-way-delay variance estimation from the wire timestamp (RE-1).
//!
//! Every GTP packet carries `timestamp_micros` — the sender's local monotonic
//! clock at TX, inside the AAD so an on-path attacker cannot forge it
//! (INV-10). Consumed here: `d = local_rx_clock − peer_tx_clock` is a constant
//! clock offset plus the one-way delay; the offset cancels against a sliding
//! floor, leaving `owd_var` — the delay *variance above the floor* — plus an
//! RFC 3550 §6.4.1 inter-arrival jitter estimate. Zero bytes added on the
//! wire, zero allocation on the RX path (INV-18; gate G1).

use gtp_types::{Duration, MonotonicTime};

/// How long a floor may serve without being re-anchored (ICD-01 RE-1).
///
/// The floor only ever tracks down, so with two clocks drifting apart the
/// apparent `owd_var` grows with the drift. Crystal drift runs 10–50 ppm
/// (0.6–3 ms per minute); a 30 s re-anchor window bounds the drift error at
/// ≤ 1.5 ms @ 50 ppm — the G1 gate value, pinned by test.
pub const FLOOR_REANCHOR_WINDOW: Duration = Duration::from_micros(30_000_000);

/// A single OWD observation handed to the control plane.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct OwdSample {
    /// One-way delay variance above the sliding floor, in microseconds.
    pub owd_var_us: u32,
    /// RFC 3550 §6.4.1 inter-arrival jitter, in microseconds.
    pub jitter_us: u32,
    /// Measurement epoch (RE-3): constant `0` until path-epoch tagging lands
    /// in G3; samples must never be mixed across epochs (INV-13).
    pub epoch: u8,
}

/// Sliding-floor one-way-delay variance estimator (ICD-01 RE-1).
///
/// `Copy`, integer-only: lives inline in the connection's hot state and never
/// allocates. All arithmetic is wrapping-safe across the `u32` microsecond
/// rollover (every 71.58 minutes): both clocks advance at the same rate, so
/// `d` is stable across the wrap and the signed interpretation of
/// `d − base_d` is exact for every `|delta| < 2^31`.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct OwdEstimator {
    /// `d` at the current floor: `local_rx − peer_tx`, wrapping µs.
    base_d: u32,
    /// Last `owd_var` sample (µs) — the jitter reference point.
    prev_owd_var: u32,
    /// RFC 3550 §6.4.1 jitter (µs).
    jitter: u32,
    /// Virtual/monotonic time the current floor was established.
    floor_time: MonotonicTime,
    /// `false` until the first post-construction sample.
    has_sample: bool,
}

impl Default for OwdEstimator {
    fn default() -> Self {
        Self {
            base_d: 0,
            prev_owd_var: 0,
            jitter: 0,
            floor_time: MonotonicTime::ZERO,
            has_sample: false,
        }
    }
}

impl OwdEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one **authenticated** packet's timestamp and derive the sample.
    ///
    /// INV-3: callers must invoke this only after the datagram authenticated —
    /// a forged timestamp must never move measurement state.
    pub fn on_packet(&mut self, ts_peer: u32, now: MonotonicTime) -> OwdSample {
        let ts_local = now.as_micros() as u32;
        let d = ts_local.wrapping_sub(ts_peer);

        if !self.has_sample {
            // First sample: the floor is the sample; variance is 0 by definition.
            self.has_sample = true;
            self.base_d = d;
            self.floor_time = now;
            self.prev_owd_var = 0;
            return self.sample();
        }

        // Re-anchor an aged floor: without this, sustained positive clock
        // drift (peer slower than us) surfaces as ever-growing `owd_var`.
        if now - self.floor_time > FLOOR_REANCHOR_WINDOW {
            self.base_d = d;
            self.floor_time = now;
        }

        let delta = d.wrapping_sub(self.base_d) as i32;
        if delta < 0 {
            // Queueing delay cannot be negative: a lower `d` is a better floor
            // (or the same offset seen across the u32 wrap edge). Re-anchor.
            self.base_d = d;
            self.floor_time = now;
        }
        let owd_var = delta.max(0) as u32;

        // RFC 3550 §6.4.1: J += (|D(i)| − J) / 16, with truncating integer
        // division and a zero floor so the estimate can never go negative.
        let diff = (owd_var as i64 - self.prev_owd_var as i64).abs();
        let adjustment = (diff - self.jitter as i64) / 16;
        self.jitter = (self.jitter as i64 + adjustment).max(0) as u32;
        self.prev_owd_var = owd_var;

        self.sample()
    }

    /// The current sample without feeding a packet. `owd_var`/`jitter` read 0
    /// until the first packet; use [`OwdEstimator::is_ready`] to distinguish.
    pub fn sample(&self) -> OwdSample {
        OwdSample {
            owd_var_us: self.prev_owd_var,
            jitter_us: self.jitter,
            epoch: 0,
        }
    }

    /// Whether at least one authenticated packet has been fed.
    pub fn is_ready(&self) -> bool {
        self.has_sample
    }

    /// Re-seed on a **validated** path migration: samples from the old path
    /// describe the old path (the X-1 discipline applied to one-way delay;
    /// pre-positions the epoch rule, INV-13).
    pub fn reset_for_new_path(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds a packet that spent exactly `delay_us` on the wire, received at
    /// virtual time `now` — i.e. clocks with zero offset.
    fn feed(est: &mut OwdEstimator, now_us: u64, delay_us: u64) -> OwdSample {
        let ts_peer = (now_us - delay_us) as u32;
        est.on_packet(ts_peer, MonotonicTime::from_micros(now_us))
    }

    #[test]
    fn known_delay_injection_accuracy() {
        // G1 gate: owd_var within ±1 ms of the injected delay change.
        let mut est = OwdEstimator::new();
        let t0 = 10_000_000u64; // 10 s into the virtual clock
        let tick = 16_667u64; // 60 FPS

        // Baseline: constant 20 ms one-way delay — variance is zero.
        for i in 0..50u64 {
            let s = feed(&mut est, t0 + i * tick, 20_000);
            assert_eq!(s.owd_var_us, 0, "constant delay must read as zero variance");
        }

        // The path degrades by exactly +15 ms: owd_var must read 15 ms ± 1 ms.
        for i in 50..80u64 {
            let s = feed(&mut est, t0 + i * tick, 35_000);
            assert!(
                (13_900..=16_100).contains(&s.owd_var_us),
                "injected +15 ms must surface as owd_var ≈ 15 ms, got {} µs",
                s.owd_var_us
            );
        }

        // The path improves below the floor: re-anchors to the new floor,
        // variance returns to zero.
        for i in 80..110u64 {
            let s = feed(&mut est, t0 + i * tick, 18_000);
            assert_eq!(s.owd_var_us, 0, "a new lower floor must reset the variance");
        }
    }

    #[test]
    fn wraparound_edge_no_false_jump() {
        // G1 gate: a timestamp pair crossing the u32 wrap (every 71.58 min)
        // must not produce a false jump. Both clocks are offset-free; the
        // wrap lands in the middle of the sequence.
        let mut est = OwdEstimator::new();
        // Receive times just past the wrap; peer timestamps ~5 ms earlier,
        // crossing u32::MAX in between.
        let start_local: u64 = (u32::MAX as u64) - 4_000; // 4 ms before wrap
        let tick = 1_000u64;
        for i in 0..12u64 {
            let local_us = start_local + i * tick; // crosses u32::MAX
            let ts_peer = ((local_us - 5_000) as u32) as u64; // wraps below
            let s = est.on_packet(ts_peer as u32, MonotonicTime::from_micros(local_us));
            assert_eq!(
                s.owd_var_us, 0,
                "constant 5 ms delay across the u32 wrap must stay zero-variance (packet {})",
                i
            );
            assert_eq!(s.jitter_us, 0);
        }
        assert!(est.is_ready());
    }

    #[test]
    fn clock_drift_bounded_by_reanchor() {
        // G1 gate: 50 ppm peer-clock drift over 40 s (past the 30 s window)
        // must keep the drift-induced error ≤ 1.5 ms.
        //
        // Integer-exact: 50 ppm = 1/20,000, so the lag after `t` µs is
        // exactly `t / 20_000` µs — no float fuzz at the 1500 µs boundary.
        let mut est = OwdEstimator::new();
        let start = 100_000_000u64; // 100 s
        let mut worst: u64 = 0;

        // Packet every 250 ms for 40 s = 160 packets; constant true delay
        // 20 ms, but the peer clock runs slow, so its timestamps lag.
        for i in 0..160u64 {
            let now_us = start + i * 250_000;
            let lag = (now_us - start) / 20_000; // 50 ppm, exact µs
            let ts_peer = (now_us - 20_000 - lag) as u32;
            let s = est.on_packet(ts_peer, MonotonicTime::from_micros(now_us));
            worst = worst.max(s.owd_var_us as u64);
        }
        assert!(
            worst <= 1_500,
            "drift error must stay ≤ 1.5 ms within the 30 s re-anchor window, got {} µs",
            worst
        );
    }

    #[test]
    fn rfc3550_jitter_spot_values() {
        // Exact RFC 3550 §6.4.1 recurrence with gain 1/16 on an alternating
        // 0/1000 µs variance pattern.
        let mut est = OwdEstimator::new();
        let t0 = 5_000_000u64;
        let tick = 16_667u64;

        // First packet establishes the floor at 20 ms.
        feed(&mut est, t0, 20_000);
        // Alternate between 20 ms and 21 ms one-way delay.
        let seq = [21_000u64, 20_000, 21_000];
        let expected_jitter = [62u32, 120, 175];
        for (i, delay) in seq.iter().enumerate() {
            let s = feed(&mut est, t0 + (i as u64 + 1) * tick, *delay);
            assert_eq!(
                s.jitter_us, expected_jitter[i],
                "RFC 3550 recurrence diverged at step {}",
                i
            );
        }
    }

    #[test]
    fn reset_for_new_path_reseeds_completely() {
        let mut est = OwdEstimator::new();
        feed(&mut est, 10_000_000, 20_000);
        feed(&mut est, 10_016_667, 35_000);
        assert!(est.is_ready());
        assert!(est.sample().owd_var_us > 0);

        est.reset_for_new_path();
        assert!(!est.is_ready(), "after a migration reset no sample survives");
        // The first packet on the new path re-seeds the floor: a 60 ms delay
        // reads zero variance, not 40 ms above the old path's floor.
        let s = feed(&mut est, 20_000_000, 60_000);
        assert_eq!(s.owd_var_us, 0);
    }
}
