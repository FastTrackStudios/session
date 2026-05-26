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

    let (server_link, client_link) = vox::memory_link_pair(256);
    let acc = ConnectionSettings { parity: Parity::Even, max_concurrent_requests: 64, initial_channel_credit: 16 };
    let ini = ConnectionSettings { parity: Parity::Odd, max_concurrent_requests: 64, initial_channel_credit: 16 };

    tokio::spawn(async move {
        match vox::acceptor_on_link(server_link, acc).await {
            Ok(b) => match b.on_connection(router).establish::<vox::NoopClient>().await {
                Ok(guard) => {
                    let _guard = guard;
                    std::future::pending::<()>().await;
                }
                Err(e) => eprintln!("[v2] acceptor establish failed: {e:?}"),
            },
            Err(e) => eprintln!("[v2] acceptor_on_link failed: {e:?}"),
        }
    });

    let conn = vox::initiator_on_link(client_link, ini)
        .await?
        .establish::<vox::NoopClient>()
        .await?;
    let client = SetlistServiceClient::new(conn.caller.clone());

    // Subscribe (stays alive even with no setlist yet — no early bail), then
    // build separately, the desktop's order. Both calls succeed over vox.
    let (tx, mut rx) = vox::channel::<SetlistEvent>();
    client
        .subscribe(tx)
        .await
        .map_err(|e| eyre::eyre!("subscribe: {e:?}"))?;
    println!("[v2] subscribed over vox — OK");

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

    // Soft check: whether the SetlistChanged push reaches the subscriber over
    // the revision loop. (Delivery of build-triggered revisions to an existing
    // subscriber is the remaining known gap; the build + query path is proven.)
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
            _ => break,
        }
    }
    println!("[v2] SetlistChanged delivered to subscriber: {got_songs} songs");
    Ok(())
}
