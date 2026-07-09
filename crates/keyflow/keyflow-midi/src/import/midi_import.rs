//! MIDI file import functionality.
//!
//! Parses Standard MIDI Files (.mid) using the `midly` crate and extracts
//! note data suitable for rhythm quantization and notation rendering.
//!
//! # Example
//!
//! ```ignore
//! use keyflow_midi::import::{MidiFile, MidiImportConfig};
//!
//! let bytes = std::fs::read("song.mid").unwrap();
//! let midi = MidiFile::parse(&bytes).unwrap();
//!
//! // Get notes from a specific track
//! let notes = midi.track_notes(1);
//!
//! // Quantize to notation
//! let config = QuantizeConfig::new(midi.ppq(), 480);
//! let quantized = quantize_duration_batch(&durations, &positions, &config);
//! ```

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use std::collections::HashMap;

use keyflow_proto::key::{Key, KeySpelling, SpellingMode};

use super::{Error, Result};

/// A parsed MIDI file with extracted musical data.
#[derive(Debug, Clone)]
pub struct MidiFile {
    /// Pulses per quarter note (ticks per beat)
    ppq: u32,
    /// Tracks containing note data
    tracks: Vec<MidiTrack>,
    /// Tempo map (tick -> microseconds per quarter note)
    tempo_map: Vec<TempoEvent>,
    /// Time signature changes
    time_signatures: Vec<TimeSignatureEvent>,
    /// Markers/cue points
    markers: Vec<MarkerEvent>,
    /// Track names
    track_names: Vec<Option<String>>,
    /// Swing ratio (0.5 = straight, 0.667 = triplet swing, None = unknown)
    swing: Option<f64>,
}

/// A single MIDI track with notes and metadata.
#[derive(Debug, Clone)]
pub struct MidiTrack {
    /// Track index (0-based)
    pub index: usize,
    /// Track name (from meta event)
    pub name: Option<String>,
    /// Notes in this track
    pub notes: Vec<MidiNote>,
    /// MIDI channel (if consistent across track)
    pub channel: Option<u8>,
}

/// A MIDI note with timing and pitch information.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiNote {
    /// MIDI pitch (0-127, where 60 = middle C)
    pub pitch: u8,
    /// Note-on velocity (0-127)
    pub velocity: u8,
    /// Start time in ticks
    pub start_tick: u32,
    /// Duration in ticks
    pub duration_ticks: u32,
    /// MIDI channel (0-15)
    pub channel: u8,
}

/// A tempo change event.
#[derive(Debug, Clone, Copy)]
pub struct TempoEvent {
    /// Tick position of tempo change
    pub tick: u32,
    /// Microseconds per quarter note
    pub microseconds_per_quarter: u32,
}

impl TempoEvent {
    /// Get tempo in BPM.
    #[must_use]
    pub fn bpm(&self) -> f64 {
        60_000_000.0 / self.microseconds_per_quarter as f64
    }
}

/// A time signature change event.
#[derive(Debug, Clone, Copy)]
pub struct TimeSignatureEvent {
    /// Tick position
    pub tick: u32,
    /// Numerator (beats per measure)
    pub numerator: u8,
    /// Denominator as power of 2 (4 = quarter note)
    pub denominator: u8,
}

/// A marker or cue point.
#[derive(Debug, Clone)]
pub struct MarkerEvent {
    /// Tick position
    pub tick: u32,
    /// Marker text
    pub text: String,
    /// Marker type
    pub marker_type: MarkerType,
}

/// Types of markers in MIDI files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerType {
    /// General marker (FF 06)
    Marker,
    /// Cue point (FF 07)
    CuePoint,
    /// Text event (FF 01)
    Text,
}

/// Configuration for MIDI import.
#[derive(Debug, Clone)]
pub struct MidiImportConfig {
    /// Target PPQ for output (default: 480)
    pub target_ppq: u32,
    /// Minimum note duration to include (in source ticks)
    pub min_note_duration: u32,
    /// Whether to merge overlapping notes on same pitch
    pub merge_overlapping: bool,
    /// Track indices to import (None = all tracks)
    pub track_filter: Option<Vec<usize>>,
}

impl Default for MidiImportConfig {
    fn default() -> Self {
        Self {
            target_ppq: 480,
            min_note_duration: 1,
            merge_overlapping: false,
            track_filter: None,
        }
    }
}

impl MidiFile {
    /// Parse a MIDI file from bytes.
    ///
    /// # Errors
    /// Returns an error if the file is not valid MIDI format.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let smf = Smf::parse(bytes).map_err(|e| Error::MidiParse(e.to_string()))?;

        // Extract timing info
        let ppq = match smf.header.timing {
            Timing::Metrical(ticks) => u32::from(ticks.as_int()),
            Timing::Timecode(fps, tpf) => {
                // Convert SMPTE timing to approximate PPQ
                // This is an approximation; SMPTE timing is frame-based
                let frames_per_second = match fps {
                    midly::Fps::Fps24 => 24.0,
                    midly::Fps::Fps25 => 25.0,
                    midly::Fps::Fps29 => 29.97,
                    midly::Fps::Fps30 => 30.0,
                };
                // Assume 120 BPM as default for conversion
                let ticks_per_second = frames_per_second * f64::from(tpf);
                (ticks_per_second / 2.0) as u32 // 2 beats per second at 120 BPM
            }
        };

        let mut tracks = Vec::new();
        let mut tempo_map = Vec::new();
        let mut time_signatures = Vec::new();
        let mut markers = Vec::new();
        let mut track_names = Vec::new();

        for (track_idx, track) in smf.tracks.iter().enumerate() {
            let mut current_tick: u32 = 0;
            let mut track_name: Option<String> = None;
            let mut notes: Vec<MidiNote> = Vec::new();
            let mut pending_notes: HashMap<(u8, u8), (u32, u8)> = HashMap::new(); // (pitch, channel) -> (start_tick, velocity)
            let mut track_channel: Option<u8> = None;

            for event in track {
                current_tick += event.delta.as_int();

                match event.kind {
                    TrackEventKind::Meta(meta) => match meta {
                        MetaMessage::TrackName(name) => {
                            track_name = Some(String::from_utf8_lossy(name).to_string());
                        }
                        MetaMessage::Tempo(tempo) => {
                            tempo_map.push(TempoEvent {
                                tick: current_tick,
                                microseconds_per_quarter: tempo.as_int(),
                            });
                        }
                        MetaMessage::TimeSignature(num, denom, _, _) => {
                            time_signatures.push(TimeSignatureEvent {
                                tick: current_tick,
                                numerator: num,
                                denominator: 1 << denom, // denom is power of 2
                            });
                        }
                        MetaMessage::Marker(text) => {
                            markers.push(MarkerEvent {
                                tick: current_tick,
                                text: String::from_utf8_lossy(text).to_string(),
                                marker_type: MarkerType::Marker,
                            });
                        }
                        MetaMessage::CuePoint(text) => {
                            markers.push(MarkerEvent {
                                tick: current_tick,
                                text: String::from_utf8_lossy(text).to_string(),
                                marker_type: MarkerType::CuePoint,
                            });
                        }
                        MetaMessage::Text(text) => {
                            markers.push(MarkerEvent {
                                tick: current_tick,
                                text: String::from_utf8_lossy(text).to_string(),
                                marker_type: MarkerType::Text,
                            });
                        }
                        _ => {}
                    },
                    TrackEventKind::Midi { channel, message } => {
                        let ch = channel.as_int();
                        if track_channel.is_none() {
                            track_channel = Some(ch);
                        }

                        match message {
                            MidiMessage::NoteOn { key, vel } => {
                                let pitch = key.as_int();
                                let velocity = vel.as_int();

                                if velocity > 0 {
                                    // Note on
                                    pending_notes.insert((pitch, ch), (current_tick, velocity));
                                } else {
                                    // Note on with velocity 0 = note off
                                    if let Some((start_tick, vel)) =
                                        pending_notes.remove(&(pitch, ch))
                                    {
                                        let duration = current_tick.saturating_sub(start_tick);
                                        if duration > 0 {
                                            notes.push(MidiNote {
                                                pitch,
                                                velocity: vel,
                                                start_tick,
                                                duration_ticks: duration,
                                                channel: ch,
                                            });
                                        }
                                    }
                                }
                            }
                            MidiMessage::NoteOff { key, .. } => {
                                let pitch = key.as_int();
                                if let Some((start_tick, vel)) = pending_notes.remove(&(pitch, ch))
                                {
                                    let duration = current_tick.saturating_sub(start_tick);
                                    if duration > 0 {
                                        notes.push(MidiNote {
                                            pitch,
                                            velocity: vel,
                                            start_tick,
                                            duration_ticks: duration,
                                            channel: ch,
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            // Sort notes by start time
            notes.sort_by_key(|n| n.start_tick);

            track_names.push(track_name.clone());

            if !notes.is_empty() {
                tracks.push(MidiTrack {
                    index: track_idx,
                    name: track_name,
                    notes,
                    channel: track_channel,
                });
            }
        }

        // Sort tempo map and time signatures by tick
        tempo_map.sort_by_key(|t| t.tick);
        time_signatures.sort_by_key(|t| t.tick);
        markers.sort_by_key(|m| m.tick);

        Ok(Self {
            ppq,
            tracks,
            tempo_map,
            time_signatures,
            markers,
            track_names,
            swing: None,
        })
    }

    /// Construct a `MidiFile` from pre-extracted components.
    ///
    /// Use this when you already have MIDI data from a DAW (e.g., REAPER)
    /// rather than a standalone `.mid` file. This lets you feed DAW track
    /// data into `generate_chart_text()` for identical output to the
    /// file-based pipeline.
    #[must_use]
    pub fn from_parts(
        ppq: u32,
        tracks: Vec<MidiTrack>,
        tempo_map: Vec<TempoEvent>,
        time_signatures: Vec<TimeSignatureEvent>,
        markers: Vec<MarkerEvent>,
        track_names: Vec<Option<String>>,
        swing: Option<f64>,
    ) -> Self {
        Self {
            ppq,
            tracks,
            tempo_map,
            time_signatures,
            markers,
            track_names,
            swing,
        }
    }

    /// Get the PPQ (pulses per quarter note) of this MIDI file.
    #[must_use]
    pub fn ppq(&self) -> u32 {
        self.ppq
    }

    /// Get the swing ratio (0.5 = straight, 0.667 = triplet, None = unknown).
    #[must_use]
    pub fn swing(&self) -> Option<f64> {
        self.swing
    }

    /// Get all tracks with note data.
    #[must_use]
    pub fn tracks(&self) -> &[MidiTrack] {
        &self.tracks
    }

    /// Get notes from a specific track by index.
    #[must_use]
    pub fn track_notes(&self, track_idx: usize) -> Option<&[MidiNote]> {
        self.tracks
            .iter()
            .find(|t| t.index == track_idx)
            .map(|t| t.notes.as_slice())
    }

    /// Get all notes across all tracks, sorted by start time.
    #[must_use]
    pub fn all_notes(&self) -> Vec<MidiNote> {
        let mut all: Vec<MidiNote> = self
            .tracks
            .iter()
            .flat_map(|t| t.notes.iter().copied())
            .collect();
        all.sort_by_key(|n| n.start_tick);
        all
    }

    /// Get all notes as references, sorted by start tick.
    pub fn all_notes_ref(&self) -> Vec<&MidiNote> {
        let mut all: Vec<&MidiNote> = self.tracks.iter().flat_map(|t| t.notes.iter()).collect();
        all.sort_by_key(|n| n.start_tick);
        all
    }

    /// Get the tempo map.
    #[must_use]
    pub fn tempo_map(&self) -> &[TempoEvent] {
        &self.tempo_map
    }

    /// Get the initial tempo in BPM.
    #[must_use]
    pub fn initial_tempo(&self) -> f64 {
        self.tempo_map.first().map(|t| t.bpm()).unwrap_or(120.0)
    }

    /// Get time signature changes.
    #[must_use]
    pub fn time_signatures(&self) -> &[TimeSignatureEvent] {
        &self.time_signatures
    }

    /// Get the initial time signature.
    #[must_use]
    pub fn initial_time_signature(&self) -> (u8, u8) {
        self.time_signatures
            .first()
            .map(|ts| (ts.numerator, ts.denominator))
            .unwrap_or((4, 4))
    }

    /// Get all markers.
    #[must_use]
    pub fn markers(&self) -> &[MarkerEvent] {
        &self.markers
    }

    /// Get track names.
    #[must_use]
    pub fn track_names(&self) -> &[Option<String>] {
        &self.track_names
    }

    /// Get the total duration in ticks.
    #[must_use]
    pub fn duration_ticks(&self) -> u32 {
        self.tracks
            .iter()
            .flat_map(|t| t.notes.iter())
            .map(|n| n.start_tick + n.duration_ticks)
            .max()
            .unwrap_or(0)
    }

    /// Convert tick position to seconds using the tempo map.
    #[must_use]
    pub fn tick_to_seconds(&self, tick: u32) -> f64 {
        let mut seconds = 0.0;
        let mut last_tick = 0u32;
        let mut current_tempo = 500_000u32; // Default 120 BPM

        for tempo_event in &self.tempo_map {
            if tempo_event.tick > tick {
                break;
            }
            // Add time from last tempo change to this one
            let ticks_in_segment = tempo_event.tick - last_tick;
            seconds += self.ticks_to_seconds_at_tempo(ticks_in_segment, current_tempo);
            last_tick = tempo_event.tick;
            current_tempo = tempo_event.microseconds_per_quarter;
        }

        // Add remaining ticks at current tempo
        let remaining_ticks = tick - last_tick;
        seconds += self.ticks_to_seconds_at_tempo(remaining_ticks, current_tempo);

        seconds
    }

    fn ticks_to_seconds_at_tempo(&self, ticks: u32, microseconds_per_quarter: u32) -> f64 {
        let seconds_per_tick = (microseconds_per_quarter as f64 / 1_000_000.0) / self.ppq as f64;
        ticks as f64 * seconds_per_tick
    }

    /// Extract note durations for quantization.
    ///
    /// Returns vectors of (duration_ticks, start_position_ticks) for each note.
    #[must_use]
    pub fn extract_durations(&self, track_idx: usize) -> (Vec<i32>, Vec<i32>) {
        let notes = match self.track_notes(track_idx) {
            Some(n) => n,
            None => return (Vec::new(), Vec::new()),
        };

        let durations: Vec<i32> = notes.iter().map(|n| n.duration_ticks as i32).collect();
        let positions: Vec<i32> = notes.iter().map(|n| n.start_tick as i32).collect();

        (durations, positions)
    }

    /// Find marker by name.
    #[must_use]
    pub fn find_marker(&self, name: &str) -> Option<&MarkerEvent> {
        self.markers.iter().find(|m| m.text == name)
    }

    /// Get the tick position of SONGSTART marker (or 0 if not found).
    #[must_use]
    pub fn songstart_tick(&self) -> u32 {
        self.find_marker("SONGSTART").map(|m| m.tick).unwrap_or(0)
    }

    /// Convert a tick position to a musical position (measure, beat, subdivision).
    ///
    /// The position is relative to SONGSTART if present, otherwise absolute.
    /// Returns (measure, beat, subdivision_ticks) where:
    /// - measure: if `SONGSTART` marker exists, that tick is measure 1.
    ///   otherwise, tick 0 is measure 0 (legacy behavior).
    /// - beat: 0-indexed beat within measure
    /// - subdivision_ticks: remaining ticks within the beat
    ///
    /// Takes into account time signature changes throughout the piece.
    #[must_use]
    pub fn tick_to_musical_position(&self, tick: u32) -> MusicalPosition {
        let songstart_opt = self.find_marker("SONGSTART").map(|m| m.tick);
        let songstart = songstart_opt.unwrap_or(0);
        let measure_origin = if songstart_opt.is_some() { 1 } else { 0 };

        // If tick is before songstart, return negative measure (count-in)
        if songstart_opt.is_some() && tick < songstart {
            let ticks_before = songstart - tick;
            let (ts_num, ts_denom) = self.initial_time_signature();
            let ticks_per_beat = self.ticks_per_beat_for_denominator(ts_denom);
            let ticks_per_measure = ticks_per_beat * u32::from(ts_num);

            let measures_before = ticks_before.div_ceil(ticks_per_measure);
            let remaining = measures_before * ticks_per_measure - ticks_before;
            let beat = (remaining / ticks_per_beat) as i32;
            let subdivision = (remaining % ticks_per_beat) as i32;

            return MusicalPosition {
                measure: measure_origin - (measures_before as i32),
                beat,
                subdivision,
            };
        }

        // Process from songstart
        let relative_tick = tick - songstart;

        // Track position through time signature changes
        let mut current_tick: u32 = 0;
        let mut current_measure: i32 = measure_origin;
        let mut ts_idx = 0;

        // Find time signatures after songstart
        let effective_time_sigs: Vec<_> = self
            .time_signatures
            .iter()
            .filter(|ts| ts.tick >= songstart)
            .map(|ts| TimeSignatureEvent {
                tick: ts.tick - songstart,
                numerator: ts.numerator,
                denominator: ts.denominator,
            })
            .collect();

        let initial_ts = self.initial_time_signature();
        let mut current_ts = initial_ts;

        loop {
            let ticks_per_beat = self.ticks_per_beat_for_denominator(current_ts.1);
            let ticks_per_measure = ticks_per_beat * u32::from(current_ts.0);

            // Check if there's another time sig change before we reach target tick
            let next_ts = effective_time_sigs.get(ts_idx);
            let next_ts_tick = next_ts.map(|ts| ts.tick).unwrap_or(u32::MAX);

            if relative_tick < next_ts_tick {
                // Target is in current time signature region
                let ticks_in_region = relative_tick - current_tick;
                let measures_in_region = ticks_in_region / ticks_per_measure;
                let remaining = ticks_in_region % ticks_per_measure;
                let beat = (remaining / ticks_per_beat) as i32;
                let subdivision = (remaining % ticks_per_beat) as i32;

                return MusicalPosition {
                    measure: current_measure + measures_in_region as i32,
                    beat,
                    subdivision,
                };
            }

            // Move through this time signature region
            let ticks_in_region = next_ts_tick - current_tick;
            let measures_in_region = ticks_in_region / ticks_per_measure;

            current_measure += measures_in_region as i32;
            current_tick = next_ts_tick;

            if let Some(ts) = next_ts {
                current_ts = (ts.numerator, ts.denominator);
            }
            ts_idx += 1;

            // Safety: prevent infinite loop
            if ts_idx > self.time_signatures.len() + 1 {
                break;
            }
        }

        // Fallback (shouldn't reach here normally)
        let (ts_num, ts_denom) = initial_ts;
        let ticks_per_beat = self.ticks_per_beat_for_denominator(ts_denom);
        let ticks_per_measure = ticks_per_beat * u32::from(ts_num);

        let measure = (relative_tick / ticks_per_measure) as i32;
        let remaining = relative_tick % ticks_per_measure;
        let beat = (remaining / ticks_per_beat) as i32;
        let subdivision = (remaining % ticks_per_beat) as i32;

        MusicalPosition {
            measure: measure + measure_origin,
            beat,
            subdivision,
        }
    }

    /// Get ticks per beat for a given time signature denominator.
    fn ticks_per_beat_for_denominator(&self, denominator: u8) -> u32 {
        // PPQ is ticks per quarter note
        // If denominator is 4 (quarter), ticks_per_beat = ppq
        // If denominator is 8 (eighth), ticks_per_beat = ppq / 2
        // If denominator is 2 (half), ticks_per_beat = ppq * 2
        match denominator {
            1 => self.ppq * 4,  // Whole note
            2 => self.ppq * 2,  // Half note
            4 => self.ppq,      // Quarter note
            8 => self.ppq / 2,  // Eighth note
            16 => self.ppq / 4, // Sixteenth note
            _ => self.ppq,      // Default to quarter
        }
    }

    /// Get chord markers (markers that look like chord symbols).
    ///
    /// Filters out special markers like "SONGSTART", "Count-In", section names, etc.
    #[must_use]
    pub fn chord_markers(&self) -> Vec<ChordMarker> {
        self.chord_markers_with_position(|tick| self.tick_to_musical_position(tick))
    }
}

/// Musical position in measure.beat.subdivision format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MusicalPosition {
    /// Measure number (SONGSTART = 1, can be <= 0 for count-in)
    pub measure: i32,
    /// Beat within measure (0-indexed)
    pub beat: i32,
    /// Subdivision ticks within beat
    pub subdivision: i32,
}

impl MusicalPosition {
    /// Format as M.B.S string (e.g., "1.2.0" for measure 1, beat 2, subdivision 0).
    #[must_use]
    pub fn to_string_1indexed(&self) -> String {
        format!(
            "{}.{}.{}",
            self.measure + 1,
            self.beat + 1,
            self.subdivision
        )
    }
}

impl MidiFile {
    fn chord_markers_with_position<F>(&self, mut position_for_tick: F) -> Vec<ChordMarker>
    where
        F: FnMut(u32) -> MusicalPosition,
    {
        let mut current_key: Option<Key> = None;
        let mut out = Vec::new();

        for marker in &self.markers {
            if let Some(key) = parse_key_signature_marker(&marker.text) {
                current_key = Some(key);
                continue;
            }

            if !is_chord_marker(&marker.text) {
                continue;
            }

            let position = position_for_tick(marker.tick);
            let mut chord_name = normalize_chord_name(&marker.text);
            if let Some(key) = current_key.as_ref() {
                chord_name = respell_chord_name_for_key(&chord_name, key);
            }

            out.push(ChordMarker {
                tick: marker.tick,
                chord_name,
                position,
            });
        }

        out
    }

    /// Convert a tick position to an absolute MIDI measure position (where tick 0 = measure 1).
    ///
    /// Unlike `tick_to_musical_position` which is relative to SONGSTART,
    /// this returns the absolute measure number as it would appear in a DAW.
    #[must_use]
    pub fn tick_to_absolute_measure(&self, tick: u32) -> MusicalPosition {
        let _ppq = self.ppq;
        let (ts_num, ts_denom) = self.initial_time_signature();
        let ticks_per_beat = self.ticks_per_beat_for_denominator(ts_denom);
        let ticks_per_measure = ticks_per_beat * u32::from(ts_num);

        // Calculate absolute position from tick 0
        let measure = (tick / ticks_per_measure) as i32;
        let remaining = tick % ticks_per_measure;
        let beat = (remaining / ticks_per_beat) as i32;
        let subdivision = (remaining % ticks_per_beat) as i32;

        // Add 1 because MIDI measures are 1-indexed
        MusicalPosition {
            measure: measure + 1,
            beat,
            subdivision,
        }
    }

    /// Get section markers with absolute MIDI measure positions.
    #[must_use]
    pub fn section_markers_absolute(&self) -> Vec<SectionMarker> {
        self.collect_section_markers(|tick| self.tick_to_absolute_measure(tick))
    }

    /// Get key signature markers with absolute MIDI measure positions.
    #[must_use]
    pub fn key_signature_markers_absolute(&self) -> Vec<KeySignatureMarker> {
        self.markers
            .iter()
            .filter_map(|m| {
                let key = parse_key_signature_marker(&m.text)?;
                Some(KeySignatureMarker {
                    tick: m.tick,
                    key_name: m.text.trim().to_string(),
                    key,
                    position: self.tick_to_absolute_measure(m.tick),
                })
            })
            .collect()
    }

    /// Get chord markers with absolute MIDI measure positions.
    #[must_use]
    pub fn chord_markers_absolute(&self) -> Vec<ChordMarker> {
        self.chord_markers_with_position(|tick| self.tick_to_absolute_measure(tick))
    }

    fn collect_section_markers<F>(&self, mut position_for_tick: F) -> Vec<SectionMarker>
    where
        F: FnMut(u32) -> MusicalPosition,
    {
        let explicit = self
            .markers
            .iter()
            .filter_map(|marker| {
                parse_keyflow_section_metadata(&marker.text).map(|meta| (marker, meta))
            })
            .map(|(marker, metadata)| {
                let mut position = position_for_tick(marker.tick);
                position.measure = metadata.start_measure;
                let (section_type, number) = SectionType::from_marker_text(&metadata.name);
                SectionMarker {
                    tick: marker.tick,
                    name: metadata.name,
                    position,
                    section_type,
                    number,
                    explicit_start_measure: Some(metadata.start_measure),
                    explicit_length: Some(metadata.length),
                }
            })
            .collect::<Vec<_>>();

        let explicit_keys = explicit
            .iter()
            .map(|marker| (marker.tick, marker.name.clone()))
            .collect::<std::collections::HashSet<_>>();

        let mut markers = self
            .markers
            .iter()
            .filter(|marker| is_section_marker(&marker.text))
            .filter(|marker| !explicit_keys.contains(&(marker.tick, marker.text.clone())))
            .map(|marker| {
                let position = position_for_tick(marker.tick);
                let (section_type, number) = SectionType::from_marker_text(&marker.text);
                SectionMarker {
                    tick: marker.tick,
                    name: marker.text.clone(),
                    position,
                    section_type,
                    number,
                    explicit_start_measure: None,
                    explicit_length: None,
                }
            })
            .collect::<Vec<_>>();

        markers.extend(explicit);
        markers.sort_by_key(|marker| marker.tick);
        markers
    }
}

impl std::fmt::Display for MusicalPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.measure, self.beat, self.subdivision)
    }
}

/// A chord marker from a MIDI file with its position.
#[derive(Debug, Clone)]
pub struct ChordMarker {
    /// Original tick position
    pub tick: u32,
    /// Chord name/symbol
    pub chord_name: String,
    /// Musical position (measure, beat, subdivision)
    pub position: MusicalPosition,
}

/// A key signature marker from a MIDI file with its position.
#[derive(Debug, Clone)]
pub struct KeySignatureMarker {
    /// Original tick position.
    pub tick: u32,
    /// Original marker text (e.g. "#D", "bBb").
    pub key_name: String,
    /// Parsed key.
    pub key: Key,
    /// Musical position (measure, beat, subdivision).
    pub position: MusicalPosition,
}

/// Push/pull timing information for a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPull {
    /// On the beat (no push/pull)
    OnBeat,
    /// Pushed (anticipated) - played BEFORE the beat
    /// The value is the subdivision amount in ticks
    Push(PushPullAmount),
    /// Pulled (delayed) - played AFTER the beat
    /// The value is the subdivision amount in ticks
    Pull(PushPullAmount),
}

/// Amount of push/pull timing adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPullAmount {
    /// Triplet eighth (1/3 beat = 320 ticks at 960 PPQ)
    TripletEighth,
    /// Triplet quarter (2/3 beat = 640 ticks at 960 PPQ)
    TripletQuarter,
    /// Straight eighth (1/2 beat = 480 ticks at 960 PPQ)
    StraightEighth,
    /// Straight sixteenth (1/4 beat = 240 ticks at 960 PPQ)
    StraightSixteenth,
    /// Other amount (ticks)
    Other(i32),
}

impl PushPullAmount {
    /// Get ticks for this amount at 960 PPQ.
    #[must_use]
    pub fn ticks_960ppq(&self) -> i32 {
        match self {
            Self::TripletEighth => 320,
            Self::TripletQuarter => 640,
            Self::StraightEighth => 480,
            Self::StraightSixteenth => 240,
            Self::Other(t) => *t,
        }
    }

    /// Create from ticks at 960 PPQ.
    #[must_use]
    pub fn from_ticks(ticks: i32) -> Self {
        // Allow small tolerance for quantization
        let tolerance = 20;
        if (ticks - 320).abs() <= tolerance {
            Self::TripletEighth
        } else if (ticks - 640).abs() <= tolerance {
            Self::TripletQuarter
        } else if (ticks - 480).abs() <= tolerance {
            Self::StraightEighth
        } else if (ticks - 240).abs() <= tolerance {
            Self::StraightSixteenth
        } else {
            Self::Other(ticks)
        }
    }

    /// Get keyflow notation for this amount.
    #[must_use]
    pub fn keyflow_notation(&self) -> &'static str {
        match self {
            Self::TripletEighth => "t",     // triplet eighth
            Self::TripletQuarter => "T",    // triplet quarter (2x triplet eighth)
            Self::StraightEighth => "e",    // straight eighth
            Self::StraightSixteenth => "s", // straight sixteenth
            Self::Other(_) => "?",
        }
    }
}

impl ChordMarker {
    /// Detect the push/pull timing of this chord.
    ///
    /// Push/pull is detected by analyzing the subdivision:
    /// - 0 = on beat (no push/pull)
    /// - Small positive (< half beat) = pull (delayed)
    /// - Large positive (> half beat) = push of next beat (anticipated)
    #[must_use]
    pub fn detect_push_pull(&self, ppq: u32) -> PushPull {
        let subdivision = self.position.subdivision;

        if subdivision == 0 {
            return PushPull::OnBeat;
        }

        let half_beat = ppq as i32 / 2;

        if subdivision <= half_beat {
            // Subdivision is in first half of beat = PULL (delayed from this beat)
            let amount = PushPullAmount::from_ticks(subdivision);
            PushPull::Pull(amount)
        } else {
            // Subdivision is in second half of beat = PUSH (anticipating next beat)
            // The push amount is (ppq - subdivision), i.e., how early before next beat
            let push_ticks = ppq as i32 - subdivision;
            let amount = PushPullAmount::from_ticks(push_ticks);
            PushPull::Push(amount)
        }
    }

    /// Generate keyflow notation for this chord.
    ///
    /// Examples:
    /// - "Ab9" (on beat)
    /// - "'tAb9" (pushed by triplet eighth)
    /// - "Ab9't" (pulled by triplet eighth)
    /// - "'TAb9" (pushed by triplet quarter = 2/3 beat)
    #[must_use]
    pub fn to_keyflow_notation(&self, ppq: u32) -> String {
        let push_pull = self.detect_push_pull(ppq);

        match push_pull {
            PushPull::OnBeat => self.chord_name.clone(),
            PushPull::Push(amount) => {
                format!("'{}{}", amount.keyflow_notation(), self.chord_name)
            }
            PushPull::Pull(amount) => {
                format!("{}{}'", self.chord_name, amount.keyflow_notation())
            }
        }
    }

    /// Get the "logical" beat this chord belongs to.
    ///
    /// For pushed chords, returns the NEXT beat (the one being anticipated).
    /// For pulled chords, returns the CURRENT beat.
    #[must_use]
    pub fn logical_beat(&self, ppq: u32) -> i32 {
        let push_pull = self.detect_push_pull(ppq);

        match push_pull {
            PushPull::OnBeat | PushPull::Pull(_) => self.position.beat,
            PushPull::Push(_) => self.position.beat + 1,
        }
    }

    /// Get the "logical" measure this chord belongs to.
    ///
    /// For pushed chords on beat 4 (0-indexed: beat 3), if they push past
    /// the measure boundary, returns the NEXT measure.
    #[must_use]
    pub fn logical_measure(&self, ppq: u32, beats_per_measure: i32) -> i32 {
        let logical_beat = self.logical_beat(ppq);

        if logical_beat >= beats_per_measure {
            // Pushed into next measure
            self.position.measure + 1
        } else {
            self.position.measure
        }
    }

    /// Calculate duration in ticks to the next chord.
    ///
    /// Returns `None` if this is the last chord.
    #[must_use]
    pub fn duration_to_next(&self, next_chord: Option<&ChordMarker>) -> Option<u32> {
        next_chord.map(|next| next.tick - self.tick)
    }

    /// Get duration in beats.
    ///
    /// # Arguments
    /// * `duration_ticks` - Duration in ticks
    /// * `ppq` - Pulses per quarter note
    #[must_use]
    pub fn duration_in_beats(duration_ticks: u32, ppq: u32) -> f64 {
        f64::from(duration_ticks) / f64::from(ppq)
    }

    /// Format duration as a note value string.
    ///
    /// At 960 PPQ:
    /// - 3840 ticks = whole note (4 beats)
    /// - 1920 ticks = half note (2 beats)
    /// - 960 ticks = quarter note (1 beat)
    /// - 480 ticks = eighth note (1/2 beat)
    /// - 2880 ticks = dotted half (3 beats)
    #[must_use]
    pub fn format_duration(duration_ticks: u32, ppq: u32) -> String {
        let beats = Self::duration_in_beats(duration_ticks, ppq);

        // Common durations at 960 PPQ
        match duration_ticks {
            t if t == ppq * 4 => "whole".to_string(),
            t if t == ppq * 3 => "dotted half".to_string(),
            t if t == ppq * 2 => "half".to_string(),
            t if t == ppq * 3 / 2 => "dotted quarter".to_string(),
            t if t == ppq => "quarter".to_string(),
            t if t == ppq / 2 => "eighth".to_string(),
            t if t == ppq / 4 => "sixteenth".to_string(),
            _ => format!("{:.2} beats", beats),
        }
    }
}

/// A rhythm element in a measure - either a chord or a rest.
#[derive(Debug, Clone)]
pub enum RhythmElement {
    /// A chord with its symbol and duration
    Chord {
        /// Chord symbol (e.g., "Ab9", "Cm7")
        symbol: String,
        /// Duration in ticks
        duration_ticks: u32,
        /// Push/pull timing
        push_pull: PushPull,
    },
    /// A rest with its duration
    Rest {
        /// Duration in ticks
        duration_ticks: u32,
    },
}

impl RhythmElement {
    /// Format this rhythm element as keyflow notation.
    ///
    /// # Arguments
    /// * `ppq` - Pulses per quarter note (for duration calculation)
    /// * `use_triplet_default` - If true, triplet pushes use simple `'` notation
    #[must_use]
    pub fn to_keyflow_notation(&self, ppq: u32, use_triplet_default: bool) -> String {
        match self {
            RhythmElement::Chord {
                symbol,
                duration_ticks,
                push_pull,
            } => {
                let normalized = normalize_chord_name(symbol);
                let duration_suffix = format_duration_suffix(*duration_ticks, ppq);

                match push_pull {
                    PushPull::OnBeat => format!("{}{}", normalized, duration_suffix),
                    PushPull::Push(amount) => {
                        // Check if this is a triplet-based push
                        let is_triplet = matches!(
                            amount,
                            PushPullAmount::TripletEighth | PushPullAmount::TripletQuarter
                        );
                        let notation = if use_triplet_default && is_triplet {
                            "'".to_string() // Use simple notation for triplet
                        } else {
                            format!("'{}", amount.keyflow_notation())
                        };
                        format!("{}{}{}", notation, normalized, duration_suffix)
                    }
                    PushPull::Pull(amount) => {
                        let is_triplet = matches!(
                            amount,
                            PushPullAmount::TripletEighth | PushPullAmount::TripletQuarter
                        );
                        let notation = if use_triplet_default && is_triplet {
                            "'".to_string()
                        } else {
                            format!("{}'", amount.keyflow_notation())
                        };
                        format!("{}{}{}", normalized, duration_suffix, notation)
                    }
                }
            }
            RhythmElement::Rest { duration_ticks } => format_rest(*duration_ticks, ppq),
        }
    }
}

/// Format duration as a keyflow suffix (e.g., "_8t" for triplet eighth).
#[must_use]
pub fn format_duration_suffix(duration_ticks: u32, ppq: u32) -> String {
    // Common durations at 960 PPQ
    let triplet_eighth = ppq / 3; // 320 at 960 PPQ
    let triplet_quarter = ppq * 2 / 3; // 640 at 960 PPQ
    let eighth = ppq / 2; // 480 at 960 PPQ
    let quarter = ppq; // 960 at 960 PPQ
    let half = ppq * 2; // 1920 at 960 PPQ
    let whole = ppq * 4; // 3840 at 960 PPQ
    let sixteenth = ppq / 4; // 240 at 960 PPQ

    // Allow small tolerance for timing variations
    let tolerance = ppq / 24; // ~40 ticks at 960 PPQ

    let is_close = |target: u32| -> bool { duration_ticks.abs_diff(target) <= tolerance };

    if is_close(triplet_eighth) {
        "_8t".to_string()
    } else if is_close(triplet_quarter) {
        "_4t".to_string()
    } else if is_close(sixteenth) {
        "_16".to_string()
    } else if is_close(eighth) {
        "_8".to_string()
    } else if is_close(quarter) {
        "_4".to_string()
    } else if is_close(half) {
        "_2".to_string()
    } else if is_close(whole) {
        "_1".to_string()
    } else {
        // Default: no suffix (use context for duration)
        String::new()
    }
}

/// Format a rest duration as keyflow notation (e.g., "r8t" for triplet eighth rest).
///
/// For triplet-based grooves, prefers triplet notation (e.g., "r8t r8t r8t" over "r4").
#[must_use]
pub fn format_rest(duration_ticks: u32, ppq: u32) -> String {
    let triplet_eighth = ppq / 3; // 320 at 960 PPQ
    let triplet_quarter = ppq * 2 / 3; // 640 at 960 PPQ
    let eighth = ppq / 2; // 480 at 960 PPQ
    let quarter = ppq; // 960 at 960 PPQ
    let half = ppq * 2; // 1920 at 960 PPQ
    let whole = ppq * 4; // 3840 at 960 PPQ
    let sixteenth = ppq / 4; // 240 at 960 PPQ

    let tolerance = ppq / 24;

    let is_close = |target: u32| -> bool { duration_ticks.abs_diff(target) <= tolerance };

    if is_close(triplet_eighth) {
        "r8t".to_string()
    } else if is_close(triplet_quarter) {
        "r4t".to_string()
    } else if is_close(sixteenth) {
        "r16".to_string()
    } else if is_close(eighth) {
        "r8".to_string()
    } else if is_close(quarter) {
        "r4".to_string()
    } else if is_close(half) {
        "r2".to_string()
    } else if is_close(whole) {
        "r1".to_string()
    } else {
        // For non-standard durations, break into components
        decompose_rest_duration(duration_ticks, ppq)
    }
}

/// Decompose a rest duration into multiple standard rest values.
///
/// Prefers triplet-based notation for grooves that use triplet subdivisions.
fn decompose_rest_duration(duration_ticks: u32, ppq: u32) -> String {
    let mut remaining = duration_ticks;
    let mut rests = Vec::new();

    let whole = ppq * 4;
    let half = ppq * 2;
    let quarter = ppq;
    let triplet_eighth = ppq / 3;
    let eighth = ppq / 2;
    let sixteenth = ppq / 4;

    // Decompose from largest to smallest
    while remaining >= whole {
        rests.push("r1");
        remaining -= whole;
    }
    while remaining >= half {
        rests.push("r2");
        remaining -= half;
    }

    // For quarter notes, check if we should use triplet eighths instead
    // If the remaining duration is exactly a quarter note and divides evenly
    // by triplet eighth, use triplet notation for groove consistency
    while remaining >= quarter {
        // Check if quarter note is better expressed as triplet eighths
        if remaining == quarter {
            // 960 ticks = 3 triplet eighths (320 each)
            rests.push("r8t");
            rests.push("r8t");
            rests.push("r8t");
            remaining -= quarter;
        } else {
            rests.push("r4");
            remaining -= quarter;
        }
    }

    // Handle remaining with triplet eighths first (for triplet grooves)
    while remaining >= triplet_eighth {
        rests.push("r8t");
        remaining -= triplet_eighth;
    }

    // Fall back to straight subdivisions
    while remaining >= eighth {
        rests.push("r8");
        remaining -= eighth;
    }
    while remaining >= sixteenth {
        rests.push("r16");
        remaining -= sixteenth;
    }

    if rests.is_empty() {
        "r4".to_string() // Default to quarter rest
    } else {
        rests.join(" ")
    }
}

/// Generate rhythm elements for a measure including rests.
///
/// Takes the chords in a measure and fills in rests for the gaps.
///
/// # Arguments
/// * `chords` - Chord markers within this measure (sorted by tick)
/// * `measure_start_tick` - Tick position of measure start
/// * `measure_duration_ticks` - Total duration of measure in ticks
/// * `ppq` - Pulses per quarter note
/// * `default_chord_duration` - Default duration for chords (e.g., triplet eighth)
#[must_use]
pub fn generate_measure_rhythm(
    chords: &[&ChordMarker],
    measure_start_tick: u32,
    measure_duration_ticks: u32,
    ppq: u32,
    default_chord_duration: u32,
) -> Vec<RhythmElement> {
    let mut elements = Vec::new();
    let mut current_tick = measure_start_tick;
    let measure_end_tick = measure_start_tick + measure_duration_ticks;

    for (i, chord) in chords.iter().enumerate() {
        // Calculate rest before this chord
        if chord.tick > current_tick {
            let rest_duration = chord.tick - current_tick;
            if rest_duration > ppq / 12 {
                // Only add rest if significant (> ~80 ticks)
                elements.push(RhythmElement::Rest {
                    duration_ticks: rest_duration,
                });
            }
        }

        // Determine chord duration
        // If there's a next chord, use the gap; otherwise use default
        let chord_duration = if let Some(next_chord) = chords.get(i + 1) {
            let gap = next_chord.tick.saturating_sub(chord.tick);
            // Cap at the default duration - chord itself is short, rest fills gap
            gap.min(default_chord_duration)
        } else {
            default_chord_duration
        };

        let push_pull = chord.detect_push_pull(ppq);

        elements.push(RhythmElement::Chord {
            symbol: chord.chord_name.clone(),
            duration_ticks: chord_duration,
            push_pull,
        });

        // Update current position to after the chord
        current_tick = chord.tick + chord_duration;
    }

    // Add trailing rest if measure isn't filled
    if current_tick < measure_end_tick {
        let rest_duration = measure_end_tick - current_tick;
        if rest_duration > ppq / 12 {
            elements.push(RhythmElement::Rest {
                duration_ticks: rest_duration,
            });
        }
    }

    elements
}

/// Format a measure's rhythm elements as keyflow notation.
///
/// # Arguments
/// * `elements` - Rhythm elements to format
/// * `ppq` - Pulses per quarter note
/// * `use_triplet_default` - If true, triplet pushes use simple `'` notation
#[must_use]
pub fn format_measure_rhythm(
    elements: &[RhythmElement],
    ppq: u32,
    use_triplet_default: bool,
) -> String {
    elements
        .iter()
        .map(|e| e.to_keyflow_notation(ppq, use_triplet_default))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Section marker from a MIDI file with its position.
#[derive(Debug, Clone)]
pub struct SectionMarker {
    /// Original tick position
    pub tick: u32,
    /// Section name/label
    pub name: String,
    /// Musical position (measure, beat, subdivision)
    pub position: MusicalPosition,
    /// Section type parsed from the name
    pub section_type: SectionType,
    /// Section number (e.g., "1" from "VS 1")
    pub number: Option<u32>,
    /// Explicit start measure supplied by the exporter, if present.
    pub explicit_start_measure: Option<i32>,
    /// Explicit section length supplied by the exporter, if present.
    pub explicit_length: Option<i32>,
}

/// Types of sections in a song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionType {
    /// Title/header marker
    Title,
    /// Count-in measures before song
    CountIn,
    /// Song start marker
    SongStart,
    /// Introduction
    Intro,
    /// Verse
    Verse,
    /// Pre-Chorus
    PreChorus,
    /// Chorus
    Chorus,
    /// Bridge
    Bridge,
    /// Instrumental break
    Instrumental,
    /// Solo section (e.g., trumpet solo, guitar solo)
    Solo,
    /// Interlude (transitional section)
    Interlude,
    /// Outro/ending
    Outro,
    /// Generic hit/stab marker
    Hits,
    /// Unknown/custom section
    Other,
}

impl SectionType {
    /// Parse section type from marker text.
    pub fn from_marker_text(text: &str) -> (Self, Option<u32>) {
        let text_lower = text.to_lowercase();
        let text_trimmed = text.trim();

        // Check for count-in and song markers
        if text_lower.contains("count-in") || text_lower.contains("countin") {
            return (Self::CountIn, None);
        }
        if text_lower == "songstart" {
            return (Self::SongStart, None);
        }
        if text_lower == "songend" {
            return (Self::Other, None);
        }
        if text_lower == "ending" || text_lower == "end" {
            return (Self::Outro, None);
        }

        // Check for hits/stabs
        if text_lower == "hits" || text_lower.starts_with("hit") {
            return (Self::Hits, None);
        }

        // Parse abbreviated section names (VS 1, CH 2, BR, etc.)
        let parts: Vec<&str> = text_trimmed.split_whitespace().collect();
        let section_abbrev = parts.first().map(|s| s.to_uppercase()).unwrap_or_default();
        let number = parts.get(1).and_then(|s| s.parse::<u32>().ok());

        let section_type = match section_abbrev.as_str() {
            "VS" | "V" | "VERSE" => Self::Verse,
            "CH" | "C" | "CHORUS" => Self::Chorus,
            "BR" | "B" | "BRIDGE" => Self::Bridge,
            "PC" | "PRE" | "PRE-CHORUS" | "PRECHORUS" => Self::PreChorus,
            "IN" | "INTRO" => Self::Intro,
            "OUT" | "OUTRO" => Self::Outro,
            "INST" | "INSTRUMENTAL" => Self::Instrumental,
            "SOLO" => Self::Solo,
            "INT" | "INTERLUDE" => Self::Interlude,
            _ => {
                // Check for longer patterns
                if text_lower.contains("intro") {
                    Self::Intro
                } else if text_lower.contains("verse") {
                    Self::Verse
                } else if text_lower.contains("chorus") {
                    Self::Chorus
                } else if text_lower.contains("bridge") {
                    Self::Bridge
                } else if text_lower.contains("pre") {
                    Self::PreChorus
                } else if text_lower.contains("outro") {
                    Self::Outro
                } else if text_lower.contains("interlude") {
                    Self::Interlude
                } else if text_lower.contains("solo") {
                    Self::Solo
                } else if text_lower.contains("inst") {
                    Self::Instrumental
                } else {
                    Self::Other
                }
            }
        };

        (section_type, number)
    }
}

impl MidiFile {
    /// Get section markers (structural markers like VS 1, CH 1, Intro, etc.).
    #[must_use]
    pub fn section_markers(&self) -> Vec<SectionMarker> {
        self.collect_section_markers(|tick| self.tick_to_musical_position(tick))
    }
}

struct KeyflowSectionMetadata {
    name: String,
    start_measure: i32,
    length: i32,
}

fn parse_keyflow_section_metadata(text: &str) -> Option<KeyflowSectionMetadata> {
    let mut parts = text.splitn(4, '\t');
    if parts.next()? != "KFSECTION" {
        return None;
    }

    let name = parts.next()?.trim().to_string();
    if name.is_empty() {
        return None;
    }

    let start_measure = parts.next()?.trim().parse::<i32>().ok()?;
    let length = parts.next()?.trim().parse::<i32>().ok()?;
    (length > 0).then_some(KeyflowSectionMetadata {
        name,
        start_measure,
        length,
    })
}

/// Check if a marker text looks like a section marker.
fn is_section_marker(text: &str) -> bool {
    let text = text.trim();

    // Skip empty strings
    if text.is_empty() {
        return false;
    }

    if parse_key_signature_marker(text).is_some() {
        return false;
    }

    // Known section marker patterns
    let section_markers = [
        "SONGSTART",
        "SONGEND",
        "ENDING",
        "END",
        "HITS",
        "VS",
        "CH",
        "BR",
        "PC",
        "IN",
        "OUT",
        "Verse",
        "Chorus",
        "Bridge",
        "Intro",
        "Outro",
        "Pre-Chorus",
        "Interlude",
        "Solo",
        "SOLO",
        "Break",
        "Tag",
        "Coda",
        "Instrumental",
    ];

    for marker in section_markers {
        if text.eq_ignore_ascii_case(marker) || text.starts_with(marker) {
            return true;
        }
    }

    let text_lower = text.to_lowercase();
    // Also match patterns like "B-Section" / "A Section"
    if text_lower.ends_with("-section") || text_lower.ends_with(" section") {
        return true;
    }

    // Also match patterns like "Intro \"Groove\"" (intro with description)
    // and "Guitar Solo", "Trumpet Solo" etc.
    text_lower.starts_with("intro")
        || text_lower.starts_with("verse")
        || text_lower.starts_with("chorus")
        || text_lower.starts_with("bridge")
        || text_lower.contains("solo")
}

/// Check if a marker text looks like a chord symbol.
fn is_chord_marker(text: &str) -> bool {
    let text = text.trim();

    // Skip empty strings
    if text.is_empty() {
        return false;
    }

    // Skip section markers (these are not chords)
    if is_section_marker(text) {
        return false;
    }

    // Skip key signature markers (e.g. "#D", "bBb")
    if parse_key_signature_marker(text).is_some() {
        return false;
    }

    // Skip other known non-chord markers
    let non_chord_markers = [
        "Count-In",
        "Count In",
        "CountIn",
        "SONGSTART",
        "SONGEND",
        "Song Start",
        "Song End",
        "Triad Melody",
        "Brass Line Here",
        "Guitar Lick Here",
        "Brass",
        "Echo Walkdown",
        "Get This Lick",
    ];

    for marker in non_chord_markers {
        if text.eq_ignore_ascii_case(marker) || text.contains(marker) {
            return false;
        }
    }

    // A chord marker typically starts with a note letter (A-G)
    // optionally followed by # or b, then quality/extension
    let first_char = text.chars().next().unwrap_or(' ');
    if !matches!(first_char, 'A'..='G') {
        return false;
    }

    true
}

fn parse_key_signature_marker(text: &str) -> Option<Key> {
    let raw = text.trim();

    // Optional `KEY:` / `Key:` / `key:` prefix from DAW marker tracks. When
    // present, we trust the user's intent and accept a wide range of forms
    // (`KEY: Em`, `Key: F# minor`, etc.).
    let (had_explicit_prefix, stripped) = match raw
        .strip_prefix("KEY:")
        .or_else(|| raw.strip_prefix("Key:"))
        .or_else(|| raw.strip_prefix("key:"))
    {
        Some(rest) => (true, rest.trim()),
        None => (false, raw),
    };

    // Original keyflow-style sigs (`#G`, `bBb`). Always accepted.
    let looks_like_keyflow_sig =
        stripped.len() <= 4 && (stripped.starts_with('#') || stripped.starts_with('b'));
    if looks_like_keyflow_sig && let Ok(k) = keyflow_text::api::parse::key(stripped) {
        return Some(k);
    }

    // Mode-qualified forms with an explicit suffix word (`F# minor`,
    // `Bb major`, `Em ionian`, …). The suffix word is a strong musical
    // signal, so we accept these regardless of `KEY:` prefix.
    if let Some(normalized) = normalize_key_text_with_explicit_mode(stripped)
        && let Ok(k) = keyflow_text::api::parse::key(&normalized)
    {
        return Some(k);
    }

    // Bare forms (`C`, `Em`, `F#`, `Bb`) only when the user opted in via
    // `KEY:` prefix, OR the string is short and clearly a musical token.
    // This avoids parsing section markers like `CH 1`, `VS 2`, `INST` as
    // keys (`MusicalNote::from_string` is permissive enough to swallow
    // arbitrary text starting with a note letter).
    if had_explicit_prefix && let Ok(k) = keyflow_text::api::parse::key(stripped) {
        return Some(k);
    }

    None
}

/// Try to interpret `s` as `<root> <mode-word>` (case-insensitive). Returns the
/// keyflow-canonical form (e.g. `F#m` for `F# minor`) only when an explicit
/// mode suffix is present; bare strings without a mode word return `None`.
fn normalize_key_text_with_explicit_mode(s: &str) -> Option<String> {
    let s = s.trim();
    let lower = s.to_lowercase();
    for (suffix, is_minor) in [
        (" major", false),
        (" maj", false),
        (" ionian", false),
        (" minor", true),
        (" min", true),
        (" aeolian", true),
    ] {
        if let Some(stripped) = lower.strip_suffix(suffix) {
            let head = s[..stripped.len()].trim();
            if head.is_empty() {
                return None;
            }
            return Some(if is_minor {
                format!("{}m", head)
            } else {
                head.to_string()
            });
        }
    }
    None
}

/// Detect the most likely musical key for a MIDI file.
///
/// Resolution order:
/// 1. Any text/marker meta event whose body parses as a key (`KEY: Em`,
///    `#G`, `Bb major`, etc.) — earliest such marker wins.
/// 2. The standard MIDI key-signature meta event (already exposed via
///    `key_signature_markers_absolute()`).
/// 3. Krumhansl-Schmuckler pitch-class correlation against all 24 major /
///    minor keys.
///
/// Returns `None` only when the file has no notes and no usable markers.
///
/// Unwired: kept (with tests) as a ready utility. The import pipeline
/// currently derives key from MIDI key-signature markers rather than note
/// content; wire this in when content-based inference is wanted.
#[allow(dead_code)]
#[must_use]
pub fn detect_key(midi: &MidiFile) -> Option<Key> {
    // (1) Marker-text scan. Any marker whose text resolves through our
    // augmented `parse_key_signature_marker` wins.
    for marker in midi.markers() {
        if let Some(k) = parse_key_signature_marker(&marker.text) {
            return Some(k);
        }
    }

    // (2) MIDI key-signature meta events.
    if let Some(first) = midi.key_signature_markers_absolute().into_iter().next() {
        return Some(first.key);
    }

    // (3) Pitch-class correlation.
    detect_key_by_pitch_class(&midi.all_notes())
}

/// Krumhansl-Schmuckler pitch-class key estimation.
///
/// Builds a 12-bin pitch-class histogram weighted by note duration and
/// correlates it against the standard major / minor profiles, returning the
/// key with the highest correlation.
///
/// Unwired in the import pipeline; reached only via [`detect_key`] and the
/// parser unit tests. See `detect_key` for context.
#[allow(dead_code)]
#[must_use]
pub fn detect_key_by_pitch_class(notes: &[MidiNote]) -> Option<Key> {
    if notes.is_empty() {
        return None;
    }

    // Duration-weighted PC histogram.
    let mut hist = [0.0f64; 12];
    for n in notes {
        let pc = (n.pitch as usize) % 12;
        hist[pc] += n.duration_ticks.max(1) as f64;
    }

    // Krumhansl-Kessler key profiles (well-established perceptual weights).
    const MAJOR: [f64; 12] = [
        6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
    ];
    const MINOR: [f64; 12] = [
        6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
    ];

    let correlate = |profile: &[f64; 12], shift: usize| -> f64 {
        let mut hist_sum = 0.0;
        let mut prof_sum = 0.0;
        for i in 0..12 {
            hist_sum += hist[i];
            prof_sum += profile[i];
        }
        let hist_mean = hist_sum / 12.0;
        let prof_mean = prof_sum / 12.0;
        let mut num = 0.0;
        let mut hist_sq = 0.0;
        let mut prof_sq = 0.0;
        for i in 0..12 {
            let h = hist[(i + shift) % 12] - hist_mean;
            let p = profile[i] - prof_mean;
            num += h * p;
            hist_sq += h * h;
            prof_sq += p * p;
        }
        let denom = (hist_sq * prof_sq).sqrt();
        if denom == 0.0 { 0.0 } else { num / denom }
    };

    // Best (mode, root_pc, score) over 24 keys.
    let mut best: Option<(bool, usize, f64)> = None;
    for tonic in 0..12 {
        let major_score = correlate(&MAJOR, tonic);
        let minor_score = correlate(&MINOR, tonic);
        for &(is_major, score) in &[(true, major_score), (false, minor_score)] {
            if best.is_none_or(|(_, _, b)| score > b) {
                best = Some((is_major, tonic, score));
            }
        }
    }

    let (is_major, tonic_pc, _) = best?;
    let pc_name = pc_to_keyflow_text(tonic_pc, is_major);
    let key_text = if is_major {
        pc_name
    } else {
        format!("{}m", pc_name)
    };
    keyflow_text::api::parse::key(&key_text).ok()
}

/// Convert a pitch-class (0-11) to a keyflow note name, picking the spelling
/// (sharp vs flat) that matches the typical key-signature convention for that
/// PC and mode (e.g. PC=1 → `Db` major but `C#` minor).
#[allow(dead_code)] // helper for the unwired `detect_key_by_pitch_class`
fn pc_to_keyflow_text(pc: usize, is_major: bool) -> String {
    // Standard circle-of-fifths spellings for the 12 keys, separately for
    // major and minor, chosen to minimize accidental count in the key sig.
    const MAJOR_SPELLING: [&str; 12] = [
        "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ];
    const MINOR_SPELLING: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "Bb", "B",
    ];
    let table = if is_major {
        MAJOR_SPELLING
    } else {
        MINOR_SPELLING
    };
    table[pc % 12].to_string()
}

fn respell_chord_name_for_key(chord_name: &str, key: &Key) -> String {
    let mut chord = match keyflow_text::api::parse::chord(chord_name) {
        Ok(chord) => chord,
        Err(_) => return chord_name.to_string(),
    };

    let key_spelling = KeySpelling::major(key.root());
    chord.respell_root(&key_spelling, SpellingMode::Relaxed);
    chord.normalized
}

/// Normalize a MIDI chord name to keyflow format.
///
/// Handles common DAW chord naming conventions:
/// - `Fmaj/C` → `F/C` (strip "maj" from major triads with slash bass)
/// - `Cmaj` → `C` (strip "maj" from simple major triads)
/// - `Abaug/maj7` → `Abmaj7#5` (augmented major 7th)
/// - `Abmaj add9` → `Abadd9` (major triad + added 9, no implied 7th)
/// - `Abmaj9 add9` → `Abmaj9` (redundant add9 on maj9)
/// - `Csus4` → `Csus` (normalize sus4)
/// - Preserves valid keyflow notation unchanged
#[must_use]
pub fn normalize_chord_name(name: &str) -> String {
    let mut result = name.to_string();

    // Handle "aug/maj7" → "maj7#5" (augmented major 7th)
    if result.contains("aug/maj7") {
        result = result.replace("aug/maj7", "maj7#5");
    } else if result.contains("augmaj7") {
        result = result.replace("augmaj7", "maj7#5");
    }

    // Keep explicit maj9, but treat "maj add9" as add9 (no implied 7th).
    if result.contains("maj9 add9") {
        result = result.replace("maj9 add9", "maj9");
    }
    if result.contains("Maj9 add9") {
        result = result.replace("Maj9 add9", "Maj9");
    }
    if result.contains("maj add9") {
        result = result.replace("maj add9", " add9");
    }
    if result.contains("Maj add9") {
        result = result.replace("Maj add9", " add9");
    }
    if result.contains(" add9") {
        // keyflow-text parses add-chords as "add9" (without parentheses)
        result = result.replace(" add9", "add9");
    }

    // Handle "Xmaj/Y" → "X/Y" for major triads with slash bass
    // Only strip "maj" if it's directly before a slash and not part of maj7, maj9, etc.
    let slash_patterns = [("maj/", "/"), ("Maj/", "/")];
    for (from, to) in slash_patterns {
        if result.contains(from) {
            // Check it's not maj7, maj9, maj11, maj13
            let before_slash = result.find(from).map(|i| &result[..i]).unwrap_or("");
            let has_extension = ["7", "9", "11", "13"]
                .iter()
                .any(|ext| before_slash.ends_with(ext));

            if !has_extension {
                result = result.replace(from, to);
            }
        }
    }

    // Handle standalone "Cmaj" → "C" (but keep "Cmaj7", "Cmaj9", etc.)
    if result.ends_with("maj") && !result.contains('/') {
        let base = &result[..result.len() - 3];
        // Verify it's not already qualified (e.g., "Cmaj" not "Cmaj7maj")
        if !base.ends_with(['7', '9']) {
            result = base.to_string();
        }
    }

    // Handle "sus4" → "sus" (keyflow convention)
    if result.contains("sus4") && !result.contains("sus4/") {
        result = result.replace("sus4", "sus");
    }

    // Use keyflow-text for final canonicalization when possible.
    if let Ok(chord) = keyflow_text::api::parse::chord(&result) {
        chord.normalized
    } else {
        result
    }
}

impl MidiNote {
    /// Get the note name (e.g., "C4", "F#5").
    #[must_use]
    pub fn note_name(&self) -> String {
        let note_names = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        let octave = (self.pitch / 12) as i8 - 1;
        let note_idx = (self.pitch % 12) as usize;
        format!("{}{}", note_names[note_idx], octave)
    }

    /// Check if this is a "black key" note.
    #[must_use]
    pub fn is_accidental(&self) -> bool {
        matches!(self.pitch % 12, 1 | 3 | 6 | 8 | 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_name() {
        let note = MidiNote {
            pitch: 60,
            velocity: 100,
            start_tick: 0,
            duration_ticks: 480,
            channel: 0,
        };
        assert_eq!(note.note_name(), "C4");

        let note2 = MidiNote {
            pitch: 69,
            velocity: 100,
            start_tick: 0,
            duration_ticks: 480,
            channel: 0,
        };
        assert_eq!(note2.note_name(), "A4");
    }

    fn note(pitch: u8, ticks: u32) -> MidiNote {
        MidiNote {
            pitch,
            velocity: 100,
            start_tick: 0,
            duration_ticks: ticks,
            channel: 0,
        }
    }

    #[test]
    fn detect_key_by_pitch_class_c_major() {
        // C major scale, two octaves, weighted toward tonic / dominant.
        let mut notes = Vec::new();
        let scale_pcs = [0, 2, 4, 5, 7, 9, 11];
        for &pc in &scale_pcs {
            notes.push(note(60 + pc, 480)); // octave 4
            notes.push(note(72 + pc, 480)); // octave 5
        }
        // Add weight to tonic (C) and dominant (G).
        for _ in 0..6 {
            notes.push(note(60, 960));
            notes.push(note(67, 960));
        }
        let key = detect_key_by_pitch_class(&notes).expect("detected key");
        assert!(
            format!("{}", key).starts_with('C'),
            "expected C-rooted key, got {}",
            key
        );
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn detect_key_by_pitch_class_a_minor_prefers_minor() {
        // Notes that fit A natural minor; should outscore C major
        // because the histogram is weighted to A and E.
        let mut notes = Vec::new();
        let scale_pcs = [9, 11, 0, 2, 4, 5, 7]; // A B C D E F G
        for &pc in &scale_pcs {
            notes.push(note(57 + (pc + 3) % 12, 480));
        }
        for _ in 0..8 {
            notes.push(note(57, 1200)); // A
            notes.push(note(64, 800)); // E
        }
        let key = detect_key_by_pitch_class(&notes).expect("detected key");
        // Accept either A natural or A harmonic spelling; just verify root.
        assert!(
            format!("{}", key).starts_with('A'),
            "expected A-rooted key, got {}",
            key
        );
    }

    #[test]
    fn parse_key_signature_marker_handles_extended_forms() {
        assert!(parse_key_signature_marker("KEY: Em").is_some());
        assert!(parse_key_signature_marker("Bb major").is_some());
        assert!(parse_key_signature_marker("F# minor").is_some());
        assert!(parse_key_signature_marker("#G").is_some());
    }

    #[test]
    fn test_is_accidental() {
        let c = MidiNote {
            pitch: 60,
            velocity: 100,
            start_tick: 0,
            duration_ticks: 480,
            channel: 0,
        };
        let csharp = MidiNote {
            pitch: 61,
            velocity: 100,
            start_tick: 0,
            duration_ticks: 480,
            channel: 0,
        };

        assert!(!c.is_accidental());
        assert!(csharp.is_accidental());
    }

    #[test]
    fn test_tempo_bpm() {
        let tempo = TempoEvent {
            tick: 0,
            microseconds_per_quarter: 500_000, // 120 BPM
        };
        assert!((tempo.bpm() - 120.0).abs() < 0.001);

        let tempo2 = TempoEvent {
            tick: 0,
            microseconds_per_quarter: 600_000, // 100 BPM
        };
        assert!((tempo2.bpm() - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_thriller_midi() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        // Basic structure checks
        println!("PPQ: {}", midi.ppq());
        println!("Initial tempo: {:.1} BPM", midi.initial_tempo());
        println!("Initial time sig: {:?}", midi.initial_time_signature());
        println!(
            "Duration: {} ticks ({:.2}s)",
            midi.duration_ticks(),
            midi.tick_to_seconds(midi.duration_ticks())
        );

        // Track info
        println!("\nTracks ({} with notes):", midi.tracks().len());
        for track in midi.tracks() {
            println!(
                "  Track {}: {:?} ({} notes, channel {:?})",
                track.index,
                track.name,
                track.notes.len(),
                track.channel
            );

            // Print first few notes
            for (i, note) in track.notes.iter().take(5).enumerate() {
                println!(
                    "    Note {}: {} at tick {} for {} ticks",
                    i,
                    note.note_name(),
                    note.start_tick,
                    note.duration_ticks
                );
            }
            if track.notes.len() > 5 {
                println!("    ... and {} more notes", track.notes.len() - 5);
            }
        }

        // Markers
        println!("\nMarkers ({}):", midi.markers().len());
        for marker in midi.markers().iter().take(20) {
            println!(
                "  Tick {}: {:?} - {:?}",
                marker.tick, marker.marker_type, marker.text
            );
        }

        // Tempo changes (first 10)
        println!("\nTempo changes (first 10 of {}):", midi.tempo_map().len());
        for tempo in midi.tempo_map().iter().take(10) {
            println!("  Tick {}: {:.1} BPM", tempo.tick, tempo.bpm());
        }

        // Extract durations for quantization test
        if let Some(track) = midi.tracks().first() {
            let (durations, positions) = midi.extract_durations(track.index);
            println!("\nFirst track durations (first 10):");
            for (i, (dur, pos)) in durations.iter().zip(positions.iter()).take(10).enumerate() {
                println!("  Note {}: start={}, duration={} ticks", i, pos, dur);
            }
        }

        // Assertions
        assert_eq!(midi.ppq(), 960, "Expected REAPER's 960 PPQ");
        assert!(!midi.tracks().is_empty(), "Should have at least one track");
    }

    #[test]
    fn test_chord_marker_positions() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        println!("=== Thriller MIDI Chord Position Analysis ===\n");
        println!("PPQ: {}", midi.ppq());
        println!("Time Signature: {:?}", midi.initial_time_signature());
        println!("SONGSTART tick: {}", midi.songstart_tick());
        println!("Ticks per measure (4/4 at 960 PPQ): {}\n", 960 * 4);

        let chord_markers = midi.chord_markers();
        println!("Found {} chord markers:\n", chord_markers.len());

        // Group by measure for easier analysis
        let mut current_measure = -999;
        for marker in &chord_markers {
            if marker.position.measure != current_measure {
                current_measure = marker.position.measure;
                println!("\n--- Measure {} ---", marker.position.measure + 1);
            }

            // Calculate beat position in human terms
            let beat_display = marker.position.beat + 1;
            let subdivision_fraction = if marker.position.subdivision > 0 {
                let frac = marker.position.subdivision as f64 / 960.0;
                format!(" + {:.3} beats", frac)
            } else {
                String::new()
            };

            println!(
                "  {:12} @ M{}.B{}{} (tick {})",
                marker.chord_name,
                marker.position.measure + 1,
                beat_display,
                subdivision_fraction,
                marker.tick
            );
        }

        // Detailed analysis of first few chords
        println!("\n\n=== Detailed First 10 Chords ===\n");
        for (i, marker) in chord_markers.iter().take(10).enumerate() {
            let relative_tick = marker.tick.saturating_sub(midi.songstart_tick());
            let ticks_per_beat = 960;
            let ticks_per_measure = 3840;

            let expected_measure = relative_tick / ticks_per_measure;
            let remaining = relative_tick % ticks_per_measure;
            let expected_beat = remaining / ticks_per_beat;
            let expected_subdiv = remaining % ticks_per_beat;

            println!(
                "Chord {}: {} @ tick {} (relative {})",
                i + 1,
                marker.chord_name,
                marker.tick,
                relative_tick
            );
            println!(
                "  Calculated: M{}.B{}.S{}",
                expected_measure + 1,
                expected_beat + 1,
                expected_subdiv
            );
            println!(
                "  From method: M{}.B{}.S{}",
                marker.position.measure + 1,
                marker.position.beat + 1,
                marker.position.subdivision
            );
            println!();
        }
    }

    #[test]
    fn test_tick_to_position_basic() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        let songstart = midi.songstart_tick();
        let ppq = midi.ppq();
        let ticks_per_measure = ppq * 4; // 4/4 time

        println!("SONGSTART: tick {}", songstart);
        println!("PPQ: {}", ppq);
        println!("Ticks per measure: {}", ticks_per_measure);

        // Test: exactly at SONGSTART should be measure 1, beat 0
        let pos = midi.tick_to_musical_position(songstart);
        println!("\nAt SONGSTART (tick {}): {:?}", songstart, pos);
        assert_eq!(pos.measure, 1, "SONGSTART should be measure 1");
        assert_eq!(pos.beat, 0, "SONGSTART should be beat 0");
        assert_eq!(pos.subdivision, 0, "SONGSTART should have no subdivision");

        // Test: one measure after SONGSTART
        let one_measure = songstart + ticks_per_measure;
        let pos = midi.tick_to_musical_position(one_measure);
        println!(
            "One measure after SONGSTART (tick {}): {:?}",
            one_measure, pos
        );
        assert_eq!(pos.measure, 2, "Should be measure 2");
        assert_eq!(pos.beat, 0, "Should be beat 0");

        // Test: beat 2 of measure 2 (1.5 measures after start)
        let beat2_m1 = songstart + ticks_per_measure + ppq;
        let pos = midi.tick_to_musical_position(beat2_m1);
        println!("Beat 2 of measure 2 (tick {}): {:?}", beat2_m1, pos);
        assert_eq!(pos.measure, 2);
        assert_eq!(pos.beat, 1); // 0-indexed, so beat 1 = 2nd beat

        // Test: First chord (Ab9) at tick 11840
        let first_chord_tick = 11840;
        let pos = midi.tick_to_musical_position(first_chord_tick);
        let relative = first_chord_tick - songstart;
        println!(
            "\nFirst chord Ab9 at tick {} (relative {}): {:?}",
            first_chord_tick, relative, pos
        );
        // relative = 320, which is 320/960 = 1/3 beat = triplet offset
        // So it should be measure 1, beat 0, subdivision 320
        assert_eq!(pos.measure, 1);
        assert_eq!(pos.beat, 0);
        assert_eq!(pos.subdivision, 320, "Ab9 has 320 tick offset (triplet)");
    }

    #[test]
    fn test_section_markers() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        let sections = midi.section_markers();

        println!("\n=== Section Markers ===\n");
        for section in &sections {
            println!(
                "{:20} @ M{}.B{} (tick {:6}) - {:?} #{}",
                section.name,
                section.position.measure + 1,
                section.position.beat + 1,
                section.tick,
                section.section_type,
                section.number.map_or("-".to_string(), |n| n.to_string())
            );
        }

        // Verify key section positions
        let count_in = sections
            .iter()
            .find(|s| s.section_type == SectionType::CountIn);
        assert!(
            count_in.is_none(),
            "Count-In should remain a cue marker, not a section marker"
        );

        let songstart = sections
            .iter()
            .find(|s| s.section_type == SectionType::SongStart);
        assert!(songstart.is_some(), "Should have SONGSTART marker");
        let songstart = songstart.unwrap();
        assert_eq!(
            songstart.position.measure, 1,
            "SONGSTART should be measure 1"
        );

        // Check Verse 1 position
        let vs1 = sections
            .iter()
            .find(|s| s.section_type == SectionType::Verse && s.number == Some(1));
        assert!(vs1.is_some(), "Should have VS 1 marker");
        let vs1 = vs1.unwrap();
        println!(
            "VS 1 at measure {} (tick {})",
            vs1.position.measure + 1,
            vs1.tick
        );

        // Check Chorus 1 position
        let ch1 = sections
            .iter()
            .find(|s| s.section_type == SectionType::Chorus && s.number == Some(1));
        assert!(ch1.is_some(), "Should have CH 1 marker");
        let ch1 = ch1.unwrap();
        println!(
            "CH 1 at measure {} (tick {})",
            ch1.position.measure + 1,
            ch1.tick
        );
    }

    #[test]
    fn test_ending_is_section_marker_not_chord() {
        assert!(is_section_marker("ENDING"));
        assert!(!is_chord_marker("ENDING"));

        let (section_type, number) = SectionType::from_marker_text("ENDING");
        assert_eq!(section_type, SectionType::Outro);
        assert_eq!(number, None);
    }

    #[test]
    fn test_count_and_song_cue_markers_are_not_chords() {
        for marker in [
            "Count-In",
            "COUNT-IN",
            "Count In",
            "CountIn",
            "SONGSTART",
            "SONGEND",
            "Song Start",
            "Song End",
        ] {
            assert!(
                !is_chord_marker(marker),
                "{marker} should not be classified as a chord marker"
            );
        }
    }

    #[test]
    fn test_hash_key_markers_are_not_chords_or_sections() {
        assert!(parse_key_signature_marker("#D").is_some());
        assert!(parse_key_signature_marker("#C").is_some());
        assert!(!is_chord_marker("#D"));
        assert!(!is_section_marker("#D"));
    }

    #[test]
    #[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
    fn test_key_signature_markers_and_key_based_respelling() {
        let ppq = 960;
        let ticks_per_measure = ppq * 4;

        let m64 = (64 - 1) * ticks_per_measure;
        let m80 = (80 - 1) * ticks_per_measure;

        let markers = vec![
            MarkerEvent {
                tick: 0,
                text: "#D".to_string(),
                marker_type: MarkerType::Marker,
            },
            MarkerEvent {
                tick: 120,
                text: "Dbm7".to_string(),
                marker_type: MarkerType::Marker,
            },
            MarkerEvent {
                tick: m64,
                text: "#C".to_string(),
                marker_type: MarkerType::Marker,
            },
            MarkerEvent {
                tick: m64 + 120,
                text: "Dbm7".to_string(),
                marker_type: MarkerType::Marker,
            },
            MarkerEvent {
                tick: m80,
                text: "#D".to_string(),
                marker_type: MarkerType::Marker,
            },
            MarkerEvent {
                tick: m80 + 120,
                text: "Gbmaj".to_string(),
                marker_type: MarkerType::Marker,
            },
        ];

        let midi = MidiFile::from_parts(ppq, vec![], vec![], vec![], markers, vec![], None);

        let keys = midi.key_signature_markers_absolute();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0].position.measure, 1);
        assert_eq!(keys[1].position.measure, 64);
        assert_eq!(keys[2].position.measure, 80);
        assert_eq!(keys[0].key.name(), "D");
        assert_eq!(keys[1].key.name(), "C");
        assert_eq!(keys[2].key.name(), "D");

        let chords = midi.chord_markers_absolute();
        assert_eq!(chords.len(), 3);
        // In key D, Db should be respelled to C#.
        assert_eq!(chords[0].chord_name, "C#m7");
        // In key C, keep flat spelling accepted from marker.
        assert_eq!(chords[1].chord_name, "Dbm7");
        // Back in key D, Gb should be respelled to F#.
        assert_eq!(chords[2].chord_name, "F#");
    }

    #[test]
    #[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
    fn test_count_in_measures() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        let ppq = midi.ppq();
        let ticks_per_measure = ppq * 4; // 4/4 time

        let count_in_tick = midi.find_marker("Count-In").map(|m| m.tick).unwrap_or(0);
        let songstart_tick = midi.songstart_tick();

        println!("Count-In tick: {}", count_in_tick);
        println!("SONGSTART tick: {}", songstart_tick);
        println!("Ticks per measure: {}", ticks_per_measure);

        // Calculate count-in measures
        let count_in_ticks = songstart_tick - count_in_tick;
        let count_in_measures = count_in_ticks / ticks_per_measure;

        println!(
            "Count-in duration: {} ticks = {} measures",
            count_in_ticks, count_in_measures
        );

        // In this file:
        // Count-In at tick 3840
        // SONGSTART at tick 11520
        // Difference = 7680 ticks = 2 measures
        assert_eq!(count_in_measures, 2, "Should have 2 count-in measures");

        // The count-in position should be -2 measures
        let count_in_pos = midi.tick_to_musical_position(count_in_tick);
        println!("Count-In musical position: {:?}", count_in_pos);
        assert_eq!(count_in_pos.measure, -2, "Count-In should be at measure -2");
    }

    #[test]
    #[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
    fn test_absolute_section_positions() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        let sections = midi.section_markers_absolute();

        println!("\n=== Section Markers (Absolute MIDI Measures) ===\n");
        for section in &sections {
            println!(
                "{:24} @ M{:3} (tick {:6}) - {:?}",
                section.name, section.position.measure, section.tick, section.section_type,
            );
        }

        // Expected structure based on user specification:
        // Count at measure 2 (2 measures)
        // Hits at measure 4 (2 measures)
        // Intro "Groove" at measure 6 (4 measures)
        // VS 1 at measure 10 (16 measures)
        // CH 1 at measure 26 (8 measures)
        // etc.

        // Verify key positions
        let count_in = sections
            .iter()
            .find(|s| s.section_type == SectionType::CountIn);
        assert!(count_in.is_some(), "Should have Count-In");
        let count_in = count_in.unwrap();
        assert_eq!(
            count_in.position.measure, 2,
            "Count-In should start at measure 2"
        );

        let hits = sections
            .iter()
            .find(|s| s.section_type == SectionType::Hits);
        assert!(hits.is_some(), "Should have HITS");
        let hits = hits.unwrap();
        assert_eq!(hits.position.measure, 4, "HITS should start at measure 4");

        let intro = sections
            .iter()
            .find(|s| s.section_type == SectionType::Intro);
        assert!(intro.is_some(), "Should have Intro");
        let intro = intro.unwrap();
        assert_eq!(intro.position.measure, 6, "Intro should start at measure 6");

        let vs1 = sections
            .iter()
            .find(|s| s.section_type == SectionType::Verse && s.number == Some(1));
        assert!(vs1.is_some(), "Should have VS 1");
        let vs1 = vs1.unwrap();
        assert_eq!(vs1.position.measure, 10, "VS 1 should start at measure 10");

        let ch1 = sections
            .iter()
            .find(|s| s.section_type == SectionType::Chorus && s.number == Some(1));
        assert!(ch1.is_some(), "Should have CH 1");
        let ch1 = ch1.unwrap();
        assert_eq!(ch1.position.measure, 26, "CH 1 should start at measure 26");

        // Print measure counts between sections
        println!("\n=== Section Lengths ===\n");
        for i in 0..sections.len().saturating_sub(1) {
            let current = &sections[i];
            let next = &sections[i + 1];
            let length = next.position.measure - current.position.measure;
            println!(
                "{:24} -> {:24} = {} measures",
                current.name, next.name, length
            );
        }
    }

    #[test]
    fn test_chord_positions_absolute() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        let chords = midi.chord_markers_absolute();

        println!("\n=== First 30 Chord Markers (Absolute MIDI Measures) ===\n");
        for (i, chord) in chords.iter().take(30).enumerate() {
            let beat_frac = if chord.position.subdivision > 0 {
                format!(" +{:.2}", chord.position.subdivision as f64 / 960.0)
            } else {
                String::new()
            };

            println!(
                "{:3}. {:12} @ M{:3}.B{}{}",
                i + 1,
                chord.chord_name,
                chord.position.measure,
                chord.position.beat + 1,
                beat_frac,
            );
        }

        // Verify first chord position
        // Ab9 should be at measure 4 beat 1 + triplet (HITS starts at measure 4)
        let first = &chords[0];
        assert_eq!(first.chord_name, "Ab9");
        assert_eq!(
            first.position.measure, 4,
            "First chord should be in measure 4 (HITS section)"
        );
        assert_eq!(
            first.position.subdivision, 320,
            "First chord should have 320 tick subdivision (triplet)"
        );
    }

    #[test]
    fn test_push_pull_detection() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");
        let ppq = midi.ppq();

        let chords = midi.chord_markers_absolute();

        println!("\n=== Push/Pull Detection Test ===\n");
        println!("PPQ: {}", ppq);
        println!("Half beat: {} ticks", ppq / 2);
        println!();

        // Test first few chords
        for (i, chord) in chords.iter().take(10).enumerate() {
            let push_pull = chord.detect_push_pull(ppq);
            let keyflow = chord.to_keyflow_notation(ppq);

            println!(
                "{:2}. {:12} @ M{}.B{}.S{:3} -> {:?} -> \"{}\"",
                i + 1,
                chord.chord_name,
                chord.position.measure,
                chord.position.beat + 1,
                chord.position.subdivision,
                push_pull,
                keyflow,
            );
        }

        // Verify specific chords based on user's description:

        // 1. Ab9 - first chord, should be a PULL by triplet eighth (320 ticks after beat)
        let ab9 = &chords[0];
        assert_eq!(ab9.chord_name, "Ab9");
        assert_eq!(ab9.position.subdivision, 320);
        let ab9_pp = ab9.detect_push_pull(ppq);
        assert_eq!(
            ab9_pp,
            PushPull::Pull(PushPullAmount::TripletEighth),
            "Ab9 should be pulled by triplet eighth"
        );
        assert_eq!(
            ab9.to_keyflow_notation(ppq),
            "Ab9t'",
            "Ab9 keyflow notation should be Ab9t' (pulled)"
        );

        // 2. F9 - second chord, should be a PUSH by triplet eighth (640 ticks = 320 before next beat)
        let f9 = &chords[1];
        assert_eq!(f9.chord_name, "F9");
        assert_eq!(f9.position.subdivision, 640);
        let f9_pp = f9.detect_push_pull(ppq);
        assert_eq!(
            f9_pp,
            PushPull::Push(PushPullAmount::TripletEighth),
            "F9 should be pushed by triplet eighth"
        );
        assert_eq!(
            f9.to_keyflow_notation(ppq),
            "'tF9",
            "F9 keyflow notation should be 'tF9 (pushed)"
        );
    }

    #[test]
    #[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
    fn test_keyflow_notation_generation() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");
        let ppq = midi.ppq();

        let chords = midi.chord_markers_absolute();

        println!("\n=== Keyflow Notation Generation ===\n");

        // Generate keyflow text for first 20 chords
        let keyflow_chords: Vec<String> = chords
            .iter()
            .take(20)
            .map(|c| c.to_keyflow_notation(ppq))
            .collect();

        for (i, kf) in keyflow_chords.iter().enumerate() {
            println!("{:2}. {}", i + 1, kf);
        }

        // Expected patterns (based on actual MIDI file content):
        // 1. Ab9t' (pull) - HITS section
        // 2. 'tF9 (push) - HITS section
        // 3. Cm (on beat) - Intro section starts
        // 4. Fmaj/C (on beat) - Verse chord

        assert_eq!(keyflow_chords[0], "Ab9t'", "First chord: Ab9 pulled");
        assert_eq!(keyflow_chords[1], "'tF9", "Second chord: F9 pushed");
        // The actual MIDI has Cm as third chord (Intro start), not pushed Fmaj/C
        assert_eq!(keyflow_chords[2], "Cm", "Third chord: Cm on beat (Intro)");
        assert_eq!(keyflow_chords[3], "Fmaj/C", "Fourth chord: Fmaj/C on beat");
    }

    #[test]
    #[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
    fn test_verse_chord_structure() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");
        let ppq = midi.ppq();

        let sections = midi.section_markers_absolute();
        let chords = midi.chord_markers_absolute();

        // Find VS 1 section
        let vs1 = sections
            .iter()
            .find(|s| s.section_type == SectionType::Verse && s.number == Some(1))
            .expect("Should have VS 1");

        println!("\n=== Verse 1 Chord Structure ===\n");
        println!("VS 1 starts at measure {}", vs1.position.measure);

        // Get chords in/around verse 1
        // VS 1 is at measure 10, so look for chords from measure 9 (anticipation) to measure 26 (CH 1)
        let verse_chords: Vec<_> = chords
            .iter()
            .filter(|c| c.position.measure >= 9 && c.position.measure < 26)
            .collect();

        println!("\nChords in Verse 1 region:");
        for (i, chord) in verse_chords.iter().enumerate() {
            let keyflow = chord.to_keyflow_notation(ppq);
            let logical_m = chord.logical_measure(ppq, 4) + 1; // 1-indexed

            println!(
                "{:2}. {:12} @ M{:2}.B{} ({:3} sub) -> \"{}\" (logical M{})",
                i + 1,
                chord.chord_name,
                chord.position.measure,
                chord.position.beat + 1,
                chord.position.subdivision,
                keyflow,
                logical_m,
            );
        }

        // The first chord in the verse region is Fmaj/C.
        // Note: The actual MIDI file has this chord on the beat, not pushed
        // (different from the expected output in the plan).
        let first_verse_chord = verse_chords.first().expect("Should have chords");
        assert_eq!(first_verse_chord.chord_name, "Fmaj/C");
        // Verify it's on beat (the actual MIDI file content)
        let first_pp = first_verse_chord.detect_push_pull(ppq);
        assert_eq!(
            first_pp,
            PushPull::OnBeat,
            "First verse chord is on beat in this MIDI file"
        );

        // The second chord should be Cm on the downbeat (measure 12)
        let second_chord = verse_chords.get(1).expect("Should have second chord");
        assert_eq!(second_chord.chord_name, "Cm");
        let second_pp = second_chord.detect_push_pull(ppq);
        assert_eq!(
            second_pp,
            PushPull::OnBeat,
            "Second verse chord (Cm) should be on beat"
        );
    }

    #[test]
    fn test_measure_keyflow_export() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");
        let ppq = midi.ppq();

        let chords = midi.chord_markers_absolute();

        println!("\n=== Keyflow Measure Export ===\n");

        // Group chords by logical measure
        let mut measures: std::collections::BTreeMap<i32, Vec<&ChordMarker>> =
            std::collections::BTreeMap::new();

        for chord in &chords {
            let logical_m = chord.logical_measure(ppq, 4);
            measures.entry(logical_m).or_default().push(chord);
        }

        // Print first 15 measures with chords
        // Note: absolute measures are already 1-indexed from tick_to_absolute_measure
        for (count, (measure, measure_chords)) in measures.iter().enumerate() {
            if count >= 15 {
                break;
            }

            let keyflow_line: Vec<String> = measure_chords
                .iter()
                .map(|c| c.to_keyflow_notation(ppq))
                .collect();

            println!("M{:3}: | {} |", measure, keyflow_line.join(" "));
        }

        // Verify measure 4 (HITS) has Ab9t' - absolute measures are 1-indexed
        let m4_chords = measures.get(&4).expect("Should have measure 4 (HITS)");
        let m4_keyflow: Vec<String> = m4_chords
            .iter()
            .map(|c| c.to_keyflow_notation(ppq))
            .collect();
        assert!(
            m4_keyflow.contains(&"Ab9t'".to_string()),
            "Measure 4 should contain Ab9t'"
        );

        // F9 is pushed so its logical measure is 5 (beat 2 of measure 4 pushes to beat 3)
        let m5_chords = measures.get(&5); // measure 5 (1-indexed)
        if let Some(m5) = m5_chords {
            let m5_keyflow: Vec<String> = m5.iter().map(|c| c.to_keyflow_notation(ppq)).collect();
            println!("\nMeasure 5 (logical): {:?}", m5_keyflow);
        }
    }

    /// Test the chorus chord structure with durations.
    ///
    /// Key: Eb major
    ///
    /// CH 1 starts at measure 26:
    /// - Cm/Eb (quarter note, on beat 1)
    /// - 'tEbmaj (pushed triplet eighth, 3 beats long)
    #[test]
    #[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
    fn test_chorus_chord_structure() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");
        let ppq = midi.ppq();

        // Get section markers to find chorus
        let sections = midi.section_markers_absolute();
        let ch1 = sections
            .iter()
            .find(|s| s.section_type == SectionType::Chorus && s.number == Some(1))
            .expect("Should have CH 1 section");

        println!("\n=== Chorus Chord Structure (Key: Eb) ===\n");
        println!(
            "CH 1 starts at measure {} (tick {})",
            ch1.position.measure, ch1.tick
        );

        // Get chord markers (using absolute for correct measure numbers)
        let chords = midi.chord_markers_absolute();

        // Filter chords in measures 26-33 (chorus is 8 measures based on structure)
        let chorus_chords: Vec<_> = chords
            .iter()
            .filter(|c| {
                let abs_measure = c.position.measure;
                (26..34).contains(&abs_measure)
            })
            .collect();

        println!("\nChorus chords ({} total):", chorus_chords.len());

        // Print each chord with duration to next
        for (i, chord) in chorus_chords.iter().enumerate() {
            let next = chorus_chords.get(i + 1).copied();
            let duration = chord.duration_to_next(next);
            let duration_str = duration
                .map(|d| ChordMarker::format_duration(d, ppq))
                .unwrap_or_else(|| "end".to_string());

            let keyflow = chord.to_keyflow_notation(ppq);

            println!(
                "  M{}.B{}.S{:3}: {} ({}) - tick {}",
                chord.position.measure,
                chord.position.beat + 1, // 1-indexed for display
                chord.position.subdivision,
                keyflow,
                duration_str,
                chord.tick
            );
        }

        // Verify first chorus chord structure:
        // 1. First chord is Cm/Eb on beat 1
        let first = chorus_chords
            .first()
            .expect("Should have first chorus chord");
        assert_eq!(
            first.chord_name, "Cm/Eb",
            "First chorus chord should be Cm/Eb"
        );
        assert_eq!(
            first.position.beat, 0,
            "First chorus chord should be on beat 1 (0-indexed)"
        );
        assert_eq!(
            first.position.subdivision, 0,
            "First chorus chord should be on the beat"
        );

        // 2. Second chord is Ebmaj, pushed by triplet eighth
        let second = chorus_chords
            .get(1)
            .expect("Should have second chorus chord");
        assert_eq!(
            second.chord_name, "Ebmaj",
            "Second chorus chord should be Ebmaj"
        );
        let second_pp = second.detect_push_pull(ppq);
        assert!(
            matches!(second_pp, PushPull::Push(PushPullAmount::TripletEighth)),
            "Second chorus chord should be pushed by triplet eighth, got: {:?}",
            second_pp
        );

        // 3. Verify durations
        let first_duration = first.duration_to_next(Some(second));
        println!("\nDuration analysis:");
        println!(
            "  Cm/Eb duration: {} ticks = {} beats",
            first_duration.unwrap_or(0),
            ChordMarker::duration_in_beats(first_duration.unwrap_or(0), ppq)
        );

        // The Cm/Eb should be roughly a quarter note (on beat push means it lasts until 0.667 of beat)
        // Actually: Cm/Eb is at beat 1.0, Ebmaj is at beat 1.667
        // So duration = 0.667 beats = 640 ticks
        if let Some(d) = first_duration {
            let beats = ChordMarker::duration_in_beats(d, ppq);
            println!("  Cm/Eb lasts {:.2} beats ({} ticks)", beats, d);
            // Should be about 2/3 beat (pushed by triplet)
            assert!(
                beats > 0.5 && beats < 1.0,
                "Cm/Eb should be about 2/3 beat, got {:.2}",
                beats
            );
        }

        // Calculate Ebmaj duration to next chord
        if let Some(third) = chorus_chords.get(2) {
            let eb_duration = second.duration_to_next(Some(third));
            if let Some(d) = eb_duration {
                let beats = ChordMarker::duration_in_beats(d, ppq);
                println!("  Ebmaj lasts {:.2} beats ({} ticks)", beats, d);
                // Should be about 3 beats (dotted half) based on user description
            }
        }

        println!("\n[PASS] Chorus chord structure validated");
    }
}

/// Tests comparing MIDI markers with keyflow chord parsing.
#[cfg(test)]
mod keyflow_comparison_tests {
    use super::*;

    /// Compare MIDI chord names with keyflow's normalized chord parsing.
    ///
    /// This test identifies discrepancies between:
    /// 1. Chord names as stored in MIDI markers
    /// 2. How keyflow normalizes those chord names
    #[test]
    fn test_midi_vs_keyflow_chord_naming() {
        use keyflow_proto::{Chord, Lexer};

        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");

        let chords = midi.chord_markers();

        println!("\n=== MIDI vs Keyflow Chord Naming Comparison ===\n");

        let mut discrepancies: Vec<(String, String, String)> = Vec::new();
        let mut parse_failures: Vec<String> = Vec::new();

        for chord in &chords {
            let midi_name = &chord.chord_name;

            // Try to parse through keyflow
            let mut lexer = Lexer::new(midi_name.clone());
            let tokens = lexer.tokenize();

            match Chord::parse(&tokens) {
                Ok(parsed) => {
                    let normalized = &parsed.normalized;

                    // Check if normalized is different from original
                    if normalized != midi_name {
                        discrepancies.push((
                            midi_name.clone(),
                            normalized.clone(),
                            format!(
                                "M{}.B{}",
                                chord.position.measure + 1,
                                chord.position.beat + 1
                            ),
                        ));
                    }
                }
                Err(e) => {
                    parse_failures.push(format!("{} ({})", midi_name, e));
                }
            }
        }

        // Report parse failures
        if !parse_failures.is_empty() {
            println!("=== Parse Failures ({}) ===", parse_failures.len());
            for failure in &parse_failures {
                println!("  [FAIL] {}", failure);
            }
            println!();
        }

        // Report discrepancies
        if !discrepancies.is_empty() {
            println!("=== Naming Discrepancies ({}) ===", discrepancies.len());
            for (midi, keyflow, pos) in &discrepancies {
                println!("  @ {}: MIDI '{}' -> Keyflow '{}'", pos, midi, keyflow);
            }
            println!();
        }

        // Collect unique chord names for analysis
        let unique_chords: std::collections::HashSet<_> =
            chords.iter().map(|c| c.chord_name.clone()).collect();

        println!("=== Unique Chord Names ({}) ===", unique_chords.len());
        let mut sorted: Vec<_> = unique_chords.iter().collect();
        sorted.sort();
        for name in sorted {
            println!("  {}", name);
        }

        println!("\n=== Summary ===");
        println!("Total chords: {}", chords.len());
        println!("Unique chords: {}", unique_chords.len());
        println!("Parse failures: {}", parse_failures.len());
        println!("Naming discrepancies: {}", discrepancies.len());

        // Assert no parse failures for properly formed chord names
        assert!(
            parse_failures.is_empty(),
            "Found {} chord names that failed to parse: {:?}",
            parse_failures.len(),
            parse_failures
        );
    }

    /// Test chord position matching between MIDI and expected chart positions.
    ///
    /// This validates that chords are placed in the correct measures/beats.
    #[test]
    fn test_chord_position_accuracy() {
        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");
        let ppq = midi.ppq();

        let chords = midi.chord_markers_absolute();

        println!("\n=== Chord Position Accuracy Test ===\n");
        println!("PPQ: {}", ppq);
        println!("Testing first 30 chords for position consistency\n");

        let mut position_issues: Vec<String> = Vec::new();

        for (i, chord) in chords.iter().take(30).enumerate() {
            // Verify tick-to-position conversion is reversible
            let ticks_per_beat = ppq;
            let ticks_per_measure = ticks_per_beat * 4; // 4/4 time

            // Calculate expected tick from position
            let expected_tick = (chord.position.measure as u32 - 1) * ticks_per_measure
                + chord.position.beat as u32 * ticks_per_beat
                + chord.position.subdivision as u32;

            let actual_tick = chord.tick;
            let diff = (actual_tick as i64 - expected_tick as i64).abs();

            // Allow small tolerance for rounding
            if diff > 10 {
                position_issues.push(format!(
                    "#{}: {} @ M{}.B{}.S{} - expected tick {}, got {} (diff: {})",
                    i + 1,
                    chord.chord_name,
                    chord.position.measure,
                    chord.position.beat + 1,
                    chord.position.subdivision,
                    expected_tick,
                    actual_tick,
                    diff
                ));
            }

            // Print chord info
            let keyflow_notation = chord.to_keyflow_notation(ppq);
            println!(
                "#{:2}: {:12} @ M{:2}.B{}.S{:3} (tick {:6}) -> {}",
                i + 1,
                chord.chord_name,
                chord.position.measure,
                chord.position.beat + 1,
                chord.position.subdivision,
                chord.tick,
                keyflow_notation
            );
        }

        if !position_issues.is_empty() {
            println!("\n=== Position Issues ===");
            for issue in &position_issues {
                println!("  {}", issue);
            }
        }

        println!("\n=== Results ===");
        println!("Position issues: {}", position_issues.len());

        assert!(
            position_issues.len() < 5,
            "Too many position issues: {}",
            position_issues.len()
        );
    }

    /// Compare chord detection from MIDI notes vs chord markers.
    ///
    /// This is the key comparison:
    /// - Detected chords: what we detect by analyzing MIDI note events
    /// - Marker chords: the "ground truth" chord labels in the MIDI file
    #[test]
    fn test_detected_vs_marker_chords() {
        use keyflow_proto::chord::{MidiNote as KeyflowMidiNote, detect_chords_from_midi_notes};
        use keyflow_proto::key::{KeySpelling, SpellingMode};
        use keyflow_proto::primitives::MusicalNote;

        let bytes = include_bytes!("../../../keyflow/tests/fixtures/thriller_dirty_loops.mid");
        let midi = MidiFile::parse(bytes).expect("Failed to parse MIDI file");
        let ppq = midi.ppq();

        println!("\n=== Detected vs Marker Chords Comparison ===\n");
        println!("PPQ: {}", ppq);

        // The song is in Eb major / C minor - use Eb major key spelling for flats
        let eb = MusicalNote::from_string("Eb").unwrap();
        let key_spelling = KeySpelling::major(&eb);
        println!("Using key: Eb major (for flat enharmonic spelling)\n");

        // Get all notes from all tracks
        let all_notes = midi.all_notes();
        println!("Total notes: {}", all_notes.len());

        // Convert to keyflow MidiNote format
        let keyflow_notes: Vec<KeyflowMidiNote> = all_notes
            .iter()
            .map(|n| {
                KeyflowMidiNote::new(
                    n.pitch,
                    n.start_tick as i64,
                    (n.start_tick + n.duration_ticks) as i64,
                    n.channel,
                    n.velocity,
                )
            })
            .collect();

        println!("Converted {} notes to keyflow format", keyflow_notes.len());

        // Detect chords from notes
        // Use min_chord_duration of ~1/8 beat (120 ticks at 960 PPQ)
        let min_duration = (ppq / 8) as i64;
        let mut detected = detect_chords_from_midi_notes(&keyflow_notes, min_duration);

        // Respell all detected chords using the key context
        for chord_event in &mut detected {
            chord_event
                .chord
                .respell_root(&key_spelling, SpellingMode::Relaxed);
        }

        println!("Detected {} chord events from notes\n", detected.len());

        // Get chord markers (ground truth)
        let markers = midi.chord_markers_absolute();
        println!("Found {} chord markers (ground truth)\n", markers.len());

        // Compare first 30 detected chords with markers
        println!("=== First 30 Detected Chords (with Eb key spelling) ===\n");
        for (i, chord) in detected.iter().take(30).enumerate() {
            // Find the nearest marker to this detected chord's start time
            let nearest_marker = markers
                .iter()
                .min_by_key(|m| ((m.tick as i64) - chord.start_ppq).abs());

            let marker_info = if let Some(marker) = nearest_marker {
                let diff = (marker.tick as i64) - chord.start_ppq;
                format!(
                    "nearest marker: {} at tick {} (diff: {})",
                    marker.chord_name, marker.tick, diff
                )
            } else {
                "no marker".to_string()
            };

            println!(
                "#{:2}: tick {:6}-{:6}: {} (root: {:?}) - {}",
                i + 1,
                chord.start_ppq,
                chord.end_ppq,
                chord.chord.normalized,
                chord.chord.root,
                marker_info
            );
        }

        // Summary statistics - compare chord names
        let mut matched_name = 0;
        let mut matched_position = 0;
        let mut unmatched = 0;
        let mut name_mismatches: Vec<(String, String, i64)> = Vec::new();
        let tolerance = (ppq / 4) as i64; // 1/4 beat tolerance

        for detected_chord in &detected {
            // Find marker at similar position
            let matching = markers
                .iter()
                .find(|m| ((m.tick as i64) - detected_chord.start_ppq).abs() < tolerance);

            if let Some(marker) = matching {
                matched_position += 1;
                // Normalize marker name for comparison (strip "maj" prefix if present)
                let marker_normalized = marker.chord_name.replace("maj", "").replace("Maj", "");
                let detected_normalized = detected_chord
                    .chord
                    .normalized
                    .replace("maj", "")
                    .replace("Maj", "");

                if marker_normalized == detected_normalized
                    || marker.chord_name == detected_chord.chord.normalized
                {
                    matched_name += 1;
                } else {
                    name_mismatches.push((
                        detected_chord.chord.normalized.clone(),
                        marker.chord_name.clone(),
                        detected_chord.start_ppq,
                    ));
                }
            } else {
                unmatched += 1;
            }
        }

        println!("\n=== Summary ===");
        println!("Detected chords with nearby markers: {}", matched_position);
        println!("  - Matching names: {}", matched_name);
        println!("  - Name mismatches: {}", name_mismatches.len());
        println!("Detected chords without nearby markers: {}", unmatched);
        println!("Total markers: {}", markers.len());

        if !name_mismatches.is_empty() {
            println!("\n=== Name Mismatches (first 20) ===");
            for (detected, marker, tick) in name_mismatches.iter().take(20) {
                println!(
                    "  tick {:6}: detected '{}' vs marker '{}'",
                    tick, detected, marker
                );
            }
        }
    }

    #[test]
    fn test_section_metadata_overrides_marker_measure_and_length() {
        let midi = MidiFile::from_parts(
            480,
            vec![],
            vec![],
            vec![TimeSignatureEvent {
                tick: 0,
                numerator: 4,
                denominator: 4,
            }],
            vec![
                MarkerEvent {
                    tick: 0,
                    text: "VS 1".to_string(),
                    marker_type: MarkerType::Marker,
                },
                MarkerEvent {
                    tick: 0,
                    text: "KFSECTION\tVS 1\t12\t16".to_string(),
                    marker_type: MarkerType::Text,
                },
            ],
            vec![],
            None,
        );

        let sections = midi.section_markers_absolute();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "VS 1");
        assert_eq!(sections[0].position.measure, 12);
        assert_eq!(sections[0].explicit_start_measure, Some(12));
        assert_eq!(sections[0].explicit_length, Some(16));
    }
}
