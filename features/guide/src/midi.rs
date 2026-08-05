//! MIDI note constants and mappings (ported from fts-guide `midi/notes.rs`).
//!
//! Kept so hosts can emit the guide as a MIDI track (e.g. for external
//! samplers) using the same note layout the legacy plugin used.

/// MIDI note constants for click subdivisions
pub const MIDI_NOTE_CLICK_ACCENT: u8 = 60; // C4
pub const MIDI_NOTE_CLICK_BEAT: u8 = 61; // C#4
pub const MIDI_NOTE_CLICK_EIGHTH: u8 = 62; // D4
pub const MIDI_NOTE_CLICK_SIXTEENTH: u8 = 63; // D#4
pub const MIDI_NOTE_CLICK_TRIPLET: u8 = 65; // F4

/// MIDI notes for count samples (1-8)
pub const MIDI_NOTES_COUNT: [u8; 8] = [72, 73, 74, 75, 76, 77, 78, 79]; // C5-C6

/// Map section types to MIDI notes (starting from C6 = 84)
pub fn get_midi_note_for_section_type(section_type: &str) -> Option<u8> {
    Some(match section_type {
        "Verse" => 84,                       // C6
        "Chorus" => 85,                      // C#6
        "Bridge" => 86,                      // D6
        "Intro" => 87,                       // D#6
        "Outro" => 88,                       // E6
        "Instrumental" => 89,                // F6
        "Pre Chorus" | "Pre-Chorus" => 90,   // F#6
        "Post Chorus" | "Post-Chorus" => 91, // G6
        "Breakdown" => 92,                   // G#6
        "Interlude" => 93,                   // A6
        "Tag" => 94,                         // A#6
        "Ending" => 95,                      // B6
        "Solo" => 96,                        // C7
        "Vamp" => 97,                        // C#7
        "Turnaround" => 98,                  // D7
        "Refrain" => 99,                     // D#7
        "Rap" => 100,                        // E7
        "Acapella" => 101,                   // F7
        "Exhortation" => 102,                // F#7
        _ => return None,                    // Unknown section type
    })
}

// ── Rendering the guide as MIDI ─────────────────────────────────────────
//
// The authoring counterpart to the playback engine: same schedule, same
// note layout, emitted as notes a DAW can hold instead of PCM mixed into
// a buffer. Pure — no DAW types, no I/O — so it is testable without
// REAPER, and so the host owns *where* the notes go.

use crate::schedule::{CueEvent, CueSchedule, GuideSection, GuideSongTiming, ScheduleOptions};
use session_proto::GuideTrackRole;

/// One note to stamp, on the track its role names.
#[derive(Debug, Clone, PartialEq)]
pub struct GuideMidiNote {
    /// Which guide track this belongs on.
    pub role: GuideTrackRole,
    pub time_seconds: f64,
    pub length_seconds: f64,
    pub pitch: u8,
    pub velocity: u8,
}

/// A constant-tempo stretch of the timeline.
///
/// The click grid is the one part of the guide that isn't in
/// [`CueSchedule`] — the engine's `click_player` generates it live from
/// the host's block clock, so there was never a precomputed version.
/// Taking segments rather than a single tempo means a caller holding a
/// real tempo map (the REAPER action does) gets a correct click across
/// tempo and time-signature changes, while a caller that only has a
/// song's nominal tempo can pass [`TempoSegment::from_timing`].
#[derive(Debug, Clone, PartialEq)]
pub struct TempoSegment {
    pub start_seconds: f64,
    pub tempo_bpm: f64,
    pub time_sig_num: u32,
    pub time_sig_den: u32,
}

impl TempoSegment {
    /// The whole span as one segment at a song's nominal tempo — what the
    /// guide engine itself assumes.
    pub fn from_timing(timing: &GuideSongTiming, start_seconds: f64) -> Self {
        Self {
            start_seconds,
            tempo_bpm: timing.tempo_bpm,
            time_sig_num: timing.time_sig_num,
            time_sig_den: timing.time_sig_den,
        }
    }

    /// Seconds per beat, where "beat" is one denominator unit.
    fn beat_seconds(&self) -> f64 {
        (60.0 / self.tempo_bpm) * (4.0 / f64::from(self.time_sig_den))
    }
}

/// How finely to lay down the click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClickSubdivision {
    /// One note per beat. Downbeats accent.
    #[default]
    Beat,
    Eighth,
    Sixteenth,
    Triplet,
}

impl ClickSubdivision {
    /// How many notes per beat, and the note to use for the off-positions.
    fn divisions(self) -> (u32, u8) {
        match self {
            Self::Beat => (1, MIDI_NOTE_CLICK_BEAT),
            Self::Eighth => (2, MIDI_NOTE_CLICK_EIGHTH),
            Self::Sixteenth => (4, MIDI_NOTE_CLICK_SIXTEENTH),
            Self::Triplet => (3, MIDI_NOTE_CLICK_TRIPLET),
        }
    }
}

/// Note length and velocity for stamped notes. Guide/count notes are
/// triggers, not sustained material, so they're short by default.
const TRIGGER_LENGTH_SECONDS: f64 = 0.1;
const ACCENT_VELOCITY: u8 = 112;
const NORMAL_VELOCITY: u8 = 96;

/// Lay down the click grid over `[start, end)`.
///
/// Beat 1 of each measure gets [`MIDI_NOTE_CLICK_ACCENT`]; other beats
/// get the beat note; subdivisions between beats get the subdivision's
/// own note. The measure count restarts at each segment boundary, which
/// is what a time-signature change means.
pub fn click_notes(
    segments: &[TempoSegment],
    end_seconds: f64,
    subdivision: ClickSubdivision,
) -> Vec<GuideMidiNote> {
    let mut notes = Vec::new();
    let (per_beat, off_pitch) = subdivision.divisions();

    for (i, segment) in segments.iter().enumerate() {
        let segment_end = segments
            .get(i + 1)
            .map(|next| next.start_seconds)
            .unwrap_or(end_seconds)
            .min(end_seconds);
        if segment_end <= segment.start_seconds {
            continue;
        }

        let beat = segment.beat_seconds();
        if beat <= 0.0 {
            continue;
        }
        let step = beat / f64::from(per_beat);

        let mut index: u64 = 0;
        loop {
            let time = segment.start_seconds + step * index as f64;
            if time >= segment_end {
                break;
            }
            let on_beat = index % u64::from(per_beat) == 0;
            let beat_index = index / u64::from(per_beat);
            let downbeat = on_beat && beat_index % u64::from(segment.time_sig_num) == 0;

            notes.push(GuideMidiNote {
                role: GuideTrackRole::Click,
                time_seconds: time,
                length_seconds: step.min(TRIGGER_LENGTH_SECONDS),
                pitch: if downbeat {
                    MIDI_NOTE_CLICK_ACCENT
                } else if on_beat {
                    MIDI_NOTE_CLICK_BEAT
                } else {
                    off_pitch
                },
                velocity: if downbeat { ACCENT_VELOCITY } else { NORMAL_VELOCITY },
            });
            index += 1;
        }
    }
    notes
}

/// Turn a built schedule's cues into Count- and Guide-track notes.
///
/// Nothing is recomputed here — the timing is whatever
/// [`CueSchedule::build`] decided, so the stamped MIDI lines up with what
/// the engine plays by construction rather than by two implementations
/// agreeing.
pub fn cue_notes(schedule: &CueSchedule) -> Vec<GuideMidiNote> {
    schedule
        .cues
        .iter()
        .filter_map(|cue| match &cue.event {
            CueEvent::Count { index } => Some(GuideMidiNote {
                role: GuideTrackRole::Count,
                time_seconds: cue.time_seconds,
                length_seconds: TRIGGER_LENGTH_SECONDS,
                pitch: *MIDI_NOTES_COUNT.get(*index)?,
                velocity: NORMAL_VELOCITY,
            }),
            CueEvent::Guide { section_type, .. } => Some(GuideMidiNote {
                role: GuideTrackRole::Guide,
                time_seconds: cue.time_seconds,
                length_seconds: TRIGGER_LENGTH_SECONDS,
                pitch: get_midi_note_for_section_type(section_type.as_deref()?)?,
                velocity: NORMAL_VELOCITY,
            }),
        })
        .collect()
}

/// Everything: click grid plus count and guide cues, sorted by time.
///
/// `tempo` drives only the click; the cues come from `sections`/`timing`
/// via [`CueSchedule::build`]. Pass `&[]` for `tempo` to skip the click
/// track entirely.
pub fn guide_midi(
    sections: &[GuideSection],
    timing: &GuideSongTiming,
    options: &ScheduleOptions,
    tempo: &[TempoSegment],
    end_seconds: f64,
    subdivision: ClickSubdivision,
) -> Vec<GuideMidiNote> {
    let schedule = CueSchedule::build(sections, timing, options);
    let mut notes = click_notes(tempo, end_seconds, subdivision);
    notes.extend(cue_notes(&schedule));
    notes.sort_by(|a, b| a.time_seconds.total_cmp(&b.time_seconds));
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, bpm: f64, num: u32, den: u32) -> TempoSegment {
        TempoSegment {
            start_seconds: start,
            tempo_bpm: bpm,
            time_sig_num: num,
            time_sig_den: den,
        }
    }

    #[test]
    fn click_accents_every_downbeat() {
        // 120bpm 4/4 → 0.5s per beat, 2s per measure. Two measures.
        let notes = click_notes(&[seg(0.0, 120.0, 4, 4)], 4.0, ClickSubdivision::Beat);

        assert_eq!(notes.len(), 8);
        let accents: Vec<f64> = notes
            .iter()
            .filter(|n| n.pitch == MIDI_NOTE_CLICK_ACCENT)
            .map(|n| n.time_seconds)
            .collect();
        assert_eq!(accents, vec![0.0, 2.0]);
        assert!(notes.iter().all(|n| n.role == GuideTrackRole::Click));
    }

    #[test]
    fn subdivisions_fill_between_beats_without_stealing_the_accent() {
        let notes = click_notes(&[seg(0.0, 120.0, 4, 4)], 2.0, ClickSubdivision::Eighth);

        assert_eq!(notes.len(), 8, "4 beats x 2 = 8 eighths in one measure");
        assert_eq!(notes[0].pitch, MIDI_NOTE_CLICK_ACCENT);
        assert_eq!(notes[1].pitch, MIDI_NOTE_CLICK_EIGHTH);
        assert_eq!(notes[2].pitch, MIDI_NOTE_CLICK_BEAT);
    }

    /// A time-signature change restarts the measure, so the accent lands
    /// on the new bar rather than continuing the old cycle.
    #[test]
    fn segment_boundary_restarts_the_measure() {
        let notes = click_notes(
            &[seg(0.0, 120.0, 4, 4), seg(2.0, 120.0, 3, 4)],
            5.0,
            ClickSubdivision::Beat,
        );

        let accents: Vec<f64> = notes
            .iter()
            .filter(|n| n.pitch == MIDI_NOTE_CLICK_ACCENT)
            .map(|n| n.time_seconds)
            .collect();
        assert_eq!(accents, vec![0.0, 2.0, 3.5], "3/4 bar is 1.5s");
    }

    #[test]
    fn denominator_scales_the_beat() {
        // 6/8 at 120bpm: an eighth-note beat is 0.25s.
        let notes = click_notes(&[seg(0.0, 120.0, 6, 8)], 1.5, ClickSubdivision::Beat);
        assert_eq!(notes.len(), 6);
        assert_eq!(notes[1].time_seconds, 0.25);
    }

    #[test]
    fn no_tempo_segments_means_no_click() {
        assert!(click_notes(&[], 10.0, ClickSubdivision::Beat).is_empty());
    }

    #[test]
    fn cues_become_count_and_guide_notes_on_their_own_tracks() {
        let schedule = CueSchedule {
            cues: vec![
                crate::schedule::ScheduledCue {
                    time_seconds: 1.0,
                    event: CueEvent::Count { index: 0 },
                },
                crate::schedule::ScheduledCue {
                    time_seconds: 2.0,
                    event: CueEvent::Guide {
                        keys: vec!["Chorus_1".into()],
                        section_type: Some("Chorus".into()),
                    },
                },
            ],
        };

        let notes = cue_notes(&schedule);

        assert_eq!(notes[0].role, GuideTrackRole::Count);
        assert_eq!(notes[0].pitch, MIDI_NOTES_COUNT[0]);
        assert_eq!(notes[1].role, GuideTrackRole::Guide);
        assert_eq!(
            notes[1].pitch,
            get_midi_note_for_section_type("Chorus").unwrap()
        );
    }

    /// A free-form spoken cue has no section behind it, and an unknown
    /// section type has no note. Neither should be stamped — silently
    /// dropping is right here, since the audio cue still plays.
    #[test]
    fn guide_cues_without_a_mappable_section_are_skipped() {
        let schedule = CueSchedule {
            cues: vec![
                crate::schedule::ScheduledCue {
                    time_seconds: 1.0,
                    event: CueEvent::Guide {
                        keys: vec!["tts:watch me".into()],
                        section_type: None,
                    },
                },
                crate::schedule::ScheduledCue {
                    time_seconds: 2.0,
                    event: CueEvent::Guide {
                        keys: vec![],
                        section_type: Some("Kazoo Solo".into()),
                    },
                },
            ],
        };

        assert!(cue_notes(&schedule).is_empty());
    }
}
