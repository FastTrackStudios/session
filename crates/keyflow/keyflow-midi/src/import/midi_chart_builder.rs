//! MIDI Chart Builder
//!
//! Converts a parsed MIDI file into Keyflow chart text notation.
//! Pipeline: `MidiFile` → chord detection → rhythm analysis → chart text
//!
//! This module detects chords from actual MIDI note data (not text markers),
//! determines push/pull timing, builds measure-aware rhythm elements,
//! and formats everything as Keyflow chart text.

use keyflow_proto::chord::{
    ChordFamily, ChordQuality, DetectedChord, MidiNote as KeyflowMidiNote,
    detect_chords_from_midi_notes,
};
use keyflow_proto::key::{KeySpelling, SpellingMode};
use keyflow_proto::primitives::MusicalNote;
use keyflow_proto::primitives::note::Note;

use super::pitch::{Pitch, PitchClass};

use super::midi_import::{MidiFile, MidiNote, SectionType as MidiSectionType};

// ============================================================================
// Public API
// ============================================================================

/// Quantization grid for melody-note onset / duration detection.
///
/// `Auto` (the default) inspects the smallest IOI (inter-onset interval)
/// in each section and picks the appropriate grid:
/// - any IOI ≤ a triplet-eighth (with tolerance) → `Triplet`
/// - any IOI < a straight eighth                → `Sixteenth`
/// - else                                        → `Eighth`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MelodyGrid {
    /// Auto-detect from onset patterns in each section (default).
    #[default]
    Auto,
    /// Snap onsets to the nearest eighth-note. Coarse, but stable for
    /// simple melodies that are mostly on the beat or off-beat.
    Eighth,
    /// Snap onsets to the nearest sixteenth-note. Better for lines that
    /// mix sixteenths and eighths.
    Sixteenth,
    /// Snap onsets to the nearest eighth-note triplet (PPQ/3 ticks).
    /// For triplet-feel material.
    Triplet,
}

/// Configuration for MIDI chart building.
#[derive(Default)]
pub struct MidiChartConfig {
    /// Key for chord spelling (e.g., "Eb" for Cm songs).
    /// If None, defaults to "C" (no respelling).
    pub key_root: Option<String>,
    /// Override title (if None, tries to extract from MIDI).
    pub title: Option<String>,
    /// Swing ratio override (0.5 = straight, 0.667 = triplet).
    /// If None, uses the value from `MidiFile::swing()`.
    pub swing: Option<f64>,
    /// Quantization grid for melody onset / duration detection.
    /// Defaults to `Auto`, which picks per-section based on the actual
    /// onset spacing in the MIDI data.
    pub melody_grid: MelodyGrid,
}

/// Generate a Keyflow chart text string from a parsed MIDI file.
///
/// This is the main entry point. It:
/// 1. Detects chords from MIDI note data
/// 2. Respells chords to the target key
/// 3. Detects push/pull timing (triplet, quarter)
/// 4. Builds measure-aware rhythm elements
/// 5. Formats as Keyflow chart text
pub fn generate_chart_text(midi: &MidiFile, config: &MidiChartConfig) -> String {
    let ppq = midi.ppq();
    let songstart = midi.songstart_tick();
    let sections = midi.section_markers_absolute();
    let (bpm, time_sig) = (midi.initial_tempo(), midi.initial_time_signature());
    let beats_per_measure = time_sig.0 as i32;
    let ticks_per_measure = (ppq as i64) * (beats_per_measure as i64);

    // Resolve swing: config override > MIDI file value
    let swing = config.swing.or(midi.swing());

    // Detect chords from MIDI notes (dequantized if swing is known)
    let detected = detect_chords_from_notes(midi, config, swing);

    // Determine if we should use /push = triplet
    // If swing is triplet (~0.667), we know it's a triplet feel — skip heuristic
    let use_triplet_setting = if swing.is_some_and(|s| (s - 0.6667).abs() < 0.05) {
        true
    } else {
        should_use_triplet_push_from_detected(&detected, ppq, songstart)
    };

    let mut output = String::new();

    // Metadata header
    if let Some(ref title) = config.title {
        output.push_str(title);
        output.push('\n');
    }

    // Key notation
    let key_str = config.key_root.as_deref().unwrap_or("C");
    output.push_str(&format!(
        "{}bpm {}/{} #{}\n",
        bpm.round() as i32,
        time_sig.0,
        time_sig.1,
        key_str,
    ));
    if use_triplet_setting {
        output.push_str("/push = triplet\n");
    }
    // Emit swing setting if known
    if let Some(s) = swing {
        if (s - 0.6667).abs() < 0.05 {
            output.push_str("/swing = triplet\n");
        } else if (s - 0.5).abs() > 0.01 {
            output.push_str(&format!("/swing = {:.4}\n", s));
        }
    }
    output.push('\n');

    // Process each section
    let section_lengths = calculate_section_lengths(
        &sections,
        &detected,
        songstart,
        ticks_per_measure,
        midi.duration_ticks() as i64,
    );

    for section in &section_lengths {
        let section_start_tick = section.start_tick;
        let section_end_tick = section.end_tick;

        // Detect if this section uses quarter push instead of triplet
        let section_push_type = detect_section_push_type(
            &detected,
            section_start_tick,
            section_end_tick,
            ppq,
            songstart,
        );

        // Section header with optional sub-label, measure count, and quoted name
        let quoted_name = if section.keyflow_type == "Interlude" {
            extract_quoted_name(&section.marker_name)
        } else if section.keyflow_type == "SOLO" {
            extract_solo_instrument(&section.marker_name)
        } else {
            None
        };
        let show_count = should_show_measure_count(&section.keyflow_type)
            || (section.keyflow_type == "VS" && section.number == Some(1));

        // Build header: TYPE [SUB_LABEL] [COUNT] ["QUOTED_NAME"]
        // Sub-labels are shown for sections with letter suffixes (CH 3A, Interlude B, etc.)
        // but not for Outro which just uses plain numbering.
        let mut header = section.keyflow_type.clone();
        let show_sub_label =
            section.sub_label.is_some() && !matches!(section.keyflow_type.as_str(), "Outro" | "VS");
        if show_sub_label && let Some(ref label) = section.sub_label {
            header.push(' ');
            header.push_str(label);
        }
        if show_count {
            header.push(' ');
            header.push_str(&section.length.to_string());
        }
        if let Some(ref name) = quoted_name {
            header.push_str(&format!(" \"{}\"", name));
        }
        output.push_str(&header);
        output.push('\n');

        // Add section-specific push directive if different from global
        if let Some(ref push_type) = section_push_type
            && use_triplet_setting
            && push_type == "4"
        {
            output.push_str("/push 4\n");
        }

        // Handle COUNT section specially - just shows silence
        if section.keyflow_type == "COUNT" {
            output.push('\n');
            continue;
        }

        // Build rhythm elements from detected chords
        let elements = build_rhythm_elements(
            &detected,
            section_start_tick,
            section_end_tick,
            ppq,
            songstart,
            use_triplet_setting,
        );

        // Merge consecutive identical chords
        let elements = merge_consecutive_chords(elements);
        // Apply groove pattern push detection (F/C -> Cm pattern)
        let elements = apply_groove_pattern_push(elements);

        // Determine push setting for this section
        let use_short_push_for_section = use_triplet_setting || section_push_type.is_some();

        // Layout preferences: default 4 measures per line; long instrumentals keep 8 with a midline bar.
        let (measures_per_line, midline_separator_at) =
            if matches!(section.keyflow_type.as_str(), "INST" | "SOLO") && section.length >= 8 {
                (8usize, Some(4usize))
            } else {
                (4usize, None)
            };

        // Format the elements with measure awareness
        let section_content = format_rhythm_elements(
            &elements,
            section_start_tick,
            section.length,
            ppq,
            beats_per_measure,
            use_short_push_for_section,
            &section.keyflow_type,
            measures_per_line,
            midline_separator_at,
        );

        // For INST/SOLO sections with only silence, try extracting melody from MIDI notes
        let section_content = if section_content
            .trim()
            .chars()
            .all(|c| c == 's' || c == '1' || c == '|' || c == ' ' || c == '\n')
            && matches!(section.keyflow_type.as_str(), "INST" | "SOLO")
        {
            if let Some(melody_content) = extract_melody_for_section(
                midi,
                section_start_tick,
                section_end_tick,
                section.length as usize,
                ppq,
                beats_per_measure,
                measures_per_line,
                config.key_root.as_deref(),
                swing,
                config.melody_grid,
            ) {
                melody_content
            } else {
                section_content
            }
        } else {
            section_content
        };

        if section_content.is_empty() {
            output.push_str("%\n");
        } else {
            let compressed = compress_repeated_lines(&section_content);
            output.push_str(&compressed);
            output.push('\n');
        }

        output.push('\n');
    }

    output
}

/// Extract melody notes from raw MIDI data for a section and format as m{} blocks.
///
/// For INST/SOLO sections that have no chord voicings (only single-note lines),
/// this extracts the melody and generates one m{} block per measure.
///
/// Returns formatted chart lines with melody blocks, or None if no melody found.
#[allow(clippy::too_many_arguments)]
fn extract_melody_for_section(
    midi: &MidiFile,
    section_start_tick: i64,
    section_end_tick: i64,
    section_length: usize,
    ppq: u32,
    beats_per_measure: i32,
    measures_per_line: usize,
    key_root: Option<&str>,
    swing: Option<f64>,
    melody_grid: MelodyGrid,
) -> Option<String> {
    let ticks_per_measure = ppq as i64 * beats_per_measure as i64;

    // Get all notes in this section from ANY track (melody may be on LINES track,
    // not the harmony/CHORDS track)
    let all_notes = midi.all_notes();
    let section_notes: Vec<&MidiNote> = all_notes
        .iter()
        .filter(|n| {
            let t = n.start_tick as i64;
            t >= section_start_tick && t < section_end_tick
        })
        .collect();

    if section_notes.is_empty() {
        return None;
    }

    // Build key spelling for note names
    let key_spelling = key_root
        .and_then(|k| MusicalNote::from_string(k))
        .map(|note| KeySpelling::major(&note));

    // Resolve the actual quantization grid for this section. Auto inspects
    // onset spacing in the section and picks a grid that won't lose detail.
    let resolved_grid = resolve_melody_grid(&section_notes, ppq, melody_grid);
    let grid_unit = resolved_grid.tick_unit(ppq);

    // Legacy aliases used elsewhere in the function. `eighth` here is the
    // quantization unit for snapping; `quarter` is unchanged.
    let eighth = grid_unit;
    let _quarter = ppq as i64;

    // Step 1: Dequantize swing and snap to eighth-note grid.
    // For melody transcription, we snap ALL onsets to the nearest eighth-note
    // grid position. This handles swing (where off-beat eighths are delayed)
    // and minor timing variations from live performance.
    let mut grid_onsets: Vec<(i64, &MidiNote)> = section_notes
        .iter()
        .map(|note| {
            let raw = note.start_tick as i64;
            // For swing: move the off-beat note back to the straight position
            let straight = if let Some(ratio) = swing {
                // Use wider tolerance for melody dequantization
                let beat_ticks = ppq as i64;
                let pos_in_beat = raw.rem_euclid(beat_ticks);
                let beat_start = raw - pos_in_beat;
                let swing_pos = (ratio * beat_ticks as f64).round() as i64;
                let straight_pos = beat_ticks / 2;
                let tolerance = ppq as i64 / 6; // wider tolerance for melody (~160 ticks at 960 PPQ)
                if (pos_in_beat - swing_pos).abs() < tolerance {
                    beat_start + straight_pos
                } else {
                    raw
                }
            } else {
                raw
            };
            // Snap to nearest eighth-note grid position
            let grid_pos = ((straight - section_start_tick + eighth / 2) / eighth) * eighth
                + section_start_tick;
            (grid_pos, *note)
        })
        .collect();

    // Deduplicate: if multiple notes land on the same grid position, keep highest pitch
    grid_onsets.sort_by_key(|(pos, note)| (*pos, std::cmp::Reverse(note.pitch)));
    grid_onsets.dedup_by_key(|(pos, _)| *pos);

    // Step 2: Build melody tokens from grid positions
    // Track accidental state per pitch class within each measure to add naturals
    // when an accidental is cancelled (e.g., Bb followed by B natural needs ♮).
    let mut melody_tokens = Vec::new();
    let section_end_grid = section_end_tick;

    // Map from diatonic pitch class (0-6 for C-B) to active accidental alteration.
    // Initialize from key signature so that e.g. in Bb major, B and E start as flat.
    // When B natural appears, it needs a natural sign to cancel the key signature flat.
    let key_sig_accidentals: std::collections::HashMap<u8, i8> = {
        let mut map = std::collections::HashMap::new();
        if let Some(ref ks) = key_spelling {
            // Check each diatonic pitch class against the key signature
            for &pc in &[
                PitchClass::C,
                PitchClass::D,
                PitchClass::E,
                PitchClass::F,
                PitchClass::G,
                PitchClass::A,
                PitchClass::B,
            ] {
                // Build a pitch at octave 4 and check if the key signature alters it
                let midi_natural = pc.base_midi() + 48; // octave 4
                let pitch_from_key = Pitch::from_midi(midi_natural);
                // Check if this pitch class has an accidental in the key
                // by looking at the key spelling for this pitch class's chromatic position
                if ks.prefers_flat() {
                    // Flat keys: check if the natural pitch is raised in the scale
                    // Bb major: Bb, Eb are flat (B=flat, E=flat)
                    // Order of flats: B, E, A, D, G, C, F
                    let flat_order = [
                        PitchClass::B,
                        PitchClass::E,
                        PitchClass::A,
                        PitchClass::D,
                        PitchClass::G,
                        PitchClass::C,
                        PitchClass::F,
                    ];
                    // Count flats from key root
                    let num_flats = match key_root.unwrap_or("C") {
                        "F" => 1,
                        "Bb" => 2,
                        "Eb" => 3,
                        "Ab" => 4,
                        "Db" => 5,
                        "Gb" => 6,
                        _ => 0,
                    };
                    for &flat_pc in flat_order.iter().take(num_flats) {
                        map.insert(flat_pc.staff_offset() as u8, -1);
                    }
                } else {
                    // Sharp keys: F#, C#, G#, D#, A#, E#, B#
                    let sharp_order = [
                        PitchClass::F,
                        PitchClass::C,
                        PitchClass::G,
                        PitchClass::D,
                        PitchClass::A,
                        PitchClass::E,
                        PitchClass::B,
                    ];
                    let num_sharps = match key_root.unwrap_or("C") {
                        "G" => 1,
                        "D" => 2,
                        "A" => 3,
                        "E" => 4,
                        "B" => 5,
                        "F#" => 6,
                        _ => 0,
                    };
                    for &sharp_pc in sharp_order.iter().take(num_sharps) {
                        map.insert(sharp_pc.staff_offset() as u8, 1);
                    }
                }
                let _ = pitch_from_key; // suppress unused
            }
        }
        map
    };

    let mut active_accidentals = key_sig_accidentals.clone();
    let mut current_measure_start = section_start_tick;

    for (ni, (onset, note)) in grid_onsets.iter().enumerate() {
        // Reset accidental tracking at measure boundaries (back to key signature defaults)
        let measure_idx = ((*onset - section_start_tick) / ticks_per_measure) as usize;
        let this_measure_start = section_start_tick + (measure_idx as i64) * ticks_per_measure;
        if this_measure_start != current_measure_start {
            active_accidentals = key_sig_accidentals.clone();
            current_measure_start = this_measure_start;
        }

        let pitch = Pitch::from_midi(note.pitch);
        let mut pitch_name = format_pitch_name(&pitch, key_spelling.as_ref());

        // Check if we need a natural sign to cancel a previous accidental
        let _pitch_class = note.pitch % 12; // 0-11
        let diatonic_class = pitch.class.staff_offset() as u8; // 0-6 (C-B)
        let current_alteration = pitch.alteration;

        if let Some(&prev_alt) = active_accidentals.get(&diatonic_class) {
            // Same pitch class appeared before in this measure with a different accidental
            if prev_alt != current_alteration && current_alteration == 0 {
                // Previous had accidental, this one is natural — add natural sign
                pitch_name.push('n'); // 'n' for natural in melody notation
            }
        }

        // Track this note's accidental state
        if current_alteration != 0 {
            active_accidentals.insert(diatonic_class, current_alteration);
        } else if active_accidentals.contains_key(&diatonic_class) {
            // Natural cancels previous accidental
            active_accidentals.remove(&diatonic_class);
        }

        // Add rest at start if first note doesn't land on beat 1
        if ni == 0 && *onset > section_start_tick {
            let gap = onset - section_start_tick;
            if gap >= eighth {
                let rest_dur = grid_ticks_to_duration_with_grid(gap, ppq, resolved_grid);
                melody_tokens.push(format!("r_{}", rest_dur));
            }
        }

        // Duration = onset-to-onset interval, or section end for last note.
        // In jazz, notes are played short (staccato) but notated at the rhythmic
        // grid value — no rests between connected eighth notes.
        let next_onset = if ni + 1 < grid_onsets.len() {
            grid_onsets[ni + 1].0
        } else {
            section_end_grid
        };
        let duration_ticks = (next_onset - onset).max(eighth);

        let dur_str = grid_ticks_to_duration_with_grid(duration_ticks, ppq, resolved_grid);
        let octave = pitch.octave.0;
        melody_tokens.push(format!("{}{}_{}", pitch_name, octave, dur_str));
    }

    if melody_tokens.is_empty() {
        return None;
    }

    let melody_block = format!("m{{ {} }}", melody_tokens.join(" "));

    // Generate section output: first measure has the melody block attached,
    // remaining measures use dot repeats (the melody spills across them)
    let mut output = String::new();
    for i in 0..section_length {
        if i > 0 && i % measures_per_line == 0 {
            output.push('\n');
        }
        if i % measures_per_line == 0 {
            output.push_str("| ");
        }
        if i == 0 {
            // First measure gets the melody block — expand_melodies_across_measures
            // will distribute it across all measures
            output.push_str(&format!(". {}", melody_block));
        } else {
            output.push('.');
        }
        output.push_str(" | ");
    }

    Some(output)
}

/// Format a Pitch as a note name string (e.g., "C", "Bb", "F#").
fn format_pitch_name(pitch: &Pitch, key_spelling: Option<&KeySpelling>) -> String {
    // Get the base note name from the pitch class
    let base = match pitch.class {
        PitchClass::C => "C",
        PitchClass::D => "D",
        PitchClass::E => "E",
        PitchClass::F => "F",
        PitchClass::G => "G",
        PitchClass::A => "A",
        PitchClass::B => "B",
    };

    let acc = match pitch.alteration {
        1 => {
            // Check if we should spell as flat instead
            if let Some(ks) = key_spelling
                && ks.prefers_flat()
            {
                return format_as_flat(pitch);
            }
            "#"
        }
        -1 => "b",
        2 => "##",
        -2 => "bb",
        _ => "",
    };

    format!("{}{}", base, acc)
}

/// Re-spell a sharp pitch as its enharmonic flat equivalent.
fn format_as_flat(pitch: &Pitch) -> String {
    // Sharp notes → flat enharmonic: C# → Db, D# → Eb, F# → Gb, G# → Ab, A# → Bb
    match pitch.class {
        PitchClass::C => "Db".to_string(),
        PitchClass::D => "Eb".to_string(),
        PitchClass::F => "Gb".to_string(),
        PitchClass::G => "Ab".to_string(),
        PitchClass::A => "Bb".to_string(),
        _ => format!(
            "{}#",
            match pitch.class {
                PitchClass::E => "E",
                PitchClass::B => "B",
                _ => "?",
            }
        ),
    }
}

/// Convert grid-quantized tick duration to a LilyPond duration string.
/// Durations are exact multiples of eighth notes from the grid.
/// For melody lines, prefer eighth notes (beamable) over quarters where possible.
/// A concrete (non-Auto) quantization grid resolved for a specific section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedMelodyGrid {
    Eighth,
    Sixteenth,
    Triplet,
}

impl ResolvedMelodyGrid {
    fn tick_unit(self, ppq: u32) -> i64 {
        match self {
            Self::Eighth => ppq as i64 / 2,
            Self::Sixteenth => ppq as i64 / 4,
            Self::Triplet => ppq as i64 / 3,
        }
    }
}

/// Pick the best resolved grid for a set of section notes given a user
/// preference (`Auto` triggers detection).
fn resolve_melody_grid(notes: &[&MidiNote], ppq: u32, requested: MelodyGrid) -> ResolvedMelodyGrid {
    match requested {
        MelodyGrid::Eighth => return ResolvedMelodyGrid::Eighth,
        MelodyGrid::Sixteenth => return ResolvedMelodyGrid::Sixteenth,
        MelodyGrid::Triplet => return ResolvedMelodyGrid::Triplet,
        MelodyGrid::Auto => {}
    }

    if notes.len() < 2 {
        return ResolvedMelodyGrid::Eighth;
    }

    // Compute consecutive IOIs (inter-onset intervals).
    let mut starts: Vec<i64> = notes.iter().map(|n| n.start_tick as i64).collect();
    starts.sort_unstable();
    let mut min_ioi = i64::MAX;
    for w in starts.windows(2) {
        let d = w[1] - w[0];
        if d > 0 && d < min_ioi {
            min_ioi = d;
        }
    }
    if min_ioi == i64::MAX {
        return ResolvedMelodyGrid::Eighth;
    }

    let eighth = ppq as i64 / 2;
    let sixteenth = ppq as i64 / 4;
    let triplet_eighth = ppq as i64 / 3;
    let tol = (ppq as i64) / 16; // ~30 ticks at 480 PPQ — a fairly tight tolerance.

    // Triplet wins if any IOI is within tolerance of a triplet-eighth.
    if (min_ioi - triplet_eighth).abs() <= tol {
        return ResolvedMelodyGrid::Triplet;
    }
    // Sixteenth wins if any IOI is materially shorter than a straight eighth.
    if min_ioi + tol < eighth && min_ioi >= sixteenth - tol {
        return ResolvedMelodyGrid::Sixteenth;
    }
    ResolvedMelodyGrid::Eighth
}

/// Map a measured tick gap to a keyflow duration token using the resolved
/// grid. Picks dotted forms when the gap is 1.5×, 3×, or 7× the grid unit
/// to preserve dotted-note rhythms (e.g., dotted-quarter from a sixteenth grid).
fn grid_ticks_to_duration_with_grid(ticks: i64, ppq: u32, grid: ResolvedMelodyGrid) -> String {
    let unit = grid.tick_unit(ppq);
    let units = ((ticks + unit / 2) / unit).max(1);
    match grid {
        ResolvedMelodyGrid::Eighth => match units {
            1 => "8",
            2 => "4",
            3 => ".4",
            4 => "2",
            5..=6 => ".2",
            7..=8 => "1",
            _ => "1",
        }
        .to_string(),
        ResolvedMelodyGrid::Sixteenth => match units {
            1 => "16",
            2 => "8",
            3 => ".8", // dotted eighth
            4 => "4",
            6 => ".4", // dotted quarter
            8 => "2",
            12 => ".2", // dotted half
            16 => "1",
            // Fallbacks for awkward in-between counts: round to nearest standard
            // value rather than emitting weird tuplets.
            5 => "4",
            7 => ".4",
            9..=11 => "2",
            13..=15 => ".2",
            _ => "1",
        }
        .to_string(),
        ResolvedMelodyGrid::Triplet => match units {
            // 8t = eighth-triplet, 4t = quarter (= 2 triplet eighths), etc.
            1 => "8t",
            2 => "4t",
            3 => "8", // 3 triplet-eighths = 1 quarter-equivalent
            4 => "2t",
            6 => "4",
            8 => "1t",
            12 => "2",
            _ => "4t",
        }
        .to_string(),
    }
}

/// Compress consecutive identical lines by appending ` xN` (e.g., "line" → "line x2").
fn compress_repeated_lines(input: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<&str> = None;
    let mut count: usize = 0;

    for line in input.lines() {
        if Some(line) == prev {
            count += 1;
        } else {
            if let Some(p) = prev {
                if count > 1 {
                    out.push(format!("{} x{}", p, count));
                } else {
                    out.push(p.to_string());
                }
            }
            prev = Some(line);
            count = 1;
        }
    }

    if let Some(p) = prev {
        if count > 1 {
            out.push(format!("{} x{}", p, count));
        } else {
            out.push(p.to_string());
        }
    }

    out.join("\n")
}

// ============================================================================
// Swing Dequantization
// ============================================================================

/// Snap a swung tick position back to the straight grid.
///
/// Within each beat, if the tick falls near `swing_ratio * beat_ticks` (the
/// swung upbeat position), snap it to the straight upbeat at `beat_ticks / 2`.
fn dequantize_tick(tick: i64, ppq: u32, swing_ratio: f64) -> i64 {
    let beat_ticks = ppq as i64;
    let pos_in_beat = tick.rem_euclid(beat_ticks);
    let beat_start = tick - pos_in_beat;
    let swing_pos = (swing_ratio * beat_ticks as f64).round() as i64;
    let straight_pos = beat_ticks / 2;
    let tolerance = ppq as i64 / 12; // ~80 ticks at 960 PPQ
    if (pos_in_beat - swing_pos).abs() < tolerance {
        beat_start + straight_pos
    } else {
        tick
    }
}

// ============================================================================
// Chord Detection
// ============================================================================

/// A rhythm element in the generated chart - either a chord with duration or a rest.
#[derive(Debug, Clone)]
enum ChordOrRest {
    Chord {
        symbol: String,
        start_ppq: i64,
        end_ppq: i64,
        is_pushed: bool,
        push_amount: Option<String>,
        is_accented: bool,
        /// Staccato flag: true when a pushed chord only lasts for the push duration
        /// (e.g., eighth-note push that ends on the downbeat). Notated as `>'.Chord`.
        is_staccato: bool,
    },
    Rest {
        start_ppq: i64,
        end_ppq: i64,
    },
}

/// Detect chords from MIDI notes and respell to target key.
/// If `swing` is provided, dequantizes note positions before chord detection.
fn detect_chords_from_notes(
    midi: &MidiFile,
    config: &MidiChartConfig,
    swing: Option<f64>,
) -> Vec<DetectedChord> {
    let ppq = midi.ppq();
    let harmony_channel = primary_harmony_channel(midi);
    let all_notes: Vec<_> = midi
        .all_notes()
        .into_iter()
        .filter(|note| harmony_channel.is_none_or(|channel| note.channel == channel))
        .collect();

    let keyflow_notes: Vec<KeyflowMidiNote> = all_notes
        .iter()
        .map(|n| {
            let (start, end) = if let Some(ratio) = swing {
                let s = dequantize_tick(n.start_tick as i64, ppq, ratio);
                let e = dequantize_tick((n.start_tick + n.duration_ticks) as i64, ppq, ratio);
                (s, e)
            } else {
                (
                    n.start_tick as i64,
                    (n.start_tick + n.duration_ticks) as i64,
                )
            };
            KeyflowMidiNote::new(n.pitch, start, end, n.channel, n.velocity)
        })
        .collect();

    let sixteenth = (ppq / 4) as i64;
    let min_duration = sixteenth / 2; // ~120 ticks - catches staccato hits
    let mut detected = detect_chords_from_midi_notes(&keyflow_notes, min_duration);

    // Respell chords to target key if provided
    if let Some(ref key_root) = config.key_root
        && let Some(note) = MusicalNote::from_string(key_root)
    {
        let key_spelling = KeySpelling::major(&note);
        for chord_event in &mut detected {
            chord_event
                .chord
                .respell_root(&key_spelling, SpellingMode::Relaxed);
        }
    }

    // Apply common chord spelling normalizations
    // (e.g., D#m -> Ebm, G#m -> Abm for readability)
    for chord_event in &mut detected {
        normalize_chord_spelling(&mut chord_event.chord);
    }

    detected
}

fn primary_harmony_channel(midi: &MidiFile) -> Option<u8> {
    let mut counts = std::collections::BTreeMap::<u8, usize>::new();
    for note in midi.all_notes() {
        *counts.entry(note.channel).or_default() += 1;
    }

    if counts.is_empty() {
        return None;
    }

    if counts.contains_key(&0) {
        return Some(0);
    }

    counts
        .into_iter()
        .max_by_key(|(channel, count)| (*count, std::cmp::Reverse(*channel)))
        .map(|(channel, _)| channel)
}

/// Normalize chord spelling to use conventional/readable names.
///
/// Some enharmonic spellings are more common in music notation:
/// - Ebm is more common than D#m
/// - Abm is more common than G#m
/// - Bb is more common than A# (always)
/// - Db is more common than C# (for certain chord types)
/// - Gb is more common than F# (for certain chord types)
///
/// Also handles diminished 7th chord inversions:
/// - Fdim/B -> Bdim7 (since dim7 chords are symmetric, use bass as root)
///
/// This function adjusts uncommon spellings to their conventional equivalents.
fn normalize_chord_spelling(chord: &mut keyflow_proto::chord::Chord) {
    // First: Handle diminished chord inversions
    // Dim7 chords are symmetric (all minor 3rds), so Fdim/B = Bdim7 = Abdim7 = Ddim7
    // When we have Xdim/Y where Y is 3 semitones (minor 3rd) from X,
    // convert to Ydim7 (use bass as root, make it fully diminished)
    if chord.quality == ChordQuality::Diminished && chord.bass.is_some() {
        // Check if bass is a minor 3rd below the root (part of the dim7 cycle)
        if let (Some(root_note), Some(bass_root)) = (
            chord.root.resolved_note(),
            chord.bass.as_ref().and_then(|b| b.resolved_note()),
        ) {
            let root_semitone = root_note.semitone();
            let bass_semitone = bass_root.semitone();
            // Check if bass is 3, 6, or 9 semitones below root (dim7 cycle)
            let interval = (root_semitone + 12 - bass_semitone) % 12;
            if interval == 3 || interval == 6 || interval == 9 {
                // Take the bass note and make it the root, make it dim7
                if let Some(bass) = chord.bass.take() {
                    chord.root = bass;
                    chord.family = Some(ChordFamily::FullyDiminished);
                    chord.normalize();
                }
            }
        }
    }

    // Second: Handle enharmonic spelling normalization
    // Get current root name from the resolved note
    let root_name = chord.root.resolved_note().map(|n| n.name());

    // Check if we need to respell with flats
    let needs_flat_spelling = root_name
        .as_ref()
        .is_some_and(|name| matches!(name.as_str(), "D#" | "G#" | "A#"));

    if needs_flat_spelling {
        // Create a key spelling that prefers flats (F major has 1 flat)
        let flat_spelling = KeySpelling::major(&MusicalNote::from_string("F").unwrap());
        chord.respell_root(&flat_spelling, SpellingMode::Relaxed);
    }
}

// ============================================================================
// Push/Pull Detection
// ============================================================================

/// Detect push/pull timing for a chord based on its position within a beat.
///
/// Returns `(is_pushed, push_amount)` where `push_amount` encodes the type:
/// - `"t"` = triplet push/pull (swing eighth)
/// - `"8"` = straight eighth push/pull
/// - `"16"` = sixteenth note push/pull
///
/// When `is_triplet_song` is true, straight eighth positions are NOT marked as pushes
/// because the song uses triplet-based timing (swing feel).
fn detect_push_pull_for_chord(
    start_ppq: i64,
    end_ppq: i64,
    ppq: u32,
    songstart: u32,
    is_triplet_song: bool,
) -> (bool, Option<String>) {
    let relative_tick = if start_ppq >= songstart as i64 {
        start_ppq - songstart as i64
    } else {
        start_ppq
    };

    let ticks_per_beat = ppq as i64;
    let subdivision = (relative_tick % ticks_per_beat) as u32;

    let triplet_eighth = ppq / 3; // 320 at 960 PPQ
    let triplet_quarter = triplet_eighth * 2; // 640 at 960 PPQ
    let straight_eighth = ppq / 2; // 480 at 960 PPQ
    let _sixteenth = ppq / 4; // 240 at 960 PPQ
    let dotted_eighth = (ppq * 3) / 4; // 720 at 960 PPQ — sixteenth before next beat
    let tolerance = ppq / 24; // ~40 ticks

    // Check if chord crosses into the next beat (true push vs syncopation)
    let start_beat = start_ppq / ticks_per_beat;
    let end_beat = end_ppq / ticks_per_beat;
    let crosses_beat = end_beat > start_beat;
    let next_beat_tick = (start_beat + 1) * ticks_per_beat;
    let ends_at_next_beat = (end_ppq - next_beat_tick).abs() < tolerance as i64;
    let short_duration = (end_ppq - start_ppq) <= ticks_per_beat / 2;

    if subdivision < tolerance || subdivision > (ppq - tolerance) {
        // On the beat
        (false, None)
    } else if (subdivision as i32 - triplet_eighth as i32).unsigned_abs() < tolerance {
        // Pull by triplet eighth (1/3 of beat after downbeat)
        (false, Some("t".to_string()))
    } else if (subdivision as i32 - triplet_quarter as i32).unsigned_abs() < tolerance {
        // Push by triplet eighth (2/3 of beat = 1/3 before next beat)
        (
            crosses_beat || (short_duration && ends_at_next_beat),
            Some("t".to_string()),
        )
    } else if (subdivision as i32 - dotted_eighth as i32).unsigned_abs() < tolerance {
        // Push by sixteenth (3/4 of beat = 1/4 before next beat)
        (
            crosses_beat || (short_duration && ends_at_next_beat),
            Some("16".to_string()),
        )
    } else if (subdivision as i32 - straight_eighth as i32).unsigned_abs() < tolerance {
        // Straight eighth — at the "and" of the beat.
        // In triplet-based songs, only mark as a push when the chord crosses into
        // the next beat (a genuine anticipation). Brief syncopations that stay
        // within the beat are left unmarked.
        // In straight-eighth songs, mark as push if the chord crosses into the next beat.
        let treat_as_push = if is_triplet_song {
            crosses_beat
        } else {
            crosses_beat || (short_duration && ends_at_next_beat)
        };
        (
            treat_as_push,
            if treat_as_push {
                Some("8".to_string())
            } else {
                None
            },
        )
    } else {
        (false, None)
    }
}

/// Detect if a pushed chord is staccato (only lasts the push duration).
///
/// A staccato push is when a chord starts on a push position and ends at or near
/// the next beat boundary. For example:
/// - Triplet push: starts at beat + 2/3, ends near beat + 1
/// - Eighth push: starts at beat + 1/2, ends near beat + 1
/// - Sixteenth push: starts at beat + 3/4, ends near beat + 1
///
/// Returns `true` if the chord should be notated as staccato (`>'.Chord`).
fn is_staccato_push(
    start_ppq: i64,
    end_ppq: i64,
    ppq: u32,
    songstart: u32,
    is_pushed: bool,
    _push_amount: &Option<String>,
) -> bool {
    // Only pushed chords can be staccato pushes
    if !is_pushed {
        return false;
    }

    let ticks_per_beat = ppq as i64;
    let tolerance = (ppq / 12) as i64; // ~80 ticks at 960 PPQ

    // Calculate where this chord started relative to songstart
    let relative_tick = if start_ppq >= songstart as i64 {
        start_ppq - songstart as i64
    } else {
        start_ppq
    };

    // Find the next beat boundary after the start
    let beat_of_start = relative_tick / ticks_per_beat;
    let next_beat_tick = (beat_of_start + 1) * ticks_per_beat + (songstart as i64);

    // Calculate the actual duration
    let actual_duration = end_ppq - start_ppq;

    // Calculate the expected push duration (time from start to next beat)
    let expected_staccato_duration = next_beat_tick - start_ppq;

    // The chord is staccato if it ends at or very close to the next beat
    // (within tolerance of the expected push duration)
    let ends_at_beat = (actual_duration - expected_staccato_duration).abs() < tolerance;

    // Also check for slightly shorter durations (true staccato - chord released before beat)
    let short_staccato = actual_duration < expected_staccato_duration
        && actual_duration >= expected_staccato_duration / 2;

    ends_at_beat || short_staccato
}

/// Check if a chord is a quarter push (starts on beat 4, majority in next measure).
fn is_quarter_push(chord: &DetectedChord, ppq: u32) -> bool {
    let ticks_per_beat = ppq as i64;
    let ticks_per_measure = ticks_per_beat * 4; // 4/4
    let tolerance = (ppq / 24) as i64;

    let measure_start = (chord.start_ppq / ticks_per_measure) * ticks_per_measure;
    let next_measure_start = measure_start + ticks_per_measure;

    let tick_in_measure = chord.start_ppq - measure_start;
    let beat_in_measure = tick_in_measure / ticks_per_beat;
    let subdivision = tick_in_measure % ticks_per_beat;

    if beat_in_measure != 3 || subdivision > tolerance {
        return false;
    }

    let chord_duration = chord.end_ppq - chord.start_ppq;
    let duration_in_current_measure = (next_measure_start - chord.start_ppq).min(chord_duration);
    let duration_in_next_measure = chord_duration - duration_in_current_measure;

    duration_in_next_measure > duration_in_current_measure
}

/// Determine push type for a section (quarter vs triplet).
/// Includes chords that start up to one beat before the section (pushed chords
/// that musically belong to this section).
fn detect_section_push_type(
    detected_chords: &[DetectedChord],
    section_start_tick: i64,
    section_end_tick: i64,
    ppq: u32,
    songstart: u32,
) -> Option<String> {
    let ticks_per_beat = ppq as i64;
    let tolerance = (ppq / 24) as i64;
    let triplet_eighth = (ppq / 3) as i64;
    let triplet_quarter = triplet_eighth * 2;

    // Include chords starting up to one beat before section (pushed chords)
    let lookback = ticks_per_beat;
    let section_chords: Vec<_> = detected_chords
        .iter()
        .filter(|c| {
            c.start_ppq >= (section_start_tick - lookback) && c.start_ppq < section_end_tick
        })
        .collect();

    if section_chords.is_empty() {
        return None;
    }

    let mut quarter_pushes = 0;
    let mut triplet_pushes = 0;

    let ticks_per_measure = ticks_per_beat * 4; // 4/4
    for chord in &section_chords {
        // Only count quarter pushes for chords with reasonable duration
        // (not sustained chords held across many measures)
        let chord_duration = chord.end_ppq - chord.start_ppq;
        if is_quarter_push(chord, ppq) && chord_duration <= ticks_per_measure * 2 {
            quarter_pushes += 1;
            continue;
        }

        let relative_tick = if chord.start_ppq >= songstart as i64 {
            chord.start_ppq - songstart as i64
        } else {
            chord.start_ppq
        };
        let subdivision = relative_tick % ticks_per_beat;

        let is_triplet_push = (subdivision - triplet_quarter).abs() < tolerance;
        let is_triplet_pull = (subdivision - triplet_eighth).abs() < tolerance;

        if is_triplet_push || is_triplet_pull {
            triplet_pushes += 1;
        }
    }

    // Require a minimum number of quarter pushes to declare /push 4,
    // and no triplet pushes in the section
    if quarter_pushes >= 2 && triplet_pushes == 0 {
        Some("4".to_string())
    } else {
        None
    }
}

/// Check if majority of detected chords use triplet pushes.
fn should_use_triplet_push_from_detected(
    chords: &[DetectedChord],
    ppq: u32,
    songstart: u32,
) -> bool {
    let mut triplet_count = 0;
    let mut other_push_count = 0;

    for chord in chords {
        // Pass is_triplet_song=false to detect ALL push types for counting
        let (is_pushed, push_amount) =
            detect_push_pull_for_chord(chord.start_ppq, chord.end_ppq, ppq, songstart, false);
        if is_pushed || push_amount.is_some() {
            if push_amount.as_deref() == Some("t") {
                triplet_count += 1;
            } else {
                other_push_count += 1;
            }
        }
    }

    let total = triplet_count + other_push_count;
    total > 0 && (triplet_count as f64 / total as f64) > 0.5
}

// ============================================================================
// Section Layout
// ============================================================================

/// Map MIDI section type to keyflow abbreviation.
/// `is_pre_songstart` distinguishes opening HITS (uppercase) from closing Hits (title case).
fn section_type_to_keyflow(section_type: MidiSectionType, is_pre_songstart: bool) -> &'static str {
    match section_type {
        MidiSectionType::CountIn => "",
        MidiSectionType::Hits => {
            if is_pre_songstart {
                "HITS"
            } else {
                "Hits"
            }
        }
        MidiSectionType::Intro => "IN",
        MidiSectionType::Verse => "VS",
        MidiSectionType::PreChorus => "PC",
        MidiSectionType::Chorus => "CH",
        MidiSectionType::Bridge => "BR",
        MidiSectionType::Instrumental => "INST",
        MidiSectionType::Solo => "SOLO",
        MidiSectionType::Interlude => "Interlude",
        MidiSectionType::Outro => "Outro",
        MidiSectionType::SongStart | MidiSectionType::Title | MidiSectionType::Other => "",
    }
}

/// Determine whether a section header should display its measure count.
/// Most song-body sections (VS, CH, BR, IN, HITS) omit the count.
/// Auxiliary/transitional sections (INST, Interlude, Outro, COUNT, Hits) show it.
fn should_show_measure_count(keyflow_type: &str) -> bool {
    matches!(
        keyflow_type,
        "COUNT" | "INST" | "SOLO" | "Interlude" | "Outro" | "Hits" | "CH"
    )
}

/// Extract a sub-label suffix from a MIDI marker name.
/// For example, `"CH 3A"` → `Some("3A")`, `"VS 1"` → `None` (plain number).
/// Only returns a label if it contains at least one letter character.
fn extract_sub_label(marker_name: &str) -> Option<String> {
    // Strip any quoted portion first (e.g., `Interlude B "HORNS"` → `Interlude B`)
    let name = marker_name.split('"').next().unwrap_or(marker_name).trim();
    // Find the first space — everything after the first token is a potential label
    let first_space = name.find(' ')?;
    let suffix = name[first_space + 1..].trim();
    if suffix.is_empty() {
        return None;
    }
    // Only keep if it contains a letter (e.g., "3A" has 'A', "1" doesn't)
    if suffix.chars().any(|c| c.is_ascii_alphabetic()) {
        Some(suffix.to_string())
    } else {
        None
    }
}

/// Extract an instrument name from a SOLO marker.
///
/// Supports parenthesized names: `"SOLO A (Trumpet)"` → `Some("Trumpet")`.
/// Also supports trailing words after SOLO + optional letter/number:
/// `"Guitar Solo"` → `Some("Guitar")`, `"SOLO Keys"` → `Some("Keys")`.
fn extract_solo_instrument(marker_name: &str) -> Option<String> {
    // Try parenthesized instrument: "SOLO A (Trumpet)" or "Solo B (Trumpet)"
    if let Some(open) = marker_name.find('(')
        && let Some(close) = marker_name[open..].find(')')
    {
        let instrument = marker_name[open + 1..open + close].trim();
        if !instrument.is_empty() {
            return Some(instrument.to_string());
        }
    }

    // Try "Instrument SOLO" pattern (e.g., "Guitar Solo", "Trumpet Solo")
    let lower = marker_name.to_lowercase();
    if let Some(pos) = lower.find("solo") {
        let before = marker_name[..pos].trim();
        if !before.is_empty() && !before.chars().all(|c| c.is_ascii_digit()) {
            return Some(before.to_string());
        }
    }

    None
}

/// Extract a quoted subsection name from a MIDI marker name.
/// For example, `Interlude B "HORNS"` → `Some("HORNS")`.
fn extract_quoted_name(marker_name: &str) -> Option<String> {
    let first_quote = marker_name.find('"')?;
    let rest = &marker_name[first_quote + 1..];
    let end_quote = rest.find('"')?;
    Some(rest[..end_quote].to_string())
}

/// Section layout info for chart generation.
struct SectionLayout {
    keyflow_type: String,
    length: i32,
    start_tick: i64,
    end_tick: i64,
    /// Original MIDI marker name (e.g., `Interlude B "HORNS"`).
    marker_name: String,
    /// Sub-label suffix from marker name (e.g., `"3A"` from `"CH 3A"`).
    /// Only set when the suffix contains a letter (not just a plain number like `"1"`).
    sub_label: Option<String>,
    /// Parsed section number (e.g., 1 from VS 1)
    number: Option<u32>,
}

/// Calculate section start measures and lengths from markers.
fn calculate_section_lengths(
    sections: &[super::midi_import::SectionMarker],
    detected_chords: &[DetectedChord],
    songstart_tick: u32,
    ticks_per_measure: i64,
    fallback_end_tick: i64,
) -> Vec<SectionLayout> {
    let mut result = Vec::new();

    for (i, section) in sections.iter().enumerate() {
        let is_pre_songstart = section.tick <= songstart_tick;
        let keyflow_type = section_type_to_keyflow(section.section_type, is_pre_songstart);
        if keyflow_type.is_empty() {
            continue;
        }

        let start_tick = i64::from(section.tick);
        let next_section_tick = sections.iter().skip(i + 1).find_map(|next| {
            let next_type = section_type_to_keyflow(next.section_type, next.tick <= songstart_tick);
            (next.tick > section.tick && !next_type.is_empty()).then_some(i64::from(next.tick))
        });

        let (length, end_tick) = if let Some(explicit_length) = section.explicit_length {
            let length = explicit_length.max(1) as i64;
            (length, start_tick + (length * ticks_per_measure))
        } else {
            let mut end_tick = next_section_tick.unwrap_or(fallback_end_tick);
            if end_tick <= start_tick {
                end_tick = start_tick + ticks_per_measure;
            }

            if let Some(last_chord_end) = detected_chords
                .iter()
                .filter(|chord| chord.start_ppq < end_tick && chord.end_ppq > start_tick)
                .map(|chord| chord.end_ppq)
                .max()
            {
                end_tick = end_tick.max(last_chord_end);
            }

            let mut length = ((end_tick - start_tick) + ticks_per_measure - 1) / ticks_per_measure;
            if length < 1 {
                length = 1;
            }
            (length, start_tick + (length * ticks_per_measure))
        };

        // Extract sub-label from marker name (e.g., "3A" from "CH 3A").
        // Only keep it if it contains a letter (plain numbers like "1" are omitted).
        let sub_label = extract_sub_label(&section.name);

        result.push(SectionLayout {
            keyflow_type: keyflow_type.to_string(),
            length: length as i32,
            start_tick,
            end_tick,
            marker_name: section.name.clone(),
            sub_label,
            number: section.number,
        });
    }

    result
}

// ============================================================================
// Rhythm Element Building
// ============================================================================

/// Build rhythm elements for a section from detected chords.
/// Includes:
/// - Chords that start within the section
/// - Chords that start before the section but sustain into it (continuing chords)
/// - Pushed chords that start up to one beat before the section boundary
///
/// `is_triplet_song` indicates whether the song uses triplet-based timing,
/// which affects whether straight eighth positions are marked as pushes.
fn build_rhythm_elements(
    detected_chords: &[DetectedChord],
    section_start_tick: i64,
    section_end_tick: i64,
    ppq: u32,
    songstart: u32,
    is_triplet_song: bool,
) -> Vec<ChordOrRest> {
    let mut elements = Vec::new();
    let triplet_eighth = (ppq / 3) as i64;
    let ticks_per_beat = ppq as i64;

    // Find chords that start within the section
    let mut section_chords: Vec<&DetectedChord> = detected_chords
        .iter()
        .filter(|c| c.start_ppq >= section_start_tick && c.start_ppq < section_end_tick)
        .collect();

    // Drop short end-of-section pickup hits from this section so they can belong to the next one
    let boundary_tolerance = (ppq / 24) as i64;
    section_chords.retain(|c| {
        let starts_in_last_beat = c.start_ppq >= section_end_tick - ticks_per_beat;
        let ends_on_boundary = (c.end_ppq - section_end_tick).abs() <= boundary_tolerance;
        let is_short = (c.end_ppq - c.start_ppq) <= ticks_per_beat / 2;
        !(starts_in_last_beat && ends_on_boundary && is_short)
    });

    // Include pickup/push chords that start up to one beat before the section and land on/downbeat
    let pickup_window = ticks_per_beat;
    let pickup_tolerance = ticks_per_beat / 12;

    // Detect a pickup/push chord that lands on the section downbeat (for staccato pushes)
    let pickup_candidate = detected_chords.iter().find(|c| {
        let starts_before =
            c.start_ppq >= section_start_tick - pickup_window && c.start_ppq < section_start_tick;
        let ends_near_downbeat = (c.end_ppq - section_start_tick).abs() <= pickup_tolerance
            || (c.end_ppq >= section_start_tick
                && c.end_ppq <= section_start_tick + ticks_per_beat / 2);
        let short_hit = (c.end_ppq - c.start_ppq) <= ticks_per_beat / 2;
        starts_before && ends_near_downbeat && short_hit
    });
    let mut pickup_chords: Vec<&DetectedChord> = detected_chords
        .iter()
        .filter(|c| {
            c.start_ppq >= section_start_tick - pickup_window
                && c.start_ppq < section_start_tick
                && c.end_ppq >= section_start_tick - pickup_tolerance
        })
        .collect();

    section_chords.append(&mut pickup_chords);

    // If there is a short pickup hit that ends on the section downbeat and isn't already included, inject it.
    let existing_starts: std::collections::HashSet<i64> =
        section_chords.iter().map(|c| c.start_ppq).collect();
    if let Some(c) = pickup_candidate
        && !existing_starts.contains(&c.start_ppq)
    {
        section_chords.push(c);
    }

    // Also find chords that start before the section but sustain into it.
    // These are "continuing" chords — or pushed chords from the previous measure.
    let continuing_chords: Vec<&DetectedChord> = detected_chords
        .iter()
        .filter(|c| {
            c.start_ppq < section_start_tick
                && c.end_ppq > section_start_tick
                // Only include if it sustains significantly into the section (at least 1 beat)
                && (c.end_ppq - section_start_tick) >= ticks_per_beat
        })
        .collect();

    // Prepend continuing chords (they represent the section's starting chord)
    for cont in &continuing_chords {
        // Don't add if there's already a chord starting at or near section_start_tick
        let already_covered = section_chords
            .iter()
            .any(|sc| (sc.start_ppq - section_start_tick).abs() < ticks_per_beat);
        if !already_covered {
            section_chords.insert(0, cont);
        }
    }

    if section_chords.is_empty() {
        // Last-resort carryover: only fire when the section is *genuinely* silent
        // (no detected chord overlaps it at all, even fractionally). The
        // pickup / continuing logic above already covers boundary-overlap
        // cases; the bug we're fixing here is the case where chord detection
        // splits a held chord into multiple `DetectedChord`s and the last
        // one ends just before this section starts.
        let ticks_per_measure = ticks_per_beat * 4;
        let any_overlap = detected_chords
            .iter()
            .any(|c| c.end_ppq > section_start_tick && c.start_ppq < section_end_tick);

        if !any_overlap {
            // Bounded look-back: at most one section length (or 8 measures at
            // 4/4), whichever is larger, to avoid grabbing arbitrarily-old
            // chords.
            let look_back_window =
                (section_end_tick - section_start_tick).max(ticks_per_measure * 8);
            let look_back_floor = section_start_tick - look_back_window;

            let recent_chord = detected_chords
                .iter()
                .filter(|c| c.end_ppq <= section_start_tick && c.end_ppq >= look_back_floor)
                .max_by_key(|c| c.end_ppq);

            if let Some(c) = recent_chord {
                // Synthesize a sustained version of that chord covering the
                // section. Treated as continuing (no fresh attack), no push,
                // no staccato; the accent is dropped because the original
                // attack lives in the prior section.
                elements.push(ChordOrRest::Chord {
                    symbol: c.chord.normalized.clone(),
                    start_ppq: section_start_tick,
                    end_ppq: section_end_tick,
                    is_pushed: false,
                    push_amount: None,
                    is_accented: false,
                    is_staccato: false,
                });
                return elements;
            }
        }

        let duration = section_end_tick - section_start_tick;
        if duration > 0 {
            elements.push(ChordOrRest::Rest {
                start_ppq: section_start_tick,
                end_ppq: section_end_tick,
            });
        }
        return elements;
    }

    let mut current_pos = section_start_tick;

    // Ensure chords are ordered by start position
    section_chords.sort_by_key(|c| c.start_ppq);

    for chord in section_chords {
        // Clamp chord start to section boundary for continuing chords
        let effective_start = chord.start_ppq.max(section_start_tick);
        let effective_end = chord.end_ppq.min(section_end_tick);

        if effective_start > current_pos {
            let gap = effective_start - current_pos;
            if gap >= (ppq / 6) as i64 {
                elements.push(ChordOrRest::Rest {
                    start_ppq: current_pos,
                    end_ppq: effective_start,
                });
            }
        }

        let actual_duration = chord.end_ppq - chord.start_ppq;

        // A chord is continuing if it started before this section boundary
        let is_continuing = chord.start_ppq < section_start_tick;

        // Continuing chords carry their accent from the original attack
        let is_accented = chord.is_accented();

        // A chord is NOT pushed if it starts and ends within the same beat.
        // Push means the chord crosses a beat/measure boundary.
        let start_beat = chord.start_ppq / ticks_per_beat;
        let end_beat = chord.end_ppq / ticks_per_beat;
        let stays_in_same_beat = start_beat == end_beat;

        // Push/pull detection:
        // - Continuing chords that cross a section boundary ARE pushed.
        //   Use the chord's actual start position to determine push type (triplet vs quarter).
        // - Chords that stay within a single beat are NOT pushed
        // - Otherwise use normal detection
        let _ticks_per_measure = ticks_per_beat * 4; // 4/4
        let (mut is_pushed, mut push_amount) = if is_continuing {
            // This chord started in the previous section and sustains into this one.
            // Detect push type from the chord's actual start position.
            let quarter_pushed = is_quarter_push(chord, ppq);
            if quarter_pushed {
                (true, Some("4".to_string()))
            } else {
                detect_push_pull_for_chord(
                    chord.start_ppq,
                    chord.end_ppq,
                    ppq,
                    songstart,
                    is_triplet_song,
                )
            }
        } else if stays_in_same_beat {
            // Chord starts and ends in the same beat — not pushed
            (false, None)
        } else {
            let quarter_pushed = is_quarter_push(chord, ppq);
            if quarter_pushed {
                (true, Some("4".to_string()))
            } else {
                detect_push_pull_for_chord(
                    chord.start_ppq,
                    chord.end_ppq,
                    ppq,
                    songstart,
                    is_triplet_song,
                )
            }
        };

        // Detect staccato push: pushed chord that only lasts the push duration
        let mut is_staccato_pushed = is_staccato_push(
            chord.start_ppq,
            chord.end_ppq,
            ppq,
            songstart,
            is_pushed,
            &push_amount,
        );

        // For very short pushed hits that effectively end on the downbeat,
        // prefer explicit short-duration notation (rest + hit) instead of push marks.
        let max_short_hit = ticks_per_beat / 2;
        if is_pushed && actual_duration <= max_short_hit && !is_staccato_pushed {
            is_pushed = false;
            push_amount = None;
            is_staccato_pushed = false;
        }

        // General staccato detection for non-pushed chords:
        // A chord is staccato if it's significantly shorter than one beat.
        // This catches verse stabs and intro hits that land on the beat but release quickly.
        // Threshold: chord duration <= half a beat (eighth note or less in 4/4).
        let is_staccato = if is_staccato_pushed {
            true
        } else {
            !is_pushed && !is_continuing && actual_duration <= ticks_per_beat / 2
        };

        // For very short chords (< triplet eighth), quantize the end to grid
        let is_very_short = actual_duration < triplet_eighth;
        let quantized_end = if is_continuing {
            effective_end
        } else if is_very_short {
            // Keep the actual short duration so sixteenth-note rhythms remain intact
            chord.end_ppq
        } else {
            chord.end_ppq
        };

        elements.push(ChordOrRest::Chord {
            symbol: chord.chord.normalized.clone(),
            start_ppq: effective_start,
            end_ppq: quantized_end,
            is_pushed,
            push_amount,
            is_accented,
            is_staccato,
        });

        current_pos = quantized_end;
    }

    // If the section still starts with a rest but we detected a pickup hit landing on the downbeat,
    // convert the leading rest into that short staccato push chord.
    if let Some(c) = pickup_candidate
        && let Some(ChordOrRest::Rest { start_ppq, end_ppq }) = elements.first()
        && *start_ppq == section_start_tick
    {
        let chord_end = c.end_ppq.min(section_start_tick + ticks_per_beat / 2);
        let remaining_rest_start = chord_end;
        let remaining_rest_end = *end_ppq;
        // Replace the leading rest with the pickup chord
        elements.remove(0);
        elements.insert(
            0,
            ChordOrRest::Chord {
                symbol: c.chord.normalized.clone(),
                start_ppq: section_start_tick,
                end_ppq: chord_end,
                is_pushed: true,
                push_amount: Some("8".to_string()),
                is_accented: c.is_accented(),
                is_staccato: true,
            },
        );
        if remaining_rest_end > remaining_rest_start {
            elements.insert(
                1,
                ChordOrRest::Rest {
                    start_ppq: remaining_rest_start,
                    end_ppq: remaining_rest_end,
                },
            );
        }
    }

    if current_pos < section_end_tick {
        let gap = section_end_tick - current_pos;
        if gap >= (ppq / 4) as i64 {
            elements.push(ChordOrRest::Rest {
                start_ppq: current_pos,
                end_ppq: section_end_tick,
            });
        }
    }

    elements
}

/// Merge consecutive identical chords.
fn merge_consecutive_chords(elements: Vec<ChordOrRest>) -> Vec<ChordOrRest> {
    let mut merged: Vec<ChordOrRest> = Vec::new();

    for elem in elements {
        match elem {
            ChordOrRest::Chord {
                symbol,
                start_ppq,
                end_ppq,
                is_pushed,
                push_amount,
                is_accented,
                is_staccato,
            } => {
                let can_merge = if let Some(ChordOrRest::Chord {
                    symbol: prev_symbol,
                    end_ppq: prev_end,
                    is_staccato: prev_staccato,
                    ..
                }) = merged.last()
                {
                    // Don't merge staccato chords - they need to stay separate
                    let gap = start_ppq - prev_end;
                    *prev_symbol == symbol
                        && (0..960).contains(&gap)
                        && !*prev_staccato
                        && !is_staccato
                } else {
                    false
                };

                if can_merge {
                    if let Some(ChordOrRest::Chord {
                        end_ppq: prev_end, ..
                    }) = merged.last_mut()
                    {
                        *prev_end = end_ppq;
                    }
                } else {
                    merged.push(ChordOrRest::Chord {
                        symbol,
                        start_ppq,
                        end_ppq,
                        is_pushed,
                        push_amount,
                        is_accented,
                        is_staccato,
                    });
                }
            }
            ChordOrRest::Rest { start_ppq, end_ppq } => {
                let can_merge = matches!(merged.last(), Some(ChordOrRest::Rest { .. }));
                if can_merge {
                    if let Some(ChordOrRest::Rest {
                        end_ppq: prev_end, ..
                    }) = merged.last_mut()
                    {
                        *prev_end = end_ppq;
                    }
                } else {
                    merged.push(ChordOrRest::Rest { start_ppq, end_ppq });
                }
            }
        }
    }

    merged
}

/// Apply groove pattern push detection (F/C → Cm pattern).
fn apply_groove_pattern_push(elements: Vec<ChordOrRest>) -> Vec<ChordOrRest> {
    let mut result: Vec<ChordOrRest> = Vec::new();

    for (i, elem) in elements.iter().enumerate() {
        match elem {
            ChordOrRest::Chord {
                symbol,
                start_ppq,
                end_ppq,
                is_pushed,
                push_amount,
                is_accented,
                is_staccato,
            } => {
                let is_f_chord = symbol == "F/C" || symbol == "F" || symbol.starts_with("F/");
                let next_is_cm = elements.get(i + 1).is_some_and(|next| {
                    if let ChordOrRest::Chord {
                        symbol: next_sym, ..
                    } = next
                    {
                        next_sym.starts_with("Cm")
                    } else {
                        false
                    }
                });

                let should_push = is_f_chord && next_is_cm && !is_pushed;

                result.push(ChordOrRest::Chord {
                    symbol: symbol.clone(),
                    start_ppq: *start_ppq,
                    end_ppq: *end_ppq,
                    is_pushed: *is_pushed || should_push,
                    push_amount: if should_push && push_amount.is_none() {
                        Some("t".to_string())
                    } else {
                        push_amount.clone()
                    },
                    is_accented: *is_accented,
                    is_staccato: *is_staccato,
                });
            }
            ChordOrRest::Rest { start_ppq, end_ppq } => {
                result.push(ChordOrRest::Rest {
                    start_ppq: *start_ppq,
                    end_ppq: *end_ppq,
                });
            }
        }
    }

    result
}

// ============================================================================
// Measure Building & Formatting
// ============================================================================

#[derive(Debug, Clone)]
enum MeasureContent {
    FullMeasure {
        symbol: String,
        is_pushed: bool,
        push_amount: Option<String>,
        is_accented: bool,
        is_staccato: bool,
    },
    Repeat,
    Silence,
    Mixed(Vec<MeasureElement>),
}

#[derive(Debug, Clone)]
enum MeasureElement {
    Chord {
        symbol: String,
        beats: i32,
        ticks: i64,
        is_pushed: bool,
        push_amount: Option<String>,
        is_accented: bool,
        is_staccato: bool,
    },
    Rest {
        beats: i32,
        ticks: i64,
        start_tick_in_measure: i64,
    },
}

/// Generate slash notation for beats.
/// Each slash represents one beat: `//` = 2 beats, `///` = 3 beats.
fn generate_slashes(beats: i32) -> String {
    if beats <= 1 {
        String::new()
    } else {
        "/".repeat(beats as usize)
    }
}

/// Convert detected chords to measure-aware format.
fn build_measures(
    elements: &[ChordOrRest],
    section_start_tick: i64,
    section_length_measures: i32,
    ppq: u32,
    beats_per_measure: i32,
) -> Vec<MeasureContent> {
    let ticks_per_beat = ppq as i64;
    let ticks_per_measure = ticks_per_beat * beats_per_measure as i64;

    let mut measures: Vec<MeasureContent> = Vec::new();
    let mut last_chord_symbol: Option<String> = None;

    for measure_idx in 0..section_length_measures {
        let measure_start = section_start_tick + (measure_idx as i64 * ticks_per_measure);
        let measure_end = measure_start + ticks_per_measure;
        let prev_measure_start = measure_start - ticks_per_measure;

        let mut measure_elements: Vec<MeasureElement> = Vec::new();
        let mut current_beat = 0i32;
        let mut current_tick = measure_start;

        // Check for chords from previous measure that continue into this one
        if measure_idx > 0 {
            for elem in elements {
                if let ChordOrRest::Chord {
                    symbol,
                    start_ppq,
                    end_ppq,
                    is_pushed,
                    push_amount,
                    is_accented,
                    is_staccato,
                } = elem
                    && *start_ppq >= prev_measure_start
                    && *start_ppq < measure_start
                    && *end_ppq > measure_start
                {
                    let chord_start_in_prev = *start_ppq - prev_measure_start;
                    let start_beat_in_prev = (chord_start_in_prev / ticks_per_beat) as i32;
                    let duration_in_prev_measure = measure_start - *start_ppq;
                    let total_duration = *end_ppq - *start_ppq;
                    let duration_in_this_measure = total_duration - duration_in_prev_measure;

                    let is_quarter_push_chord = start_beat_in_prev == beats_per_measure - 1
                        && duration_in_this_measure > duration_in_prev_measure;

                    let chord_end_clamped = (*end_ppq).min(measure_end);
                    let duration_ticks = chord_end_clamped - measure_start;
                    let duration_beats =
                        ((duration_ticks + ticks_per_beat - 1) / ticks_per_beat) as i32;

                    let display_beats = if is_quarter_push_chord {
                        duration_beats.max(1)
                    } else {
                        duration_beats.max(2)
                    };

                    measure_elements.push(MeasureElement::Chord {
                        symbol: symbol.clone(),
                        beats: display_beats,
                        ticks: duration_ticks,
                        is_pushed: *is_pushed,
                        push_amount: push_amount.clone(),
                        is_accented: *is_accented,
                        is_staccato: *is_staccato,
                    });
                    current_beat = display_beats.min(beats_per_measure);
                    current_tick = chord_end_clamped.min(measure_end);

                    if !is_quarter_push_chord {
                        break;
                    }
                }
            }
        }

        for (elem_idx, elem) in elements.iter().enumerate() {
            match elem {
                ChordOrRest::Chord {
                    symbol,
                    start_ppq,
                    end_ppq,
                    is_pushed,
                    push_amount,
                    is_accented,
                    is_staccato,
                } => {
                    if *end_ppq <= measure_start || *start_ppq >= measure_end {
                        continue;
                    }

                    if *start_ppq < measure_start {
                        continue;
                    }

                    let chord_start_in_measure = (*start_ppq - measure_start).max(0);
                    let start_beat = (chord_start_in_measure / ticks_per_beat) as i32;

                    // Quarter push check: skip chords on last beat with majority in next measure
                    if start_beat == beats_per_measure - 1 {
                        let duration_in_this_measure = measure_end - *start_ppq;
                        let total_duration = *end_ppq - *start_ppq;
                        let duration_in_next_measure = total_duration - duration_in_this_measure;
                        if duration_in_next_measure > duration_in_this_measure {
                            continue;
                        }
                    }

                    // In rhythm charts, a chord sustains until the next chord starts.
                    // Extend chord_end to the next chord's start (within this measure)
                    // so that short note-offs don't create false gaps.
                    // In rhythm charts, extend chord to fill until the next chord change.
                    // Find the next chord that starts in this measure after this one.
                    let this_start = *start_ppq;
                    let next_chord_start_in_measure = elements[elem_idx + 1..]
                        .iter()
                        .filter_map(|e| match e {
                            ChordOrRest::Chord {
                                start_ppq: next_s, ..
                            } => {
                                if *next_s > this_start
                                    && *next_s >= measure_start
                                    && *next_s < measure_end
                                {
                                    Some(*next_s)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        })
                        .next();
                    // Extend to next chord's target beat, or measure end if last chord.
                    // For pushed chords, the target beat is the beat they anticipate
                    // (ceiling of their position), not the beat they start from (floor).
                    let effective_end = if let Some(next_start) = next_chord_start_in_measure {
                        let pos = next_start - measure_start;
                        let target_beat = if pos % ticks_per_beat == 0 {
                            pos // already on a beat
                        } else {
                            (pos / ticks_per_beat + 1) * ticks_per_beat // ceiling to next beat
                        };
                        (target_beat + measure_start).min(measure_end)
                    } else {
                        // Last chord in measure — extend to fill
                        measure_end
                    };

                    let chord_start_clamped = (*start_ppq).max(measure_start);
                    let chord_end_clamped = effective_end;
                    let duration_ticks = chord_end_clamped - chord_start_clamped;

                    let chord_end_in_measure =
                        (chord_end_clamped - measure_start).min(ticks_per_measure);
                    let end_beat =
                        ((chord_end_in_measure + ticks_per_beat - 1) / ticks_per_beat) as i32;
                    // For pushed chords, count display beats from the target beat
                    // (the beat they anticipate into), not their actual start position.
                    let display_start_beat = if *is_pushed {
                        (start_beat + 1).min(end_beat)
                    } else {
                        start_beat
                    };
                    let duration_beats = (end_beat - display_start_beat).max(1);

                    // Add rest if there's a gap before this chord
                    let gap_ticks = chord_start_clamped - current_tick;
                    let min_rest_ticks = ticks_per_beat / 6;
                    if start_beat >= current_beat && gap_ticks >= min_rest_ticks {
                        let gap_beats = ((gap_ticks + ticks_per_beat - 1) / ticks_per_beat) as i32;
                        let start_pos = current_tick - measure_start;
                        measure_elements.push(MeasureElement::Rest {
                            beats: gap_beats.max(1),
                            ticks: gap_ticks,
                            start_tick_in_measure: start_pos,
                        });
                    }

                    measure_elements.push(MeasureElement::Chord {
                        symbol: symbol.clone(),
                        beats: duration_beats,
                        ticks: duration_ticks,
                        is_pushed: *is_pushed,
                        push_amount: push_amount.clone(),
                        is_accented: *is_accented,
                        is_staccato: *is_staccato,
                    });

                    current_beat = display_start_beat + duration_beats;
                    current_tick = chord_end_clamped;
                }
                ChordOrRest::Rest {
                    start_ppq: rest_start,
                    end_ppq: rest_end,
                } => {
                    if *rest_end <= measure_start || *rest_start >= measure_end {
                        continue;
                    }

                    let rest_start_clamped = (*rest_start).max(measure_start);
                    let rest_end_clamped = (*rest_end).min(measure_end);
                    let duration_ticks = rest_end_clamped - rest_start_clamped;

                    if duration_ticks < (ppq / 4) as i64 {
                        continue;
                    }

                    let start_beat_in_measure =
                        ((rest_start_clamped - measure_start) / ticks_per_beat) as i32;
                    let rest_beats =
                        ((duration_ticks + ticks_per_beat - 1) / ticks_per_beat) as i32;

                    if start_beat_in_measure >= current_beat && current_beat < beats_per_measure {
                        let actual_beats = rest_beats.min(beats_per_measure - current_beat);
                        let start_pos = rest_start_clamped - measure_start;
                        measure_elements.push(MeasureElement::Rest {
                            beats: actual_beats,
                            ticks: duration_ticks,
                            start_tick_in_measure: start_pos,
                        });
                        current_beat = start_beat_in_measure + actual_beats;
                        current_tick = rest_end_clamped;
                    }
                }
            }
        }

        // Fill remaining beats: extend last chord to fill the measure,
        // or add trailing rest if the measure has no chords.
        if !measure_elements.is_empty() && current_beat < beats_per_measure {
            let remaining_beats = beats_per_measure - current_beat;
            let last_is_chord =
                matches!(measure_elements.last(), Some(MeasureElement::Chord { .. }));
            if last_is_chord {
                // Extend the last chord to fill the remaining beats
                if let Some(MeasureElement::Chord { beats, ticks, .. }) =
                    measure_elements.last_mut()
                {
                    *beats += remaining_beats;
                    *ticks += measure_end - current_tick;
                }
            } else {
                let remaining_ticks = measure_end - current_tick;
                let start_pos = current_tick - measure_start;
                measure_elements.push(MeasureElement::Rest {
                    beats: remaining_beats,
                    ticks: remaining_ticks,
                    start_tick_in_measure: start_pos,
                });
            }
        }

        // Convert to MeasureContent
        let content = if measure_elements.is_empty() {
            let continuing_chord = elements.iter().find_map(|e| {
                if let ChordOrRest::Chord {
                    symbol,
                    start_ppq,
                    end_ppq,
                    is_pushed,
                    push_amount,
                    is_accented,
                    is_staccato,
                } = e
                {
                    if *start_ppq < measure_start && *end_ppq > measure_start {
                        Some((
                            symbol.clone(),
                            *is_pushed,
                            push_amount.clone(),
                            *is_accented,
                            *is_staccato,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            if let Some((symbol, is_pushed, push_amount, is_accented, is_staccato)) =
                continuing_chord
            {
                if last_chord_symbol.as_ref() == Some(&symbol) {
                    MeasureContent::Repeat
                } else {
                    last_chord_symbol = Some(symbol.clone());
                    MeasureContent::FullMeasure {
                        symbol,
                        is_pushed,
                        push_amount,
                        is_accented,
                        is_staccato,
                    }
                }
            } else {
                MeasureContent::Silence
            }
        } else if measure_elements.len() == 1 {
            match &measure_elements[0] {
                MeasureElement::Chord {
                    symbol,
                    beats,
                    is_pushed,
                    push_amount,
                    is_accented,
                    is_staccato,
                    ticks,
                } if *beats >= beats_per_measure
                    && *ticks >= ticks_per_measure - ticks_per_beat =>
                {
                    if last_chord_symbol.as_ref() == Some(symbol) {
                        MeasureContent::Repeat
                    } else {
                        last_chord_symbol = Some(symbol.clone());
                        MeasureContent::FullMeasure {
                            symbol: symbol.clone(),
                            is_pushed: *is_pushed,
                            push_amount: push_amount.clone(),
                            is_accented: *is_accented,
                            is_staccato: *is_staccato,
                        }
                    }
                }
                MeasureElement::Rest { beats, .. } if *beats >= beats_per_measure => {
                    MeasureContent::Silence
                }
                _ => {
                    if let MeasureElement::Chord { symbol, .. } = &measure_elements[0] {
                        last_chord_symbol = Some(symbol.clone());
                    }
                    MeasureContent::Mixed(measure_elements.clone())
                }
            }
        } else {
            // Check if this is a chord on beat 1 followed only by trailing rest.
            // If so, treat as full-measure for repeat/dot notation purposes.
            // Only apply when: first element is a chord, rest follows, chord fills ≥3 beats.
            let is_chord_then_rest =
                matches!(measure_elements.first(), Some(MeasureElement::Chord { .. }))
                    && measure_elements.len() == 2
                    && matches!(measure_elements.get(1), Some(MeasureElement::Rest { .. }));

            if is_chord_then_rest {
                if let Some(MeasureElement::Chord {
                    symbol,
                    beats,
                    is_pushed,
                    push_amount,
                    is_accented,
                    is_staccato,
                    ..
                }) = measure_elements.first()
                {
                    // Only collapse if chord fills at least 3 out of 4 beats
                    if *beats >= beats_per_measure - 1 {
                        if last_chord_symbol.as_ref() == Some(symbol) {
                            MeasureContent::Repeat
                        } else {
                            last_chord_symbol = Some(symbol.clone());
                            MeasureContent::FullMeasure {
                                symbol: symbol.clone(),
                                is_pushed: *is_pushed,
                                push_amount: push_amount.clone(),
                                is_accented: *is_accented,
                                is_staccato: *is_staccato,
                            }
                        }
                    } else {
                        last_chord_symbol = Some(symbol.clone());
                        MeasureContent::Mixed(measure_elements.clone())
                    }
                } else {
                    MeasureContent::Mixed(measure_elements.clone())
                }
            } else {
                if let Some(MeasureElement::Chord { symbol, .. }) = measure_elements
                    .iter()
                    .rfind(|e| matches!(e, MeasureElement::Chord { .. }))
                {
                    last_chord_symbol = Some(symbol.clone());
                }
                MeasureContent::Mixed(measure_elements.clone())
            }
        };

        measures.push(content);
    }

    measures
}

// ============================================================================
// Duration & Rest Formatting
// ============================================================================

/// Format a duration suffix from ticks (e.g., "_8t" for triplet eighth).
fn format_duration_suffix_from_ticks(ticks: i64, ppq: u32) -> String {
    let ticks_per_beat = ppq as i64;
    let triplet_eighth = ticks_per_beat / 3;
    let triplet_quarter = triplet_eighth * 2;
    let sixteenth = ticks_per_beat / 4;
    let eighth = ticks_per_beat / 2;
    let quarter = ticks_per_beat;
    let tolerance = (ppq / 24) as i64;

    if (ticks - triplet_eighth).abs() < tolerance {
        "_8t".to_string()
    } else if (ticks - triplet_quarter).abs() < tolerance {
        "_4t".to_string()
    } else if (ticks - sixteenth).abs() < tolerance {
        "_16".to_string()
    } else if (ticks - eighth).abs() < tolerance {
        "_8".to_string()
    } else if (ticks - quarter).abs() < tolerance {
        String::new()
    } else if ticks < quarter {
        "_8t".to_string()
    } else {
        String::new()
    }
}

/// Format a rest from ticks (e.g., "r8t" for triplet eighth rest).
fn format_rest_from_ticks(ticks: i64, ppq: u32) -> String {
    let ticks_per_beat = ppq as i64;
    let triplet_eighth = ticks_per_beat / 3;
    let triplet_quarter = triplet_eighth * 2;
    let sixteenth = ticks_per_beat / 4;
    let eighth = ticks_per_beat / 2;
    let quarter = ticks_per_beat;
    let half = ticks_per_beat * 2;
    let whole = ticks_per_beat * 4;
    let tolerance = (ppq / 24) as i64;

    if (ticks - triplet_eighth).abs() < tolerance {
        "r8t".to_string()
    } else if (ticks - triplet_quarter).abs() < tolerance {
        "r4t".to_string()
    } else if (ticks - sixteenth).abs() < tolerance {
        "r16".to_string()
    } else if (ticks - eighth).abs() < tolerance {
        "r8".to_string()
    } else if (ticks - quarter).abs() < tolerance {
        "r4".to_string()
    } else if (ticks - half).abs() < tolerance {
        "r2".to_string()
    } else if ticks >= whole - tolerance {
        "r1".to_string()
    } else if ticks < triplet_eighth {
        String::new()
    } else {
        "r4".to_string()
    }
}

/// Split a rest at beat boundaries for optimal notation.
fn split_rest_at_beat_boundaries(
    duration_ticks: i64,
    start_tick_in_measure: i64,
    ppq: u32,
) -> Vec<i64> {
    let ticks_per_beat = ppq as i64;
    let triplet_eighth = ticks_per_beat / 3;
    let triplet_quarter = triplet_eighth * 2;
    let tolerance = 40i64;

    if duration_ticks < triplet_eighth - tolerance {
        return vec![];
    }

    let mut result = Vec::new();
    let mut remaining = duration_ticks;
    let pos_in_beat = start_tick_in_measure % ticks_per_beat;

    // Fill to next beat boundary with triplet eighths if off-beat
    if pos_in_beat > tolerance {
        let ticks_to_beat = ticks_per_beat - pos_in_beat;

        if ticks_to_beat <= remaining + tolerance {
            let mut fill_remaining = ticks_to_beat.min(remaining);
            while fill_remaining >= triplet_eighth - tolerance {
                let chunk = triplet_eighth.min(fill_remaining);
                result.push(chunk);
                fill_remaining -= chunk;
                remaining -= chunk;
            }
        }
    }

    // Full quarter notes on beat boundaries
    while remaining >= ticks_per_beat - tolerance {
        result.push(ticks_per_beat);
        remaining -= ticks_per_beat;
    }

    // Remaining triplet-based duration
    if (remaining - triplet_quarter).abs() < tolerance {
        result.push(triplet_quarter);
    } else {
        while remaining >= triplet_eighth - tolerance {
            let chunk = triplet_eighth.min(remaining);
            result.push(chunk);
            remaining -= chunk;

            if remaining < triplet_eighth / 2 {
                break;
            }
        }
    }

    if result.is_empty() {
        return vec![duration_ticks];
    }

    result
}

/// Format rests split at beat boundaries.
fn format_rests_split_at_beats(
    duration_ticks: i64,
    start_tick_in_measure: i64,
    ppq: u32,
) -> String {
    let chunks = split_rest_at_beat_boundaries(duration_ticks, start_tick_in_measure, ppq);

    if chunks.is_empty() {
        return String::new();
    }

    chunks
        .iter()
        .map(|&ticks| format_rest_from_ticks(ticks, ppq))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ============================================================================
// Measure Formatting
// ============================================================================

/// Format a pushed/pulled chord with the correct push modifier prefix/suffix.
///
/// Push types:
/// - Triplet (`"t"`): `'C` (short) or `'tC` (explicit)
/// - Straight eighth (`"8"`): `'C`
/// - Sixteenth (`"16"`): `''C`
/// - Quarter (`"4"`): `'C`
///
/// Pull types (same modifiers, trailing):
/// - Triplet: `C'` (short) or `Ct'` (explicit)
/// - Straight eighth: `C'`
/// - Sixteenth: `C''`
///
/// Staccato pushes (pushed chord that only lasts the push duration):
/// - `>'.C` (accent + staccato + push)
fn format_chord_with_push(
    accent: &str,
    symbol: &str,
    is_pushed: bool,
    push_amount: &Option<String>,
    use_short_push: bool,
    is_staccato: bool,
) -> String {
    let is_pull = !is_pushed && push_amount.is_some();

    // Staccato marker goes between accent and push
    let staccato = if is_staccato { "." } else { "" };

    if is_pushed {
        match push_amount.as_deref() {
            Some("16") => format!("{}{}''{}", accent, staccato, symbol),
            Some("t") if !use_short_push => format!("{}{}'t{}", accent, staccato, symbol),
            // Triplet (short), straight eighth, quarter, or unknown — single apostrophe
            _ => format!("{}{}'{}", accent, staccato, symbol),
        }
    } else if is_pull {
        match push_amount.as_deref() {
            Some("16") => format!("{}{}{}''", accent, staccato, symbol),
            Some("t") if !use_short_push => format!("{}{}{}t'", accent, staccato, symbol),
            _ => format!("{}{}{}'", accent, staccato, symbol),
        }
    } else {
        format!("{}{}{}", accent, staccato, symbol)
    }
}

/// Format a single measure as Keyflow notation.
fn format_measure(
    content: &MeasureContent,
    beats_per_measure: i32,
    use_short_push: bool,
    ppq: u32,
) -> String {
    let ticks_per_beat = ppq as i64;

    match content {
        MeasureContent::FullMeasure {
            symbol,
            is_pushed,
            push_amount,
            is_accented,
            is_staccato,
        } => {
            let accent = if *is_accented { ">" } else { "" };
            format_chord_with_push(
                accent,
                symbol,
                *is_pushed,
                push_amount,
                use_short_push,
                *is_staccato,
            )
        }
        MeasureContent::Repeat => ".".to_string(),
        MeasureContent::Silence => "s1".to_string(),
        MeasureContent::Mixed(elements) => {
            let chord_elements: Vec<_> = elements
                .iter()
                .filter_map(|elem| match elem {
                    MeasureElement::Chord {
                        symbol,
                        beats,
                        ticks,
                        is_pushed,
                        push_amount,
                        is_accented,
                        is_staccato,
                    } => Some((
                        symbol,
                        beats,
                        ticks,
                        is_pushed,
                        push_amount,
                        is_accented,
                        is_staccato,
                    )),
                    MeasureElement::Rest { .. } => None,
                })
                .collect();
            let has_only_rests = !elements.is_empty() && chord_elements.is_empty();

            if has_only_rests {
                return "s1".to_string();
            }

            if chord_elements.len() == 1 {
                let (symbol, _beats, ticks, is_pushed, push_amount, is_accented, is_staccato) =
                    chord_elements[0];
                let accent = if *is_accented { ">" } else { "" };
                let chord_base = format_chord_with_push(
                    accent,
                    symbol,
                    *is_pushed,
                    push_amount,
                    use_short_push,
                    *is_staccato,
                );
                let half_beat = ticks_per_beat / 2;
                if *ticks < half_beat {
                    let suffix = format_duration_suffix_from_ticks(*ticks, ppq);
                    return format!("{}{}", chord_base, suffix);
                }
                return chord_base;
            }

            let has_chords = !chord_elements.is_empty();

            let mut parts: Vec<String> = Vec::new();

            for (idx, elem) in elements.iter().enumerate() {
                match elem {
                    MeasureElement::Chord {
                        symbol,
                        beats,
                        ticks,
                        is_pushed,
                        push_amount,
                        is_accented,
                        is_staccato,
                    } => {
                        let accent = if *is_accented { ">" } else { "" };

                        let chord_base = format_chord_with_push(
                            accent,
                            symbol,
                            *is_pushed,
                            push_amount,
                            use_short_push,
                            *is_staccato,
                        );

                        let adjusted_beats = *beats;

                        let _next_is_pushed = elements.get(idx + 1).is_some_and(|next| {
                            matches!(
                                next,
                                MeasureElement::Chord {
                                    is_pushed: true,
                                    ..
                                }
                            )
                        });

                        // (Pushed chords borrow from the preceding chord's last beat;
                        //  the preceding chord's beat count already includes that beat.)

                        let chord_count = elements
                            .iter()
                            .filter(|e| matches!(e, MeasureElement::Chord { .. }))
                            .count();
                        let is_sole_chord = chord_count == 1;

                        let half_beat = ticks_per_beat / 2;
                        let needs_duration_suffix = *ticks < half_beat;

                        let is_last_element = idx == elements.len() - 1;

                        if needs_duration_suffix {
                            let suffix = format_duration_suffix_from_ticks(*ticks, ppq);
                            parts.push(format!("{}{}", chord_base, suffix));
                        } else if is_sole_chord && adjusted_beats >= beats_per_measure {
                            parts.push(chord_base);
                        } else if !is_sole_chord && !is_last_element {
                            let display_beats = adjusted_beats.max(1);
                            let slashes = generate_slashes(display_beats);
                            if slashes.is_empty() {
                                parts.push(format!("{} /", chord_base));
                            } else {
                                parts.push(format!("{} {}", chord_base, slashes));
                            }
                        } else if adjusted_beats <= 1 && !is_sole_chord && is_last_element {
                            // A 1-beat chord at the end of a multi-chord measure needs
                            // an explicit slash to avoid being parsed as a whole measure.
                            // In Keyflow, `Fm/Ab` alone = whole measure, but `Fm/Ab /` = 1 beat.
                            parts.push(format!("{} /", chord_base));
                        } else {
                            let slashes = generate_slashes(adjusted_beats);
                            if slashes.is_empty() {
                                parts.push(chord_base);
                            } else {
                                parts.push(format!("{} {}", chord_base, slashes));
                            }
                        }
                    }
                    MeasureElement::Rest {
                        ticks,
                        start_tick_in_measure,
                        ..
                    } => {
                        if has_chords {
                            continue;
                        }
                        let rest_str =
                            format_rests_split_at_beats(*ticks, *start_tick_in_measure, ppq);
                        if !rest_str.is_empty() {
                            parts.push(rest_str);
                        }
                    }
                }
            }

            if parts.is_empty() {
                "s1".to_string()
            } else {
                parts.join(" ")
            }
        }
    }
}

/// Format rhythm elements as Keyflow notation with measure awareness.
/// All sections use compact formatting: single space between measures, 4 measures per line.
#[allow(clippy::too_many_arguments)]
fn format_rhythm_elements(
    elements: &[ChordOrRest],
    section_start_tick: i64,
    section_length_measures: i32,
    ppq: u32,
    beats_per_measure: i32,
    use_short_push: bool,
    _section_type: &str,
    measures_per_line: usize,
    midline_separator_at: Option<usize>,
) -> String {
    let measures = build_measures(
        elements,
        section_start_tick,
        section_length_measures,
        ppq,
        beats_per_measure,
    );

    format_measures(
        &measures,
        beats_per_measure,
        use_short_push,
        ppq,
        measures_per_line,
        midline_separator_at,
    )
}

/// Format measures with compact notation.
/// Single space between measures, 4 measures per line, repeats shown as `.`.
fn format_measures(
    measures: &[MeasureContent],
    beats_per_measure: i32,
    use_short_push: bool,
    ppq: u32,
    measures_per_line: usize,
    midline_separator_at: Option<usize>,
) -> String {
    let mut result = String::new();

    for (i, content) in measures.iter().enumerate() {
        if i > 0 {
            if i % measures_per_line == 0 {
                result.push('\n');
            } else {
                let _ = midline_separator_at;
                result.push_str(" | ");
            }
        }

        result.push_str(&format_measure(
            content,
            beats_per_measure,
            use_short_push,
            ppq,
        ));
    }

    result
}

#[cfg(test)]
mod melody_grid_tests {
    use super::*;

    fn note(start: u32, dur: u32, pitch: u8) -> MidiNote {
        MidiNote {
            pitch,
            velocity: 100,
            start_tick: start,
            duration_ticks: dur,
            channel: 0,
        }
    }

    #[test]
    fn auto_picks_sixteenth_when_min_ioi_is_a_sixteenth() {
        let ppq = 480;
        // Onsets at 0, 120, 240, 360 (sixteenth-grid spacing).
        let n = [
            note(0, 120, 60),
            note(120, 120, 62),
            note(240, 120, 64),
            note(360, 120, 65),
        ];
        let refs: Vec<&MidiNote> = n.iter().collect();
        let g = resolve_melody_grid(&refs, ppq, MelodyGrid::Auto);
        assert_eq!(g, ResolvedMelodyGrid::Sixteenth);
    }

    #[test]
    fn auto_picks_eighth_for_eighth_only_lines() {
        let ppq = 480;
        let n = [note(0, 240, 60), note(240, 240, 62), note(480, 240, 64)];
        let refs: Vec<&MidiNote> = n.iter().collect();
        let g = resolve_melody_grid(&refs, ppq, MelodyGrid::Auto);
        assert_eq!(g, ResolvedMelodyGrid::Eighth);
    }

    #[test]
    fn auto_picks_triplet_when_min_ioi_is_triplet_eighth() {
        let ppq = 480;
        let triplet_eighth = ppq / 3;
        let n = [
            note(0, triplet_eighth, 60),
            note(triplet_eighth, triplet_eighth, 62),
            note(triplet_eighth * 2, triplet_eighth, 64),
        ];
        let refs: Vec<&MidiNote> = n.iter().collect();
        let g = resolve_melody_grid(&refs, ppq, MelodyGrid::Auto);
        assert_eq!(g, ResolvedMelodyGrid::Triplet);
    }

    #[test]
    fn duration_table_dotted_quarter_from_sixteenth_grid() {
        let ppq = 480;
        let dotted_quarter_ticks = (ppq as i64 * 3) / 2; // 720 ticks = 6 sixteenths
        let s = grid_ticks_to_duration_with_grid(
            dotted_quarter_ticks,
            ppq,
            ResolvedMelodyGrid::Sixteenth,
        );
        assert_eq!(s, ".4");
    }

    #[test]
    fn duration_table_dotted_eighth_from_sixteenth_grid() {
        let ppq = 480;
        let dotted_eighth = (ppq as i64 * 3) / 4; // 360 ticks = 3 sixteenths
        let s = grid_ticks_to_duration_with_grid(dotted_eighth, ppq, ResolvedMelodyGrid::Sixteenth);
        assert_eq!(s, ".8");
    }

    #[test]
    fn duration_table_triplet_eighth() {
        let ppq = 480;
        let triplet_eighth = ppq as i64 / 3;
        let s = grid_ticks_to_duration_with_grid(triplet_eighth, ppq, ResolvedMelodyGrid::Triplet);
        assert_eq!(s, "8t");
    }
}
