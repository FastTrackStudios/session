//! Recording Mode — drive a real, running REAPER instead of playing the
//! setlist ourselves.
//!
//! Live Mode ([`crate::session_engine`]) embeds a `daw-standalone` backend
//! and plays the setlist's audio itself. Recording Mode is the same
//! `SetlistService` wire surface — same `SetlistServiceClient` type, same
//! `session_ui::Session::init` call, same performance view — just dialed
//! at a REAPER extension's `SetlistServiceImpl<daw_reaper::Reaper>`
//! instead of an in-process one. `fts-extensions` already mounts that
//! service (`session::daw_services::layer_services_with_daw`) — along
//! with every other `daw::service` trait `Reaper` implements, via
//! `Reaper.into_router()` — onto the `LayerRouter` its REAPER-hosted
//! `daw-reaper` publishes over a local Unix-domain-socket vox link
//! (`/tmp/fts-daw-{pid}.sock`).
//!
//! Two client connections share that one socket, opened separately
//! because they need different client types:
//! - [`daw::cli`] (already-written REAPER launcher + socket discovery +
//!   the generic `Projects`/`Tracks`/etc. surface) handles "find or
//!   launch REAPER" and "open each song's project".
//! - This module's own [`connect`] opens the `SetlistService` lanes
//!   directly — `daw::cli`'s `Daw` type doesn't expose those (they're
//!   session's own services, not core `daw` ones).
//!
//! No audio thread here: REAPER is the one making sound.

use std::path::{Path, PathBuf};

use session::services::setlist_service::SetlistServiceStreamClient;
use session::SetlistServiceClient;
use session_vault_sync::library::LibrarySong;

const SOCKET_DIR: &str = "/tmp";
const SOCKET_PREFIX: &str = "fts-daw-";
const SOCKET_SUFFIX: &str = ".sock";

/// The dev-rig REAPER profile (`~/fts-dev`, per `daw::cli::daw_profiles()`)
/// — the one this whole workflow (organizing songs, testing Recording
/// Mode) has been running against. A profile picker is future work; this
/// is the one that matters today.
const REAPER_PROFILE: &str = "fts-dev";

/// The most recently started live REAPER's socket, if any — the highest
/// pid among sockets whose owning process is still alive. Stale sockets
/// (REAPER exited without cleanup) are skipped, not deleted; this app has
/// no business unlinking another process's files.
fn discover_socket() -> Option<PathBuf> {
    let entries = std::fs::read_dir(SOCKET_DIR).ok()?;
    let mut sockets: Vec<(u32, PathBuf)> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let filename = path.file_name()?.to_str()?;
            let pid: u32 = filename
                .strip_prefix(SOCKET_PREFIX)?
                .strip_suffix(SOCKET_SUFFIX)?
                .parse()
                .ok()?;
            process_alive(pid).then_some((pid, path))
        })
        .collect();
    sockets.sort_by_key(|(pid, _)| std::cmp::Reverse(*pid));
    sockets.into_iter().next().map(|(_, path)| path)
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // Signal 0: no-op existence check (see `kill(2)`) — this never
    // actually signals the process, just probes whether it's ours to see.
    unsafe { libc::kill(i32::try_from(pid).unwrap_or(-1), 0) == 0 }
}

/// A live connection to a REAPER extension's `SetlistService`. Keeps
/// `session_ui`'s `Session` singleton pointed at real REAPER for the rest
/// of the process — dropping this would close the link, so it's leaked
/// into a static the same way [`crate::session_engine::SessionEngine`] is.
pub struct ReaperEngine {
    pub client: SetlistServiceClient,
    pub stream_client: SetlistServiceStreamClient,
    pub socket: PathBuf,
    /// Keeps the vox connection (and its socket) open for the engine's
    /// lifetime — both clients above borrow it implicitly through their
    /// lanes, and dropping it tears down the whole link.
    _connection: vox_core::ConnectionHandle,
}

/// Reason Recording Mode couldn't attach — surfaced to the mode-switch UI
/// rather than just logged, since "REAPER isn't running" is the expected
/// first thing a user sees, not a bug report.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("no REAPER socket found in /tmp — is REAPER running with the FTS extension loaded?")]
    NoSocket,
    #[error("connecting to {}: {source}", .path.display())]
    Connect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("vox handshake with {}: {source:?}", .path.display())]
    Handshake {
        path: PathBuf,
        source: vox_core::ConnectionError,
    },
}

/// Connect to the most recently started live REAPER and hand back a
/// `SetlistService` client pair — the same client type
/// [`session_ui::Session::init`] takes in Live Mode, just backed by real
/// REAPER instead of the standalone player.
pub async fn connect() -> Result<ReaperEngine, ConnectError> {
    connect_to(&discover_socket().ok_or(ConnectError::NoSocket)?).await
}

/// [`connect`] against an explicit socket path — for the future connect
/// UI (picking among several sockets) and for tests that spawn their own
/// isolated REAPER instance.
pub async fn connect_to(socket: &Path) -> Result<ReaperEngine, ConnectError> {
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|source| ConnectError::Connect {
            path: socket.to_path_buf(),
            source,
        })?;
    let link = vox_stream::StreamLink::unix(stream);
    let connection = vox_core::initiator_on(link)
        .establish_connection()
        .await
        .map_err(|source| ConnectError::Handshake {
            path: socket.to_path_buf(),
            source,
        })?;

    let client = connection
        .open_lane::<SetlistServiceClient>()
        .await
        .map_err(|source| ConnectError::Handshake {
            path: socket.to_path_buf(),
            source,
        })?;
    let stream_client = connection
        .open_lane::<SetlistServiceStreamClient>()
        .await
        .map_err(|source| ConnectError::Handshake {
            path: socket.to_path_buf(),
            source,
        })?;

    Ok(ReaperEngine {
        client,
        stream_client,
        socket: socket.to_path_buf(),
        _connection: connection,
    })
}

static ENGINE: std::sync::OnceLock<ReaperEngine> = std::sync::OnceLock::new();

/// The Recording Mode connection, once established. `None` in Live Mode,
/// before the first successful connect, or if REAPER isn't reachable.
pub fn engine() -> Option<&'static ReaperEngine> {
    ENGINE.get()
}

static RUNTIME: std::sync::OnceLock<std::sync::Arc<tokio::runtime::Runtime>> =
    std::sync::OnceLock::new();

fn runtime() -> &'static std::sync::Arc<tokio::runtime::Runtime> {
    RUNTIME.get_or_init(|| {
        std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("fts-reaper-engine")
                .enable_all()
                .build()
                .expect("build the reaper-engine tokio runtime"),
        )
    })
}

/// Install `daw::get()`'s global singleton against this Recording Mode
/// connection, once. Live Mode installs it at boot
/// (`session_engine::bootstrap_blocking` -> `daw::init_from_parts`); this
/// module never did, so anything that self-connects through `daw::get()`
/// — `daw_ui::MixerPanel` (the app's actual Mixer tab), `mixer_view.rs`'s
/// "Open in REAPER" — silently found no DAW and rendered nothing in
/// Recording Mode. Idempotent: safe to call every time `ensure_reaper_running`
/// runs (app boot with REAPER already up, and again after `load_playlist`
/// launches it fresh).
fn install_daw_singleton(daw: daw::rpc::Daw) {
    if daw::get().is_some() {
        return;
    }
    daw::init_from_parts(daw, runtime().clone());
}

/// Connect to REAPER and install the client as session-ui's `Session`
/// singleton, if that hasn't already happened. Idempotent — a no-op once
/// `engine()` is `Some`, so it's safe to call both at boot (REAPER may
/// already be running from a previous session) and after
/// [`load_playlist`] launches/attaches to REAPER later.
async fn ensure_connected() -> eyre::Result<()> {
    if ENGINE.get().is_some() {
        return Ok(());
    }
    let engine = connect()
        .await
        .map_err(|e| eyre::eyre!("connect to REAPER: {e}"))?;

    // REAPER was already running (this is the "app started after REAPER"
    // path) — `load_playlist` isn't guaranteed to run again this session,
    // so this is the only chance to install `daw::get()`'s singleton
    // before the Mixer tab (`daw_ui::MixerPanel`) tries to self-connect.
    if let Ok(daw_connection) = daw::cli::connect(Some(engine.socket.clone())).await {
        install_daw_singleton(daw_connection.daw);
    }

    session_ui::Session::init(engine.client.clone())
        .map_err(|e| eyre::eyre!("Session::init: {e:?}"))?;

    // `--engine` mode's LAN control surface (engine_server.rs) serves
    // Recording Mode through this proxy — installed here, not there, so
    // it's ready the moment REAPER is regardless of whether the LAN
    // server is even running in this process (the GUI app installs it
    // too; it's cheap — two subscriber tasks — and unused if nothing ever
    // dials /vox).
    crate::reaper_lan_proxy::install(engine.client.clone(), engine.stream_client.clone());

    ENGINE
        .set(engine)
        .map_err(|_| eyre::eyre!("reaper engine initialized twice"))?;
    Ok(())
}

/// The Recording Mode counterpart to
/// [`crate::session_engine::bootstrap_blocking`] — called once at app
/// startup. Unlike Live Mode, failure here is the *expected* common case
/// (REAPER usually isn't running yet): it just means the performance view
/// shows "not connected" until [`load_playlist`] launches REAPER and
/// connects for real. Blocking: runs on a dedicated leaked runtime that
/// then keeps hosting the vox connection for the life of the process.
pub fn bootstrap_blocking() -> eyre::Result<()> {
    // Leak the runtime handle now so it's ready for `load_playlist`'s
    // later async calls even if this first connect attempt fails.
    let rt = runtime();
    match rt.block_on(ensure_connected()) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::info!("REAPER not reachable yet ({e}); waiting for Load & Play");
            Ok(())
        }
    }
}

/// Open every song's REAPER project as its own tab — launching REAPER
/// first (the `fts-dev` dev-rig profile) if nothing is running yet — then
/// rebuild the setlist from whatever's now open, exactly like Live Mode's
/// `SessionEngine::load_setlist` does for the standalone backend.
///
/// This is the "open the playlist/album in REAPER" entry point: each
/// `LibrarySong` names a folder holding an already-organized `.RPP` (the
/// same files `dynamic-template --apply-buses` / `convert-markers`
/// produce) — REAPER opens the real project, not a synthesized one.
pub async fn load_playlist(songs: &[LibrarySong]) -> eyre::Result<()> {
    let daw = ensure_reaper_running().await?;

    for song in songs {
        let Some(rpp_path) = find_rpp(&song.folder) else {
            tracing::warn!(
                "no .RPP found under {} for '{}'; skipping",
                song.folder.display(),
                song.title
            );
            continue;
        };
        daw.open_project(rpp_path.to_string_lossy().into_owned())
            .await
            .map_err(|e| eyre::eyre!("opening '{}' ({rpp_path:?}): {e:?}", song.title))?;
    }

    // REAPER is confirmed running now (we just opened projects on it) —
    // (re)connect the persistent SetlistService link + Session singleton
    // if `bootstrap_blocking` couldn't at app startup.
    ensure_connected().await?;
    let engine = engine().ok_or_else(|| {
        eyre::eyre!("REAPER is running but Recording Mode's own SetlistService link isn't up")
    })?;
    engine
        .client
        .build_from_open_projects()
        .await
        .map_err(|e| eyre::eyre!("build_from_open_projects: {e:?}"))?;
    engine
        .client
        .seek_to_section(0, 0)
        .await
        .map_err(|e| eyre::eyre!("seek_to_section: {e:?}"))?;
    Ok(())
}

/// Connect to a running REAPER, or launch the `fts-dev` profile and wait
/// for it to come up if nothing's listening yet.
async fn ensure_reaper_running() -> eyre::Result<daw::rpc::Daw> {
    if let Some(socket) = discover_socket() {
        let daw = daw::cli::connect(Some(socket)).await?.daw;
        install_daw_singleton(daw.clone());
        return Ok(daw);
    }
    tracing::info!("no live REAPER found; launching the '{REAPER_PROFILE}' profile");
    let (connection, pid, socket) = daw::cli::launch_and_connect(REAPER_PROFILE).await?;
    tracing::info!(pid, socket = %socket.display(), "REAPER launched");
    install_daw_singleton(connection.daw.clone());
    Ok(connection.daw)
}

/// The `.RPP` an already-organized song folder holds — there's exactly
/// one per song by convention (this repo's own `dynamic-template
/// --apply-buses` never produces more than one `.organized.RPP` per
/// input), but prefer an `.organized.RPP` over a bare one if both exist,
/// since that's the file this whole pipeline has been building toward.
fn find_rpp(folder: &Path) -> Option<PathBuf> {
    let entries: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rpp"))
        })
        .collect();
    entries
        .iter()
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".organized"))
        })
        .or_else(|| entries.first())
        .cloned()
}
