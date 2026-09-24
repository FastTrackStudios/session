//! How close to the band's beat can the watch's clock mapping put a tap?
//!
//! A model, not a measurement: the relay's own code (`WatchRelay`, the
//! desktop's `ClockEstimator`) run against a simulated `WatchConnectivity`
//! link, with a watch clock offset and drifting against the phone's, and
//! one-way delays drawn from a Bluetooth-like distribution — a floor, a
//! long tail, the odd stall, and the two directions deliberately unequal
//! (the part no clock sync can see). It reports how far each beat's
//! watch-clock instant lands from where the beat truly is.
//!
//! What it leaves out is on the watch itself: when a timer actually fires
//! (the app measures that — Settings › Timing) and how long the Taptic
//! Engine takes to move (the app's Calibrate measures that with the
//! accelerometer). And Task's side: the phone's `SharedClock` error, which
//! is half its own round trip's asymmetry, typically a millisecond or two.
//!
//! ```bash
//! cargo run -p session-watch-guide --example link_accuracy
//! ```

use daw_transport_sync::Position;
use session_proto::watch::WatchPong;
use session_watch_guide::GuideTimeline;
use session_watch_guide::relay::{Lead, RelayInput, SongRef, WatchRelay};

/// A small deterministic generator (no dependency for a model).
struct Lcg(u64);

impl Lcg {
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
        let v = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
        v
    }

    /// One way over the link, µs: a floor, an exponential tail, and one
    /// message in twenty stalled behind something else.
    fn delay(&mut self, floor_us: f64, tail_us: f64) -> f64 {
        let stall = if self.unit() < 0.05 {
            150_000.0 * self.unit()
        } else {
            0.0
        };
        (-tail_us).mul_add((1.0 - self.unit()).ln(), floor_us) + stall
    }
}

struct Link {
    /// phone→watch and watch→phone floors, µs — unequal on purpose.
    out_floor: f64,
    back_floor: f64,
    tail: f64,
}

fn main() {
    let song = demo_song();
    let timeline = GuideTimeline::build(&song, &[]);
    let links = [
        (
            "close, quiet (BT, watch app frontmost)",
            Link {
                out_floor: 12_000.0,
                back_floor: 14_000.0,
                tail: 8_000.0,
            },
        ),
        (
            "typical",
            Link {
                out_floor: 15_000.0,
                back_floor: 22_000.0,
                tail: 25_000.0,
            },
        ),
        (
            "poor (busy radio, 10 ms asymmetry)",
            Link {
                out_floor: 20_000.0,
                back_floor: 40_000.0,
                tail: 60_000.0,
            },
        ),
    ];
    for (name, link) in &links {
        let mut errors: Vec<f64> = Vec::new();
        for seed in 1..=200u64 {
            errors.extend(trial(&timeline, link, seed));
        }
        errors.sort_by(f64::total_cmp);
        let pick = |q: f64| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                clippy::as_conversions
            )]
            let i = ((errors.len() as f64 - 1.0) * q).round() as usize;
            errors.get(i).copied().unwrap_or(f64::NAN) / 1000.0
        };
        println!(
            "{name:<42} |error| p50 {:5.2} ms  p95 {:5.2} ms  max {:5.2} ms  (asymmetry alone: {:.1} ms)",
            pick(0.5),
            pick(0.95),
            pick(1.0),
            (link.back_floor - link.out_floor) / 2000.0
        );
    }
}

/// Ten seconds of pings at 4 Hz, then the error of every beat in one feed.
fn trial(timeline: &GuideTimeline, link: &Link, seed: u64) -> Vec<f64> {
    let mut rng = Lcg(seed);
    // The watch's clock: an arbitrary epoch, running 25 ppm fast.
    let watch_at = |t: f64| t.mul_add(1.0 + 25e-6, 123_456_789.0);
    let mut relay = WatchRelay::new();
    // Forty pings, a quarter second apart: ten seconds of them.
    let mut t = 0.0;
    for _ in 0..40 {
        let sent = t; // phone clock = true time
        let arrives = sent + rng.delay(link.out_floor, link.tail);
        let replies = 2_000.0f64.mul_add(rng.unit(), arrives + 500.0); // the watch decodes, then answers
        let lands = replies + rng.delay(link.back_floor, link.tail);
        relay.pong(
            &WatchPong {
                sent_us: sent,
                received_us: watch_at(arrives),
                replied_us: watch_at(replies),
            },
            lands,
        );
        t += 250_000.0;
    }
    // The shared clock is the phone's here (its own error is Task's link).
    let lead = Lead {
        song: None,
        position: Position {
            host_micros: t,
            playhead_seconds: 30.0,
            playrate: 1.0,
            is_playing: true,
        },
    };
    let input = RelayInput {
        set_title: "",
        song: Some(SongRef {
            key: "demo",
            title: "Demo",
            index: 0,
            count: 1,
            timeline,
        }),
        lead: Some(&lead),
        shared_offset_us: Some(0.0),
        shared_round_trip_us: Some(0.0),
    };
    let feed = relay.feed(&input, t);
    feed.beats
        .iter()
        .map(|b| {
            let truly = watch_at((b.position - 30.0).mul_add(1e6, t));
            (b.at_us - truly).abs()
        })
        .collect()
}

fn demo_song() -> session_proto::Song {
    use session_proto::{Section, SectionId, SectionType, Song, SongId};
    let section = |name: &str, ty: SectionType, start: f64, end: f64| Section {
        section_id: SectionId::default(),
        id: None,
        name: name.into(),
        comment: None,
        section_type: ty,
        start_seconds: start,
        end_seconds: end,
        number: None,
        color: None,
    };
    Song {
        id: SongId::default(),
        name: "Demo".into(),
        project_guid: String::new(),
        start_seconds: 0.0,
        end_seconds: 120.0,
        count_in_seconds: None,
        sections: vec![section("Verse", SectionType::Verse, 0.0, 120.0)],
        comments: vec![],
        tempo: Some(120.0),
        time_signature: None,
        measure_positions: vec![],
        chart_text: None,
        parsed_chart: None,
        detected_chords: vec![],
        chart_fingerprint: None,
        advance_mode: None,
        color: None,
    }
}
