//! # gtp-route — pure route scoring and selection
//!
//! The deterministic heart of the adaptive-routing engine (ARDP Track B).
//! This crate is **pure**: no tokio, no gtp-core dependency, no clocks, no
//! randomness — every function is a total function over its inputs, so every
//! behaviour is unit-testable with exact expected values and the future
//! `gtp-route-tokio` adapter (B-2) can only *feed* it, never bend it.
//!
//! Scope of this first slice (G3 prelude, per
//! `docs/routing/G2-and-route-proto-design.md`):
//!
//! - [`PathStats`] — per-direction one-way measurements + RTT as they exist
//!   on today's telemetry surface (`DetailedMetrics` / `OwdSample`). The
//!   loss axis is deliberately absent: RT-1 established `loss_ratio()` is a
//!   mixed-unit proxy, and a wrong unit must never drive a decision.
//! - [`score`] — a **continuous, monotone** mapping (B-3 discipline): every
//!   axis normalizes lower-is-better against a documented saturation point;
//!   there are no step cliffs to generate flapping. Weights are starting
//!   points pending the G3 24-hour calibration (ARDP §11.1).
//! - [`confidence`] — the B-9 sample-count factor, applied multiplicatively;
//!   below the floor a candidate may be confirmed but never fast-picked.
//! - [`select`] — chooses among candidates and returns a machine-readable
//!   reason code (the B-10 explainability slice). **Shadow discipline**: the
//!   caller decides what executing a selection means; today the answer is
//!   "print it" (INV-15 — nothing here actuates anything).
//! - [`health`] — a single-path verdict for probe tooling.
//! - [`report`] — the compact `ReliableOrdered` measurement exchange between
//!   the two endpoints (app-layer, no wire change, ARDP §2.3).
//!
//! INV-11 discipline: a [`PathStats`] describes exactly one measured path in
//! one epoch; comparing stats from different scopes is the caller's bug.

pub mod report;
pub mod score;
pub mod select;

pub use report::{MeasurementReport, REPORT_GROUP_ID, REPORT_PREFIX};
pub use score::{confidence, score};
pub use select::{health, select, HealthVerdict, Selection, SelectionReason};

/// One candidate path's measurements, both directions, one epoch.
///
/// Field semantics (who measures what):
/// - `fwd_*` — the direction **A→B** as observed at B's receiver (B's
///   `OwdEstimator` fed by A's wire timestamps).
/// - `rev_*` — the direction **B→A** as observed at A's receiver.
/// - `rtt_us` — round-trip estimate (`smoothed_rtt`) from either endpoint;
///   one number for both directions by nature.
///
/// All fields optional: telemetry starts empty and the scoring layer
/// renormalizes over whatever axes are present rather than inventing zeros
/// (a missing axis is *unknown*, not *excellent*).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathStats {
    /// Caller-assigned candidate identifier (reported back by [`select`]).
    pub path_id: u32,
    /// Forward (A→B) one-way delay variance above floor, µs.
    pub fwd_owd_var_us: Option<u32>,
    /// Forward RFC 3550 inter-arrival jitter, µs.
    pub fwd_jitter_us: Option<u32>,
    /// Reverse (B→A) one-way delay variance above floor, µs.
    pub rev_owd_var_us: Option<u32>,
    /// Reverse RFC 3550 inter-arrival jitter, µs.
    pub rev_jitter_us: Option<u32>,
    /// Smoothed round-trip time, µs.
    pub rtt_us: Option<u32>,
    /// Measurement basis size (received samples behind these aggregates).
    /// Feeds [`confidence`]; 0 reads as "no basis" (B-9).
    pub sample_count: u32,
}

impl PathStats {
    /// A stats record with every axis present — test/tooling convenience.
    pub fn full(
        path_id: u32,
        fwd_owd_var_us: u32,
        fwd_jitter_us: u32,
        rev_owd_var_us: u32,
        rev_jitter_us: u32,
        rtt_us: u32,
        sample_count: u32,
    ) -> Self {
        Self {
            path_id,
            fwd_owd_var_us: Some(fwd_owd_var_us),
            fwd_jitter_us: Some(fwd_jitter_us),
            rev_owd_var_us: Some(rev_owd_var_us),
            rev_jitter_us: Some(rev_jitter_us),
            rtt_us: Some(rtt_us),
            sample_count,
        }
    }
}
