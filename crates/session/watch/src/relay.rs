//! The phone's side: turn where the band is into a few seconds of beats on
//! the watch's clock.
//!
//! Three clocks are in play, and only one is anyone's to trust:
//!
//! - **Task's** (the shared clock) — every position in a live set is
//!   stamped in it. The phone already follows it: its
//!   [`session_sync::clock::SharedClock`] pings Task and holds `shared −
//!   phone`.
//! - **The phone's** — the monotonic clock of this process
//!   ([`daw_transport_sync::clock`]), which that offset is against.
//! - **The watch's** — its continuous monotonic clock, which is what its
//!   timers can fire on. [`WatchRelay`] pings it over `WatchConnectivity` and
//!   keeps `watch − phone` in the same [`ClockEstimator`] the shared clock
//!   uses (the fastest quarter of the round trips, smoothed), so the watch
//!   never estimates anything itself.
//!
//! A beat's instant is then plain arithmetic, done here: the leader's
//! stamped playhead ([`SyncPosition`], the very stamp a following desktop
//! locks its engine to) says when the beat sounds in Task's clock; the two
//! offsets carry it to the watch's. The watch is sent the next few seconds
//! of those, and taps them.

use std::collections::HashMap;

use daw_transport_sync::{ClockEstimator, Position};
use session_proto::watch::{WatchBeat, WatchGuideFeed, WatchPing, WatchPong};
use session_sync::loro::LoroValue;
use session_sync::transport::{self, SharedTransport, SyncPosition, TransportMode};

use crate::timeline::GuideTimeline;

/// How far ahead the watch is sent beats, seconds.
///
/// Long enough to ride out a `WatchConnectivity` hiccup; short enough that
/// a phone gone quiet stops the click within a few bars rather than leaving
/// it running blind.
pub const HORIZON: f64 = 6.0;

/// At most this many beats a feed (a very fast song's six seconds).
pub const MAX_BEATS: usize = 32;

/// How often a feed goes out while playing, and while stopped, µs.
const EVERY_PLAYING_US: f64 = 250_000.0;
const EVERY_STOPPED_US: f64 = 1_000_000.0;

/// How far the playhead may move from where the last feed put it before
/// it counts as a jump (a new run), seconds. The leader's stamps wander
/// by far less than this; a seek moves it by far more.
const JUMP: f64 = 0.020;

/// Who leads, and where: a playhead stamped in Task's clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Lead {
    /// The song it is on (the set's name for it), if said.
    pub song: Option<String>,
    /// The playhead, stamped in the shared clock (µs).
    pub position: Position,
}

impl Lead {
    /// The shared transport's leader, from the set's presence: whoever
    /// pressed last publishes their engine's playhead under their `sync`
    /// key while everyone plays together — the same stamp
    /// `session-daw`'s follower locks to. `None` when playing apart, or
    /// before the leader's first stamp.
    #[must_use]
    pub fn from_presence(states: &HashMap<String, LoroValue>) -> Option<Self> {
        let entry = states
            .get(transport::KEY)
            .and_then(SharedTransport::decode)?;
        if entry.mode != TransportMode::Shared {
            return None;
        }
        let stamped = states
            .get(&transport::sync_key(&entry.by))
            .and_then(SyncPosition::decode)?;
        Some(Self {
            song: stamped.song,
            position: stamped.position,
        })
    }

    /// This phone's own engine as the lead (playing apart, or leading):
    /// its snapshot's position (in the phone's clock) carried into the
    /// shared one, exactly as a leader stamps it for the others.
    #[must_use]
    pub fn local(song: Option<String>, position: Position, shared_minus_phone_us: f64) -> Self {
        Self {
            song,
            position: position.shifted(shared_minus_phone_us),
        }
    }
}

/// The song on the phone, as the watch sees it.
#[derive(Debug, Clone, Copy)]
pub struct SongRef<'a> {
    /// The set's name for it — what [`Lead::song`] says.
    pub key: &'a str,
    pub title: &'a str,
    /// Its place in the set, and the set's length.
    pub index: i32,
    pub count: u32,
    pub timeline: &'a GuideTimeline,
}

/// What the relay is given each tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct RelayInput<'a> {
    pub set_title: &'a str,
    pub song: Option<SongRef<'a>>,
    pub lead: Option<&'a Lead>,
    /// `shared − phone`, µs (`SharedClock::offset_micros`); `None` before
    /// the phone's first ping to Task is back.
    pub shared_offset_us: Option<f64>,
    /// Its round trip, µs (`SharedClock::round_trip_micros`).
    pub shared_round_trip_us: Option<f64>,
}

/// The phone's end of the watch link: the watch's clock, and what it was
/// last told.
#[derive(Debug, Clone)]
pub struct WatchRelay {
    watch: ClockEstimator,
    revision: u64,
    run: u64,
    /// What the last feed said: its song, whether it played, and its lead
    /// (to see a jump).
    last: Option<(Option<String>, bool, Position)>,
    last_sent_us: f64,
}

impl Default for WatchRelay {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchRelay {
    #[must_use]
    pub fn new() -> Self {
        Self {
            watch: ClockEstimator::default(),
            revision: 0,
            run: 0,
            last: None,
            last_sent_us: f64::NEG_INFINITY,
        }
    }

    /// A ping to send the watch now (`phone_now_us` on the phone's clock).
    #[must_use]
    pub const fn ping(phone_now_us: f64) -> WatchPing {
        WatchPing {
            sent_us: phone_now_us,
        }
    }

    /// The watch's answer, arrived at `phone_now_us`.
    pub fn pong(&mut self, pong: &WatchPong, phone_now_us: f64) {
        self.watch.record(
            pong.sent_us,
            pong.received_us,
            pong.replied_us,
            phone_now_us,
        );
    }

    /// `watch − phone`, µs, once the watch has answered.
    #[must_use]
    pub const fn watch_offset_us(&self) -> Option<f64> {
        self.watch.offset_micros()
    }

    /// The typical round trip to the watch, µs.
    #[must_use]
    pub fn watch_round_trip_us(&self) -> Option<f64> {
        self.watch.round_trip_micros()
    }

    /// The feed to send now, if one is due: on any change of run, and
    /// otherwise every quarter second while playing (every second while
    /// stopped) so a watch that missed one catches up.
    pub fn tick(&mut self, input: &RelayInput<'_>, phone_now_us: f64) -> Option<WatchGuideFeed> {
        let run = self.run;
        let feed = self.feed(input, phone_now_us);
        let every = if feed.playing {
            EVERY_PLAYING_US
        } else {
            EVERY_STOPPED_US
        };
        let due = feed.run != run || phone_now_us - self.last_sent_us >= every;
        if !due {
            return None;
        }
        self.last_sent_us = phone_now_us;
        Some(feed)
    }

    /// The feed for `phone_now_us`, whether or not one is due.
    pub fn feed(&mut self, input: &RelayInput<'_>, phone_now_us: f64) -> WatchGuideFeed {
        self.revision = self.revision.saturating_add(1);
        let shared_offset = input.shared_offset_us;
        let watch_offset = self.watch.offset_micros();
        let clock_locked = shared_offset.is_some() && watch_offset.is_some();
        // Unlocked, times are still given (the phone's own clock stands in)
        // so the face can move; the watch taps none of them.
        let shared_to_watch =
            |shared_us: f64| shared_us - shared_offset.unwrap_or(0.0) + watch_offset.unwrap_or(0.0);
        let now_shared = phone_now_us + shared_offset.unwrap_or(0.0);
        let clock_error_us = input
            .shared_round_trip_us
            .unwrap_or(0.0)
            .midpoint(self.watch.round_trip_micros().unwrap_or(0.0));

        let mut feed = WatchGuideFeed {
            revision: self.revision,
            run: self.run,
            set_title: input.set_title.to_owned(),
            song_index: -1,
            clock_locked,
            clock_error_us,
            ..WatchGuideFeed::default()
        };
        let Some(song) = input.song else {
            self.note(None, false, None);
            feed.run = self.run;
            return feed;
        };
        song.title.clone_into(&mut feed.song_title);
        feed.song_index = song.index;
        feed.song_count = song.count;
        feed.sections.clone_from(&song.timeline.sections);

        // The lead, if it is on this song.
        let lead = input
            .lead
            .filter(|l| l.song.as_deref().is_none_or(|s| s == song.key));
        let position_now = lead.map_or(song.timeline.start, |l| l.position.at(now_shared));
        let playing = lead.is_some_and(|l| l.position.is_playing && l.position.playrate > 0.0);
        self.note(Some(song.key), playing, lead.map(|l| l.position));
        feed.run = self.run;
        feed.playing = playing;

        let here_at = shared_to_watch(now_shared);
        feed.here = song.timeline.frame_at(position_now).cloned().map_or_else(
            WatchBeat::default,
            |mut b| {
                b.at_us = here_at;
                b.progress = song.timeline.progress(position_now);
                b
            },
        );

        if let Some(lead) = lead.filter(|_| playing) {
            let rate = lead.position.playrate;
            let until = HORIZON.mul_add(rate, position_now);
            feed.beats = song
                .timeline
                .beats_between(position_now, until)
                .iter()
                .take(MAX_BEATS)
                .map(|b| {
                    // When the leader's playhead reaches this beat, in
                    // Task's clock, then in the watch's.
                    let shared_us = ((b.position - lead.position.playhead_seconds) / rate)
                        .mul_add(1e6, lead.position.host_micros);
                    WatchBeat {
                        at_us: shared_to_watch(shared_us),
                        ..b.clone()
                    }
                })
                .collect();
        }
        feed
    }

    /// Record what this feed says, starting a new run on a start, a stop,
    /// a jump or another song.
    fn note(&mut self, song: Option<&str>, playing: bool, lead: Option<Position>) {
        let now = lead.unwrap_or(Position {
            host_micros: 0.0,
            playhead_seconds: 0.0,
            playrate: 1.0,
            is_playing: false,
        });
        let changed = match &self.last {
            None => true,
            Some((was_song, was_playing, was)) => {
                was_song.as_deref() != song
                    || *was_playing != playing
                    // Where the last lead would be at this lead's instant.
                    || (playing && (was.at(now.host_micros) - now.playhead_seconds).abs() > JUMP)
            }
        };
        if changed {
            self.run = self.run.saturating_add(1);
        }
        self.last = Some((song.map(str::to_owned), playing, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session_sync::transport::TransportMode;

    fn timeline() -> GuideTimeline {
        GuideTimeline::build(&crate::timeline::tests::song(), &[])
    }

    fn input<'a>(t: &'a GuideTimeline, lead: Option<&'a Lead>) -> RelayInput<'a> {
        RelayInput {
            set_title: "Set",
            song: Some(SongRef {
                key: "test-song",
                title: "Test Song",
                index: 0,
                count: 3,
                timeline: t,
            }),
            lead,
            shared_offset_us: Some(5_000_000.0),
            shared_round_trip_us: Some(20_000.0),
        }
    }

    /// The leader plays from 4.0 s at shared 10 s; the phone is 5 s behind
    /// the shared clock and the watch 1 s ahead of the phone.
    #[test]
    fn beats_land_on_the_watch_clock() {
        let t = timeline();
        let lead = Lead {
            song: Some("test-song".into()),
            position: Position {
                host_micros: 10_000_000.0,
                playhead_seconds: 4.0,
                playrate: 1.0,
                is_playing: true,
            },
        };
        let mut relay = WatchRelay::new();
        // A perfectly symmetric exchange: watch = phone + 1 s.
        for i in 0..8 {
            let sent = f64::from(i) * 100_000.0;
            relay.pong(
                &WatchPong {
                    sent_us: sent,
                    received_us: sent + 1_010_000.0,
                    replied_us: sent + 1_010_000.0,
                },
                sent + 20_000.0,
            );
        }
        // Phone time 5.0 s = shared 10.0 s: the downbeat at 4.0 s is now.
        let feed = relay.feed(&input(&t, Some(&lead)), 5_000_000.0);
        assert!(feed.clock_locked && feed.playing);
        assert_eq!(feed.here.bar, 1);
        // The next beat (4.5 s) sounds half a second on: phone 5.5 s, watch 6.5 s.
        let first = &feed.beats[0];
        assert!((first.position - 4.5).abs() < 1e-9);
        assert!((first.at_us - 6_500_000.0).abs() < 1.0, "{}", first.at_us);
        assert_eq!(feed.beats.len(), 12, "six seconds of 120 bpm");
    }

    #[test]
    fn a_jump_or_a_stop_starts_a_new_run_and_drift_does_not() {
        let grid = timeline();
        let mut relay = WatchRelay::new();
        let at = |host: f64, head: f64, playing: bool| Lead {
            song: None,
            position: Position {
                host_micros: host,
                playhead_seconds: head,
                playrate: 1.0,
                is_playing: playing,
            },
        };
        let started = relay
            .feed(&input(&grid, Some(&at(0.0, 4.0, true))), 0.0)
            .run;
        // A fresh stamp half a second on, 2 ms off: the same run.
        let drifted = relay
            .feed(&input(&grid, Some(&at(500_000.0, 4.502, true))), 0.0)
            .run;
        assert_eq!(started, drifted);
        // A seek.
        let sought = relay
            .feed(&input(&grid, Some(&at(600_000.0, 20.0, true))), 0.0)
            .run;
        assert_ne!(drifted, sought);
        // A stop.
        let stopped = relay.feed(&input(&grid, Some(&at(700_000.0, 20.1, false))), 0.0);
        assert_ne!(sought, stopped.run);
        assert!(stopped.beats.is_empty() && !stopped.playing);
    }

    #[test]
    fn unlocked_until_the_watch_answers() {
        let t = timeline();
        let lead = Lead {
            song: None,
            position: Position {
                host_micros: 0.0,
                playhead_seconds: 4.0,
                playrate: 1.0,
                is_playing: true,
            },
        };
        let feed = WatchRelay::new().feed(&input(&t, Some(&lead)), 0.0);
        assert!(!feed.clock_locked);
    }

    #[test]
    fn the_leader_is_read_from_presence() {
        let mut states = HashMap::new();
        let entry = SharedTransport {
            mode: TransportMode::Shared,
            playing: true,
            position: 4.0,
            at_ms: 0.0,
            song: Some("test-song".into()),
            seq: 7,
            by: "peer-a".into(),
        };
        states.insert(transport::KEY.to_owned(), entry.encode());
        assert_eq!(Lead::from_presence(&states), None, "no stamp yet");
        let position = Position {
            host_micros: 1.0e6,
            playhead_seconds: 4.0,
            playrate: 1.0,
            is_playing: true,
        };
        states.insert(
            transport::sync_key("peer-a"),
            SyncPosition {
                song: Some("test-song".into()),
                position,
            }
            .encode(),
        );
        assert_eq!(
            Lead::from_presence(&states),
            Some(Lead {
                song: Some("test-song".into()),
                position
            })
        );
    }
}
