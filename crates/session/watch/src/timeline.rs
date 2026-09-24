//! Every beat of a song: where it falls, and what it means.
//!
//! Laid from the song's tempo map by the same code that stamps the Click
//! track the band hears ([`session_guide::midi::segments_in_span`] and
//! [`click_notes`]), so a tap on the wrist and a click in the ear are one
//! grid. The count and the section announcements come from the guide's own
//! schedule ([`CueSchedule::build`], with the options `Guide::generate`
//! stamps with), so the number the watch shows is the one the guide says.
//!
//! What this adds is only the reading a watch face needs: bars counted
//! from the first musical downbeat, the section each beat is in and its bar
//! within it, and which beats are the count into the song.
//!
//! The beats are one per beat of the meter. The audible click plays eighths
//! below 75 bpm ([`session_guide::midi::ClickSubdivision::Auto`]); on the
//! wrist the pulse stays on the beat, which is what it is for.

use session_guide::midi::{
    ClickSubdivision, MIDI_NOTE_CLICK_ACCENT, TempoMark, TempoSegment, click_notes,
    segments_in_span,
};
use session_guide::{CueEvent, CueSchedule, GuideSongTiming, ScheduleOptions, sections_from_song};
use session_proto::watch::{WatchAccent, WatchBeat, WatchSection};
use session_proto::{SectionType, Song};

/// How close two instants must be to be the same one, seconds — the
/// guide's own tolerance for a cue landing on a count.
const SAME: f64 = 0.001;

/// A song's whole beat grid, built once per song (and again when its tempo
/// map or sections change).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuideTimeline {
    /// The song's span, project seconds.
    pub start: f64,
    pub end: f64,
    pub sections: Vec<WatchSection>,
    /// Every beat in time order. `at_us` is unset (0) here: a beat's
    /// instant depends on the transport, which [`crate::relay`] supplies.
    pub beats: Vec<WatchBeat>,
}

impl GuideTimeline {
    /// The grid of `song` under `tempo` (its project's tempo map). With no
    /// map at all, the song's own nominal tempo and meter.
    #[must_use]
    pub fn build(song: &Song, tempo: &[TempoMark]) -> Self {
        let (start, end) = span_of(song);
        if end <= start {
            return Self {
                start,
                end: start,
                ..Self::default()
            };
        }
        let timing = GuideSongTiming::from_song(song);
        let nominal = TempoMark {
            at_seconds: start,
            tempo_bpm: timing.tempo_bpm,
            time_sig_num: timing.time_sig_num,
            time_sig_den: timing.time_sig_den,
        };
        let segments = segments_in_span(tempo, start, end, || {
            TempoMark::in_force(tempo, start).unwrap_or(nominal)
        });
        let clicks = click_notes(&segments, end, ClickSubdivision::Beat);

        // The first musical downbeat: where bar 1 is, and where the count
        // into the song ends.
        let downbeat = song
            .sections
            .iter()
            .find(|s| s.section_type != SectionType::CountIn)
            .map_or(start, |s| s.start_seconds);
        let accents: Vec<f64> = clicks
            .iter()
            .filter(|n| n.pitch == MIDI_NOTE_CLICK_ACCENT)
            .map(|n| n.time_seconds)
            .collect();
        // Bar 1 is the bar the downbeat is in.
        let bars_before_one = count_upto(&accents, downbeat);

        let sections = sections_of(song, &accents);

        let span = end - start;
        let mut beats: Vec<WatchBeat> = Vec::with_capacity(clicks.len());
        let mut beat_in_bar = 0u32;
        for note in &clicks {
            let at = note.time_seconds;
            let segment = segment_at(&segments, at);
            beat_in_bar = if note.pitch == MIDI_NOTE_CLICK_ACCENT {
                1
            } else {
                beat_in_bar.saturating_add(1)
            };
            let bar =
                signed_difference(count_upto(&accents, at), bars_before_one).saturating_add(1);
            let section_index = song
                .sections
                .iter()
                .rposition(|s| s.start_seconds <= at + SAME);
            let section = section_index.and_then(|i| sections.get(i));
            let section_bar = section.map_or(0, |s| {
                count_upto(&accents, at)
                    .saturating_sub(count_before(&accents, s.start))
                    .saturating_add(u32::from(!on_a_bar_line(&accents, s.start)))
                    .max(1)
            });
            beats.push(WatchBeat {
                index: u32::try_from(beats.len()).unwrap_or(u32::MAX),
                at_us: 0.0,
                position: at,
                progress: narrow((at - start) / span),
                bar,
                beat: beat_in_bar.max(1),
                beats_per_bar: segment.map_or(timing.time_sig_num, |s| s.time_sig_num),
                bpm: narrow(segment.map_or(timing.tempo_bpm, |s| s.tempo_bpm)),
                section: section_index
                    .and_then(|i| i32::try_from(i).ok())
                    .unwrap_or(-1),
                section_bar,
                section_bars: section.map_or(0, |s| s.bars),
                count: 0,
                cue: String::new(),
                accent: if at < downbeat - SAME {
                    WatchAccent::CountIn
                } else if note.pitch == MIDI_NOTE_CLICK_ACCENT {
                    WatchAccent::Downbeat
                } else {
                    WatchAccent::Beat
                },
            });
        }

        mark_cues(&mut beats, song, &timing);

        Self {
            start,
            end,
            sections,
            beats,
        }
    }

    /// The beat at or before `position` — where the song is — or the first
    /// one before the song has started.
    #[must_use]
    pub fn frame_at(&self, position: f64) -> Option<&WatchBeat> {
        let next = self
            .beats
            .partition_point(|b| b.position <= position + SAME);
        next.checked_sub(1)
            .and_then(|i| self.beats.get(i))
            .or_else(|| self.beats.first())
    }

    /// The beats after `position`, up to `until` (both project seconds).
    #[must_use]
    pub fn beats_between(&self, position: f64, until: f64) -> &[WatchBeat] {
        let from = self.beats.partition_point(|b| b.position <= position);
        let to = self.beats.partition_point(|b| b.position <= until);
        self.beats.get(from..to.max(from)).unwrap_or(&[])
    }

    /// 0..1 through the song at `position`.
    #[must_use]
    pub fn progress(&self, position: f64) -> f32 {
        let span = self.end - self.start;
        if span <= 0.0 {
            0.0
        } else {
            narrow((position - self.start) / span)
        }
    }
}

/// The song's span: its start to its end, or to its last section's end
/// when it has no end of its own.
fn span_of(song: &Song) -> (f64, f64) {
    let start = song.start_seconds;
    let end = if song.end_seconds > start {
        song.end_seconds
    } else {
        song.sections
            .iter()
            .map(|s| s.end_seconds)
            .fold(start, f64::max)
    };
    (start, end)
}

/// The song's sections as the watch draws them, each with its bar count.
fn sections_of(song: &Song, accents: &[f64]) -> Vec<WatchSection> {
    song.sections
        .iter()
        .map(|s| WatchSection {
            name: s.name.clone(),
            start: s.start_seconds,
            end: s.end_seconds,
            color: s.color.unwrap_or(0) & 0x00FF_FFFF,
            bars: bars_between(accents, s.start_seconds, s.end_seconds),
        })
        .collect()
}

/// The guide's count and announcements, on the beats they land on — with
/// the options the stamped Count/Guide tracks are built with.
///
/// The schedule is the guide's own, at the song's nominal tempo: a cue it
/// puts off the grid (after a tempo change) lands on no beat and is left
/// out, rather than shown on a beat it is not on.
fn mark_cues(beats: &mut [WatchBeat], song: &Song, timing: &GuideSongTiming) {
    let options = ScheduleOptions {
        guide_replace_beat1: false,
        ..ScheduleOptions::default()
    };
    let schedule = CueSchedule::build(&sections_from_song(song), timing, &options);
    for cue in &schedule.cues {
        let Some(beat) = beats
            .iter_mut()
            .find(|b| (b.position - cue.time_seconds).abs() < SAME)
        else {
            continue;
        };
        match &cue.event {
            CueEvent::Count { index } => {
                beat.count = u32::try_from(index.saturating_add(1)).unwrap_or(0);
            }
            CueEvent::Guide { keys, section_type } => {
                beat.cue = section_type.as_ref().map_or_else(
                    || {
                        keys.first()
                            .map(|k| k.trim_start_matches("tts:").to_owned())
                            .unwrap_or_default()
                    },
                    SectionType::full_name,
                );
            }
        }
    }
}

/// How many of `accents` (sorted) fall at or before `t`: the bars begun
/// by then.
fn count_upto(accents: &[f64], t: f64) -> u32 {
    u32::try_from(accents.partition_point(|a| *a <= t + SAME)).unwrap_or(u32::MAX)
}

/// How many of `accents` fall strictly before `t`.
fn count_before(accents: &[f64], t: f64) -> u32 {
    u32::try_from(accents.partition_point(|a| *a < t - SAME)).unwrap_or(u32::MAX)
}

/// Whether a bar begins at `t`.
fn on_a_bar_line(accents: &[f64], t: f64) -> bool {
    count_upto(accents, t) > count_before(accents, t)
}

/// The bars in `[from, to)` — a section that starts mid-bar also has the
/// bar it starts in.
fn bars_between(accents: &[f64], from: f64, to: f64) -> u32 {
    count_before(accents, to)
        .saturating_sub(count_before(accents, from))
        .saturating_add(u32::from(!on_a_bar_line(accents, from)))
        .max(1)
}

/// The tempo segment in force at `t`.
fn segment_at(segments: &[TempoSegment], t: f64) -> Option<&TempoSegment> {
    segments
        .iter()
        .rev()
        .find(|s| s.start_seconds <= t + SAME)
        .or_else(|| segments.first())
}

/// `a − b` as a bar number (count-in bars go to zero and below).
fn signed_difference(a: u32, b: u32) -> i32 {
    i32::try_from(i64::from(a).saturating_sub(i64::from(b))).unwrap_or(0)
}

/// A ratio or tempo for the wire, which carries `f32`.
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)] // clamped/small values
const fn narrow(v: f64) -> f32 {
    v.clamp(-1.0e6, 1.0e6) as f32
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use session_proto::{Section, SectionId, SongId};

    fn section(name: &str, ty: SectionType, start: f64, end: f64) -> Section {
        Section {
            section_id: SectionId::default(),
            id: None,
            name: name.to_string(),
            comment: None,
            section_type: ty,
            start_seconds: start,
            end_seconds: end,
            number: None,
            color: Some(0x00FF_0000),
        }
    }

    /// 120 bpm 4/4 (a beat 0.5 s, a bar 2 s): a 2-bar count-in, 8 bars of
    /// intro, 8 of verse.
    pub fn song() -> Song {
        Song {
            id: SongId::default(),
            name: "Test Song".into(),
            project_guid: "guid".into(),
            start_seconds: 0.0,
            end_seconds: 36.0,
            count_in_seconds: Some(4.0),
            sections: vec![
                section("Count-In", SectionType::CountIn, 0.0, 4.0),
                section("Intro", SectionType::Intro, 4.0, 20.0),
                section("Verse 1", SectionType::Verse, 20.0, 36.0),
            ],
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

    #[test]
    fn beats_number_from_the_first_musical_downbeat() {
        let t = GuideTimeline::build(&song(), &[]);
        assert_eq!(t.beats.len(), 72, "36 s at 0.5 s a beat");
        let first = &t.beats[0];
        assert_eq!(
            (first.bar, first.beat, first.accent),
            (-1, 1, WatchAccent::CountIn)
        );
        let one = t
            .beats
            .iter()
            .find(|b| (b.position - 4.0).abs() < 1e-9)
            .unwrap();
        assert_eq!(
            (one.bar, one.beat, one.accent),
            (1, 1, WatchAccent::Downbeat)
        );
        assert_eq!(one.section, 1);
        assert_eq!((one.section_bar, one.section_bars), (1, 8));
        // The count-in's second bar is its bar 2; the intro's last, its 8th.
        assert_eq!((t.beats[4].section_bar, t.beats[4].section_bars), (2, 2));
        let last_of_intro = t.beats.iter().rfind(|b| b.section == 1).unwrap();
        assert_eq!((last_of_intro.bar, last_of_intro.section_bar), (8, 8));
        assert_eq!(
            t.sections.iter().map(|s| s.bars).collect::<Vec<_>>(),
            vec![2, 8, 8]
        );
        let two = &t.beats[9];
        assert_eq!((two.bar, two.beat, two.accent), (1, 2, WatchAccent::Beat));
    }

    #[test]
    fn the_count_into_the_song_is_the_guides() {
        let t = GuideTimeline::build(&song(), &[]);
        // The guide counts the final count-in bar: 1 2 3 4 at 2.0 … 3.5 s.
        let counts: Vec<(f64, u32)> = t
            .beats
            .iter()
            .filter(|b| b.position < 4.0 && b.count > 0)
            .map(|b| (b.position, b.count))
            .collect();
        assert_eq!(counts, vec![(2.0, 1), (2.5, 2), (3.0, 3), (3.5, 4)]);
        // …and the section it counts into is announced.
        assert!(
            t.beats.iter().any(|b| b.cue == "Intro"),
            "the intro is announced"
        );
    }

    #[test]
    fn a_tempo_map_moves_the_grid() {
        // 60 bpm from the start: a beat a second.
        let map = [TempoMark {
            at_seconds: 0.0,
            tempo_bpm: 60.0,
            time_sig_num: 3,
            time_sig_den: 4,
        }];
        let t = GuideTimeline::build(&song(), &map);
        assert_eq!(t.beats.len(), 36);
        assert_eq!(t.beats[3].beat, 1, "3/4: the fourth beat starts bar two");
        assert!((t.beats[1].bpm - 60.0).abs() < 1e-6);
    }

    #[test]
    fn frames_and_windows() {
        let t = GuideTimeline::build(&song(), &[]);
        assert_eq!(t.frame_at(4.2).map(|b| b.position), Some(4.0));
        assert_eq!(t.frame_at(-3.0).map(|b| b.index), Some(0));
        let next: Vec<f64> = t
            .beats_between(4.0, 5.0)
            .iter()
            .map(|b| b.position)
            .collect();
        assert_eq!(next, vec![4.5, 5.0]);
        assert!(t.beats_between(40.0, 50.0).is_empty());
    }
}
