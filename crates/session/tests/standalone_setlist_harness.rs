//! Ephemeral setlist harness — no REAPER.
//!
//! Two layers of fidelity, both backed by `daw-standalone` over in-process
//! vox memory links (no REAPER, no socket):
//!
//! * `build_setlist_from_standalone_demo` — drives `SetlistBuilder` directly
//!   against a `Daw` client. Proves the build logic + the daw RPC path.
//! * `setlist_service_over_vox` — mirrors the desktop exactly: hosts
//!   `SetlistServiceImpl` behind a `LayerRouter` over vox, then drives it
//!   through a `SetlistServiceClient` (`build_from_open_projects` + the
//!   `subscribe(Tx<SetlistEvent>)` stream). This is the path the desktop uses.
//!
//! Run: cargo test -p session --test standalone_setlist_harness -- --nocapture

use std::time::Duration;

use daw_proto::ProjectInfo;
use daw_standalone::bootstrap::build_in_process_daw;
use daw_standalone::sync::Standalone;
use session::setlist_service::demo::stamp_demo_setlist_with;
use session::{
    SetlistBuilder, SetlistEvent, SetlistServiceClient, SetlistServiceImpl, serve_setlist_service,
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

/// Full desktop path over vox, no REAPER: host SetlistService behind a
/// LayerRouter, drive it through SetlistServiceClient — subscribe, build, and
/// confirm the SetlistChanged push reaches the subscriber.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "blocked on vox-postcard lower_enum index-OOB (fork f31f1040) — same bug repro_inprocess isolates"]
async fn setlist_service_over_vox() -> eyre::Result<()> {
    use vox::{ConnectionSettings, Parity};

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
    // the streaming behaviour).
    let _ = ConnectionSettings {
        parity: Parity::Even,
        max_concurrent_requests: 64,
        initial_channel_credit: 16,
    };
    let _ = Parity::Even;
    let sock = format!("/tmp/fts-setlist-harness-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&sock);
    let addr = format!("local://{sock}");
    let serve_addr = addr.clone();
    tokio::spawn(async move {
        if let Err(e) = vox::serve(&serve_addr, router).await {
            eprintln!("[v2] serve failed: {e:?}");
        }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    let client: SetlistServiceClient = vox::connect(&addr)
        .await
        .map_err(|e| eyre::eyre!("connect: {e:?}"))?;

    // Desktop's exact order: BUILD first, THEN subscribe, then pump. The
    // subscriber must receive the already-built setlist as the initial snapshot.
    client
        .build_from_open_projects()
        .await
        .map_err(|e| eyre::eyre!("build_from_open_projects: {e:?}"))?;
    println!("[v2] build_from_open_projects returned (service built over standalone)");

    match client.setlist().await {
        Ok(sl) => {
            println!("[v2] setlist() query: {} songs", sl.songs.len());
            assert!(!sl.songs.is_empty(), "service built an empty setlist");
        }
        Err(e) => panic!("[v2] setlist() error: {e:?}"),
    }

    // The subscribe handler runs its loop in-flight (never returns until the
    // stream ends), so poll the receiver CONCURRENTLY with the subscribe future
    // instead of awaiting subscribe() first. Reproduces the desktop/web-server
    // client pattern exactly.
    let (tx, mut rx) = vox::channel::<SetlistEvent>();
    let mut sub = std::pin::pin!(client.subscribe(tx));
    println!("[v2] subscribing (concurrent pump)");

    let mut got_songs = 0usize;
    let mut stream_ended = false;
    let end_at = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::time::Instant::now() >= end_at {
            break;
        }
        tokio::select! {
            res = &mut sub => {
                println!("[v2] subscribe() returned: {res:?} — stream ENDED");
                stream_ended = true;
                break;
            }
            ev = tokio::time::timeout(Duration::from_millis(400), rx.recv()) => {
                match ev {
                    Ok(Ok(Some(ev_ref))) => {
                        if let SetlistEvent::SetlistChanged(sl) = ev_ref.get() {
                            got_songs = sl.songs.len();
                            println!("[v2] SetlistChanged: {got_songs} songs");
                        }
                    }
                    Ok(Ok(None)) => {
                        println!("[v2] rx closed — stream ENDED (reproduces spin)");
                        stream_ended = true;
                        break;
                    }
                    Ok(Err(e)) => {
                        println!("[v2] rx error: {e:?}");
                        stream_ended = true;
                        break;
                    }
                    Err(_) => { /* idle tick; stream still open */ }
                }
            }
        }
    }

    println!("[v2] delivered={got_songs} songs, stream_ended={stream_ended}");
    assert!(
        got_songs > 0,
        "subscriber got no initial SetlistChanged snapshot"
    );
    assert!(
        !stream_ended,
        "subscription stream ended early — would cause the resubscribe spin"
    );
    Ok(())
}
