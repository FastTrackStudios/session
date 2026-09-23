//! Saving and loading a session in the native `.session` format — the
//! engine's own (`daw_standalone::session_file`), re-exported for the app.
//!
//! A `Song.session` is saved beside the `Song.RPP` it was prepared from,
//! and its media paths resolve against that same folder.

pub use daw::standalone::session_file::{load_session, save_session, session_rpp_text};
