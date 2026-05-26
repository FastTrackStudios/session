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
    SetlistBuilder, SetlistEvent, SetlistServiceClient, SetlistServiceImpl,
    serve_setlist_service, setlist_service_service_descriptor,
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

    println!("[v1] built '{}' with {} songs", setlist.name, setlist.songs.len());
    assert!(!setlist.songs.is_empty(), "expected demo songs");
    Ok(())
}

/// Full desktop path over vox, no REAPER: host SetlistService behind a
/// LayerRouter, drive it through SetlistServiceClient — subscribe, build, and
/// confirm the SetlistChanged push reaches the subscriber.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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
    let _ = ConnectionSettings { parity: Parity::Even, max_concurrent_requests: 64, initial_channel_credit: 16 };
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

    let (tx, mut rx) = vox::channel::<SetlistEvent>();
    client
        .subscribe(tx)
        .await
        .map_err(|e| eyre::eyre!("subscribe: {e:?}"))?;
    println!("[v2] subscribed over vox — OK");

    // The subscriber must get the initial SetlistChanged snapshot and the stream
    // must stay open (the desktop spun resubscribing because the stream ended).
    let mut got_songs = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            break;
        }
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Ok(Some(ev_ref))) => {
                if let SetlistEvent::SetlistChanged(sl) = ev_ref.get() {
                    got_songs = sl.songs.len();
                    break;
                }
            }
            Ok(Ok(None)) => {
                println!("[v2] stream ended (rx closed) — reproduces desktop spin");
                break;
            }
            _ => break,
        }
    }
    println!("[v2] SetlistChanged delivered to subscriber: {got_songs} songs");
    assert!(got_songs > 0, "subscriber got no initial SetlistChanged snapshot");
    Ok(())
}
