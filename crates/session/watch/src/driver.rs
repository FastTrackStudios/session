//! The relay, running: pings and feeds on a thread of its own, over
//! whatever reaches the watch.
//!
//! The app hands it a closure that reads what the phone knows (the live
//! set's presence, the shared clock, the song on screen and its timeline);
//! the thread does the rest. Its own thread rather than the app's runtime:
//! a ping's two stamps must be taken the moment it goes and the moment its
//! answer lands, not whenever an executor gets round to it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use daw_transport_sync::clock::now_micros_f64;
use session_proto::watch::{WatchMessage, WatchPong};

use crate::relay::{Lead, RelayInput, SongRef, WatchRelay};
use crate::timeline::GuideTimeline;
use crate::wire;

/// What reaches the watch app (`WatchConnectivity` on iOS; a simulated link
/// in tests).
pub trait WatchLink: Send + Sync {
    /// Whether the watch app is running and can be messaged now.
    fn reachable(&self) -> bool;
    /// Send one message; `reply` gets the watch's answer, if it gives one.
    fn send(&self, bytes: Vec<u8>, reply: Option<Box<dyn FnOnce(Vec<u8>) + Send>>);
    /// Leave the latest state for a watch app that is not running, to show
    /// when it opens (`WatchConnectivity`'s application context).
    fn remember(&self, _bytes: Vec<u8>) {}
}

/// What the phone knows, read once a tick — owned, because it is read on
/// the relay's thread.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub set_title: String,
    pub song: Option<SongSnapshot>,
    pub lead: Option<Lead>,
    /// `SharedClock::offset_micros` / `round_trip_micros`.
    pub shared_offset_us: Option<f64>,
    pub shared_round_trip_us: Option<f64>,
}

/// The song on the phone.
#[derive(Debug, Clone)]
pub struct SongSnapshot {
    /// The set's name for it (what the leader's stamp says).
    pub key: String,
    pub title: String,
    pub index: i32,
    pub count: u32,
    /// Built once per song ([`GuideTimeline::build`]) and shared.
    pub timeline: Arc<GuideTimeline>,
}

/// How often the watch is pinged. Four a second fills the estimator's
/// window of 32 in eight seconds, without crowding a Bluetooth link the
/// feeds share.
const PING_EVERY: Duration = Duration::from_millis(250);

/// The relay's heartbeat: fine enough that a start or a jump goes out
/// within a frame of being seen.
const TICK: Duration = Duration::from_millis(20);

/// How often a watch that is not running is left the latest state.
const REMEMBER_EVERY_US: f64 = 2_000_000.0;

/// A running relay; dropping it stops it.
pub struct Relay {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    state: Arc<Mutex<WatchRelay>>,
}

impl Relay {
    /// Start relaying to `link`, reading the phone through `read`.
    pub fn spawn(
        link: Arc<dyn WatchLink>,
        mut read: impl FnMut() -> Snapshot + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(WatchRelay::new()));
        let (stopping, relay) = (Arc::clone(&stop), Arc::clone(&state));
        let thread = std::thread::Builder::new()
            .name("watch-relay".into())
            .spawn(move || {
                let mut next_ping = 0.0;
                let mut remembered = f64::NEG_INFINITY;
                let mut was_locked = false;
                while !stopping.load(Ordering::Relaxed) {
                    let now = now_micros_f64();
                    let reachable = link.reachable();
                    if reachable && now >= next_ping {
                        next_ping = now + micros(PING_EVERY);
                        ping(link.as_ref(), &relay, now);
                    }
                    let snapshot = read();
                    let feed = relay
                        .lock()
                        .ok()
                        .and_then(|mut r| r.tick(&input(&snapshot), now_micros_f64()));
                    if let Some(feed) = feed {
                        if feed.clock_locked != was_locked {
                            was_locked = feed.clock_locked;
                            let (offset, rtt) = relay
                                .lock()
                                .map(|r| (r.watch_offset_us(), r.watch_round_trip_us()))
                                .unwrap_or_default();
                            tracing::info!(
                                watch.locked = was_locked,
                                watch.offset_us = offset,
                                watch.round_trip_us = rtt,
                                watch.clock_error_us = feed.clock_error_us,
                                "watch relay: clock {}",
                                if was_locked { "locked" } else { "lost" }
                            );
                        }
                        let message = WatchMessage {
                            ping: None,
                            feed: Some(feed),
                        };
                        if let Some(bytes) = wire::encode(&message) {
                            if reachable {
                                link.send(bytes, None);
                            } else if now - remembered >= REMEMBER_EVERY_US {
                                remembered = now;
                                link.remember(bytes);
                            }
                        }
                    }
                    std::thread::sleep(TICK);
                }
            })
            .ok();
        Self {
            stop,
            thread,
            state,
        }
    }

    /// `watch − phone` and its round trip, µs, once known.
    #[must_use]
    pub fn watch_clock(&self) -> (Option<f64>, Option<f64>) {
        self.state
            .lock()
            .map(|r| (r.watch_offset_us(), r.watch_round_trip_us()))
            .unwrap_or_default()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One ping: stamped as it leaves, and its answer as it lands.
fn ping(link: &dyn WatchLink, relay: &Arc<Mutex<WatchRelay>>, now: f64) {
    let Some(bytes) = wire::encode(&WatchMessage {
        ping: Some(WatchRelay::ping(now)),
        feed: None,
    }) else {
        return;
    };
    let relay = Arc::clone(relay);
    link.send(
        bytes,
        Some(Box::new(move |answer: Vec<u8>| {
            let landed = now_micros_f64();
            if let Some(pong) = wire::decode::<WatchPong>(&answer)
                && let Ok(mut r) = relay.lock()
            {
                r.pong(&pong, landed);
            }
        })),
    );
}

fn input(s: &Snapshot) -> RelayInput<'_> {
    RelayInput {
        set_title: &s.set_title,
        song: s.song.as_ref().map(|song| SongRef {
            key: &song.key,
            title: &song.title,
            index: song.index,
            count: song.count,
            timeline: &song.timeline,
        }),
        lead: s.lead.as_ref(),
        shared_offset_us: s.shared_offset_us,
        shared_round_trip_us: s.shared_round_trip_us,
    }
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}
