//! The Session watch app, fed from the set this window is in.
//!
//! The live set's tick ([`crate::collab`]) hands this what it already
//! holds — the set's presence, the shared clock, the song on screen — once
//! a tick ([`live_tick`]); that is the whole of the hook. From it this
//! keeps the latest reading, and on iOS (feature `watch-link`) runs
//! `session::watch_guide::Relay` over WatchConnectivity, which pings the
//! watch's clock and sends it the next few seconds of beats in it.
//!
//! What leads the watch:
//!
//! - **playing together** — the shared transport's leader, by the stamp it
//!   publishes under its `sync` key (this window's own, when it leads):
//!   the same stamp a following window locks its engine to;
//! - **playing apart** — this window's own engine, by its per-buffer
//!   snapshot carried into the shared clock (Engine mode only: a Remote
//!   window has no engine of its own to stamp).
//!
//! Songs are matched by their library slug (`session::sync::slug`, what
//! `LiveSong.slug` and a leader's `SyncPosition.song` carry), whatever
//! spelling of the name each side holds.
//!
//! The song's beats come from `Guide::watch_timeline` on this window's
//! engine — the song, span and tempo map the Click track is stamped from —
//! kept current by `session::guide::WatchTimelines`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use session::sync::clock::SharedClock;
use session::sync::loro::LoroValue;
use session::sync::slug::slugify;
use session::watch_guide::{GuideTimeline, Lead, SongSnapshot, Snapshot};

/// What the last tick said.
#[derive(Default)]
struct Reading {
    at: Option<Instant>,
    set_title: String,
    /// The song on screen: its project here, and its slug.
    project: Option<String>,
    slug: Option<String>,
    title: String,
    index: i32,
    count: u32,
    lead: Option<Lead>,
    shared_offset_us: Option<f64>,
    shared_round_trip_us: Option<f64>,
}

static READING: Mutex<Option<Reading>> = Mutex::new(None);
/// The song's beats, on this window's engine (Engine mode only: a Remote
/// window has no project of its own to read the grid from).
static TIMELINES: std::sync::OnceLock<session::guide::WatchTimelines<daw::standalone::Standalone>> =
    std::sync::OnceLock::new();

/// A reading older than this is a set no longer ticking (left, or the
/// window gone): the watch is told there is no song.
const STALE: Duration = Duration::from_secs(1);

/// The live set's tick: everyone's presence, the set's clock, the song on
/// screen (the set's name for it) and the set's songs in order.
pub fn live_tick<'a>(
    seen: &HashMap<String, LoroValue>,
    clock: &SharedClock,
    here: Option<&str>,
    songs: impl Iterator<Item = &'a str>,
) {
    start_relay();
    let offset = clock.offset_micros();
    let slug = here.map(slugify);
    let songs: Vec<String> = songs.map(slugify).collect();
    let index = slug
        .as_ref()
        .and_then(|s| songs.iter().position(|k| k == s))
        .and_then(|i| i32::try_from(i).ok())
        .unwrap_or(-1);
    let project = crate::open::current_song();
    // A leader's stamp already names its song by slug.
    let lead = Lead::from_presence(seen).or_else(|| own_lead(project.as_deref(), slug.clone(), offset));
    let set_title = crate::collab::status().map(|s| s.set).unwrap_or_default();
    let reading = Reading {
        at: Some(Instant::now()),
        set_title,
        project,
        slug,
        title: here.unwrap_or_default().to_owned(),
        index,
        count: u32::try_from(songs.len()).unwrap_or(u32::MAX),
        lead,
        shared_offset_us: offset,
        shared_round_trip_us: clock.round_trip_micros(),
    };
    if let Ok(mut slot) = READING.lock() {
        *slot = Some(reading);
    }
}

/// This window's own engine as the lead: its per-buffer snapshot (in this
/// process's clock) carried into the shared one.
fn own_lead(project: Option<&str>, slug: Option<String>, offset: Option<f64>) -> Option<Lead> {
    let backend = crate::collab::sync_backend(project?)?;
    let snapshot = backend.snapshot()?;
    Some(Lead::local(slug, snapshot.position(), offset?))
}

/// What the relay reads, a tick at a time (on its own thread).
fn snapshot() -> Snapshot {
    let Ok(slot) = READING.lock() else { return Snapshot::default() };
    let Some(r) = slot.as_ref().filter(|r| r.at.is_some_and(|at| at.elapsed() < STALE)) else {
        return Snapshot::default();
    };
    let playing = r.lead.as_ref().is_some_and(|l| l.position.is_playing);
    let song = r.project.as_deref().and_then(|p| timeline(p, playing)).map(|timeline| SongSnapshot {
        key: r.slug.clone().unwrap_or_default(),
        title: r.title.clone(),
        index: r.index,
        count: r.count,
        timeline,
    });
    Snapshot {
        set_title: r.set_title.clone(),
        song,
        lead: r.lead.clone(),
        shared_offset_us: r.shared_offset_us,
        shared_round_trip_us: r.shared_round_trip_us,
    }
}

/// The song's beats (`session::guide::WatchTimelines`, on this window's
/// engine).
fn timeline(project: &str, playing: bool) -> Option<Arc<GuideTimeline>> {
    let timelines = match TIMELINES.get() {
        Some(t) => t,
        None => {
            let daw = crate::open::local_engine()?;
            TIMELINES.get_or_init(|| session::guide::WatchTimelines::new(daw.clone()))
        }
    };
    timelines.get(project, playing)
}

/// Start the WatchConnectivity relay once, where there is one (iOS, with
/// the `watch-link` feature); elsewhere the readings are kept and unused.
fn start_relay() {
    #[cfg(all(target_os = "ios", feature = "watch-link"))]
    {
        use std::sync::OnceLock;
        static RELAY: OnceLock<Option<session::watch_guide::Relay>> = OnceLock::new();
        RELAY.get_or_init(|| {
            let link = session::watch_guide::link::WcLink::activate()?;
            tracing::info!(watch.link = "watch-connectivity", "watch relay: started");
            Some(session::watch_guide::Relay::spawn(Arc::new(link), snapshot))
        });
    }
}

/// What the relay would send now — for a probe or a test on any platform.
#[must_use]
pub fn current() -> Snapshot {
    snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::sync::transport::{self, SharedTransport, SyncPosition, TransportMode};

    /// Playing together, the watch follows the shared transport's leader —
    /// by the stamp it publishes, naming the song by slug — and knows the
    /// song on screen by the same slug, whatever spelling this window holds.
    #[test]
    fn together_the_watch_follows_the_leaders_stamp() {
        let position = daw_transport_sync::Position {
            host_micros: 2.0e6,
            playhead_seconds: 12.5,
            playrate: 1.0,
            is_playing: true,
        };
        let mut seen = HashMap::new();
        let entry = SharedTransport {
            mode: TransportMode::Shared,
            playing: true,
            position: 12.5,
            at_ms: 2_000.0,
            song: Some("God, I'm Just Grateful".into()),
            seq: 3,
            by: "peer-lead".into(),
        };
        seen.insert(transport::KEY.to_owned(), entry.encode());
        seen.insert(
            transport::sync_key("peer-lead"),
            SyncPosition { song: Some(slugify("God, I'm Just Grateful")), position }.encode(),
        );
        let songs = ["Washed", "God, I’m Just Grateful", "Always On Time"];
        live_tick(&seen, &SharedClock::owned(), Some("God, I'm Just Grateful"), songs.into_iter());

        let reading = READING.lock().ok().and_then(|r| r.as_ref().map(|r| (r.slug.clone(), r.index, r.count)));
        assert_eq!(reading, Some((Some("god-im-just-grateful".to_owned()), 1, 3)));
        let lead = current().lead.expect("the leader's stamp leads");
        assert_eq!(lead.song.as_deref(), Some("god-im-just-grateful"));
        assert_eq!(lead.position, position);
    }
}
