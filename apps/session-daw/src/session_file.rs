//! Saving and loading a session in the native `.session` format — the
//! engine's own (`daw_standalone::session_file`), re-exported for the app.
//!
//! A `Song.session` is saved beside the `Song.RPP` it was prepared from,
//! and its media paths resolve against that same folder.

pub use daw::standalone::session_file::{load_session, load_session_history, session_rpp_text};

/// Save `project_guid` as a `.session` at `dir`, with its edit history:
/// the song's live session doc when one is keeping it (every edit, by
/// anyone it was shared with), or else the history already saved there,
/// carried forward rather than started over.
///
/// # Errors
/// As `daw::standalone::session_file::save_session`.
pub fn save_session(
    daw: &daw::standalone::Standalone,
    project_guid: &str,
    dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    #[cfg(feature = "native")]
    let live = crate::collab::history(project_guid);
    #[cfg(not(feature = "native"))]
    let live = None;
    let history = live.or_else(|| load_session_history(dir));
    daw::standalone::session_file::save_session_with_history(daw, project_guid, dir, history.as_ref())
}
