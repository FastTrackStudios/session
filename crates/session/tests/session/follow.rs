//! Following a playhead with no engine: a chart or a lighting rig on
//! another machine, its clock elsewhere, over a jittery network.

use session::sync::{ClockEstimator, Schedule, Tracker};
use session::time::TimeSignature;
use session::{ExtensionId, Item, ItemId, LaneId, Session, TempoMap, Time, Transport, Value};

use crate::close;

/// Deterministic jitter.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        f64::from(u32::try_from(self.0 >> 40).unwrap()) / f64::from(1_u32 << 24)
    }
}

/// The follower's clock reads `shared − OFFSET`.
const OFFSET: f64 = 3_700_000.0;

/// What the leader's transport is at shared-clock `t` (µs): playing from 0
/// at 0.5 s, a seek to 30 s at 6 s, stopped at 9 s.
fn truth(t: f64) -> Transport {
    if t < 500_000.0 {
        Transport::stopped(0.0, 0.0)
    } else if t < 6_000_000.0 {
        Transport::playing(0.0, 500_000.0, 1.0)
    } else if t < 9_000_000.0 {
        Transport::playing(30.0, 6_000_000.0, 1.0)
    } else {
        Transport::stopped(33.0, 9_000_000.0)
    }
}

#[test]
fn a_follower_on_another_clock_tracks_the_playhead_within_a_millisecond() {
    let mut rng = Lcg(7);
    let mut clock = ClockEstimator::default();
    let mut tracker = Tracker::default();
    // In flight: (arrives at shared µs, the transport sent).
    let mut wire: Vec<(f64, Transport)> = Vec::new();
    let changes = [500_000.0, 6_000_000.0, 9_000_000.0];
    let mut last_sent = f64::NEG_INFINITY;
    let mut worst: f64 = 0.0;
    let mut frames_checked = 0;
    let mut t = 0.0;
    while t < 11_000_000.0 {
        // The leader publishes a change at once and a keepalive each second,
        // stamped with when it was true.
        let changed = changes.iter().any(|c| *c <= t && *c > last_sent);
        if changed || t - last_sent >= 1_000_000.0 {
            let mut sent = truth(t);
            sent.playhead_seconds = sent.at(t);
            sent.host_micros = t;
            let delay = 5_000.0 + 75_000.0 * rng.next();
            wire.push((t + delay, sent));
            last_sent = t;
        }
        // Ten pings a second, slower one way than the other at random.
        if (t / 100_000.0).fract() < 1e-9 {
            let there = 2_000.0 + 20_000.0 * rng.next();
            let back = 2_000.0 + 20_000.0 * rng.next();
            let t1 = t - OFFSET;
            clock.record(t1, t + there, t + there, t + there + back - OFFSET);
        }
        // Messages arrive, in whatever order the network chose.
        let (arrived, flying): (Vec<_>, Vec<_>) = wire.into_iter().partition(|(at, _)| *at <= t);
        wire = flying;
        for (_, sent) in arrived {
            tracker.observe(sent);
        }
        // A frame: where to draw the playhead.
        let local = t - OFFSET;
        let shown = clock
            .offset_micros()
            .and_then(|offset| tracker.position(local, offset));
        let settling = t < 1_000_000.0 || changes.iter().any(|c| t >= *c && t < c + 120_000.0);
        if let (Some(shown), false) = (shown, settling) {
            let exact = truth(t).at(t);
            worst = worst.max((shown - exact).abs());
            frames_checked += 1;
        }
        t += 16_000.0;
    }
    assert!(frames_checked > 500, "{frames_checked}");
    assert!(worst < 0.001, "worst error {worst} s");
}

#[test]
fn a_stale_message_never_moves_the_playhead_back() {
    let mut tracker = Tracker::default();
    tracker.observe(Transport::playing(10.0, 2_000_000.0, 1.0));
    // A message from before, arriving late.
    tracker.observe(Transport::playing(0.0, 1_000_000.0, 1.0));
    close(tracker.exact(3_000_000.0, 0.0).unwrap(), 11.0, 1e-9);
}

#[test]
fn a_small_correction_slews_and_a_seek_jumps() {
    let mut tracker = Tracker::default();
    tracker.observe(Transport::playing(0.0, 0.0, 1.0));
    close(tracker.position(1_000_000.0, 0.0).unwrap(), 1.0, 1e-9);
    // The leader says it is 10 ms further on than it seemed.
    tracker.observe(Transport::playing(1.010, 1_000_000.0, 1.0));
    let next = tracker.position(1_016_000.0, 0.0).unwrap();
    assert!(next > 1.016 && next < 1.026, "slewed, not jumped: {next}");
    // A seek shows at once.
    tracker.observe(Transport::playing(42.0, 1_020_000.0, 1.0));
    close(tracker.position(1_032_000.0, 0.0).unwrap(), 42.012, 1e-9);
}

#[test]
fn a_chart_follows_bars_through_a_tempo_change() {
    // 120 BPM in 4/4, 60 BPM from bar 3 (4 s).
    let tempo = TempoMap::new(
        120.0,
        TimeSignature::COMMON_TIME,
        vec![TempoMap::point(4.0, 60.0, None)],
    );
    let t = Transport::playing(0.0, 0.0, 1.0);
    let bar = |seconds: f64| t.bar_beat_at(seconds * 1e6, &tempo);
    assert_eq!((bar(3.9).bar, bar(3.9).beat), (2, 4));
    assert_eq!((bar(4.0).bar, bar(4.0).beat), (3, 1));
    // Beats are a second long now.
    assert_eq!((bar(5.0).bar, bar(5.0).beat), (3, 2));
}

/// Signal's patch changes, built off the audio thread from the session's
/// items and read per buffer on it.
#[test]
fn a_schedule_answers_a_buffer_and_a_locate() {
    let mut session = Session::new("Song");
    let signal = ExtensionId::from("signal");
    let lane = LaneId::from("patches");
    for (id, qn, patch) in [("a", 0.0, 1), ("b", 8.0, 2), ("c", 16.0, 3)] {
        session.items.insert(
            ItemId::from(id),
            Item {
                owner: signal.clone(),
                lane: lane.clone(),
                kind: "signal.patch".into(),
                at: Time::QuarterNotes(qn),
                end: None,
                payload: Value::Int(patch),
            },
        );
    }
    let build = |s: &Session| {
        Schedule::new(
            s.timed("signal.patch")
                .into_iter()
                .filter_map(|(at, item)| item.payload.as_i64().map(|p| (at, p)))
                .collect(),
        )
    };
    let schedule = build(&session);
    // At 120 BPM: 0 s, 4 s, 8 s. A 512-frame buffer at 48 kHz from 3.995 s.
    let buffer = schedule.between(3.995, 3.995 + 512.0 / 48_000.0);
    assert_eq!(buffer, &[(4.0, 2)]);
    assert_eq!(schedule.between(4.5, 7.9), &[]);
    assert_eq!(schedule.last_at_or_before(7.0), Some(&(4.0, 2)));
    assert_eq!(schedule.last_at_or_before(-1.0), None);

    // Half the tempo: the same items, rebuilt, land twice as late.
    session.tempo = TempoMap::new(60.0, TimeSignature::COMMON_TIME, Vec::new());
    let schedule = build(&session);
    assert_eq!(schedule.last_at_or_before(15.0), Some(&(8.0, 2)));
}
