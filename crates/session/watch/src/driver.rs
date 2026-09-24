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
    /// Send one message; `done` gets the watch's answer once it is back
    /// (`None` if the send failed). The watch answers every message — a
    /// feed with nothing — so the relay knows when each has landed.
    fn send(&self, bytes: Vec<u8>, done: Box<dyn FnOnce(Option<Vec<u8>>) + Send>);
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
/// window of 32 in eight seconds.
const PING_EVERY: Duration = Duration::from_millis(250);

/// The relay's heartbeat: fine enough that a start or a jump goes out
/// within a frame of being seen.
const TICK: Duration = Duration::from_millis(20);

/// How often a watch that is not running is left the latest state.
const REMEMBER_EVERY_US: f64 = 2_000_000.0;

/// A message not answered in this long is taken as lost, µs.
const LOST_US: f64 = 3_000_000.0;

/// One message in flight at a time, pings and feeds alike.
///
/// A link slower than the messages offered to it queues them, and a queued
/// ping measures the queue: its round trip grows without bound, and since
/// the queue is on the way out only, the offset drifts with it (measured on
/// the simulator pair: a second of error a minute). Waiting for each answer
/// keeps the queue empty — a ping's trip is the link's own — and a feed that
/// could not go yet is replaced by a newer one rather than queued behind.
#[derive(Clone, Default)]
struct InFlight(Arc<Mutex<Option<f64>>>);

impl InFlight {
    /// Claim the link at `now` if nothing is in flight (or it was lost).
    fn claim(&self, now: f64) -> bool {
        let Ok(mut slot) = self.0.lock() else {
            return false;
        };
        if slot.is_some_and(|sent| now - sent < LOST_US) {
            return false;
        }
        *slot = Some(now);
        true
    }

    fn release(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }
}

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
                let flight = InFlight::default();
                let mut next_ping = 0.0;
                let mut remembered = f64::NEG_INFINITY;
                let mut was_locked = false;
                // The newest feed not yet sent, and whether it starts a run.
                let mut pending: Option<(Vec<u8>, bool)> = None;
                let mut last_run = None;
                while !stopping.load(Ordering::Relaxed) {
                    let now = now_micros_f64();
                    let reachable = link.reachable();
                    let snapshot = read();
                    let feed = relay
                        .lock()
                        .ok()
                        .and_then(|mut r| r.tick(&input(&snapshot), now));
                    if let Some(feed) = feed {
                        if feed.clock_locked != was_locked {
                            was_locked = feed.clock_locked;
                            log_lock(&relay, was_locked, feed.clock_error_us);
                        }
                        let new_run = last_run != Some(feed.run);
                        last_run = Some(feed.run);
                        if let Some(bytes) = wire::encode(&WatchMessage {
                            ping: None,
                            feed: Some(feed),
                        }) {
                            if reachable {
                                let urgent = new_run || pending.as_ref().is_some_and(|(_, u)| *u);
                                pending = Some((bytes, urgent));
                            } else if now - remembered >= REMEMBER_EVERY_US {
                                remembered = now;
                                link.remember(bytes);
                            }
                        }
                    }
                    if reachable {
                        // A new run goes first; then a due ping; then the
                        // latest feed.
                        let urgent = pending.as_ref().is_some_and(|(_, u)| *u);
                        if !urgent && now >= next_ping && flight.claim(now) {
                            next_ping = now + micros(PING_EVERY);
                            ping(link.as_ref(), &relay, &flight, now);
                        } else if pending.is_some()
                            && flight.claim(now)
                            && let Some((bytes, _)) = pending.take()
                        {
                            let done = flight.clone();
                            link.send(bytes, Box::new(move |_| done.release()));
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

/// The clock locking or losing its lock: one line, with what it rests on.
fn log_lock(relay: &Arc<Mutex<WatchRelay>>, locked: bool, clock_error_us: f64) {
    let (offset, rtt) = relay
        .lock()
        .map(|r| (r.watch_offset_us(), r.watch_round_trip_us()))
        .unwrap_or_default();
    tracing::info!(
        watch.locked = locked,
        watch.offset_us = offset,
        watch.round_trip_us = rtt,
        watch.clock_error_us = clock_error_us,
        "watch relay: clock {}",
        if locked { "locked" } else { "lost" }
    );
}

/// One ping: stamped as it leaves, and its answer as it lands.
fn ping(link: &dyn WatchLink, relay: &Arc<Mutex<WatchRelay>>, flight: &InFlight, now: f64) {
    let Some(bytes) = wire::encode(&WatchMessage {
        ping: Some(WatchRelay::ping(now)),
        feed: None,
    }) else {
        flight.release();
        return;
    };
    let (relay, flight) = (Arc::clone(relay), flight.clone());
    link.send(
        bytes,
        Box::new(move |answer: Option<Vec<u8>>| {
            let landed = now_micros_f64();
            flight.release();
            if let Some(pong) = answer.as_deref().and_then(wire::decode::<WatchPong>)
                && let Ok(mut r) = relay.lock()
            {
                r.pong(&pong, landed);
            }
        }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    type Job = (Vec<u8>, Box<dyn FnOnce(Option<Vec<u8>>) + Send>);

    /// A link that serves one message at a time — 30 ms out, 40 ms back —
    /// with a watch whose clock is a second ahead: what a Bluetooth link
    /// looks like to the relay, minus the noise.
    struct SlowLink {
        jobs: Mutex<mpsc::Sender<Job>>,
        served: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl SlowLink {
        fn new() -> Self {
            let (tx, rx) = mpsc::channel::<Job>();
            let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = Arc::clone(&served);
            std::thread::spawn(move || {
                for (bytes, done) in rx {
                    std::thread::sleep(Duration::from_millis(30));
                    let watch = now_micros_f64() + 1_000_000.0;
                    let answer = wire::decode::<WatchMessage>(&bytes)
                        .and_then(|m| m.ping)
                        .and_then(|p| {
                            wire::encode(&WatchPong {
                                sent_us: p.sent_us,
                                received_us: watch,
                                replied_us: watch,
                            })
                        })
                        .unwrap_or_default();
                    std::thread::sleep(Duration::from_millis(40));
                    count.fetch_add(1, Ordering::Relaxed);
                    done(Some(answer));
                }
            });
            Self {
                jobs: Mutex::new(tx),
                served,
            }
        }
    }

    impl WatchLink for SlowLink {
        fn reachable(&self) -> bool {
            true
        }

        fn send(&self, bytes: Vec<u8>, done: Box<dyn FnOnce(Option<Vec<u8>>) + Send>) {
            if let Ok(tx) = self.jobs.lock() {
                let _ = tx.send((bytes, done));
            }
        }
    }

    /// Offered more than it can carry (a feed and a ping every quarter
    /// second, each taking 70 ms), the link never queues: the round trip
    /// stays the link's own, and the offset is off by half its asymmetry.
    #[test]
    fn a_slow_link_does_not_queue_and_the_offset_holds() {
        let link = Arc::new(SlowLink::new());
        let relay = Relay::spawn(Arc::clone(&link) as Arc<dyn WatchLink>, Snapshot::default);
        std::thread::sleep(Duration::from_secs(3));
        let (offset, rtt) = relay.watch_clock();
        let (offset, rtt) = (offset.unwrap(), rtt.unwrap());
        assert!(rtt < 100_000.0, "round trip {rtt} µs: a queue built up");
        // True offset 1 s; the slower way back reads as 5 ms less.
        assert!(
            (offset - (1_000_000.0 - 5_000.0)).abs() < 3_000.0,
            "offset {offset}"
        );
        assert!(link.served.load(Ordering::Relaxed) >= 10, "pings kept flowing");
    }
}
