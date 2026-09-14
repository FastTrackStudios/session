//! The patch list: an album's inputs by performer, resolved through a
//! studio profile.
//!
//! See `Cargo.toml` for what this is; the spec is
//! `docs/spec/session/workflows.md` (`flow.patch-list.*`) and the
//! decision is session #29.

pub mod discover;
pub mod list;
pub mod profile;

pub use discover::{Studios, find_album};
pub use list::{Bus, Entry, Lowered, PatchList, Performer, Rig};
pub use profile::{Resolved, StudioProfile};
