//! Ephemeral setlist harness — no REAPER.
//!
//! Two layers of fidelity, both backed by `daw-standalone` over in-process
//! vox memory links (no REAPER, no socket):
//!
//! * `build_setlist_from_standalone_demo` — drives `SetlistBuilder` directly
//!   against a `Daw` client. Proves the build logic + the daw RPC path.
//! * `setlist_service_over_vox` — mirrors the desktop exactly: hosts
//!   `SetlistServiceImpl` behind a `LayerRouter` over vox, then drives it
//!   through a `SetlistServiceClient` (`build_from_open_projects`, `setlist`,
//!   `seek_to_section`, active-song queries). Live streaming rides the
//!   `events` / `active_indices` `#[subscribe]` streams (architect PubSub).
//!
//! Run: cargo test -p session --test standalone_setlist_harness -- --nocapture

use std::time::Duration;

use daw_proto::ProjectInfo;
use daw_standalone::bootstrap::build_in_process_daw;
use daw_standalone::sync::Standalone;
use session::setlist_service::demo::stamp_demo_setlist_with;
use session::{
    SetlistBuilder, SetlistServiceClient, SetlistServiceImpl, serve_setlist_service,
    setlist_service_service_descriptor,
};

fn seeded_stamped() -> Standalone {
    let standalone = Standalone::new();
    standalone.seed_project(ProjectInfo {
        guid: "demo-proj".into(),
        name: "Demo".into(),
        path: String::new(),
    });
    stamp_demo_setlist_with(&standalone).expect("stamp demo setlist");
    standalone
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_setlist_from_standalone_demo() -> eyre::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let bundle = build_in_process_daw(seeded_stamped()).await?;
    let setlist = SetlistBuilder::build_from_open_projects(&bundle.daw).await?;

    println!(
        "[v1] built '{}' with {} songs",
        setlist.name,
        setlist.songs.len()
    );
    assert!(!setlist.songs.is_empty(), "expected demo songs");
    Ok(())
}

/// Rich scenario: 10 projects, one song each, varied complex section layouts
/// and alternating count-in. Verifies the full setlist view structure builds
/// correctly over the daw facade (no REAPER).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rich_setlist_ten_projects_one_song_each() -> eyre::Result<()> {
    use daw::service::ProjectContext;
    use session::setlist_service::demo::{fixture_songs, stamp_song_native};

    let _ = tracing_subscriber::fmt::try_init();

    let standalone = Standalone::new();
    let songs = fixture_songs(10);
    for (i, song) in songs.iter().enumerate() {
        let guid = standalone.seed_project(ProjectInfo {
            guid: format!("proj-{i:02}"),
            name: format!("Project {i:02}"),
            path: String::new(),
        });
        stamp_song_native(&standalone, ProjectContext::Project(guid), song)
            .map_err(|e| eyre::eyre!("stamp song {i}: {e:?}"))?;
    }

    let bundle = build_in_process_daw(standalone).await?;
    let setlist = SetlistBuilder::build_from_open_projects(&bundle.daw).await?;

    println!("[v3] {} songs:", setlist.songs.len());
    for s in &setlist.songs {
        println!(
            "  - {:<24} | {:>2} sections | count_in={:?}",
            s.name,
            s.sections.len(),
            s.count_in_seconds
        );
    }

    // One song per project, all 10 present.
    assert_eq!(
        setlist.songs.len(),
        10,
        "expected 10 songs (one per project)"
    );
    // Every song has sections.
    for s in &setlist.songs {
        assert!(!s.sections.is_empty(), "song '{}' has no sections", s.name);
    }
    // Complex section work: at least one song with a deep layout (>= 8 sections).
    let max_sections = setlist
        .songs
        .iter()
        .map(|s| s.sections.len())
        .max()
        .unwrap_or(0);
    assert!(
        max_sections >= 8,
        "expected a complex song with >=8 sections, got max {max_sections}"
    );
    // Count-in present on at least some songs.
    let with_count_in = setlist
        .songs
        .iter()
        .filter(|s| s.count_in_seconds.is_some_and(|c| c > 0.0))
        .count();
    println!("[v3] songs with count-in: {with_count_in}");
    assert!(
        with_count_in > 0,
        "expected at least one song with a count-in"
    );
    Ok(())
}

/// Structural contract for the demo setlist the desktop app plays: every
/// song must carry sections with real, ordered, positive-duration timing
/// (so the sidebar has sections to show and the player can seek into them),
/// plus tempo + time signature (so measure lengths are derivable). Built
/// through the same standalone → `SetlistBuilder` path the desktop uses —
/// no global daw, no vox.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn demo_setlist_songs_have_sections_with_timing() -> eyre::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let bundle = build_in_process_daw(seeded_stamped()).await?;
    let setlist = SetlistBuilder::build_from_open_projects(&bundle.daw).await?;

    println!("setlist '{}' — {} songs", setlist.name, setlist.songs.len());
    assert!(!setlist.songs.is_empty(), "demo setlist has no songs");

    for (si, song) in setlist.songs.iter().enumerate() {
        println!(
            "  song {si}: {:<28} {:>2} sections  tempo={:?} ts={:?}  {:.2}..{:.2}s",
            song.name,
            song.sections.len(),
            song.tempo,
            song.time_signature
                .as_ref()
                .map(|t| (t.numerator, t.denominator)),
            song.start_seconds,
            song.end_seconds,
        );
        assert!(
            !song.sections.is_empty(),
            "song '{}' has no sections",
            song.name
        );

        let mut prev_end: Option<f64> = None;
        for (ci, sec) in song.sections.iter().enumerate() {
            println!(
                "      [{ci}] {:<16} {:.2}..{:.2}s ({:.2}s)",
                sec.display_name(),
                sec.start_seconds,
                sec.end_seconds,
                sec.duration(),
            );
            assert!(
                sec.duration() > 0.0,
                "section '{}' in '{}' has non-positive duration ({:.3}s)",
                sec.name,
                song.name,
                sec.duration(),
            );
            if let Some(pe) = prev_end {
                assert!(
                    sec.start_seconds + 0.01 >= pe,
                    "sections in '{}' out of order at [{ci}] ({:.2} < prev end {:.2})",
                    song.name,
                    sec.start_seconds,
                    pe,
                );
            }
            prev_end = Some(sec.end_seconds);
        }

        assert!(
            song.tempo.is_some_and(|t| t > 0.0),
            "song '{}' has no positive tempo — measure length undefined",
            song.name
        );
        assert!(
            song.time_signature.is_some(),
            "song '{}' has no time signature",
            song.name
        );
    }
    Ok(())
}

/// End-to-end transport streaming, headless (no REAPER, no GUI): the
/// standalone playhead must advance AND its position ticks must reach a
/// consumer through the architect `daw.transport_events()` `#[subscribe]`
/// stream — the exact path that drives the UI playhead. Regression guard for
/// the transport→architect migration (no `SynchronizationEngine`, no poll
/// fallback). If this passes but the UI playhead is frozen, the break is in
/// the app's stream consumption, not the daw/session pipeline.
#[test]
fn transport_stream_delivers_advancing_position() -> eyre::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()?;
    rt.block_on(transport_stream_inner())
}

async fn transport_stream_inner() -> eyre::Result<()> {
    use daw_proto::transport::TransportStreamEvent;

    let _ = tracing_subscriber::fmt::try_init();

    // Isolated: drive the daw handle from THIS build directly (no global
    // `init_from_parts`) so parallel tests can't cross-wire the transport.
    let bundle = build_in_process_daw(seeded_stamped()).await?;
    let daw = bundle.daw.clone();

    // Subscribe to the architect transport stream first.
    let mut stream = daw.transport_events();

    // Put the playhead at a known spot inside song 0 and start playback. This
    // lazily creates the per-project transport engine (spawning the transport
    // bridge that publishes onto the hub) and advances the soft-clock playhead.
    let project = daw
        .project("demo-proj".to_string())
        .await
        .map_err(|e| eyre::eyre!("project: {e:?}"))?;
    project
        .transport()
        .set_position(4.0)
        .await
        .map_err(|e| eyre::eyre!("set_position: {e:?}"))?;
    project
        .transport()
        .play()
        .await
        .map_err(|e| eyre::eyre!("play: {e:?}"))?;

    // Collect position ticks for ~1.5s.
    let mut ticks = 0usize;
    let mut first_pos: Option<f64> = None;
    let mut last_pos = 0.0f64;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(300), stream.recv()).await {
            Ok(Ok(Some(ev))) => {
                if let TransportStreamEvent::Position(tick) = ev.get() {
                    let pos = tick.playhead.time.map(|t| t.as_seconds()).unwrap_or(0.0);
                    ticks += 1;
                    first_pos.get_or_insert(pos);
                    last_pos = pos;
                }
            }
            Ok(Ok(None)) | Ok(Err(_)) => break,
            Err(_) => { /* idle tick; keep waiting */ }
        }
    }

    let reported = project.transport().get_position().await.unwrap_or(-1.0);
    println!("[transport] ticks={ticks} first={first_pos:?} last={last_pos} reported={reported:.3}");

    assert!(
        ticks > 3,
        "no position ticks through daw.transport_events() (got {ticks}) — the playhead stream is not delivering"
    );
    assert!(
        last_pos > first_pos.unwrap_or(0.0) + 0.01,
        "playhead did not advance ({first_pos:?} -> {last_pos})"
    );
    Ok(())
}

/// Headless proof that the `SynchronizationEngine` is backend-agnostic:
/// a mutation on a daw-standalone project must surface as a `SyncEvent`
/// through the engine. The engine used to read `daw_reaper::event_hub()`
/// directly (REAPER-only); after the migration it consumes the daw facade's
/// architect event bus (`Daw::events()`), so it works against standalone too.
#[test]
fn sync_engine_backend_agnostic_over_standalone() -> eyre::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()?;
    rt.block_on(sync_engine_inner())
}

async fn sync_engine_inner() -> eyre::Result<()> {
    use daw_synchronization::{SyncConfig, SyncDomain, SyncSession, SynchronizationEngine};

    let _ = tracing_subscriber::fmt::try_init();

    let bundle = build_in_process_daw(seeded_stamped()).await?;
    let daw = bundle.daw.clone();

    let engine = SynchronizationEngine::new(
        daw.clone(),
        SyncSession {
            session_id: "test-session".into(),
            peer_id: "test-peer".into(),
            display_name: "Test".into(),
        },
        SyncConfig::all(),
    );
    engine
        .start()
        .await
        .map_err(|e| eyre::eyre!("engine start: {e:?}"))?;
    let mut rx = engine.subscribe();

    // Let the per-project bus forwarder finish its async subscribe.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Mutate the demo project via the facade — daw-standalone publishes these
    // onto the event bus, which the (now backend-agnostic) engine forwards.
    let project = daw
        .project("demo-proj".to_string())
        .await
        .map_err(|e| eyre::eyre!("project: {e:?}"))?;
    project
        .markers()
        .add(2.5, "SyncMarker")
        .await
        .map_err(|e| eyre::eyre!("marker add: {e:?}"))?;

    let mut got_marker = false;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
            Ok(Ok(ev)) => {
                println!("[sync] domain={:?}", std::mem::discriminant(&ev.domain));
                if matches!(ev.domain, SyncDomain::Marker(_)) {
                    got_marker = true;
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => { /* idle */ }
        }
    }

    assert!(
        got_marker,
        "sync engine produced no Marker SyncEvent from a standalone mutation — it is not backend-agnostic"
    );
    Ok(())
}

/// Full desktop path over vox, no REAPER: host SetlistService behind a
/// LayerRouter, drive it through SetlistServiceClient — build, query the
/// setlist, and confirm seeking selects the active song/section.
#[test]
fn setlist_service_over_vox() -> eyre::Result<()> {
    // Manual runtime instead of #[tokio::test]: vox 0.10's debug-build
    // channel encode recurses deeply on the Setlist payload, overflowing
    // tokio's default 2 MiB worker stacks — give the workers more room.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()?;
    rt.block_on(setlist_service_over_vox_inner())
}

async fn setlist_service_over_vox_inner() -> eyre::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Standalone backend, shared between the service's own handle and the
    // global `daw` facade. The service builds songs through `daw::get()`
    // (backend-agnostic), so wire the global facade to a standalone-backed
    // client — exactly as REAPER wires the global facade to the REAPER daw.
    let standalone = seeded_stamped();
    let bundle = build_in_process_daw(standalone.clone()).await?;
    // Current-thread runtime: init_from_parts only needs it for daw::block_on
    // (unused here, since everything is async), and a current-thread runtime
    // spawns no worker threads to keep the test process alive after it passes.
    let runtime = std::sync::Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    );
    // Single init: this now installs the one global Daw (owned by daw-control)
    // that both `daw::get()` and the service's background tasks resolve through.
    daw::init_from_parts(bundle.daw.clone(), runtime);

    let setlist_impl = SetlistServiceImpl::with_daw(standalone);
    let router = daw::LayerRouter::new().merge(daw::Mounted::new(
        setlist_service_service_descriptor(),
        serve_setlist_service(setlist_impl),
    ));

    // Serve over a real Unix socket — the desktop's actual transport — so this
    // exercises the same conduit/channel path REAPER uses (memory_link masked
    // the streaming behaviour). vox 0.10 lane model: hand every inbound lane
    // to the shared LayerRouter (dispatches by method id).
    let sock = format!("/tmp/fts-setlist-harness-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&sock);
    let addr = format!("local://{sock}");
    let serve_addr = addr.clone();
    tokio::spawn(async move {
        let lane_acceptor = vox::lane_acceptor_fn(move |_req, lane: vox::PendingLane| {
            lane.handle_with(router.clone());
            Ok(())
        });
        if let Err(e) = vox::serve(&serve_addr, lane_acceptor).await {
            eprintln!("[v2] serve failed: {e:?}");
        }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    /// Minimal `FromVoxLane` client capturing the lane's `Caller`.
    #[derive(Clone)]
    struct HarnessLaneClient {
        caller: vox::Caller,
    }
    impl vox::FromVoxLane for HarnessLaneClient {
        const SERVICE_NAME: &'static str = "setlist-harness";
        fn from_vox_lane(caller: vox::Caller, _connection: Option<vox::ConnectionHandle>) -> Self {
            Self { caller }
        }
    }

    let connection = vox::connect(&addr)
        .await
        .map_err(|e| eyre::eyre!("connect: {e:?}"))?;
    let lane = connection
        .open_lane::<HarnessLaneClient>()
        .await
        .map_err(|e| eyre::eyre!("open lane: {e:?}"))?;
    let client = SetlistServiceClient::new(lane.caller);

    // Desktop's exact order: BUILD first, THEN subscribe, then pump. The
    // subscriber must receive the already-built setlist as the initial snapshot.
    client
        .build_from_open_projects()
        .await
        .map_err(|e| eyre::eyre!("build_from_open_projects: {e:?}"))?;
    println!("[v2] build_from_open_projects returned (service built over standalone)");

    let expected_first_song = match client.setlist().await {
        Ok(sl) => {
            println!("[v2] setlist() query: {} songs", sl.songs.len());
            assert!(!sl.songs.is_empty(), "service built an empty setlist");
            let first = &sl.songs[0];
            assert!(
                !first.sections.is_empty(),
                "song 0 ('{}') built with no sections",
                first.name
            );
            first.name.clone()
        }
        Err(e) => panic!("[v2] setlist() error: {e:?}"),
    };

    // ── Selecting an active song (the sidebar-expand path) ──────────────
    //
    // The desktop app opens with nothing playing, so the ONLY way the
    // sidebar learns which song is active — and therefore which sections to
    // show — is a seek publishing the cursor. Reproduce that here over the
    // real vox path: song 0 must carry a measure grid, and seeking to it
    // (while STOPPED) must make song 0 / section 0 the active selection.
    //
    // (Live cursor/setlist *delivery* now rides the `active_indices` /
    // `events` `#[subscribe]` streams — exercised in-process by the app's
    // SessionEventBridge — rather than the old `subscribe(Tx)` verb.)

    let measures = client
        .measures(0)
        .await
        .map_err(|e| eyre::eyre!("measures(0): {e:?}"))?;
    println!("[v2] song 0 measures: {}", measures.len());
    assert!(!measures.is_empty(), "song 0 reports no measures");

    // Before the seek nothing is guaranteed active (demo cursor sits at the
    // timeline end) — active_song() may legitimately error; just log.
    match client.active_song().await {
        Ok(s) => println!("[v2] active song before seek: '{}'", s.name),
        Err(e) => println!("[v2] no active song before seek (expected): {e:?}"),
    }

    client
        .seek_to_section(0, 0)
        .await
        .map_err(|e| eyre::eyre!("seek_to_section(0,0): {e:?}"))?;

    let active_song = client
        .active_song()
        .await
        .map_err(|e| eyre::eyre!("active_song after seek: {e:?}"))?;
    let active_section = client
        .active_section()
        .await
        .map_err(|e| eyre::eyre!("active_section after seek: {e:?}"))?;
    println!(
        "[v2] after seek_to_section(0,0): active song='{}' section='{}'",
        active_song.name, active_section.name
    );
    assert_eq!(
        active_song.name, expected_first_song,
        "seek_to_section(0,0) did not make song 0 ('{expected_first_song}') active — the sidebar would never expand it"
    );
    assert_eq!(
        active_section.section_id, active_song.sections[0].section_id,
        "seek_to_section(0,0) did not make section 0 active"
    );

    Ok(())
}
