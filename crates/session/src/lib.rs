//! The session: the primitive everything agrees on.
//!
//! See `docs/design/session.md`.

pub mod time;

#[cfg(feature = "sync")]
pub mod sync;
