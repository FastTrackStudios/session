//! The session's tests, one binary: the time model, the timeline and
//! extension data (always), the Architect document (feature `architect`),
//! following a playhead (feature `sync`).

#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::while_float,
    clippy::suboptimal_flops,
    clippy::panic
)]

mod extensions;
mod time;

#[cfg(feature = "architect")]
mod doc;
#[cfg(feature = "sync")]
mod follow;
#[cfg(feature = "service")]
mod service;

/// Close enough, for f64 arithmetic over a tempo map.
#[track_caller]
pub fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b} (±{eps})");
}
