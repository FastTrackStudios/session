//! Cue scheduling: turns song sections into timeline events (count-in
//! voices, section-guide announcements, spoken TTS cues).
//!
//! This replaces the legacy plugin's live REAPER region/transport polling.
//! The session domain fills [`GuideSection`]s from its `Song`/`Setlist`
//! types (a `From<&session_proto::Section>` impl and
//! [`sections_from_song`] are provided), then [`CueSchedule::build`]
//! precomputes every cue in seconds using the battle-tested
//! `CountInPattern` / `CountInCalculator` logic ported verbatim from
//! fts-guide.
//!
//! Cue times are computed against a fixed per-song tempo/time-signature
//! ([`GuideSongTiming`]). If the tempo map changes mid-song, rebuild the
//! schedule.

use crate::count_in::{CountInCalculator, CountInPattern};
use crate::samples::get_guide_key;
use crate::tts::tts_cue_key;

/// A section, in engine terms. Field-compatible with the legacy
/// `SectionInfo`; the session domain fills these from `session_proto`.
#[derive(Debug, Clone)]
pub struct GuideSection {
    /// Section start position in seconds
    pub start_seconds: f64,
    /// Section end position in seconds
    pub end_seconds: f64,
    /// Section name (e.g. "Verse 1")
    pub name: String,
    /// Count-in marker position (if this is the first section of a song)
    pub count_in_position: Option<f64>,
    /// SONGEND marker position (if this is the last section before SONGEND)
    pub song_end_position: Option<f64>,
    /// Whether this is the first section of its song
    pub is_first_section: bool,
    /// Section type name for guide mapping (e.g. "Verse", "Chorus", "Bridge")
    pub section_type_name: String,
    /// Section number (if numbered, e.g. 1, 2, 3)
    pub section_number: Option<u32>,
    /// Optional note to speak via TTS along with (instead of) the canned
    /// guide voice, e.g. "Keys only" or "Bridge in 2".
    pub spoken_note: Option<String>,
}

impl GuideSection {
    /// The full spoken form for TTS — "Verse 1", "Chorus", "Pre-Chorus 2" —
    /// built from the section *type* (full word) + number, rather than the
    /// abbreviated on-screen name ("VS 1", "CH", "PRE-CH 2"). This is what a
    /// human would say, so it reads well through Chatterbox.
    pub fn spoken_name(&self) -> String {
        match self.section_number {
            Some(n) => format!("{} {}", self.section_type_name, n),
            None => self.section_type_name.clone(),
        }
    }
}

impl From<&session_proto::Section> for GuideSection {
    fn from(s: &session_proto::Section) -> Self {
        Self {
            start_seconds: s.start_seconds,
            end_seconds: s.end_seconds,
            name: s.name.clone(),
            count_in_position: None,
            song_end_position: None,
            is_first_section: false,
            section_type_name: s.section_type.full_name(),
            section_number: s.number,
            spoken_note: None,
        }
    }
}

/// Build [`GuideSection`]s from a `session_proto::Song`: marks the first
/// section (attaching the song's count-in position, if any) and the last
/// section (attaching the SONGEND position).
pub fn sections_from_song(song: &session_proto::Song) -> Vec<GuideSection> {
    let mut sections: Vec<GuideSection> = song.sections.iter().map(GuideSection::from).collect();
    let count = sections.len();
    // The count-in must count INTO the first musical downbeat, not into the
    // synthetic "Count-In" section the song builder prepends. `SongBuilder`
    // sets `song.start_seconds` to the count-in START and inserts a
    // `SectionType::CountIn` section at index 0, so we attach
    // `count_in_position` to the first section that ISN'T that count-in
    // marker. `count_in_position` is the count START (`song.start_seconds`);
    // `CueSchedule::build` derives the measure count from the gap to that
    // section's own `start_seconds` (the real downbeat) and counts up to it.
    // (The old code pinned it to index 0 as `start_seconds - count_in`, which
    // for the first song landed the whole count at negative time and dropped
    // it — the "announced the section but never counted 1-2-3-4" bug.)
    let first_musical = song
        .sections
        .iter()
        .position(|s| s.section_type != session_proto::SectionType::CountIn);
    for (i, section) in sections.iter_mut().enumerate() {
        if i == 0 {
            section.is_first_section = true;
        }
        if Some(i) == first_musical {
            section.count_in_position = song.count_in_seconds.map(|_| song.start_seconds);
        }
        if i + 1 == count {
            section.song_end_position = Some(song.end_seconds);
        }
    }
    sections
}

/// Per-song timing needed to convert beats/measures to seconds.
#[derive(Debug, Clone, Copy)]
pub struct GuideSongTiming {
    /// Tempo in quarter notes per minute.
    pub tempo_bpm: f64,
    /// Time signature numerator.
    pub time_sig_num: u32,
    /// Time signature denominator.
    pub time_sig_den: u32,
}

impl Default for GuideSongTiming {
    fn default() -> Self {
        Self {
            tempo_bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
        }
    }
}

impl GuideSongTiming {
    /// Build from a `session_proto::Song`'s tempo / time signature fields.
    pub fn from_song(song: &session_proto::Song) -> Self {
        let default = Self::default();
        Self {
            tempo_bpm: song.tempo.unwrap_or(default.tempo_bpm),
            time_sig_num: song
                .time_signature
                .map(|ts| ts.numerator)
                .unwrap_or(default.time_sig_num),
            time_sig_den: song
                .time_signature
                .map(|ts| ts.denominator)
                .unwrap_or(default.time_sig_den),
        }
    }

    /// Duration of one time-signature beat unit in seconds.
    pub fn beat_seconds(&self) -> f64 {
        (60.0 / self.tempo_bpm) * (4.0 / f64::from(self.time_sig_den))
    }

    /// Duration of one measure in seconds.
    pub fn measure_seconds(&self) -> f64 {
        self.beat_seconds() * f64::from(self.time_sig_num)
    }

    /// Duration of one measure in quarter notes.
    pub fn measure_quarters(&self) -> f64 {
        f64::from(self.time_sig_num) * 4.0 / f64::from(self.time_sig_den)
    }

    /// Convert seconds to quarter notes.
    pub fn seconds_to_quarters(&self, seconds: f64) -> f64 {
        seconds * self.tempo_bpm / 60.0
    }
}

/// What a scheduled cue plays.
#[derive(Debug, Clone, PartialEq)]
pub enum CueEvent {
    /// Speak a count voice (0-based index: 0 speaks "1").
    Count { index: usize },
    /// Play a guide announcement. `keys` are tried in order against the
    /// sample bank; the first present wins (e.g. `["tts:Chorus", "Chorus_2"]`
    /// prefers a TTS-rendered cue over the canned voice library).
    Guide { keys: Vec<String> },
}

/// A cue at an absolute timeline position (seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledCue {
    pub time_seconds: f64,
    pub event: CueEvent,
}

/// Scheduling options (the count-in-related legacy plugin params).
#[derive(Debug, Clone)]
pub struct ScheduleOptions {
    /// Legacy "Offset Count By One" (affects /16 multi-measure count-ins).
    pub offset_count_by_one: bool,
    /// Legacy "Extend SONGEND Count": 2-measure count before SONGEND.
    pub extend_songend_count: bool,
    /// Legacy "Full Count for Odd Time".
    pub full_count_odd_time: bool,
    /// Legacy "Guide Replaces Beat 1": drop the count voice that coincides
    /// with a guide announcement.
    pub guide_replace_beat1: bool,
    /// Count only the FINAL measure of the count-in (a full "1 2 3 4"),
    /// leaving the earlier measures silent so the section announcement can
    /// breathe: "Verse 1  .  .  . | 1 2 3 4". When false, the full ported
    /// multi-measure ramp ("1 _ 2 _ | 1 2 3 4") is used instead. Applies to
    /// the section count-in only — the SONGEND count-out always ramps.
    pub count_final_measure_only: bool,
    /// Speak the section-name cue this many beat units before the section
    /// starts ("Chorus in 2..." style). Announced at the start of the
    /// count-in; two measures (8 beats in 4/4) is the default so the name
    /// lands clearly before the count.
    pub speak_lead_beats: f64,
}

impl Default for ScheduleOptions {
    fn default() -> Self {
        Self {
            offset_count_by_one: true,
            extend_songend_count: true,
            full_count_odd_time: true,
            guide_replace_beat1: true,
            count_final_measure_only: true,
            speak_lead_beats: 8.0,
        }
    }
}

/// The precomputed cue timeline for a song.
#[derive(Debug, Clone, Default)]
pub struct CueSchedule {
    /// Cues sorted by `time_seconds`.
    pub cues: Vec<ScheduledCue>,
}

impl CueSchedule {
    /// Build the cue timeline for `sections` under `timing`.
    ///
    /// - Count-in: for the first section with a count-in marker, the
    ///   measure count comes from `CountInCalculator` and the per-beat
    ///   count pattern from `CountInPattern` (both ported verbatim).
    /// - Section guides: each section gets a guide announcement
    ///   `speak_lead_beats` beat units before its start (clamped to the
    ///   count-in start, if any).
    /// - SONGEND: with `extend_songend_count`, a 2-measure full count into
    ///   the SONGEND position plus an "Ending" announcement.
    pub fn build(
        sections: &[GuideSection],
        timing: &GuideSongTiming,
        options: &ScheduleOptions,
    ) -> Self {
        let beat_secs = timing.beat_seconds();
        let measure_secs = timing.measure_seconds();
        let num = timing.time_sig_num as i32;
        let den = timing.time_sig_den as i32;

        let mut cues: Vec<ScheduledCue> = Vec::new();
        let mut guide_times: Vec<f64> = Vec::new();

        for section in sections {
            // ── Count-in into the section start ─────────────────────────
            if let Some(count_pos) = section.count_in_position {
                let total_measures = CountInCalculator::calculate_count_in_measures(
                    timing.seconds_to_quarters(count_pos),
                    timing.seconds_to_quarters(section.start_seconds),
                    timing.measure_quarters(),
                );
                let count_start = section.start_seconds - f64::from(total_measures) * measure_secs;
                Self::push_count_pattern(
                    &mut cues,
                    count_start,
                    total_measures,
                    num,
                    den,
                    beat_secs,
                    options,
                    options.count_final_measure_only,
                );
            }

            // ── Section guide announcement ───────────────────────────────
            let mut lead = options.speak_lead_beats * beat_secs;
            if let Some(count_pos) = section.count_in_position {
                // Never announce before the count-in marker.
                lead = lead.min(section.start_seconds - count_pos);
            }
            let guide_time = (section.start_seconds - lead).max(0.0);
            let mut keys = Vec::new();
            // Priority: a custom spoken note, then the REAL recorded guide
            // sample (numbered, then unnumbered), then TTS of the full spoken
            // name ("Verse 1"), then TTS of the abbreviated name — so real
            // audio wins and TTS only fills gaps.
            if let Some(note) = &section.spoken_note {
                keys.push(tts_cue_key(note));
            }
            keys.push(get_guide_key(
                &section.section_type_name,
                section.section_number,
            ));
            if section.section_number.is_some() {
                keys.push(get_guide_key(&section.section_type_name, None));
            }
            keys.push(tts_cue_key(&section.spoken_name()));
            keys.push(tts_cue_key(&section.name));
            cues.push(ScheduledCue {
                time_seconds: guide_time,
                event: CueEvent::Guide { keys },
            });
            guide_times.push(guide_time);

            // ── SONGEND count-out ────────────────────────────────────────
            if let (Some(song_end), true) =
                (section.song_end_position, options.extend_songend_count)
            {
                // Always 2 measures before SONGEND (legacy behavior).
                let total_measures = 2;
                let count_start = song_end - f64::from(total_measures) * measure_secs;
                // The count-OUT always ramps (never final-measure-only).
                Self::push_count_pattern(
                    &mut cues,
                    count_start,
                    total_measures,
                    num,
                    den,
                    beat_secs,
                    options,
                    false,
                );
                cues.push(ScheduledCue {
                    time_seconds: count_start.max(0.0),
                    event: CueEvent::Guide {
                        keys: vec![tts_cue_key("Ending"), "Ending_None".to_string()],
                    },
                });
                guide_times.push(count_start.max(0.0));
            }
        }

        // "Guide Replaces Beat 1": drop count cues that coincide with a
        // guide announcement (within half a beat).
        if options.guide_replace_beat1 {
            let tol = beat_secs * 0.5;
            cues.retain(|cue| {
                !(matches!(cue.event, CueEvent::Count { .. })
                    && guide_times
                        .iter()
                        .any(|gt| (cue.time_seconds - gt).abs() < tol))
            });
        }

        cues.sort_by(|a, b| a.time_seconds.total_cmp(&b.time_seconds));
        Self { cues }
    }

    fn push_count_pattern(
        cues: &mut Vec<ScheduledCue>,
        count_start: f64,
        total_measures: i32,
        time_sig_num: i32,
        time_sig_den: i32,
        beat_secs: f64,
        options: &ScheduleOptions,
        final_measure_only: bool,
    ) {
        // When `final_measure_only`, silence every measure but the last so a
        // section announcement can breathe ("Verse 1  .  .  . | 1 2 3 4").
        // The last measure is then counted as a standalone full measure
        // (total_measures = 1) so standard and odd-time patterns still fill
        // it; it stays positioned at its real time via `measure_index`.
        let first_measure = if final_measure_only {
            (total_measures - 1).max(0)
        } else {
            0
        };
        for measure_index in first_measure..total_measures {
            let (pat_measure, pat_total) = if final_measure_only {
                (0, 1)
            } else {
                (measure_index, total_measures)
            };
            for beat_in_measure in 1..=time_sig_num {
                let Some(count_number) = CountInPattern::should_count(
                    pat_measure,
                    beat_in_measure,
                    pat_total,
                    time_sig_num,
                    time_sig_den,
                    options.offset_count_by_one,
                    options.full_count_odd_time,
                ) else {
                    continue;
                };
                if !(1..=8).contains(&count_number) {
                    continue;
                }
                let time = count_start
                    + (f64::from(measure_index) * f64::from(time_sig_num)
                        + f64::from(beat_in_measure - 1))
                        * beat_secs;
                if time < 0.0 {
                    continue;
                }
                cues.push(ScheduledCue {
                    time_seconds: time,
                    event: CueEvent::Count {
                        index: (count_number - 1) as usize,
                    },
                });
            }
        }
    }

    /// Add a free-form spoken cue ("speak `text` at `time_seconds`").
    /// The matching PCM must be preloaded into the bank under
    /// [`tts_cue_key`]`(text)` (see `CueBank`).
    pub fn push_speak(&mut self, time_seconds: f64, text: &str) {
        let cue = ScheduledCue {
            time_seconds,
            event: CueEvent::Guide {
                keys: vec![tts_cue_key(text)],
            },
        };
        let idx = self
            .cues
            .partition_point(|c| c.time_seconds <= time_seconds);
        self.cues.insert(idx, cue);
    }

    /// Every distinct text this schedule would speak via TTS, in order —
    /// feed this to `CueBank::prepare` at setlist-build time.
    pub fn tts_texts(sections: &[GuideSection]) -> Vec<String> {
        let mut texts = Vec::new();
        for section in sections {
            if let Some(note) = &section.spoken_note {
                if !texts.contains(note) {
                    texts.push(note.clone());
                }
            }
            let spoken = section.spoken_name();
            if !texts.contains(&spoken) {
                texts.push(spoken);
            }
        }
        if !texts.iter().any(|t| t == "Ending") {
            texts.push("Ending".to_string());
        }
        texts
    }
}
