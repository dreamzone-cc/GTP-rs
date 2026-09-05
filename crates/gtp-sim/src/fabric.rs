//! Multi-link deterministic simulation fabric (D-1, gate G2 groundwork).
//!
//! `SimulatedNetwork` is a single symmetric pipe: one profile, one queue, one
//! RNG. The adaptive-routing engine needs more than that to be testable at
//! all — per ICD-01 §8.2 (G2) three things are mandatory:
//!
//! 1. **Independent per-direction profiles** on every link — without them the
//!    directional detection of RE-1 (`owd_var` forward vs reverse) cannot be
//!    tested: an asymmetric impairment is invisible to a symmetric simulator.
//! 2. **A time-scripted impairment schedule** — `t=30s: link 0 += 40 ms`
//!    drives the degradation/recovery scenarios of G5 deterministically.
//! 3. **Per-link, per-direction RNG streams** (`splitmix64(master, link, dir)`),
//!    so link *i*'s randomness never depends on how traffic interleaves
//!    across links or directions — the precondition for the determinism
//!    gate: same master seed ⟹ identical event sequence.
//!
//! The fabric records an `event_log` of every transmit outcome (delivered /
//! dropped, per direction) which is exactly what the determinism test
//! compares between two runs.

use crate::impairments::NetworkProfile;
use crate::simulated_network::SimulatedPacket;
use gtp_types::{Duration, MonotonicTime};
use std::collections::BinaryHeap;
use std::net::SocketAddr;

/// Which side of a link a transmission travels.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum FabricDirection {
    /// Endpoint A → endpoint B (the link's `fwd` profile).
    Forward,
    /// Endpoint B → endpoint A (the link's `rev` profile).
    Reverse,
}

/// A candidate path: two endpoints joined by two independent directions.
pub struct SimulatedLink {
    /// A → B impairment profile.
    pub fwd: NetworkProfile,
    /// B → A impairment profile (independent of `fwd` — the point of D-1).
    pub rev: NetworkProfile,
    queue: BinaryHeap<SimulatedPacket>,
    /// One RNG stream per direction, seeded independently so cross-direction
    /// and cross-link scheduling cannot shift either stream.
    rng_fwd: u64,
    rng_rev: u64,
}

/// A scripted change applied to one link at a scheduled virtual time.
#[derive(Clone, Debug)]
pub struct ScriptedImpairment {
    /// Virtual time at which the delta applies.
    pub at: MonotonicTime,
    /// Link index the delta targets.
    pub link: usize,
    /// Which direction(s) the delta applies to.
    pub direction: ImpairDirection,
    pub delta: ProfileDelta,
}

/// Scope of a scripted impairment.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum ImpairDirection {
    Forward,
    Reverse,
    Both,
}

/// Additive impairment change — starts at zero (no-op) and clamps into valid
/// ranges on apply, so scripts compose without validating arithmetic.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProfileDelta {
    pub one_way_delay_add: Duration,
    pub jitter_add: Duration,
    /// Added to the profile's loss rate, clamped to `[0.0, 1.0]`.
    pub loss_rate_add: f64,
}

impl ProfileDelta {
    fn apply(&self, profile: &mut NetworkProfile) {
        profile.one_way_delay = profile.one_way_delay + self.one_way_delay_add;
        profile.jitter = profile.jitter + self.jitter_add;
        profile.loss_rate = (profile.loss_rate + self.loss_rate_add).clamp(0.0, 1.0);
    }
}

/// One recorded transmit outcome — the fabric's determinism ledger.
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum FabricEvent {
    Delivered {
        link: usize,
        direction: FabricDirection,
        deliver_at_us: u64,
        len: usize,
    },
    Dropped {
        link: usize,
        direction: FabricDirection,
        len: usize,
    },
}

/// Multi-link fabric: N independent candidate paths between two endpoints,
/// a time-scripted impairment schedule, and a per-run event log.
pub struct SimulatedFabric {
    links: Vec<SimulatedLink>,
    script: Vec<ScriptedImpairment>,
    /// Index of the next unapplied script entry (entries are time-ordered).
    script_cursor: usize,
    master_seed: u64,
    pub event_log: Vec<FabricEvent>,
}

/// What one transmit decided, produced inside the link borrow so event/packet
/// bookkeeping never overlaps the mutable link access.
enum TxOutcome {
    Dropped,
    Delivered {
        deliver_at: MonotonicTime,
        duplicate: bool,
    },
}

impl SimulatedFabric {
    /// `link_profiles[i]` becomes link *i* (`(fwd, rev)`); every RNG stream is
    /// derived from `master_seed` via splitmix64 so identical seeds yield
    /// identical streams regardless of link count or traffic order.
    pub fn new(master_seed: u64, link_profiles: &[(NetworkProfile, NetworkProfile)]) -> Self {
        let links = link_profiles
            .iter()
            .enumerate()
            .map(|(i, (fwd, rev))| SimulatedLink {
                fwd: fwd.clone(),
                rev: rev.clone(),
                queue: BinaryHeap::new(),
                rng_fwd: splitmix64(master_seed ^ splitmix64((i as u64) * 2)),
                rng_rev: splitmix64(master_seed ^ splitmix64((i as u64) * 2 + 1)),
            })
            .collect();
        Self {
            links,
            script: Vec::new(),
            script_cursor: 0,
            master_seed,
            event_log: Vec::new(),
        }
    }

    pub fn with_script(mut self, script: Vec<ScriptedImpairment>) -> Self {
        // Time-ordered application; stable sort keeps same-time entries in
        // submission order.
        let mut s = script;
        s.sort_by_key(|e| e.at);
        self.script = s;
        self.script_cursor = 0;
        self
    }

    pub fn master_seed(&self) -> u64 {
        self.master_seed
    }

    pub fn link_count(&self) -> usize {
        self.links.len()
    }

    /// Read-only access to a link's current (post-script) profiles.
    pub fn link_profiles(&self, link: usize) -> (&NetworkProfile, &NetworkProfile) {
        let l = &self.links[link];
        (&l.fwd, &l.rev)
    }

    /// Apply every scripted impairment whose time has come at or before
    /// `now`. Idempotent per entry via the cursor.
    pub fn tick_script(&mut self, now: MonotonicTime) {
        while self.script_cursor < self.script.len() && self.script[self.script_cursor].at <= now {
            let entry = self.script[self.script_cursor].clone();
            let link = &mut self.links[entry.link];
            match entry.direction {
                ImpairDirection::Forward => entry.delta.apply(&mut link.fwd),
                ImpairDirection::Reverse => entry.delta.apply(&mut link.rev),
                ImpairDirection::Both => {
                    entry.delta.apply(&mut link.fwd);
                    entry.delta.apply(&mut link.rev);
                }
            }
            self.script_cursor += 1;
        }
    }

    /// Transmit `data` on `link` in `direction`, applying that direction's
    /// profile and consuming only that direction's RNG stream. The
    /// impairment pipeline matches `SimulatedNetwork::transmit` so profile
    /// semantics do not diverge between the two simulators.
    pub fn transmit(
        &mut self,
        link: usize,
        direction: FabricDirection,
        src: SocketAddr,
        dest: SocketAddr,
        data: Vec<u8>,
        now: MonotonicTime,
    ) {
        let outcome = {
            let l = &mut self.links[link];
            let (profile, rng) = match direction {
                FabricDirection::Forward => (&l.fwd, &mut l.rng_fwd),
                FabricDirection::Reverse => (&l.rev, &mut l.rng_rev),
            };
            if next_random_f64(rng) < profile.loss_rate {
                TxOutcome::Dropped
            } else {
                let jitter_factor = (next_random_f64(rng) * 2.0) - 1.0;
                let jitter_micros = (profile.jitter.as_micros() as f64 * jitter_factor) as i64;
                let base_delay_micros = profile.one_way_delay.as_micros() as i64;
                let effective_delay = (base_delay_micros + jitter_micros).max(100) as u64;
                let serialization_micros = ((data.len() as f64
                    / profile.bandwidth_bytes_per_sec as f64)
                    * 1_000_000.0) as u64;
                let deliver_at =
                    now + Duration::from_micros(effective_delay + serialization_micros);
                let deliver_at = if next_random_f64(rng) < profile.reorder_rate {
                    deliver_at + Duration::from_millis(15)
                } else {
                    deliver_at
                };
                let duplicate = next_random_f64(rng) < profile.duplicate_rate;
                TxOutcome::Delivered {
                    deliver_at,
                    duplicate,
                }
            }
        };

        match outcome {
            TxOutcome::Dropped => {
                self.event_log.push(FabricEvent::Dropped {
                    link,
                    direction,
                    len: data.len(),
                });
            }
            TxOutcome::Delivered {
                deliver_at,
                duplicate,
            } => {
                self.event_log.push(FabricEvent::Delivered {
                    link,
                    direction,
                    deliver_at_us: deliver_at.as_micros(),
                    len: data.len(),
                });
                let l = &mut self.links[link];
                l.queue.push(SimulatedPacket {
                    deliver_at,
                    src,
                    dest,
                    data: data.clone(),
                });
                if duplicate {
                    l.queue.push(SimulatedPacket {
                        deliver_at: deliver_at + Duration::from_micros(500),
                        src,
                        dest,
                        data,
                    });
                }
            }
        }
    }

    /// Pop every packet delivered at or before `now` across all links,
    /// ordered by `deliver_at` (stable across links).
    pub fn drain_ready(&mut self, now: MonotonicTime) -> Vec<SimulatedPacket> {
        let mut ready = Vec::new();
        for l in &mut self.links {
            while let Some(top) = l.queue.peek() {
                if top.deliver_at <= now {
                    ready.push(l.queue.pop().unwrap());
                } else {
                    break;
                }
            }
        }
        ready.sort_by(|a, b| a.deliver_at.cmp(&b.deliver_at));
        ready
    }
}

/// Fast deterministic PRNG step (XorShift64) on an explicit state cell —
/// same generator semantics as `SimulatedNetwork`.
fn next_random_f64(state: &mut u64) -> f64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    (x as f64) / (u64::MAX as f64)
}

/// splitmix64 — the per-stream seed derivation (ICD-01 G2: the seed of link
/// *i*, direction *d* is `splitmix64(master ⊕ splitmix64(2i+d))`).
fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "10.0.0.1:5000";
    const B: &str = "10.0.0.2:6000";

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    fn zero_jitter(delay_ms: u64) -> NetworkProfile {
        NetworkProfile {
            one_way_delay: Duration::from_millis(delay_ms),
            jitter: Duration::from_micros(0),
            loss_rate: 0.0,
            duplicate_rate: 0.0,
            reorder_rate: 0.0,
            bandwidth_bytes_per_sec: 1_000_000_000,
        }
    }

    /// G2 gate: two runs with the same master seed produce an identical
    /// event sequence — the determinism contract everything downstream
    /// (shadow-mode scoring, calibration, regression baselines) stands on.
    #[test]
    fn same_seed_same_event_sequence() {
        let run = |seed: u64| -> Vec<FabricEvent> {
            let mut fabric = SimulatedFabric::new(
                seed,
                &[
                    (zero_jitter(20), zero_jitter(25)),
                    (zero_jitter(50), zero_jitter(45)),
                ],
            )
            .with_script(vec![ScriptedImpairment {
                // Mid-run (t0 + 5 s, inside the 10 s driver) so the 10% loss
                // phase actually executes and the log carries seed-dependent
                // drop decisions — without this both seeds see only the
                // zero-jitter deterministic phase and the logs match trivially.
                at: MonotonicTime::from_micros(6_000_000),
                link: 0,
                direction: ImpairDirection::Both,
                delta: ProfileDelta {
                    one_way_delay_add: Duration::from_millis(40),
                    loss_rate_add: 0.10,
                    ..Default::default()
                },
            }]);
            let t0 = MonotonicTime::from_micros(1_000_000);
            for i in 0..500u64 {
                let now = t0 + Duration::from_millis(i * 20);
                fabric.tick_script(now);
                for (link, dir) in [(1, FabricDirection::Reverse), (0, FabricDirection::Forward)] {
                    fabric.transmit(
                        link,
                        dir,
                        addr(A),
                        addr(B),
                        vec![0u8; 100 + (i % 7) as usize],
                        now,
                    );
                }
                let _ = fabric.drain_ready(now + Duration::from_millis(120));
            }
            fabric.event_log.clone()
        };
        let first = run(0xABCD_1234);
        let second = run(0xABCD_1234);
        assert!(!first.is_empty());
        assert_eq!(first, second, "same seed must reproduce the event log");
        // A different seed must differ: the log carries stream-dependent
        // content (drops from the scripted 10% loss after t=30 s).
        let third = run(0xDEAD_BEEF);
        assert_ne!(first, third);
    }

    /// D-1's core requirement: the two directions of one link are impaired
    /// independently — the precondition for testing RE-1's directional
    /// detection (M6) at all.
    #[test]
    fn per_direction_profiles_are_independent() {
        let mut fabric = SimulatedFabric::new(7, &[(zero_jitter(10), zero_jitter(80))]);
        let now = MonotonicTime::from_micros(1_000_000);

        fabric.transmit(
            0,
            FabricDirection::Forward,
            addr(A),
            addr(B),
            vec![0; 50],
            now,
        );
        fabric.transmit(
            0,
            FabricDirection::Reverse,
            addr(B),
            addr(A),
            vec![0; 50],
            now,
        );

        let delivered = fabric.drain_ready(now + Duration::from_millis(200));
        assert_eq!(delivered.len(), 2);
        // Forward arrived at ~+10 ms; reverse only at ~+80 ms.
        let fwd = delivered
            .iter()
            .find(|p| p.dest == addr(B))
            .expect("forward packet delivered");
        let rev = delivered
            .iter()
            .find(|p| p.dest == addr(A))
            .expect("reverse packet delivered");
        assert!(fwd.deliver_at <= now + Duration::from_millis(11));
        assert!(rev.deliver_at >= now + Duration::from_millis(79));
    }

    /// The scripted impairment schedule: a delta applies exactly at its
    /// scheduled time, only to its link and direction.
    #[test]
    fn scripted_impairment_applies_on_time_and_scope() {
        let t0 = MonotonicTime::from_micros(1_000_000);
        let at = t0 + Duration::from_millis(30_000);
        let mut fabric = SimulatedFabric::new(11, &[(zero_jitter(10), zero_jitter(10))])
            .with_script(vec![ScriptedImpairment {
                at,
                link: 0,
                direction: ImpairDirection::Reverse,
                delta: ProfileDelta {
                    one_way_delay_add: Duration::from_millis(40),
                    ..Default::default()
                },
            }]);

        // Before the script time: reverse is still 10 ms.
        fabric.tick_script(t0);
        let (f, r) = fabric.link_profiles(0);
        assert_eq!(r.one_way_delay, f.one_way_delay);

        // After the script time: reverse +40 ms, forward untouched.
        fabric.tick_script(at);
        let (f, r) = fabric.link_profiles(0);
        assert_eq!(f.one_way_delay, Duration::from_millis(10));
        assert_eq!(r.one_way_delay, Duration::from_millis(50));
    }

    /// Script deltas clamp into valid ranges (loss never exceeds 1.0).
    #[test]
    fn loss_delta_clamps_to_one() {
        let t0 = MonotonicTime::from_micros(1_000);
        let mut fabric =
            SimulatedFabric::new(13, &[(zero_jitter(5), zero_jitter(5))]).with_script(vec![
                ScriptedImpairment {
                    at: t0,
                    link: 0,
                    direction: ImpairDirection::Both,
                    delta: ProfileDelta {
                        loss_rate_add: 1.5,
                        ..Default::default()
                    },
                },
            ]);
        fabric.tick_script(t0);
        let (f, _) = fabric.link_profiles(0);
        assert_eq!(f.loss_rate, 1.0);
    }
}
