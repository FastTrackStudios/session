//! `--engine --project PATH` / `--setlist PATH`: what the headless engine
//! plays before any remote has connected.
//!
//! A setlist goes through [`SessionEngine::load_setlist`] — exactly what the
//! Home page's "Load & Play" does, so each song's audio is decoded lazily by
//! the engine's audio thread. A single project is loaded here instead: its
//! structure into the engine's `Standalone`, its audio decoded up front
//! (there is only one song to hold), then made current so the audio thread
//! attaches the output device to it. Either way the setlist is rebuilt, so
//! the browser remote sees what the daw facade sees.

use std::path::Path;

use daw_standalone::audio_engine::materialize::materialize_via_bay;
use daw_standalone::media_bay::ProjectRelativeResolver;
use daw_standalone::project_loader::load_rpp_text;

use crate::engine_server::OpenTarget;
use crate::session_engine::SessionEngine;

/// Open `target` into the Live Mode engine.
///
/// # Errors
///
/// The project could not be read or parsed, or the setlist resolved to no
/// songs. Individual songs of a setlist that fail are skipped (logged by
/// `load_setlist`), as on the Home page.
pub async fn open(engine: &'static SessionEngine, target: &OpenTarget) -> eyre::Result<()> {
    match target {
        OpenTarget::Project(path) => {
            let path = path.clone();
            // Parsing and decoding are blocking, and can take seconds.
            tokio::task::spawn_blocking(move || open_project(engine, &path))
                .await
                .map_err(|e| eyre::eyre!("project load task: {e}"))??;
            // A project with no song markers builds an empty setlist; the
            // daw facade still serves it, so that is not a failure.
            if let Err(e) = engine.client.build_from_open_projects().await {
                tracing::warn!(error = ?e, "--engine: the project did not build a setlist");
            }
            Ok(())
        }
        OpenTarget::Setlist(path) => {
            let songs = setlist_songs(path)?;
            let reports = engine.load_setlist(songs).await?;
            tracing::info!(
                engine.songs = reports.len(),
                engine.tracks = reports.iter().map(|r| r.track_count).sum::<usize>(),
                "--engine: setlist loaded"
            );
            Ok(())
        }
    }
}

/// Load one project's structure and audio, and make it current.
fn open_project(engine: &SessionEngine, path: &Path) -> eyre::Result<()> {
    let standalone = &engine.standalone;
    let is_session = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("session"));
    let text = if is_session {
        daw_standalone::session_file::session_rpp_text(path).map_err(|e| eyre::eyre!(e))?
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| eyre::eyre!("could not read {}: {e}", path.display()))?
    };
    // Media resolves against the folder the project sits in — for a
    // `.session`, the folder of the `.rpp` it was prepared from.
    let media_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Project".to_string());

    let loaded = load_rpp_text(standalone, &name, &path.to_string_lossy(), &text)
        .map_err(|e| eyre::eyre!("loading {}: {e}", path.display()))?;
    standalone
        .media_bay()
        .set_file_resolver(Box::new(ProjectRelativeResolver::new(media_dir)));
    let audio = materialize_via_bay(standalone, &loaded.project_guid)
        .map_err(|e| eyre::eyre!("decoding {}: {e}", path.display()))?;
    // Current only once decoded: the audio thread attaches the device to
    // whatever is current, and must find the audio already there.
    standalone.set_current_project(&loaded.project_guid);
    tracing::info!(
        engine.tracks = loaded.track_count,
        engine.items = loaded.item_count,
        engine.audio_sources = audio.loaded,
        engine.audio_failed = audio.failed.len(),
        "--engine: project loaded"
    );
    Ok(())
}

/// The songs a `--setlist` path names: a setlist note resolved against the
/// track libraries, or every song in a folder of song folders.
fn setlist_songs(path: &Path) -> eyre::Result<Vec<session_vault_sync::library::LibrarySong>> {
    let songs = if path.is_dir() {
        session_vault_sync::library::scan(path)
    } else {
        let libraries = crate::setlist_library::scan_libraries();
        let (songs, unresolved) = crate::setlist_library::read_setlist_songs(path, &libraries)
            .map_err(|e| eyre::eyre!("could not read {}: {e}", path.display()))?;
        if !unresolved.is_empty() {
            tracing::warn!(
                engine.unresolved = unresolved.len(),
                "--engine: setlist songs not found in any track library"
            );
        }
        songs
    };
    if songs.is_empty() {
        eyre::bail!("{} names no songs", path.display());
    }
    Ok(songs)
}
