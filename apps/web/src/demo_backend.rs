//! The in-process demo backend — a real `daw-standalone` + `session`
//! setlist service running entirely in the browser, no server.
//!
//! Mirrors `apps/desktop/src/session_engine.rs`'s bootstrap exactly
//! (`Standalone` → `build_in_process_daw` → `daw::init_from_parts` →
//! `SetlistServiceImpl` behind an `architect::LocalServer` `LayerRouter`),
//! minus RPP loading, the media bay, and audio-engine attachment: the
//! setlist is five structural fixtures (see [`demo_songs`]) stamped
//! straight into `Standalone` via `stamp_song_native`, and the transport's
//! soft clock (`daw-standalone/src/transport.rs`) already advances the
//! playhead on a wall-clock tick with no audio device attached — proven
//! wasm-safe there, so nothing native-only is needed here at all.

use std::sync::{Arc, OnceLock};

use daw::LayerRouter;
use daw::service::ProjectContext;
use daw_proto::ProjectInfo;
use daw_standalone::bootstrap::{InProcessDaw, build_in_process_daw};
use daw_standalone::sync::Standalone;
use session::services::setlist_service::{
    SetlistServiceStreamClient, setlist_service_stream_service_descriptor,
    stream_serve as setlist_service_stream_serve,
};
use session::setlist::chart_import::chart_to_layout;
use session::setlist::service::demo::{
    chart_layout_to_demo_song, fixture_songs, stamp_song_native,
};
use session::{
    SetlistServiceClient, SetlistServiceImpl, serve_setlist_service,
    setlist_service_service_descriptor,
};

/// The "Praise" chart — the one song in this repo laid out end-to-end from
/// real keyflow chart text (see `crates/session/session/tests/chart_import_praise.rs`,
/// the golden case this text is kept in sync with).
pub(crate) const PRAISE_CHART: &str = "\
Praise - Elevation Worship
#A 127bpm 4/4

Count 2
In 4
Refrain 8
VS 8
VS
PRE 2
CH 8
VS
VS
PRE
CH
CH
Interlude \"Breakdown\" 8
BR \"Down\" 8
BR \"Build\"
CH
CH
CH
INST \"Guitar Lead\" 8
Refrain
Refrain";

/// The demo setlist: the real Praise chart, plus four procedurally-varied
/// fixture songs (`fixture_songs` already gives them realistic, varied
/// section layouts and recognizable worship-song titles) — five songs
/// total, so the demo reads as a real set rather than a single-song toy.
fn demo_songs() -> eyre::Result<Vec<session::setlist::service::demo::DemoSong>> {
    let praise_layout =
        chart_to_layout(PRAISE_CHART).map_err(|e| eyre::eyre!("chart_to_layout: {e}"))?;
    let mut songs = vec![chart_layout_to_demo_song("Praise", &praise_layout)];
    songs.extend(fixture_songs(4));
    Ok(songs)
}

/// Everything a page needs to drive the live demo: the RPC client for
/// transport commands and the stream client for the `#[subscribe]`
/// events/active_indices hubs. Kept alive for the lifetime of the tab —
/// dropping `_scope` or `_daw` would tear down the in-process link.
///
/// `Standalone` (inside `_daw`) wraps `architect::PubSub` hubs holding vox
/// connection types that are `!Sync` on wasm's single-threaded runtime, so
/// a plain `static` (which must be `Sync`) can't hold a `DemoHandle`
/// directly — same shape as `daw`'s own `WasmDaw` (see `daw::init_from_parts`).
pub struct DemoHandle {
    pub client: SetlistServiceClient,
    pub stream_client: SetlistServiceStreamClient,
    _daw: InProcessDaw,
    _scope: Arc<architect::Scope>,
}

// SAFETY: wasm32 is single-threaded — there is no other thread to share
// with or send to, so the `!Send`/`!Sync` vox connection types inside
// `DemoHandle` are never accessed across threads.
unsafe impl Sync for DemoHandle {}
unsafe impl Send for DemoHandle {}

static HANDLE: OnceLock<Arc<DemoHandle>> = OnceLock::new();

/// Boot the in-process backend once per tab and cache the handle —
/// idempotent, so revisiting `/demo` after navigating away just re-reads
/// the same live backend instead of re-booting (and re-erroring on
/// `Session::init`, which only accepts one client for the process).
///
/// # Errors
///
/// Returns an error if the chart fails to parse, stamping the fixtures
/// fails, or the in-process RPC plumbing fails to stand up.
pub async fn boot() -> eyre::Result<Arc<DemoHandle>> {
    if let Some(handle) = HANDLE.get() {
        return Ok(Arc::clone(handle));
    }

    // 1. Seed one project per song and stamp its structure (markers +
    //    section regions) — the same shape
    //    `standalone_setlist_harness::rich_setlist_ten_projects_one_song_each`
    //    proves in-repo.
    let standalone = Standalone::new();
    for (i, song) in demo_songs()?.iter().enumerate() {
        let guid = standalone.seed_project(ProjectInfo {
            guid: format!("demo-song-{i:02}"),
            name: song.name.to_string(),
            path: String::new(),
        });
        stamp_song_native(&standalone, &ProjectContext::Project(guid), song)
            .map_err(|e| eyre::eyre!("stamp song {i} ({}): {e:?}", song.name))?;
    }

    // 2. In-process daw facade over a vox memory link — `daw::get()`
    //    resolves through this for the setlist build/hydration path.
    let bundle = build_in_process_daw(standalone.clone())
        .await
        .map_err(|e| eyre::eyre!("build_in_process_daw: {e:?}"))?;
    // wasm: no block_on runtime (the browser has none — the setlist path
    // is driven through async calls only). Native (a plain `cargo check`
    // on the host, since this crate isn't wasm32-only at the type-check
    // level): a lightweight current-thread runtime, same shape
    // `session_engine::bootstrap` builds for the desktop app.
    #[cfg(target_arch = "wasm32")]
    daw::init_from_parts(bundle.daw.clone());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let block_on_rt = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| eyre::eyre!("block_on runtime: {e}"))?,
        );
        daw::init_from_parts(bundle.daw.clone(), block_on_rt);
    }

    // 3. The setlist service over the standalone backend.
    let setlist = SetlistServiceImpl::with_daw(standalone);

    // 4. In-process RPC client (architect::LocalServer over a memory
    //    link) — the same conduit shape every remote uses.
    let router = LayerRouter::new()
        .with(
            setlist_service_service_descriptor(),
            serve_setlist_service(setlist.clone()),
        )
        .with(
            setlist_service_stream_service_descriptor(),
            setlist_service_stream_serve(setlist.clone()),
        );
    let scope = architect::Scope::new();
    let server = architect::LocalServer::serve(router, Arc::clone(&scope));
    let caller = server
        .caller()
        .await
        .map_err(|e| eyre::eyre!("local setlist caller: {e:?}"))?;
    let client = SetlistServiceClient::new(caller);
    let stream_client = server
        .establish::<SetlistServiceStreamClient>()
        .await
        .map_err(|e| eyre::eyre!("local setlist stream client: {e:?}"))?;

    // Stream pumps start now, before anything subscribes, so the very
    // first republish reaches whoever subscribes next.
    setlist.start_stream_pumps();

    // Stamping markers/regions above doesn't build a `Setlist` by itself —
    // that's a separate step (mirrors `SessionEngine::load_setlist`'s
    // step 4): read every currently-open project back into one.
    client
        .build_from_open_projects()
        .await
        .map_err(|e| eyre::eyre!("build_from_open_projects: {e:?}"))?;

    let handle = Arc::new(DemoHandle {
        client,
        stream_client,
        _daw: bundle,
        _scope: scope,
    });
    // Single-threaded (wasm) — no real race on this cache.
    let _ = HANDLE.set(Arc::clone(&handle));
    Ok(handle)
}
