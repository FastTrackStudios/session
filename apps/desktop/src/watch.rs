//! The Session watch app, fed from this iPhone.
//!
//! The phone app runs the session in-process (`session_engine`), so the
//! watch follows what that engine plays: its current song's beat grid
//! (`session::guide::WatchTimelines` — the grid the Click track is stamped
//! from) and its transport's per-buffer snapshot, stamped on the audio
//! thread (daw-transport-sync). `session::watch_guide::Relay` does the rest
//! over WatchConnectivity: pings the watch's clock, and sends it the next
//! few seconds of beats already in it.
//!
//! The shared clock is this phone's own — nobody else is in the set. When
//! the phone app joins live sets (the Blitz app on iOS, #151), the feed
//! comes from the set's tick instead (`session_daw::watch_relay`), led by
//! the set's leader, in Task's clock.
//!
//! `FTS_DEMO=1` seeds the demo setlist into the engine and plays it — the
//! simulator pair test (see apps/session-watch/README.md).

use std::sync::{Arc, OnceLock};

use daw::service::{ProjectContext, Projects as _};
use session::watch_guide::daw_transport_sync::TransportBackend as _;
use session::watch_guide::{Lead, Relay, Snapshot, SongSnapshot};

/// The running relay (kept for the life of the app).
static RELAY: OnceLock<Relay> = OnceLock::new();

/// Start relaying to the watch, once the engine is up. A device with no
/// WatchConnectivity (an iPad) has nothing to do.
pub fn start() {
    let Some(engine) = crate::session_engine::engine() else { return };
    let Some(link) = session::watch_guide::link::WcLink::activate() else { return };
    let daw = engine.standalone.clone();
    let timelines = session::guide::WatchTimelines::new(daw.clone());
    // Making a project's sync backend may start its soft transport clock,
    // which needs a runtime to spawn on; the relay's thread has none.
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build() else {
        return;
    };
    let mut titles = Titles::default();
    let relay = Relay::spawn(Arc::new(link), move || {
        let _entered = runtime.enter();
        snapshot(&daw, &timelines, &mut titles)
    });
    tracing::info!(watch.link = "watch-connectivity", "watch relay: started");
    if RELAY.set(relay).is_ok() {
        report_clock();
    }
}

/// Every few seconds, at debug level: the relay's estimate of the watch's
/// clock (`watch − phone`), and — read at one instant — the phone's clock
/// against the host's continuous monotonic one, which is the clock the watch
/// answers pings with. On a simulator pair both run on one host, so the
/// second IS the true offset and the difference is the link's error. Only
/// with debug on (`RUST_LOG`): a thread nobody reads is not started.
fn report_clock() {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let Some(relay) = RELAY.get() else { return };
            let (offset, round_trip) = relay.watch_clock();
            let phone = session::watch_guide::daw_transport_sync::clock::now_micros_f64();
            let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            // SAFETY: `ts` is a valid timespec for the call to fill.
            if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &raw mut ts) } != 0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss, clippy::as_conversions)] // µs of uptime fit an f64 exactly
            let raw_minus_phone = (ts.tv_sec as f64).mul_add(1e6, ts.tv_nsec as f64 / 1_000.0) - phone;
            tracing::debug!(
                watch.offset_us = offset,
                watch.round_trip_us = round_trip,
                watch.host_offset_us = raw_minus_phone,
                watch.offset_error_us = offset.map(|o| o - raw_minus_phone),
                "watch relay: clock"
            );
        }
    });
}

/// What the phone is playing, as the relay reads it.
fn snapshot(
    daw: &daw_standalone::sync::Standalone,
    timelines: &session::guide::WatchTimelines<daw_standalone::sync::Standalone>,
    titles: &mut Titles,
) -> Snapshot {
    let Some(current) = daw.current() else { return Snapshot::default() };
    let projects = daw.list();
    // This phone's clock is the set's: its position needs no carrying.
    let lead = daw
        .sync_backend(&current.guid)
        .and_then(|b| b.snapshot())
        .map(|s| Lead::local(None, s.position(), 0.0));
    let playing = lead.as_ref().is_some_and(|l| l.position.is_playing);
    let song = timelines.get(&current.guid, playing).map(|timeline| SongSnapshot {
        key: current.guid.clone(),
        title: titles.of(daw, &current),
        index: projects
            .iter()
            .position(|p| p.guid == current.guid)
            .and_then(|i| i32::try_from(i).ok())
            .unwrap_or(-1),
        count: u32::try_from(projects.len()).unwrap_or(u32::MAX),
        timeline,
    });
    Snapshot {
        set_title: String::new(),
        song,
        lead,
        shared_offset_us: Some(0.0),
        shared_round_trip_us: Some(0.0),
    }
}

/// The song's name, read once per project: the current project's first
/// song (a project holding a setlist holds several; the grid is the
/// first's, as `Guide::watch_timeline`'s), else the project's own name.
#[derive(Default)]
struct Titles {
    last: Option<(String, String)>,
}

impl Titles {
    fn of(&mut self, daw: &daw_standalone::sync::Standalone, project: &daw_proto::ProjectInfo) -> String {
        if let Some((guid, title)) = &self.last
            && *guid == project.guid
        {
            return title.clone();
        }
        let title = session::SongBuilder::build_on(daw, ProjectContext::Current)
            .ok()
            .and_then(|songs| songs.into_iter().next())
            .map_or_else(|| project.name.clone(), |s| s.name);
        self.last = Some((project.guid.clone(), title.clone()));
        title
    }
}

/// `FTS_DEMO=1`: seed the demo setlist, build the setlist over it, and
/// play from the top — something to follow with no library on the device.
pub fn demo_if_asked() {
    if std::env::var("FTS_DEMO").ok().as_deref() != Some("1") {
        return;
    }
    let Some(engine) = crate::session_engine::engine() else { return };
    let (daw, client) = (engine.standalone.clone(), engine.client.clone());
    // The engine spawns onto a runtime as projects change (its transport's
    // soft clock among them), so this one lives as long as the app.
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build() else {
        return;
    };
    let runtime: &'static tokio::runtime::Runtime = Box::leak(Box::new(runtime));
    runtime.spawn(async move {
        daw.seed_project(daw_proto::ProjectInfo { guid: "watch-demo".into(), name: "Demo".into(), path: String::new() });
        if let Err(e) = session::setlist::service::demo::stamp_demo_setlist_with(&daw) {
            tracing::warn!(watch.error = ?e, "watch demo: the demo setlist was not stamped");
            return;
        }
        let built = client.build_from_open_projects().await;
        let seek = client.seek_to_section(0, 0).await;
        let play = client.play().await;
        tracing::info!(
            watch.demo_built = built.is_ok(),
            watch.demo_seek = seek.is_ok(),
            watch.demo_play = play.is_ok(),
            "watch demo: playing the demo setlist"
        );
    });
}
