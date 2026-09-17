//! Opening a project, and standing its backend up.
//!
//! Two ways in, and they differ in who owns the project.
//!
//! [`open_and_serve`] **owns** it: a `.rpp` becomes an in-process
//! standalone backend that this window is the only writer of.
//!
//! [`attach_to_reaper`] **borrows** it: REAPER owns the project, the FTS
//! extension publishes its whole service surface on a Unix socket, and
//! this window becomes one more client of it — the same position the
//! CLI and the desktop app already hold. Everything above this module is
//! written against the facade rather than a backend, so only this file
//! knows which of the two it is.
//!
//! The owning path is three steps, and the order is the whole point:
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
    open_with_audio(path, true)
}

/// The same, without opening an audio device.
///
/// For anything that draws a project and exits — a renderer, a
/// screenshot, a fixture. It has nothing to play and no transport to
/// run, and standing up an engine costs it a dependency on a working
/// sound server that it does not otherwise have.
///
/// That is not hypothetical. `attach_audio` is written to treat failure
/// as harmless, and it is — but cpal does not always FAIL when a host
/// is unavailable; sometimes it enumerates, and enumeration can block
/// for minutes on a busy machine. Three render tests timed out at ten
/// minutes each on a box whose audio was working fine and merely busy,
/// which is a long way to travel from "the picture is wrong".
pub fn open_silent(path: &Path) -> eyre::Result<Opened> {
    open_with_audio(path, false)
}

fn open_with_audio(path: &Path, audio: bool) -> eyre::Result<Opened> {
    let opened = parse(path)?;
    bootstrap(&opened.daw)?;
    opened
        .daw
        .set_meters(daw::standalone::metering::Meters::new(opened.track_count));
    if audio {
        attach_audio(&opened);
    }
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

    let track_count = daw::service::Tracks::all(&daw, daw::service::ProjectContext::Current).len();
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
    let rt = engine_runtime()?;

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

/// Build the engine runtime once, or hand back the one there is.
///
/// Leaked, multi-threaded, with 16 MiB worker stacks — vox's debug-build
/// channel encode recurses deeply enough to overflow tokio's default
/// 2 MiB, and it presents as a stack overflow somewhere entirely
/// unrelated the first time a panel subscribes to anything.
///
/// Shared by both ways in, so attaching to REAPER cannot end up on a
/// runtime with different stacks from the one the owning path proved.
fn engine_runtime() -> eyre::Result<&'static tokio::runtime::Runtime> {
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("session-daw")
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()?;
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(rt));
    let _ = RUNTIME.set(rt);
    Ok(rt)
}

/// The engine's runtime, for work that must not run on the event loop.
///
/// `None` until the project has been opened — a window that is still
/// loading has nothing to edit.
#[must_use]
pub fn runtime() -> Option<&'static tokio::runtime::Runtime> {
    RUNTIME.get().copied()
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

// ── Attaching to a live REAPER ───────────────────────────────────────

/// A live project this window is attached to but does not own.
///
/// Deliberately not an [`Opened`]: that one carries the `Standalone` it
/// stood up, and there is no backend here to carry. Handing back an
/// unused `Standalone::new()` to fit the shape would be a lie the
/// compiler would happily keep.
pub struct Attached {
    pub name: String,
    pub project_guid: String,
    pub track_count: usize,
    /// Where REAPER has the project on disk, when it has been saved.
    ///
    /// Carried because the album's patch list is found by walking up
    /// from the project, and an attached window has no path of its own
    /// to walk from. `None` for a project REAPER has never saved, which
    /// is a real state and not an error: it simply has no album yet.
    pub path: Option<std::path::PathBuf>,
}

/// Keeps the vox connection alive for the process's lifetime.
///
/// The same reason as [`BUNDLE`], for a different link: the connection
/// handle owns the lanes, so dropping it disconnects every panel at
/// once — and because the panels read through a global facade rather
/// than through this value, nothing would name the cause.
/// The live connection, replaceable.
///
/// A `OnceLock` here was the same mistake `daw_control`'s global made
/// and fixed: a window attaches to one REAPER for its life only if that
/// REAPER lives as long as the window. Quit REAPER and reopen it and
/// the pid changes, so the socket changes, so the connection has to be
/// re-made — and a cell that can only be written once left every panel
/// holding a dead link with no way to re-point it, and nothing on
/// screen to say so.
///
/// The old connection is dropped when a new one replaces it, which is
/// what actually closes the dead socket.
static CONNECTION: std::sync::RwLock<Option<daw::cli::DawConnection>> =
    std::sync::RwLock::new(None);

/// The socket the last attach used, so a re-attach can repeat it.
///
/// `None` means discovery, which is the common case and the one that
/// must be repeated rather than remembered: the pid in the path is the
/// very thing that changed.
static ATTACHED_TO: std::sync::RwLock<Option<Option<std::path::PathBuf>>> =
    std::sync::RwLock::new(None);

/// Attach to a REAPER that is already running the FTS extension.
///
/// `socket` is the extension's Unix socket; `None` discovers one in
/// `/tmp`. Returns what the live project turned out to be, read back
/// through the facade rather than assumed — a window that reports what
/// it asked for rather than what it got is how a silent
/// wrong-project-attached bug survives.
///
/// # What this deliberately does not do
///
/// No media materialising, no meters, no audio engine. REAPER owns the
/// audio; standing a second engine up beside it would be two things
/// playing the same project.
///
/// # Errors
///
/// When no socket is found, the connection cannot be established, or
/// the facade was already installed by the owning path — the two are
/// mutually exclusive by construction, since there is one global facade
/// and it can only point at one backend.
pub fn attach_to_reaper(socket: Option<std::path::PathBuf>) -> eyre::Result<Attached> {
    if BUNDLE.get().is_some() {
        eyre::bail!("this window already owns a project; it cannot also attach to REAPER");
    }
    let rt = engine_runtime()?;
    let socket_for_retry = socket.clone();
    let connection = rt
        .block_on(daw::cli::connect(socket))
        .map_err(|e| eyre::eyre!("could not attach to REAPER: {e}"))?;

    let block_on_rt = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    );
    daw::init_from_parts(connection.daw.clone(), block_on_rt);
    if let Ok(mut slot) = CONNECTION.write() {
        // Assigning drops whatever was there, which is what closes a
        // socket to a REAPER that has gone.
        *slot = Some(connection);
    }
    if let Ok(mut slot) = ATTACHED_TO.write() {
        *slot = Some(socket_for_retry);
    }

    // Read the project back rather than reporting what we hoped for. A
    // window that prints the project it asked for, instead of the one it
    // got, is how an attached-to-the-wrong-REAPER bug survives a whole
    // session unnoticed.
    let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("the facade did not install"))?;
    let (name, guid, path, track_count) = rt.block_on(async {
        let project = daw.current_project().await?;
        let info = project.info().await?;
        let tracks = project.tracks().all().await?;
        Ok::<_, daw::rpc::Error>((info.name, info.guid, info.path, tracks.len()))
    })?;

    Ok(Attached {
        name,
        project_guid: guid,
        path: Some(path).filter(|p| !p.is_empty()).map(Into::into),
        track_count,
    })
}

// ── which of the two ways in ─────────────────────────────────────────

/// Where this window's project comes from.
///
/// Two, and they are exclusive: the window either owns a project it
/// parsed, or it is one client of a REAPER that owns one. There is a
/// single global facade and it points at one backend, so "both" is not
/// a state this can be in.
pub enum Source {
    /// A `.rpp` this window parses and is the only writer of.
    Own(std::path::PathBuf),
    /// A REAPER already running the FTS extension. `None` discovers its
    /// socket in `/tmp`; `Some` names one, which is what the test
    /// harness needs when several REAPERs are up at once.
    Reaper(Option<std::path::PathBuf>),
}

/// Read the source from the command line and the environment.
///
/// `--reaper` wins over a path, because asking for both is a mistake
/// worth answering rather than a preference worth guessing at — and the
/// live one is the one you would have meant.
pub fn source() -> Option<Source> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(at) = args.iter().position(|a| a == "--reaper") {
        // An optional socket path may follow, so `--reaper /tmp/x.sock`
        // works when discovery would find more than one REAPER.
        let socket = args
            .get(at + 1)
            .filter(|a| !a.starts_with("--"))
            .map(std::path::PathBuf::from);
        return Some(Source::Reaper(socket));
    }
    if std::env::var(REAPER_ENV).is_ok_and(|v| v != "0") {
        return Some(Source::Reaper(
            std::env::var(SOCKET_ENV).ok().map(std::path::PathBuf::from),
        ));
    }
    std::env::args()
        .nth(1)
        .filter(|a| !a.is_empty() && !a.starts_with("--"))
        .or_else(|| std::env::var("SESSION_DAW_PROJECT").ok())
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .map(Source::Own)
}

/// Attach to a live REAPER rather than opening a file.
pub const REAPER_ENV: &str = "SESSION_DAW_REAPER";
/// Which REAPER, when more than one is running.
pub const SOCKET_ENV: &str = "FTS_SOCKET";

/// Attach again, to whatever REAPER is there now.
///
/// Called when the window notices its subscriptions have ended, which
/// is what a quit REAPER looks like from this side. It repeats the
/// discovery rather than the path it found last time, because the pid
/// in that path is exactly what changed.
///
/// # Errors
///
/// When this window owns a project rather than attaching to one, when
/// no REAPER was ever attached to, or when nothing answers — the last
/// of which is the ordinary case while REAPER is starting up, and is a
/// reason to try again rather than a reason to stop.
pub fn reattach() -> eyre::Result<Attached> {
    let socket = ATTACHED_TO
        .read()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or_else(|| eyre::eyre!("this window never attached to a REAPER"))?;
    attach_to_reaper(socket)
}

/// Is this window attached to a REAPER rather than owning a file?
#[must_use]
pub fn is_attached() -> bool {
    ATTACHED_TO.read().is_ok_and(|slot| slot.is_some())
}
