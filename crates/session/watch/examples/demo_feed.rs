//! The watch app's Demo: one whole song's guide feed, built by the same
//! timeline the relay uses, every beat's `at_us` counted from the song's
//! first beat. The watch replays it on a loop (shifted onto its own clock)
//! when no phone is there — so the face and the haptic click can be tried,
//! and timed, in the simulator.
//!
//! ```bash
//! cargo run -p session-watch-guide --example demo_feed \
//!     > apps/session-watch/SessionWatch/Resources/demo-feed.json
//! ```

use session_proto::watch::{WatchBeat, WatchGuideFeed};
use session_proto::{Section, SectionId, SectionType, Song, SongId};
use session_watch_guide::{GuideTimeline, TempoMark};

fn main() -> Result<(), String> {
    let song = song();
    // A tempo map with a change: the bridge pulls back to 112.
    let tempo = [
        TempoMark {
            at_seconds: 0.0,
            tempo_bpm: 124.0,
            time_sig_num: 4,
            time_sig_den: 4,
        },
        TempoMark {
            at_seconds: bar(26.0),
            tempo_bpm: 112.0,
            time_sig_num: 4,
            time_sig_den: 4,
        },
    ];
    let timeline = GuideTimeline::build(&song, &tempo);
    let first = timeline.beats.first().map_or(0.0, |b| b.position);
    let feed = WatchGuideFeed {
        revision: 1,
        run: 1,
        set_title: "Demo Set".into(),
        song_title: song.name,
        song_index: 0,
        song_count: 1,
        playing: true,
        clock_locked: true,
        clock_error_us: 0.0,
        sections: timeline.sections.clone(),
        here: timeline.beats.first().cloned().unwrap_or_default(),
        beats: timeline
            .beats
            .iter()
            .map(|b| WatchBeat {
                at_us: (b.position - first) * 1e6,
                ..b.clone()
            })
            .collect(),
    };
    let json = facet_json::to_string(&feed).map_err(|e| e.to_string())?;
    println!("{json}");
    Ok(())
}

/// Where bar `n` (from 0) starts at 124 bpm 4/4, seconds.
fn bar(n: f64) -> f64 {
    n * 4.0 * 60.0 / 124.0
}

fn song() -> Song {
    let section =
        |name: &str, ty: SectionType, number: Option<u32>, from: f64, to: f64, color: u32| {
            Section {
                section_id: SectionId::default(),
                id: None,
                name: name.into(),
                comment: None,
                section_type: ty,
                start_seconds: from,
                end_seconds: to,
                number,
                color: Some(color),
            }
        };
    // Bars (from 0): count 0–2, intro 2–6, verse 6–14, chorus 14–22,
    // verse 22–26 … at 124; the bridge at 112 from bar 26.
    let slow = |n: f64| bar(26.0) + (n - 26.0) * 4.0 * 60.0 / 112.0;
    Song {
        id: SongId::default(),
        name: "Always On Time".into(),
        project_guid: String::new(),
        start_seconds: 0.0,
        end_seconds: slow(38.0),
        count_in_seconds: Some(bar(2.0)),
        sections: vec![
            section(
                "Count-In",
                SectionType::CountIn,
                None,
                0.0,
                bar(2.0),
                0x0052_525B,
            ),
            section(
                "Intro",
                SectionType::Intro,
                None,
                bar(2.0),
                bar(6.0),
                0x0060_A5FA,
            ),
            section(
                "Verse 1",
                SectionType::Verse,
                Some(1),
                bar(6.0),
                bar(14.0),
                0x0034_D399,
            ),
            section(
                "Chorus",
                SectionType::Chorus,
                None,
                bar(14.0),
                bar(22.0),
                0x00F4_7272,
            ),
            section(
                "Verse 2",
                SectionType::Verse,
                Some(2),
                bar(22.0),
                bar(26.0),
                0x0034_D399,
            ),
            section(
                "Bridge",
                SectionType::Bridge,
                None,
                bar(26.0),
                slow(30.0),
                0x00C0_84FC,
            ),
            section(
                "Chorus",
                SectionType::Chorus,
                None,
                slow(30.0),
                slow(36.0),
                0x00F4_7272,
            ),
            section(
                "Outro",
                SectionType::Outro,
                None,
                slow(36.0),
                slow(38.0),
                0x00FB_BF24,
            ),
        ],
        comments: vec![],
        tempo: Some(124.0),
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
