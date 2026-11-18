//! The time model: positions, durations, tempo, meters and the tempo map.
//!
//! Copied from `daw-proto` (FastTrackStudios/studio `crates/daw/proto`,
//! `primitives` and `tempo_map::{engine, tempo_point}`) at studio
//! `6019839d0`, with their tests. `daw-proto`'s copy is moving to the
//! `daw` repo; `docs/design/session.md` ("Two copies") says how the two
//! become one.

// Lint debt: a verbatim copy (see the module docs), kept verbatim so the
// two copies reconcile by diff. New code here gets no allowance.
#[allow(
    clippy::pedantic,
    clippy::nursery,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::panic
)]
mod primitives;
// Lint debt: a verbatim copy (see the module docs), kept verbatim so the
// two copies reconcile by diff. New code here gets no allowance.
#[allow(
    clippy::pedantic,
    clippy::nursery,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::panic
)]
mod tempo_map;
// New code: under the crate's lints.
mod tempo_plan;

pub use primitives::*;
pub use tempo_map::*;
pub use tempo_plan::{TempoPlan, TempoSpan};
