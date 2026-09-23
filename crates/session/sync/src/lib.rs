//! The live session as a Loro CRDT document.
//!
//! Everyone in a session edits the same [`SessionDoc`]. Each replica is
//! the source of truth for its own engine: local edits are read back
//! from the engine as a [`SessionModel`] and reconciled into the doc
//! ([`SessionDoc::write`]); remote edits arrive as Loro updates, and
//! [`diff`] between the model the engine last showed and the doc's new
//! one says what the engine must do. Concurrent edits merge field by
//! field, and the doc keeps the whole history.
//!
//! This crate is transport- and engine-free: no daw, no vox, no iroh.
//! The bridge to daw-standalone and the sync drivers build on it.

pub mod diff;
pub mod doc;
pub mod model;

pub use diff::{Change, ItemField, TrackField, diff};
pub use doc::{ORIGIN_LOCAL, SCHEMA, SessionDoc};
pub use loro;
pub use model::{
    ItemState, MarkerState, RegionState, SessionModel, TakeState, TempoPoint, TrackState,
};
