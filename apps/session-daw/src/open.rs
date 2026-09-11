//! Opening a project, and standing its backend up.
//!
//! Three steps, and the order is the whole point:
//!
//! 1. Parse the `.rpp` into a [`Standalone`] backend and materialize its
//!    media through the media bay.
//! 2. Serve that backend over an in-process memory link and install the
//!    global `daw` facade every panel reads through.
//! 3. Attach the audio engine, so the transport moves audio and not just
//!    a line.
//!
//! None of it happens before there is a window. A real session takes
//! seconds to parse and its media longer, and a blank screen for the
//! duration is a self-inflicted wait — so this runs on a worker thread
//! and the UI waits for the facade to appear
//! (`daw_ui::studio::project::fetch_when_ready`).

use std::path::Path;
use std::sync::{Arc, OnceLock};

use daw::standalone::Standalone;
use daw::standalone::project_loader::load_rpp_text;

/// Keeps the in-process link's acceptor alive for the process's
/// lifetime. Dropping it silently disconnects every panel.
static BUNDLE: OnceLock<daw::standalone::bootstrap::InProcessDaw> = OnceLock::new();

/// The engine runtime. Held because anything that spawns tasks — the
/// audio engine does, on construction — needs to be built inside it, and
/// a plain worker thread has no reactor of its own.
static RUNTIME: OnceLock<&'static tokio::runtime::Runtime> = OnceLock::new();

/// A project that has been parsed and had its backend stood up.
pub struct Opened {
    pub daw: Standalone,
    pub name: String,
    pub project_guid: String,
    pub track_count: usize,
}

/// Parse the project, stand up its backend, install the facade, and
/// start audio. Returns once the panels have something to read.
pub fn open_and_serve(path: &Path) -> eyre::Result<Opened> {
    let opened = parse(path)?;
    bootstrap(&opened.daw)?;
    opened
        .daw
        .set_meters(daw::standalone::metering::Meters::new(opened.track_count));
    attach_audio(&opened);
    Ok(opened)
}

/// Step one: the file becomes a backend.
///
/// Media references resolve against the project's own directory, the way
/// REAPER stores them, and uncompressed PCM is mmap'd rather than
/// decoded. A source that cannot be found is a warning, never a failed
/// open — a session with one missing take is still a session worth
/// looking at.
fn parse(path: &Path) -> eyre::Result<Opened> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| eyre::eyre!("could not read {}: {e}", path.display()))?;
    let daw = Standalone::new();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    daw.media_bay().set_file_resolver(Box::new(
        daw::standalone::media_bay::ProjectRelativeResolver::new(dir),
    ));
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let summary = load_rpp_text(&daw, &name, &path.to_string_lossy(), &text)
        .map_err(|e| eyre::eyre!("{name} did not parse: {e}"))?;

    let audio = daw::standalone::audio_engine::materialize::materialize_via_bay(
        &daw,
        &summary.project_guid,
    )
    .map_err(|e| eyre::eyre!("{name}'s media did not materialize: {e}"))?;
    if !audio.failed.is_empty() {
        tracing::warn!(
            failed = audio.failed.len(),
            loaded = audio.loaded,
            "some sources did not materialize"
        );
    }

    let track_count =
        daw::service::Tracks::all(&daw, daw::service::ProjectContext::Current).len();
    Ok(Opened {
        daw,
        name,
        project_guid: summary.project_guid.clone(),
        track_count,
    })
}

/// Step two: serve the backend and install the facade.
///
/// The runtime is leaked multi-threaded with 16 MiB worker stacks — vox's
/// debug-build channel encode recurses deeply enough to overflow tokio's
/// default 2 MiB, which presents as a stack overflow somewhere entirely
/// unrelated the first time a panel subscribes to anything.
fn bootstrap(standalone: &Standalone) -> eyre::Result<()> {
    if BUNDLE.get().is_some() {
        return Ok(());
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("session-daw")
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()?;
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(rt));
    let _ = RUNTIME.set(rt);

    let bundle = rt.block_on(daw::standalone::bootstrap::build_in_process_daw(
        standalone.clone(),
    ))?;
    // A separate current-thread runtime for `daw::block_on` in sync
    // contexts, so it can never be entered from inside the engine
    // runtime's own workers.
    let block_on_rt = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    );
    daw::init_from_parts(bundle.daw.clone(), block_on_rt);
    BUNDLE
        .set(bundle)
        .map_err(|_| eyre::eyre!("the daw facade was bootstrapped twice"))?;
    Ok(())
}

/// Step three: real playback.
///
/// The engine renders the project graph into the default output and
/// drives the transport clock sample-accurately. Leaked for the
/// process's life — dropping it stops the stream. Failure is not fatal:
/// the soft clock still moves the playhead, silently, which is enough to
/// read a session by.
fn attach_audio(opened: &Opened) {
    let Some(rt) = RUNTIME.get() else {
        return;
    };
    let _guard = rt.enter();
    match opened.daw.attach_audio_engine(&opened.project_guid) {
        Ok(engine) => {
            Box::leak(Box::new(engine));
        }
        Err(e) => tracing::warn!(error = %e, "no audio engine; the transport will run silent"),
    }
}
