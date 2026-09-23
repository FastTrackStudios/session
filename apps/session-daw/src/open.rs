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
//!
//! # The audio mode
//!
//! Which of the two a window is, is its audio mode ([`AudioMode`], issue
//! #142), held here as process state and read everywhere through [`mode`]:
//! **Engine** owns (the in-process engine plays), **Remote** borrows (the
//! other system plays; nothing here does), **Cue** borrows with a local
//! click and guide. It is set once at launch ([`launch_mode`],
//! [`set_mode`]) and can climb at runtime (see [`crate::audio_mode`]).
//!
//! Everything that reaches the local engine rather than the facade goes
//! through here and says what it needs ([`with_local_engine`],
//! [`LocalOnly`]): in Remote it is skipped with one warning, never run
//! against an engine that holds nothing. What the facade CAN do in Remote
//! is done through it — the song on screen ([`current_song`]), switching
//! songs ([`switch_song`]), a song's colour ([`set_song_color`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use daw::standalone::Standalone;
use daw::standalone::project_loader::load_rpp_text;

pub use crate::audio_mode::{
    AudioMode, Assets, ModeState, RemoteTarget, begin_asset_load, set_assets_progress, set_cue_ready,
};

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
    refuse_unless_local(path)?;
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
        equip(&daw);
        daw
    })
}

/// Make an engine a Session engine: what every engine a song opens into
/// needs, whoever runs it — this window, or `session-desktop --engine`.
/// Today that is the instrument the Click / Count / Guide MIDI tracks
/// play through.
pub fn equip(daw: &Standalone) {
    crate::guide_instrument::install(
        daw,
        crate::guide_instrument::Library::Folder(crate::guide_instrument::samples_dir()),
    );
}

// ── opening a song: the one way ─────────────────────────────────────────

/// What opening a song file means: which file is read, whether it is
/// prepared on the way, and where the prepared song is saved.
///
/// Preparing (organize, build from the chart, generate the guide) is done
/// ONCE: the result is saved as `Song.session` beside `Song.RPP`, and from
/// then on the saved session is what opens — the `.RPP` is only the
/// multitrack it started from. A `.session` opened directly is never
/// prepared again. `FTS_SESSION_REPREPARE=1` prepares the `.RPP` afresh and
/// saves over the old session; nothing else overwrites one.
#[derive(Debug, PartialEq, Eq)]
pub struct SongPlan {
    pub open: std::path::PathBuf,
    pub prepare: bool,
    pub save_to: Option<std::path::PathBuf>,
}

/// How `path` opens (see [`SongPlan`]).
#[must_use]
pub fn song_plan(path: &Path, prepare: &crate::prepare::Prepare) -> SongPlan {
    let reprepare = std::env::var("FTS_SESSION_REPREPARE").is_ok_and(|v| v == "1");
    song_plan_with(path, prepare, reprepare)
}

pub(crate) fn song_plan_with(path: &Path, prepare: &crate::prepare::Prepare, reprepare: bool) -> SongPlan {
    if is_session(path) {
        return SongPlan { open: path.to_path_buf(), prepare: false, save_to: None };
    }
    let saved = path.with_extension("session");
    if saved.is_dir() && !reprepare {
        return SongPlan { open: saved, prepare: false, save_to: None };
    }
    let prepare = !prepare.is_empty();
    SongPlan { open: path.to_path_buf(), prepare, save_to: prepare.then_some(saved) }
}

/// After a song's file is open: prepare it if its plan says so, and save
/// the prepared song as its `.session` — so the next open is the prepared
/// one. A preparation that fails saves nothing (a half-prepared session
/// would open as prepared next time).
pub fn prepare_and_save(opened: &Opened, plan: &SongPlan, prepare: &crate::prepare::Prepare) {
    let mut save_to = plan.save_to.clone();
    if plan.prepare
        && let Err(e) = prepare.run(opened)
    {
        tracing::error!(error = %e, "preparing the session failed; opening it as it was");
        save_to = None;
    }
    if let Some(dir) = &save_to {
        match crate::session_file::save_session(&opened.daw, &opened.project_guid, dir) {
            Ok(at) => tracing::info!(
                session.saved = %at.display(),
                session.from = %plan.open.display(),
                "prepared once; saved as a session"
            ),
            Err(e) => tracing::warn!(
                session.save_error = %e,
                "the prepared session could not be saved; it will be prepared again next time"
            ),
        }
    }
}

/// Open a song into `daw` — THE way a song is opened, whoever runs the
/// engine (this window in Engine mode, `session-desktop --engine`): the
/// prepared `.session` when there is one, otherwise the file itself,
/// prepared and saved as its `.session` once. `daw` should be
/// [`equip`]ped. The song does not become current.
///
/// # Errors
///
/// The file could not be read or parsed, or its media did not materialize.
pub fn open_song_into(
    daw: &Standalone,
    path: &Path,
    prepare: &crate::prepare::Prepare,
) -> eyre::Result<(Opened, SongPlan)> {
    let plan = song_plan(path, prepare);
    let opened = load_into(daw, &plan.open)?;
    prepare_and_save(&opened, &plan, prepare);
    Ok((opened, plan))
}

/// Load another song into the engine the first open made — for a
/// setlist, after the first song. It does not become current and nothing
/// plays it until [`switch_to`] says so.
///
/// # Errors
///
/// The file could not be read or parsed, or its media did not materialize.
pub fn open_another(path: &Path) -> eyre::Result<Opened> {
    refuse_unless_local(path)?;
    load(path)
}

/// Opening a file is standing it up in the local engine — which a Remote
/// or Cue window has none of: the system it drives owns the projects.
fn refuse_unless_local(path: &Path) -> eyre::Result<()> {
    let state = mode();
    if state.owns_project() {
        return Ok(());
    }
    let target = state.target.as_ref().map_or_else(|| "another system".to_owned(), RemoteTarget::describe);
    eyre::bail!(
        "{} is not opened here: this window is in {} mode, driving {target}",
        path.display(),
        state.requested.name()
    )
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

/// Make `project_guid` the song on screen — what picking a setlist tab
/// does.
///
/// - **Engine**: [`switch_to`] on the process's engine; the audio moves
///   with it when the window plays at all.
/// - **Remote / Cue**: the facade's project service selects it — a REAPER
///   tab — and waits for the answer, so a play pressed next plays this
///   song. A Cue window's click and guide go with it
///   ([`cue_follow_song`]).
pub fn switch_song(project_guid: &str) {
    let state = mode();
    if state.owns_project() {
        switch_to(engine(), project_guid, AUDIBLE.load(std::sync::atomic::Ordering::Relaxed));
        return;
    }
    set_remote_current(Some(project_guid.to_owned()));
    let selected = facade_blocking("switch song", |daw| {
        let guid = project_guid.to_owned();
        async move { daw.select_project(guid).await.map(|_| ()) }
    });
    if let Err(e) = selected {
        tracing::warn!(
            audio.mode = state.requested.name(),
            song.project = project_guid,
            error = %e,
            "remote: the song could not be selected on the system this window drives"
        );
    }
    if state.requested == AudioMode::Cue {
        cue_follow_song(project_guid);
    }
}

/// A Cue window's click and guide follow the song by themselves: the Cue
/// task ([`crate::cue`]) watches the remote's current song, whoever changed
/// it (a pick here, a tab in REAPER).
fn cue_follow_song(project_guid: &str) {
    tracing::debug!(song.project = project_guid, "cue: the song changed; the Cue task follows");
}

/// The engine a Cue window plays its click and guide on: this process's
/// own — the one Engine mode plays a whole song on (Cue is its first
/// stage).
pub(crate) fn cue_engine() -> &'static Standalone {
    engine()
}

/// Something only the local engine can do — skipped, with one warning each,
/// in a window that drives another system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalOnly {
    /// Rebuilding the song's structure from an edited chart
    /// (`session::keyflow::from_chart` runs on the sync backend traits,
    /// which only an in-process backend implements).
    ChartRebuild,
    /// Writing a song's `.session` — the native format is the local
    /// engine's project; a remote system saves its own.
    SessionSave,
    /// daw-transport-sync's per-buffer backend, which followers lock to.
    /// Following over the facade is a separate issue.
    TransportSync,
}

impl LocalOnly {
    const fn name(self) -> &'static str {
        match self {
            Self::ChartRebuild => "chart_rebuild",
            Self::SessionSave => "session_save",
            Self::TransportSync => "transport_sync",
        }
    }

    const fn bit(self) -> u32 {
        match self {
            Self::ChartRebuild => 1,
            Self::SessionSave => 2,
            Self::TransportSync => 4,
        }
    }
}

/// The capabilities already warned about, so each warns once.
static WARNED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The process's engine, when this window owns its project (Engine mode);
/// `None` in Remote and Cue — for a caller that has its own remote path
/// (the Organize toolbar), so nothing is warned about.
#[must_use]
pub fn local_engine() -> Option<&'static Standalone> {
    crate::audio_mode::owns_project().then(engine)
}

/// Run `f` against the process's engine when this window owns its project
/// (Engine mode) — for what acts on the session directly rather than
/// through the facade. `None` in Remote and Cue, where there is no local
/// project to act on: `what` says what was skipped, once.
pub fn with_local_engine<R>(what: LocalOnly, f: impl FnOnce(&Standalone) -> R) -> Option<R> {
    if let Some(daw) = local_engine() {
        return Some(f(daw));
    }
    let first = WARNED.fetch_or(what.bit(), std::sync::atomic::Ordering::Relaxed) & what.bit() == 0;
    if first {
        let state = mode();
        tracing::warn!(
            audio.mode = state.requested.name(),
            audio.target = state.target.as_ref().map(RemoteTarget::kind),
            audio.capability = what.name(),
            "remote: this needs the local engine, which this window does not run; skipped"
        );
    }
    None
}

/// The song on screen: the engine's current project in Engine; in Remote
/// and Cue, the facade's current project — as last selected here or seen
/// on the remote (a tab picked in REAPER itself), kept fresh by a poll
/// rather than asked for on every call: panels ask every frame.
#[must_use]
pub fn current_song() -> Option<String> {
    use daw::service::Projects as _;
    if crate::audio_mode::owns_project() {
        return engine().current().map(|p| p.guid);
    }
    REMOTE_CURRENT.read().ok().and_then(|slot| slot.clone())
}

/// The remote's current project, as [`current_song`] reports it.
static REMOTE_CURRENT: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Bumped by every selection made here, so a poll that was already in
/// flight when the song changed cannot write the old song back.
static REMOTE_PICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn set_remote_current(guid: Option<String>) {
    REMOTE_PICKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut slot) = REMOTE_CURRENT.write() {
        *slot = guid;
    }
}

/// Keep [`REMOTE_CURRENT`] following the remote's current project, twice a
/// second, for the life of the process. Started once, by the first attach.
fn watch_remote_current(rt: &'static tokio::runtime::Runtime) {
    static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if WATCHING.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    rt.spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if crate::audio_mode::owns_project() {
                continue;
            }
            let Some(daw) = daw::rpc::Daw::try_get() else { continue };
            let picks = REMOTE_PICKS.load(std::sync::atomic::Ordering::SeqCst);
            // A gone REAPER is the reattach loop's business; the last
            // known song stays until it answers again.
            if let Ok(project) = daw.current_project().await
                && REMOTE_PICKS.load(std::sync::atomic::Ordering::SeqCst) == picks
                && let Ok(mut slot) = REMOTE_CURRENT.write()
                && slot.as_deref() != Some(project.guid())
            {
                *slot = Some(project.guid().to_owned());
            }
        }
    });
}

/// Run one facade call to completion from a sync context (the UI thread,
/// a worker), on the engine runtime, bounded so a remote that has gone
/// quiet cannot hang the caller.
pub(crate) fn facade_blocking<F, Fut, T>(what: &str, f: F) -> eyre::Result<T>
where
    F: FnOnce(&'static daw::rpc::Daw) -> Fut,
    Fut: std::future::Future<Output = Result<T, daw::rpc::Error>> + Send,
    T: Send,
{
    let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("{what}: the daw facade is not up"))?;
    let call = f(daw);
    block_on_engine(async {
        tokio::time::timeout(std::time::Duration::from_secs(3), call)
            .await
            .map_err(|_| eyre::eyre!("{what}: the remote did not answer"))?
            .map_err(|e| eyre::eyre!("{what}: {e}"))
    })
    .ok_or_else(|| eyre::eyre!("{what}: the engine runtime is not up"))?
}

/// Run `future` to completion on the engine runtime from sync code — on a
/// scoped thread of its own when the caller is already inside a runtime,
/// where blocking on another would panic. `None` before there is one.
pub(crate) fn block_on_engine<T: Send>(future: impl std::future::Future<Output = T> + Send) -> Option<T> {
    let rt = runtime()?;
    if tokio::runtime::Handle::try_current().is_err() {
        return Some(rt.block_on(future));
    }
    std::thread::scope(|scope| scope.spawn(|| rt.block_on(future)).join().ok())
}

/// Colour a song's SONG region (`0xRRGGBB`; `0` clears it) and save the
/// song into `saved`, its `.session`, when it has one. A save that fails
/// is logged — the colour still holds for this run.
///
/// In Remote and Cue the colour goes through the facade's region service
/// (REAPER shows it on its region too); there is no `.session` to save —
/// the remote system saves its own project.
pub fn set_song_color(project_guid: &str, rgb: u32, saved: Option<&Path>) {
    let song_lane = session::ruler_lanes::CoreLane::Song.lane_index();
    if !crate::audio_mode::owns_project() {
        let coloured = facade_blocking("song colour", |daw| {
            let guid = project_guid.to_owned();
            async move {
                let regions = daw.project(guid).await?.regions();
                let region = regions.all().await?.into_iter().find(|r| r.lane == Some(song_lane));
                match region.and_then(|r| r.id) {
                    Some(id) => regions.set_color(id, rgb).await.map(|()| true),
                    None => Ok(false),
                }
            }
        });
        match coloured {
            Ok(true) => {}
            Ok(false) => tracing::warn!(song.project = project_guid, "no SONG region to colour"),
            Err(e) => tracing::warn!(error = %e, "the song's colour could not be set"),
        }
        if saved.is_some() {
            with_local_engine(LocalOnly::SessionSave, |_| ());
        }
        return;
    }
    use daw::service::Regions as _;
    let daw = engine();
    let project = daw::service::ProjectContext::Project(project_guid.to_owned());
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
    load_into(engine(), path)
}

/// Load exactly `path` (a `.RPP` or a `.session`) into `daw`: parse it,
/// resolve and anchor its media, materialize its audio. No preparing, no
/// choosing — see [`open_song_into`] for opening a *song*.
///
/// # Errors
///
/// The file could not be read or parsed, or its media did not materialize.
pub fn load_into(daw: &Standalone, path: &Path) -> eyre::Result<Opened> {
    // Absolute from here on: the project's path is what a later save
    // writes its media relative to, and a path relative to wherever the
    // app was started from means nothing once it is saved.
    let path = &std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let ProjectText { text, media_dir } = project_text(path)?;
    let daw = daw.clone();
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
    let socket_for_mode = socket.clone();
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
    read_back(rt, RemoteTarget::Reaper { socket: socket_for_mode })
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
/// Remote on REAPER ([`launch_mode`]) wins over a path, because asking for
/// both is a mistake worth answering rather than a preference worth
/// guessing at — and the live one is the one you would have meant.
pub fn source() -> Option<Source> {
    if let Some(RemoteTarget::Reaper { socket }) = launch_mode().target {
        return Some(Source::Reaper(socket));
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
pub const REAPER_ENV: &str = crate::audio_mode::REAPER_ENV;
/// Which REAPER, when more than one is running.
pub const SOCKET_ENV: &str = crate::audio_mode::SOCKET_ENV;

// ── the audio mode ───────────────────────────────────────────────────

/// The process's audio mode: requested, effective, the target, loading.
#[must_use]
pub fn mode() -> ModeState {
    crate::audio_mode::state()
}

/// Set the process's audio mode — at launch, before the first open or
/// attach.
pub fn set_mode(state: ModeState) {
    crate::audio_mode::set(state);
}

/// The mode this launch asks for, from the command line
/// (`--audio engine|remote|cue`, `--reaper [socket]`), the environment
/// (`FTS_AUDIO_MODE`, `SESSION_DAW_REAPER` + `FTS_SOCKET`,
/// `FTS_AUDIO_TARGET`), whether a project was named
/// (`FTS_SESSION_PROJECT` / `FTS_SESSION_SETLIST` / `SESSION_DAW_PROJECT`
/// or a path argument — Engine), and what the picker last chose — in that
/// order (see [`crate::audio_mode::from_launch`]). Does not install it.
#[must_use]
pub fn launch_mode() -> ModeState {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |key: &str| std::env::var(key).ok();
    let named = ["FTS_SESSION_PROJECT", "FTS_SESSION_SETLIST", "SESSION_DAW_PROJECT"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
        || args
            .first()
            .is_some_and(|a| !a.starts_with("--") && Path::new(a).exists());
    crate::audio_mode::from_launch(&args, &env, named, remembered_mode())
}

/// Where the picker's choice is kept between launches.
fn mode_memory() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Session").join("audio-mode"))
}

fn remembered_mode() -> Option<AudioMode> {
    AudioMode::parse(&std::fs::read_to_string(mode_memory()?).ok()?)
}

/// Keep `mode` as the one the next launch opens in (when the launch does
/// not name one — see [`launch_mode`]).
pub fn remember_mode(mode: AudioMode) {
    let Some(file) = mode_memory() else { return };
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, mode.name().to_ascii_lowercase()));
    if let Err(e) = written {
        tracing::warn!(error = %e, audio.requested = mode.name(), "the audio mode could not be remembered");
    }
}

/// Change the mode while the window runs, as far as it can be changed
/// without re-pointing the facade: Remote and Cue drive the same backend,
/// so either becomes the other at once. Engine and Remote are different
/// backends — the window's projects live in one or the other — so that
/// change is remembered for the next launch and `false` comes back.
pub fn request_mode(to: AudioMode) -> bool {
    remember_mode(to);
    let state = mode();
    if state.requested == to {
        return true;
    }
    let live = !state.owns_project() && to != AudioMode::Engine;
    if live {
        crate::audio_mode::update_requested(to);
    }
    live
}

/// Attach to the system `target` names — the Remote and Cue way in.
///
/// # Errors
///
/// As [`attach_to_reaper`]; a Session engine target is not wired yet
/// (#142 follow-up) and says so rather than attaching to something else.
pub fn attach(target: &RemoteTarget) -> eyre::Result<Attached> {
    let attached = match target {
        RemoteTarget::Reaper { socket } => attach_to_reaper(socket.clone()),
        RemoteTarget::Session { address } => attach_to_engine(address),
    }?;
    // Idle unless Cue is asked for; then the click and guide play here,
    // locked to what this window drives.
    crate::cue::start();
    Ok(attached)
}

/// Something that keeps a dialed engine's connection open for as long
/// as it lives (dropping it closes the connection).
pub type EngineConnection = Box<dyn std::any::Any + Send + Sync>;

/// Dials a Session engine: its address as a person pastes it
/// (`fts-engine:<id>`, `ws://host:4040/vox`) in, its daw facade out.
///
/// Installed by the app (the dialer lives with the app's network
/// identity — its iroh endpoint and key — not here).
pub type EngineDialer = fn(
    String,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = eyre::Result<(daw::rpc::Daw, EngineConnection)>> + Send>,
>;

static ENGINE_DIALER: OnceLock<EngineDialer> = OnceLock::new();

/// The connection to the Session engine this window is attached to.
static ENGINE_CONNECTION: std::sync::RwLock<Option<EngineConnection>> = std::sync::RwLock::new(None);

/// Install how this app dials a Session engine (once, at launch).
pub fn set_engine_dialer(dialer: EngineDialer) {
    let _ = ENGINE_DIALER.set(dialer);
}

/// Attach to a Session engine (`session-desktop --engine`) the way
/// [`attach_to_reaper`] attaches to REAPER: the engine owns the project
/// and plays it; this window drives it through the daw facade.
///
/// # Errors
///
/// When this window already owns a project, no dialer is installed, the
/// engine cannot be reached, or its project cannot be read back.
pub fn attach_to_engine(address: &str) -> eyre::Result<Attached> {
    if BUNDLE.get().is_some() {
        eyre::bail!("this window already owns a project; it cannot also drive a Session engine");
    }
    let dial = ENGINE_DIALER
        .get()
        .ok_or_else(|| eyre::eyre!("this build cannot dial a Session engine"))?;
    let rt = engine_runtime()?;
    let (daw, connection) = rt
        .block_on(dial(address.to_owned()))
        .map_err(|e| eyre::eyre!("could not reach the Session engine at {address}: {e}"))?;
    let block_on_rt = Arc::new(tokio::runtime::Builder::new_current_thread().enable_all().build()?);
    daw::init_from_parts(daw, block_on_rt);
    if let Ok(mut slot) = ENGINE_CONNECTION.write() {
        *slot = Some(connection);
    }
    read_back(rt, RemoteTarget::Session { address: address.to_owned() })
}

/// After an attach: read the project back (never report what was hoped
/// for), and record this window as Remote on `target`.
fn read_back(rt: &'static tokio::runtime::Runtime, target: RemoteTarget) -> eyre::Result<Attached> {
    let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("the facade did not install"))?;
    let (name, guid, path, track_count) = rt.block_on(async {
        let project = daw.current_project().await?;
        let info = project.info().await?;
        let tracks = project.tracks().all().await?;
        Ok::<_, daw::rpc::Error>((info.name, info.guid, info.path, tracks.len()))
    })?;
    // Attached is Remote, whatever the launch said — a caller that attaches
    // without choosing a mode first (a test) is not left reporting Engine.
    // A Cue window stays Cue.
    if mode().owns_project() {
        set_mode(ModeState::remote(target, false));
    }
    set_remote_current(Some(guid.clone()));
    watch_remote_current(rt);
    Ok(Attached {
        name,
        project_guid: guid,
        path: Some(path).filter(|p| !p.is_empty()).map(Into::into),
        track_count,
    })
}

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
