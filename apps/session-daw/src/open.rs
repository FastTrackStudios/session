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

use std::path::{Path, PathBuf};
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
    let opened = load(path)?;
    bootstrap(&opened.daw)?;
    AUDIBLE.store(audio, std::sync::atomic::Ordering::Relaxed);
    switch_to(&opened.daw, &opened.project_guid, audio);
    Ok(opened)
}

/// Whether this window plays — set by the first open (a screenshot or a
/// test opens silent), and what [`switch_song`] follows.
static AUDIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The process's engine: every song this window opens lives in it, side by
/// side, one of them current — a setlist is several projects in one
/// `Standalone`, not several engines. Made by the first open.
static ENGINE: OnceLock<Standalone> = OnceLock::new();

fn engine() -> &'static Standalone {
    ENGINE.get_or_init(|| {
        let daw = Standalone::new();
        // The instrument the Click / Count / Guide MIDI tracks play through.
        crate::guide_instrument::install(
            &daw,
            crate::guide_instrument::Library::Folder(crate::guide_instrument::samples_dir()),
        );
        daw
    })
}

/// Load another song into the engine the first open made — for a
/// setlist, after the first song. It does not become current and nothing
/// plays it until [`switch_to`] says so.
///
/// # Errors
///
/// The file could not be read or parsed, or its media did not materialize.
pub fn open_another(path: &Path) -> eyre::Result<Opened> {
    load(path)
}

/// Make `project_guid` the song the window shows and the transport plays:
/// the engine's current project, its meters, and — with `audio` — the
/// audio engine, moved from whichever song had it.
///
/// Moving the audio engine closes the device and opens it again on the
/// new song, a short gap. Between songs that is fine; a set that runs
/// straight on without one wants the engine to hand the stream over
/// instead, which daw-standalone does not do yet.
pub fn switch_to(daw: &Standalone, project_guid: &str, audio: bool) {
    daw.set_current_project(project_guid);
    let tracks = daw::service::Tracks::all(daw, daw::service::ProjectContext::Current).len();
    daw.set_meters(daw::standalone::metering::Meters::new(tracks));
    if audio {
        attach_audio(daw, project_guid);
    }
}

/// [`switch_to`] on the process's engine — what picking a setlist tab
/// does. The audio moves with it when the window plays at all.
pub fn switch_song(project_guid: &str) {
    switch_to(engine(), project_guid, AUDIBLE.load(std::sync::atomic::Ordering::Relaxed));
}

/// The project the engine has current — the song on screen.
#[must_use]
pub fn current_song() -> Option<String> {
    use daw::service::Projects as _;
    engine().current().map(|p| p.guid)
}

/// Colour a song's SONG region (`0xRRGGBB`; `0` clears it) and save the
/// song into `saved`, its `.session`, when it has one. A save that fails
/// is logged — the colour still holds for this run.
pub fn set_song_color(project_guid: &str, rgb: u32, saved: Option<&Path>) {
    use daw::service::Regions as _;
    let daw = engine();
    let project = daw::service::ProjectContext::Project(project_guid.to_owned());
    let song_lane = session::ruler_lanes::CoreLane::Song.lane_index();
    let region = daw
        .all(project.clone())
        .into_iter()
        .find(|r| r.lane == Some(song_lane));
    let Some(id) = region.and_then(|r| r.id) else {
        tracing::warn!(song.project = project_guid, "no SONG region to colour");
        return;
    };
    if let Err(e) = daw.set_color(project, id, rgb) {
        tracing::warn!(error = %e, "the song's colour could not be set");
        return;
    }
    if let Some(dir) = saved
        && let Err(e) = crate::session_file::save_session(daw, project_guid, dir)
    {
        tracing::warn!(session.save_error = %e, "the song's colour could not be saved");
    }
}

/// Step one: the file becomes a project in the engine.
///
/// Media references are anchored to the project's own folder (see
/// `project_loader::anchor_media` — every song has its own
/// `Media/Click.wav`), and uncompressed PCM is mmap'd rather than
/// decoded. A source that cannot be found is a warning, never a failed
/// open — a session with one missing take is still a session worth
/// looking at.
fn load(path: &Path) -> eyre::Result<Opened> {
    // Absolute from here on: the project's path is what a later save
    // writes its media relative to, and a path relative to wherever the
    // app was started from means nothing once it is saved.
    let path = &std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let ProjectText { text, media_dir } = project_text(path)?;
    let daw = engine().clone();
    daw.media_bay().set_file_resolver(Box::new(
        daw::standalone::media_bay::ProjectRelativeResolver::new(media_dir.clone()),
    ));
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let summary = load_rpp_text(&daw, &name, &path.to_string_lossy(), &text)
        .map_err(|e| eyre::eyre!("{name} did not parse: {e}"))?;
    daw::standalone::project_loader::anchor_media(&daw, &summary.project_guid, &media_dir);

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

    let track_count = daw::service::Tracks::all(&daw, daw::service::ProjectContext::Project(summary.project_guid.clone())).len();
    Ok(Opened {
        daw,
        name,
        project_guid: summary.project_guid.clone(),
        track_count,
    })
}

/// A project's text as the loader reads it, and the folder its media
/// paths are relative to.
pub struct ProjectText {
    pub text: String,
    pub media_dir: PathBuf,
}

/// Whether `path` is a saved session — a `Song.session` project directory,
/// the native format — rather than a REAPER project.
#[must_use]
pub fn is_session(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("session"))
}

/// Read a project for the loader: a `.RPP` as it is, a `.session` through
/// the format's own `.rpp` export (the round trip it is proven lossless
/// on). Media resolves against the folder either one sits in — a session
/// is saved beside the `.RPP` it was prepared from, so `Media/Bass.wav`
/// means the same file to both.
///
/// # Errors
///
/// The file could not be read, or the session could not be exported.
pub fn project_text(path: &Path) -> eyre::Result<ProjectText> {
    let text = if is_session(path) {
        crate::session_file::session_rpp_text(path)
            .map_err(|e| eyre::eyre!("could not read the session {}: {e}", path.display()))?
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| eyre::eyre!("could not read {}: {e}", path.display()))?
    };
    let media_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok(ProjectText { text, media_dir })
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
/// The audio engine, playing whichever song is current. Replaced — the old
/// one dropped, which closes its stream — when the current song changes.
static AUDIO: std::sync::Mutex<Option<daw::standalone::audio_engine::AudioEngine>> =
    std::sync::Mutex::new(None);

fn attach_audio(daw: &Standalone, project_guid: &str) {
    let Some(rt) = RUNTIME.get() else {
        return;
    };
    let _guard = rt.enter();
    let mut slot = AUDIO.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Close the old stream before opening the device again.
    drop(slot.take());
    match daw.attach_audio_engine(project_guid) {
        Ok(engine) => {
            if let Some(stats) = engine.stats() {
                log_audio_health(stats, engine.sample_rate());
            }
            *slot = Some(engine);
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

/// Every two seconds, how the audio callback is keeping up: the block the
/// device runs at, render time (mean and worst in the interval) against
/// that block's budget, and blocks that overran it or that the device
/// reported as xruns. Logged as `session_daw::audio` — the numbers to read
/// when playback stutters.
fn log_audio_health(stats: std::sync::Arc<daw::standalone::audio_engine::EngineStats>, rate: u32) {
    use std::sync::atomic::Ordering::Relaxed;
    let spawned = std::thread::Builder::new()
        .name("session-daw-audio-health".into())
        .spawn(move || {
            let (mut calls, mut total, mut over, mut xruns) = (0u64, 0u64, 0u64, 0u64);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let now_calls = stats.calls.load(Relaxed);
                let now_total = stats.total_render_ns.load(Relaxed);
                let now_over = stats.over_budget.load(Relaxed);
                let now_xruns = stats.xruns.load(Relaxed);
                let blocks = now_calls.saturating_sub(calls);
                if blocks > 0 {
                    let frames = stats.block_frames.load(Relaxed);
                    let budget_ms = f64::from(frames) * 1000.0 / f64::from(rate.max(1));
                    let mean_ms = now_total.saturating_sub(total) as f64 / blocks as f64 / 1e6;
                    let peak_ms = stats.peak_render_ns.swap(0, Relaxed) as f64 / 1e6;
                    let over_now = now_over.saturating_sub(over);
                    let xruns_now = now_xruns.saturating_sub(xruns);
                    if over_now > 0 || xruns_now > 0 {
                        tracing::warn!(
                            target: "session_daw::audio",
                            blocks, frames, budget_ms, mean_ms, peak_ms,
                            over_budget = over_now, xruns = xruns_now,
                            "audio: blocks missed their deadline"
                        );
                    } else {
                        tracing::info!(
                            target: "session_daw::audio",
                            blocks, frames, budget_ms, mean_ms, peak_ms,
                            "audio: keeping up"
                        );
                    }
                }
                (calls, total, over, xruns) = (now_calls, now_total, now_over, now_xruns);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "audio health logging not started");
    }
}
