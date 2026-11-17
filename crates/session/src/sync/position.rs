//! A playhead at a known instant, in a known clock.
//!
//! In daw-transport-sync this was its own struct; here it is the session's
//! [`crate::Transport`] (the same four fields, the same `at` and
//! `shifted`), so the leader's position and transport-as-data are one
//! type.

pub use crate::transport::Transport as Position;
