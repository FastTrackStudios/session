//! Rhythm building utilities for chart layout.
//!
//! This module provides a unified pipeline for converting keyflow rhythm notation
//! to engraver Duration types and building rhythm patterns for measures.
//!
//! ## Pipeline
//!
//! All rhythm building flows through a single entry point:
//! ```text
//! Extract → Fill to Measure → Detect Triplets → Auto-expand
//! ```
//!
//! This consolidates the previous 4+ code paths into one clean API.

use crate::chart::types::{Measure, RhythmElement};
use crate::chord::{ChordRhythm, LilySyntax, PushPullBase};
use crate::engraver::layout::tlayout::Accidental;
use crate::engraver::model::DurationKind;
use crate::engraver::notation::{Duration, RhythmEntry, TupletRatio, TupletSpec};
use crate::time::{MusicalPositionExt, TimeSignatureExt};

use super::PushSpillback;
use super::types::MeasureMelodyData;

// ============================================================================
// Core Types
// ============================================================================

/// Input source for rhythm - unifies all input types.
///
/// This enum allows the rhythm builder to handle different input sources
/// through a single code path, eliminating branching in the caller.
pub enum RhythmSource<'a> {
    /// Explicit chord rhythms (r8t Ab9_8t r8t...)
    /// These have LilyPond-style duration notation attached to each element.
    /// Now includes optional spillbacks for pushed chords from the next measure.
    ExplicitRhythm {
        elements: &'a [RhythmElement],
        spillbacks: Option<&'a [PushSpillback]>,
    },

    /// Preprocessed melody segments from cross-barline expansion.
    /// These come from `expand_melodies_across_measures()`.
    MelodyData(&'a MeasureMelodyData),

    /// Slash/chord notation with optional push/pull timing support.
    /// This is the most common case for chord charts.
    SlashNotation {
        chords: &'a [crate::ChordInstance],
        spillbacks: Option<&'a [PushSpillback]>,
    },
}

/// Configuration for rhythm building.
#[derive(Debug, Clone)]
pub struct RhythmBuildConfig {
    /// Time signature as (numerator, denominator), e.g., (4, 4) for 4/4.
    pub time_signature: (u8, u8),
    /// Whether to use stemmed notation (for charts with push/pull timing).
    pub use_stems: bool,
    /// Whether to expand whole/half notes to quarter slashes for master rhythm charts.
    pub auto_rhythm_slashes: bool,
    /// Whether push/pull notation creates rhythmic subdivisions (triplet groups).
    /// When true, triplet pushes generate quarter+eighth triplet groups.
    /// When false, pushed chords show apostrophe markers instead.
    pub push_alters_rhythm: bool,
}

impl Default for RhythmBuildConfig {
    fn default() -> Self {
        Self {
            time_signature: (4, 4),
            use_stems: false,
            auto_rhythm_slashes: false,
            push_alters_rhythm: true,
        }
    }
}

/// Identifies the type of notehead to use for a rhythm entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteHeadOverride {
    /// Use default notehead for the duration
    Default,
    /// Force slash notehead (for fill slashes after melody)
    Slash,
}

/// Complete output of rhythm building.
///
/// Contains all the information needed to render a measure's rhythm,
/// including entries, tuplet brackets, and position tracking.
#[derive(Debug, Clone, Default)]
pub struct RhythmBuildResult {
    /// The rhythm entries (notes and rests) for this measure.
    pub entries: Vec<RhythmEntry>,
    /// Total duration in ticks.
    pub total_ticks: i32,
    /// Tuplet bracket specifications (for triplet groups, etc.).
    pub tuplet_specs: Vec<TupletSpec>,
    /// Head type overrides for mixed melody/slash notation.
    /// When non-empty, same length as entries - Some(Slash) means force slash head.
    pub head_type_overrides: Vec<Option<NoteHeadOverride>>,
    /// Spillback chord positions: (rhythm_idx, chord_symbol).
    /// These are chords from the next measure that push back into this measure.
    pub spillback_positions: Vec<(usize, String)>,
    /// Internal push positions: (chord_idx, rhythm_idx).
    /// Maps pushed chords to their rhythm entry positions within this measure.
    pub internal_push_positions: Vec<(usize, usize)>,
    /// Per-note pitch information for melody rendering.
    /// Parallel to `entries` — Some((staff_line, accidental)) for the primary
    /// pitched notehead, None for rests/slashes. Use [`note_pitch_stacks`]
    /// for polyphony / octave-doubling cases.
    pub note_pitches: Vec<Option<(i32, Accidental)>>,
    /// Extra pitches stacked on the same stem as the primary `note_pitches`
    /// entry at the same index — empty when no chord-notes / double-stops.
    /// Parallel to `note_pitches`; the engraver loops over both to emit one
    /// notehead per pitch sharing a single stem.
    pub note_pitch_stacks: Vec<Vec<(i32, Accidental)>>,
}

impl RhythmBuildResult {
    /// Returns true if any triplet groups were detected.
    #[must_use]
    pub fn has_triplets(&self) -> bool {
        !self.tuplet_specs.is_empty()
    }

    /// Returns the number of rhythm entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if there are no rhythm entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ============================================================================
// Main Entry Point
// ============================================================================

/// Build rhythm entries for a measure from any source.
///
/// This is the unified entry point for all rhythm building. It handles:
/// - Explicit rhythms (LilyPond notation)
/// - Melody data (cross-barline expanded notes)
/// - Slash notation (chord charts with optional push/pull)
///
/// The pipeline is:
/// 1. Extract base rhythm from source
/// 2. Fill incomplete measures with quarter slashes
/// 3. Detect triplet groups for brackets
/// 4. Optionally expand whole/half notes to quarters
#[must_use]
pub fn build_rhythm(source: RhythmSource<'_>, config: &RhythmBuildConfig) -> RhythmBuildResult {
    // Step 1: Extract base rhythm from source
    let mut result = extract_base_rhythm(source, config);

    // Step 2: Fill incomplete measures with quarter slashes
    fill_to_measure(&mut result, config);

    // Step 3: Detect triplet groups for brackets
    // (ExplicitRhythm and SlashNotation already set tuplet_specs during extraction)
    // This step is mainly for melody data which doesn't have triplets

    // Step 4: Apply auto_rhythm_slashes expansion if enabled
    // This converts whole/half notes to quarters for master rhythm chart style
    if config.auto_rhythm_slashes {
        expand_to_quarters(&mut result);
    }

    result
}

// ============================================================================
// Source-Specific Extractors
// ============================================================================

/// Extract base rhythm from the given source.
fn extract_base_rhythm(source: RhythmSource<'_>, config: &RhythmBuildConfig) -> RhythmBuildResult {
    match source {
        RhythmSource::ExplicitRhythm {
            elements,
            spillbacks,
        } => {
            tracing::debug!(
                "[rhythm-source] Using ExplicitRhythm ({} elements, {} spillbacks)",
                elements.len(),
                spillbacks.map_or(0, |s| s.len())
            );
            extract_from_explicit(elements, spillbacks, config)
        }
        RhythmSource::MelodyData(data) => {
            tracing::debug!("[rhythm-source] Using MelodyData");
            extract_from_melody(data)
        }
        RhythmSource::SlashNotation { chords, spillbacks } => {
            tracing::debug!(
                "[rhythm-source] Using SlashNotation ({} chords, {} spillbacks)",
                chords.len(),
                spillbacks.map_or(0, |s| s.len())
            );
            extract_from_slash(chords, spillbacks, config)
        }
    }
}

/// Extract rhythm from explicit chord rhythms (r8t Ab9_8t r8t...).
///
/// These have LilyPond-style duration notation attached to each element.
/// Now handles spillbacks from pushed chords in the next measure.
fn extract_from_explicit(
    elements: &[RhythmElement],
    spillbacks: Option<&[PushSpillback]>,
    config: &RhythmBuildConfig,
) -> RhythmBuildResult {
    let mut result = RhythmBuildResult::default();
    let mut triplet_group_start: Option<usize> = None;
    let mut triplet_group_ticks: i32 = 0;
    let mut chord_count: usize = 0;

    // Triplets are grouped by beat: 3 triplet eighths = 480 ticks (one quarter note)
    const TRIPLET_BEAT_TICKS: i32 = 480;
    const QUARTER_TICKS: i32 = 480;

    for element in elements.iter() {
        let (duration, is_rest, is_triplet) = extract_rhythm_parts(element);

        // Skip 0-duration elements (like space markers for push)
        if duration.ticks() <= 0 {
            continue;
        }

        // Check if this is a push chord that needs rhythm alteration
        let is_push_chord = matches!(element, RhythmElement::Chord(c)
            if c.push_pull.as_ref().is_some_and(|(is_push, _)| *is_push));
        let is_first_chord = chord_count == 0;

        // Internal push chord: split the previous beat to create the push anticipation
        if config.push_alters_rhythm
            && is_push_chord
            && !is_first_chord
            && let RhythmElement::Chord(chord) = element
            && let Some((_, amount)) = &chord.push_pull
        {
            // We need to carve out space from the previous rhythm entry
            // to place the push anticipation
            if let Some(last_entry) = result.entries.last()
                && last_entry.duration().ticks() >= QUARTER_TICKS
            {
                let last_ticks = last_entry.duration().ticks();
                let was_rest = matches!(last_entry, RhythmEntry::Rest(_));

                // Remove the last entry
                result.entries.pop();

                // If the last entry was longer than a quarter, preserve the extra
                let remaining_ticks = last_ticks - QUARTER_TICKS;
                if remaining_ticks > 0 {
                    let remaining_dur = ticks_to_duration(remaining_ticks);
                    if was_rest {
                        result.entries.push(RhythmEntry::Rest(remaining_dur));
                    } else {
                        result.entries.push(RhythmEntry::Note(remaining_dur));
                    }
                }

                let start_idx = result.entries.len();
                let is_triplet_group = match (amount.base, amount.level) {
                    (PushPullBase::Triplet, 1) => {
                        if was_rest {
                            result
                                .entries
                                .push(RhythmEntry::Rest(Duration::TripletQuarter));
                        } else {
                            result
                                .entries
                                .push(RhythmEntry::Note(Duration::TripletQuarter));
                        }
                        result
                            .entries
                            .push(RhythmEntry::Note(Duration::TripletEighth));
                        true
                    }
                    (PushPullBase::Standard, 1) => {
                        if was_rest {
                            result.entries.push(RhythmEntry::Rest(Duration::Eighth));
                        } else {
                            result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        }
                        result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        false
                    }
                    (PushPullBase::Standard, 2) => {
                        if was_rest {
                            result
                                .entries
                                .push(RhythmEntry::Rest(Duration::DottedEighth));
                        } else {
                            result
                                .entries
                                .push(RhythmEntry::Note(Duration::DottedEighth));
                        }
                        result.entries.push(RhythmEntry::Note(Duration::Sixteenth));
                        false
                    }
                    _ => {
                        if was_rest {
                            result.entries.push(RhythmEntry::Rest(Duration::Eighth));
                        } else {
                            result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        }
                        result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        false
                    }
                };

                // Record internal push position (chord_idx -> segment_idx of push note)
                result
                    .internal_push_positions
                    .push((chord_count, result.entries.len() - 1));

                if is_triplet_group {
                    result
                        .tuplet_specs
                        .push(TupletSpec::triplet(start_idx, result.entries.len()));
                }

                // Now add the remaining duration of the push chord itself
                // (the slash beats that follow the push)
                let push_chord_ticks = duration.ticks();
                if push_chord_ticks > QUARTER_TICKS {
                    // The push chord has more duration beyond the push beat
                    let extra_ticks = push_chord_ticks - QUARTER_TICKS;
                    let extra_dur = ticks_to_duration(extra_ticks);
                    result.entries.push(RhythmEntry::Note(extra_dur));
                }
                // If push chord is exactly 1 beat, the push subdivision already covers it

                chord_count += 1;
                continue;
            }
        }

        // Track triplet groups for bracket rendering
        if is_triplet {
            if triplet_group_start.is_none() {
                triplet_group_start = Some(result.entries.len());
                triplet_group_ticks = 0;
            }
            triplet_group_ticks += duration.ticks();
        } else if let Some(start) = triplet_group_start {
            // Non-triplet closes the group
            result
                .tuplet_specs
                .push(TupletSpec::triplet(start, result.entries.len()));
            triplet_group_start = None;
            triplet_group_ticks = 0;
        }

        // Create RhythmEntry
        // For staccato chords, replace with quarter-note hit + rests filling the remaining duration
        let is_staccato_chord = matches!(element, RhythmElement::Chord(c)
            if c.commands.iter().any(|cmd| matches!(cmd, crate::chart::commands::Command::Staccato)));

        if is_staccato_chord && duration.ticks() > Duration::Quarter.ticks() {
            // Record chord→segment mapping so chord renderer places symbol at the Note
            let note_entry_idx = result.entries.len();
            result
                .internal_push_positions
                .push((chord_count, note_entry_idx));

            // Staccato: quarter note hit + rests for the remainder
            result.entries.push(RhythmEntry::Note(Duration::Quarter));
            let remaining = duration.ticks() - Duration::Quarter.ticks();
            fill_rests(&mut result.entries, remaining);
        } else if is_rest {
            result.entries.push(RhythmEntry::Rest(duration));
        } else {
            result.entries.push(RhythmEntry::Note(duration));
        }

        // Track chord count for push position mapping
        if matches!(element, RhythmElement::Chord(_)) {
            chord_count += 1;
        }

        // Close triplet group if we've hit a beat boundary
        if is_triplet
            && triplet_group_ticks >= TRIPLET_BEAT_TICKS
            && let Some(start) = triplet_group_start
        {
            result
                .tuplet_specs
                .push(TupletSpec::triplet(start, result.entries.len()));
            triplet_group_start = None;
            triplet_group_ticks = 0;
        }
    }

    // Close any pending triplet group
    if let Some(start) = triplet_group_start {
        result
            .tuplet_specs
            .push(TupletSpec::triplet(start, result.entries.len()));
    }

    result.total_ticks = result.entries.iter().map(|e| e.duration().ticks()).sum();

    // Handle triplet spillbacks from the next measure
    // If there are triplet spillbacks and push_alters_rhythm is enabled,
    // we need to modify the end of the measure to accommodate the pushed chord
    if config.push_alters_rhythm
        && let Some(spillbacks) = spillbacks
    {
        tracing::debug!(
            "[explicit-spillback] Processing {} spillbacks in extract_from_explicit",
            spillbacks.len()
        );
        for spillback in spillbacks {
            tracing::debug!(
                "[explicit-spillback] spillback: '{}' base={:?} level={}",
                spillback.chord_symbol,
                spillback.push_base,
                spillback.push_level
            );

            // Check if the measure has room for the spillback
            // We need to carve out a quarter note's worth (480 ticks) from the end
            // and split it into the appropriate subdivision pair
            if let Some(last_entry) = result.entries.last() {
                let last_ticks = last_entry.duration().ticks();
                const QUARTER_TICKS: i32 = 480;

                tracing::debug!(
                    "[explicit-spillback] last_ticks={} QUARTER_TICKS={}",
                    last_ticks,
                    QUARTER_TICKS
                );

                // We can handle any duration >= quarter note
                if last_ticks >= QUARTER_TICKS {
                    tracing::debug!(
                        "[explicit-spillback] Expanding rhythm for spillback '{}'",
                        spillback.chord_symbol
                    );
                    let was_rest = matches!(last_entry, RhythmEntry::Rest(_));
                    let remaining_ticks = last_ticks - QUARTER_TICKS;

                    // Remove the last entry
                    result.entries.pop();

                    // If there's remaining time before the split, add it back as quarters
                    let num_remaining_quarters = remaining_ticks / QUARTER_TICKS;
                    for _ in 0..num_remaining_quarters {
                        if was_rest {
                            result.entries.push(RhythmEntry::Rest(Duration::Quarter));
                        } else {
                            result.entries.push(RhythmEntry::Note(Duration::Quarter));
                        }
                    }

                    let start_idx = result.entries.len();
                    let is_triplet_group = match (spillback.push_base, spillback.push_level) {
                        // Triplet push: [TripletQuarter, TripletEighth]
                        (PushPullBase::Triplet, 1) => {
                            let main_dur = Duration::triplet(DurationKind::Quarter);
                            let push_dur = Duration::triplet(DurationKind::Eighth);
                            if was_rest {
                                result.entries.push(RhythmEntry::Rest(main_dur));
                            } else {
                                result.entries.push(RhythmEntry::Note(main_dur));
                            }
                            result.entries.push(RhythmEntry::Note(push_dur));
                            true
                        }
                        // Straight eighth push: [Eighth, Eighth]
                        (PushPullBase::Standard, 1) => {
                            if was_rest {
                                result.entries.push(RhythmEntry::Rest(Duration::Eighth));
                            } else {
                                result.entries.push(RhythmEntry::Note(Duration::Eighth));
                            }
                            result.entries.push(RhythmEntry::Note(Duration::Eighth));
                            false
                        }
                        // Sixteenth push: [DottedEighth, Sixteenth]
                        (PushPullBase::Standard, 2) => {
                            if was_rest {
                                result
                                    .entries
                                    .push(RhythmEntry::Rest(Duration::DottedEighth));
                            } else {
                                result
                                    .entries
                                    .push(RhythmEntry::Note(Duration::DottedEighth));
                            }
                            result.entries.push(RhythmEntry::Note(Duration::Sixteenth));
                            false
                        }
                        // Fallback: straight eighth split
                        _ => {
                            if was_rest {
                                result.entries.push(RhythmEntry::Rest(Duration::Eighth));
                            } else {
                                result.entries.push(RhythmEntry::Note(Duration::Eighth));
                            }
                            result.entries.push(RhythmEntry::Note(Duration::Eighth));
                            false
                        }
                    };

                    // Add spillback position mapping
                    let spillback_segment_idx = result.entries.len() - 1;
                    result
                        .spillback_positions
                        .push((spillback_segment_idx, spillback.chord_symbol.clone()));

                    // Add triplet spec only for triplet groups
                    if is_triplet_group {
                        result
                            .tuplet_specs
                            .push(TupletSpec::triplet(start_idx, result.entries.len()));
                    }

                    // Update total ticks (should remain the same)
                    result.total_ticks = result.entries.iter().map(|e| e.duration().ticks()).sum();
                }
            }
        }
    }

    result
}

/// Extract rhythm from preprocessed melody data.
///
/// Melody segments come from `expand_melodies_across_measures()` and represent
/// notes that may have been split at barlines.
fn extract_from_melody(data: &MeasureMelodyData) -> RhythmBuildResult {
    use super::types::melody_pitch_to_line_for_clef_and_key;

    let mut result = RhythmBuildResult::default();
    let num_melody_notes = data.segments.len();

    for segment in &data.segments {
        let duration = segment.to_duration();
        if segment.is_rest {
            result.entries.push(RhythmEntry::Rest(duration));
            result.note_pitches.push(None);
            result.note_pitch_stacks.push(Vec::new());
        } else {
            result.entries.push(RhythmEntry::Note(duration));

            // Primary pitch + any polyphony stack (octave doublings /
            // double-stops). All resolved through the active clef.
            if let Some(octave) = segment.octave {
                let (line, acc) = melody_pitch_to_line_for_clef_and_key(
                    &segment.pitch,
                    octave,
                    data.clef,
                    data.key_signature,
                );
                result.note_pitches.push(Some((line, acc)));
                let mut stack: Vec<(i32, Accidental)> =
                    Vec::with_capacity(segment.extra_pitches.len());
                for (idx, (extra_pitch, extra_octave)) in segment.extra_pitches.iter().enumerate() {
                    let oct = extra_octave.unwrap_or_else(|| {
                        if let Some((class, _, _)) = super::types::parse_melody_pitch(extra_pitch) {
                            super::types::resolve_relative_octave(
                                class,
                                super::types::parse_melody_pitch(&segment.pitch)
                                    .map_or(class, |(primary_class, _, _)| primary_class),
                                octave,
                                segment
                                    .extra_pitch_modifiers
                                    .get(idx)
                                    .copied()
                                    .unwrap_or_default(),
                            )
                        } else {
                            octave
                        }
                    });
                    let (eline, eacc) = melody_pitch_to_line_for_clef_and_key(
                        extra_pitch,
                        oct,
                        data.clef,
                        data.key_signature,
                    );
                    stack.push((eline, eacc));
                }
                result.note_pitch_stacks.push(stack);
            } else {
                result.note_pitches.push(None);
                result.note_pitch_stacks.push(Vec::new());
            }
        }
        result.head_type_overrides.push(None);
    }

    result.total_ticks = result.entries.iter().map(|e| e.duration().ticks()).sum();

    // Store the count of melody notes so fill_to_measure can add Slash overrides
    // We use a marker by ensuring head_type_overrides has exactly num_melody_notes entries
    // (fill_to_measure will extend it with Slash entries for fill notes)
    debug_assert_eq!(result.head_type_overrides.len(), num_melody_notes);

    result
}

/// Info about a push that affects a particular beat.
#[derive(Debug, Clone, Copy)]
struct BeatPushInfo {
    /// The push subdivision base (Standard, Triplet, etc.)
    base: PushPullBase,
    /// The push level (1 = eighth, 2 = sixteenth)
    level: u8,
    /// Index of the pushed chord within the measure (None for spillbacks)
    chord_idx: Option<usize>,
}

/// Extract rhythm from slash/chord notation with push/pull support.
///
/// This handles:
/// - Basic slash notation (one quarter per chord)
/// - Triplet pushes (chord anticipates by triplet eighth)
/// - Eighth note pushes (chord anticipates by straight eighth)
/// - Sixteenth note pushes (chord anticipates by sixteenth)
/// - Spillback chords (pushed chords from next measure)
fn extract_from_slash(
    chords: &[crate::ChordInstance],
    spillbacks: Option<&[PushSpillback]>,
    config: &RhythmBuildConfig,
) -> RhythmBuildResult {
    use crate::chart::commands::Command;

    // Check if any chord in the measure has staccato — if so, use staccato rhythm pattern
    let has_staccato = chords.iter().any(|c| {
        c.commands
            .iter()
            .any(|cmd| matches!(cmd, Command::Staccato))
    });

    if has_staccato {
        return extract_staccato_rhythm(chords, config);
    }

    let mut result = RhythmBuildResult::default();

    // Count non-pushed chords to determine number of beats
    // Pushed chords don't consume their own beat position
    let non_pushed_count = chords
        .iter()
        .filter(|c| c.push_pull.as_ref().is_none_or(|(is_push, _)| !is_push))
        .count();
    // If every chord declares a dotted-slash rhythm (e.g. 6/8's "/. /.")
    // the beat unit IS the dotted quarter — the bar is already covered by
    // `sum(count)` dotted slashes, so we must NOT pad up to the meter
    // numerator (which would emit 6 plain quarter slashes for 6/8).
    let all_dotted_slashes = !chords.is_empty()
        && chords
            .iter()
            .all(|c| matches!(c.rhythm, ChordRhythm::Slashes { dotted: true, .. }));
    let num_beats = if all_dotted_slashes {
        chords
            .iter()
            .map(|c| match &c.rhythm {
                ChordRhythm::Slashes { count, .. } => *count as usize,
                _ => 1,
            })
            .sum::<usize>()
            .max(1)
    } else {
        non_pushed_count.max(config.time_signature.0 as usize)
    };

    // Build a list of beats, tracking which ones have pushes
    let mut beats_with_pushes: Vec<Option<BeatPushInfo>> = vec![None; num_beats];

    // Calculate natural beat positions and mark pushes
    let mut cumulative_beats = 0usize;
    for (chord_idx, chord) in chords.iter().enumerate() {
        let is_push = chord
            .push_pull
            .as_ref()
            .is_some_and(|(is_push, _)| *is_push);

        let chord_duration_beats = match &chord.rhythm {
            ChordRhythm::Slashes { count, .. } => *count as usize,
            _ => 1,
        };

        // Push affects the PREVIOUS beat (where the anticipation lands)
        if is_push
            && chord_idx > 0
            && let Some((_, amount)) = &chord.push_pull
        {
            let target_beat = cumulative_beats.saturating_sub(1);
            if target_beat < num_beats && beats_with_pushes[target_beat].is_none() {
                beats_with_pushes[target_beat] = Some(BeatPushInfo {
                    base: amount.base,
                    level: amount.level,
                    chord_idx: Some(chord_idx),
                });
            }
        }

        cumulative_beats += chord_duration_beats;
    }

    // Check spillbacks from next measure
    if let Some(spills) = spillbacks {
        if !spills.is_empty() {
            tracing::debug!(
                "[rhythm-builder] Processing {} spillbacks, num_beats={}, push_alters_rhythm={}",
                spills.len(),
                num_beats,
                config.push_alters_rhythm
            );
        }
        for spillback in spills {
            tracing::debug!(
                "[rhythm-builder] spillback: chord='{}' beat={} base={:?} level={}",
                spillback.chord_symbol,
                spillback.beat_position,
                spillback.push_base,
                spillback.push_level
            );
            let target_beat = spillback.beat_position;
            if target_beat < num_beats && beats_with_pushes[target_beat].is_none() {
                beats_with_pushes[target_beat] = Some(BeatPushInfo {
                    base: spillback.push_base,
                    level: spillback.push_level,
                    chord_idx: None, // spillback, not internal
                });
            }
        }
    }

    // Build the rhythm array
    let mut rhythm_index = 0;
    for (beat_idx, push_info) in beats_with_pushes.iter().enumerate().take(num_beats) {
        if let Some(info) = push_info {
            if config.push_alters_rhythm {
                let start_idx = rhythm_index;
                let is_triplet_group = match (info.base, info.level) {
                    // Triplet push: [TripletQuarter, TripletEighth] = 480 ticks
                    (PushPullBase::Triplet, 1) => {
                        result
                            .entries
                            .push(RhythmEntry::Note(Duration::TripletQuarter));
                        result
                            .entries
                            .push(RhythmEntry::Note(Duration::TripletEighth));
                        true
                    }
                    // Straight eighth push: [Eighth, Eighth] = 480 ticks
                    (PushPullBase::Standard, 1) => {
                        result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        false
                    }
                    // Sixteenth push: [DottedEighth, Sixteenth] = 480 ticks
                    (PushPullBase::Standard, 2) => {
                        result
                            .entries
                            .push(RhythmEntry::Note(Duration::DottedEighth));
                        result.entries.push(RhythmEntry::Note(Duration::Sixteenth));
                        false
                    }
                    // Triplet sixteenth: [TripletQuarter, TripletSixteenth]
                    // For now, fall back to triplet eighth
                    (PushPullBase::Triplet, 2) => {
                        result
                            .entries
                            .push(RhythmEntry::Note(Duration::TripletQuarter));
                        result
                            .entries
                            .push(RhythmEntry::Note(Duration::TripletEighth));
                        true
                    }
                    // Other push types: fall back to straight eighth split
                    _ => {
                        result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        result.entries.push(RhythmEntry::Note(Duration::Eighth));
                        false
                    }
                };

                // Track internal push position
                if let Some(chord_idx) = info.chord_idx {
                    result
                        .internal_push_positions
                        .push((chord_idx, rhythm_index + 1));
                }

                // Track spillback chord position
                if let Some(spills) = spillbacks
                    && let Some(spillback) = spills.iter().find(|s| s.beat_position == beat_idx)
                {
                    result
                        .spillback_positions
                        .push((rhythm_index + 1, spillback.chord_symbol.clone()));
                }

                rhythm_index += 2;

                // Only add tuplet spec for triplet groups
                if is_triplet_group {
                    result
                        .tuplet_specs
                        .push(TupletSpec::triplet(start_idx, rhythm_index));
                }
            } else {
                // push_alters_rhythm is false — just render a standard quarter
                result.entries.push(RhythmEntry::Note(Duration::Quarter));
                rhythm_index += 1;
            }
        } else {
            // Standard quarter note beat (no push)
            // Check for standard (non-triplet) spillbacks
            if let Some(spills) = spillbacks
                && let Some(spillback) = spills
                    .iter()
                    .find(|s| s.beat_position == beat_idx && s.push_base == PushPullBase::Standard)
            {
                result
                    .spillback_positions
                    .push((rhythm_index, spillback.chord_symbol.clone()));
            }

            let dur = if all_dotted_slashes {
                Duration::DottedQuarter
            } else {
                Duration::Quarter
            };
            result.entries.push(RhythmEntry::Note(dur));
            rhythm_index += 1;
        }
    }

    result.total_ticks = result.entries.iter().map(|e| e.duration().ticks()).sum();
    let measure_ticks = calculate_measure_ticks(config.time_signature);
    let tick_delta = result.total_ticks - measure_ticks;
    tracing::debug!(
        target: "engraver_proto::engraver::layout::chart::rhythm",
        time_signature = ?config.time_signature,
        chord_count = chords.len(),
        non_pushed_count,
        all_dotted_slashes,
        num_beats,
        entries = result.entries.len(),
        durations_ticks = ?result.entries.iter().map(|entry| entry.duration().ticks()).collect::<Vec<_>>(),
        total_ticks = result.total_ticks,
        measure_ticks,
        tick_delta,
        overfull = tick_delta > 0,
        chords = ?chords.iter().map(|chord| (&chord.full_symbol, &chord.rhythm, chord.position.beats(), chord.position.subdivisions())).collect::<Vec<_>>(),
        "[rhythm-slash] built slash notation rhythm"
    );
    result
}

/// Extract staccato rhythm for a measure with staccato chords.
///
/// Staccato chords render as a short hit (eighth note) followed by rests filling
/// the remaining duration. For a 4/4 measure with a single staccato chord:
///   eighth note + eighth rest + quarter rest + half rest
///
/// This gives the visual appearance of a short, detached chord hit followed by silence.
fn extract_staccato_rhythm(
    chords: &[crate::ChordInstance],
    config: &RhythmBuildConfig,
) -> RhythmBuildResult {
    use crate::chart::commands::Command;

    let mut result = RhythmBuildResult::default();
    let measure_ticks = calculate_measure_ticks(config.time_signature);

    let mut current_ticks: i32 = 0;

    for (chord_idx, chord) in chords.iter().enumerate() {
        let is_staccato = chord
            .commands
            .iter()
            .any(|cmd| matches!(cmd, Command::Staccato));

        let chord_beats = match &chord.rhythm {
            ChordRhythm::Slashes { count, .. } => *count as i32,
            _ => 1,
        };
        let chord_ticks = chord_beats * 480; // quarter = 480 ticks

        // Record the rhythm entry index where this chord's Note lands
        // so the chord renderer can place the chord symbol at the right segment
        let note_entry_idx = result.entries.len();
        result
            .internal_push_positions
            .push((chord_idx, note_entry_idx));

        if is_staccato {
            // Staccato hit: quarter note (480 ticks)
            result.entries.push(RhythmEntry::Note(Duration::Quarter));
            let remaining = chord_ticks - 480;

            // Fill remaining duration with beat-boundary-aware rests
            fill_rests(&mut result.entries, remaining);
        } else {
            // Non-staccato chord in same measure: standard quarter beats
            for _ in 0..chord_beats {
                result.entries.push(RhythmEntry::Note(Duration::Quarter));
            }
        }

        current_ticks += chord_ticks;
    }

    // If we haven't filled the measure, pad with quarter rests
    let remaining = measure_ticks - current_ticks;
    if remaining > 0 {
        fill_rests(&mut result.entries, remaining);
    }

    result.total_ticks = result.entries.iter().map(|e| e.duration().ticks()).sum();
    result
}

/// Fill a duration with rests that respect beat boundaries.
///
/// Follows MuseScore's approach: split at the strongest beat boundary crossed.
/// Given a starting tick offset within the measure (from prior entries) and a
/// remaining duration to fill, emits rests that never cross a beat boundary.
///
/// For `current_tick=240` (mid-beat-1) with 1680 ticks remaining in 4/4:
///   eighth rest (240)  — completes beat 1
///   quarter rest (480) — beat 2
///   half rest (960)    — beats 3–4
fn fill_rests_at(entries: &mut Vec<RhythmEntry>, current_tick: i32, remaining_ticks: i32) {
    const QUARTER: i32 = 480;

    // Available rest durations from largest to smallest
    let rest_durations: &[(Duration, i32)] = &[
        (Duration::Whole, 1920),
        (Duration::Half, 960),
        (Duration::Quarter, 480),
        (Duration::Eighth, 240),
        (Duration::Sixteenth, 120),
    ];

    let mut pos = current_tick;
    let end = current_tick + remaining_ticks;

    while pos < end {
        let left = end - pos;

        // Distance to the next beat boundary (quarter-note grid)
        let beat_offset = pos % QUARTER;
        let to_next_beat = if beat_offset == 0 {
            QUARTER
        } else {
            QUARTER - beat_offset
        };

        // The maximum we can emit without crossing the next beat boundary.
        // If we're on a beat and the remaining is >= a full beat, we can use
        // multi-beat rests (half, whole) as long as they align to their own grid.
        let max_span = if beat_offset == 0 {
            // On a beat — check how many aligned beats we can span
            // A half rest (2 beats) needs to start on an even beat
            // A whole rest (4 beats) needs to start on beat 0
            let beat_index = pos / QUARTER;
            if left >= 1920 && beat_index % 4 == 0 {
                1920 // whole rest, aligned to bar start
            } else if left >= 960 && beat_index % 2 == 0 {
                960 // half rest, aligned to even beat
            } else {
                to_next_beat.min(left)
            }
        } else {
            to_next_beat.min(left)
        };

        // Find the largest single rest that fits within max_span
        let mut found = false;
        for &(ref dur, ticks) in rest_durations {
            if ticks <= max_span {
                entries.push(RhythmEntry::Rest(dur.clone()));
                pos += ticks;
                found = true;
                break;
            }
        }

        if !found {
            // Shouldn't happen, but safety: advance by smallest unit
            entries.push(RhythmEntry::Rest(Duration::Sixteenth));
            pos += 120;
        }
    }
}

/// Convenience wrapper: fill rests starting from tick 0 (used in extract_staccato_rhythm)
fn fill_rests(entries: &mut Vec<RhythmEntry>, remaining_ticks: i32) {
    // Calculate current tick position from existing entries
    let current_tick: i32 = entries.iter().map(|e| e.duration().ticks()).sum();
    fill_rests_at(entries, current_tick, remaining_ticks);
}

// ============================================================================
// Pipeline Stages
// ============================================================================

/// Fill incomplete measures with quarter slashes to match time signature.
fn fill_to_measure(result: &mut RhythmBuildResult, config: &RhythmBuildConfig) {
    let measure_ticks = calculate_measure_ticks(config.time_signature);
    let remaining = measure_ticks - result.total_ticks;

    if remaining > 0 {
        let quarter_ticks = 480;
        let num_quarters = remaining / quarter_ticks;

        // Track if we have head_type_overrides (from melody extraction)
        let had_overrides = !result.head_type_overrides.is_empty();
        let had_pitches = !result.note_pitches.is_empty();

        for _ in 0..num_quarters {
            result.entries.push(RhythmEntry::Note(Duration::Quarter));

            // If we had head type overrides, add Slash for fill notes
            if had_overrides {
                result
                    .head_type_overrides
                    .push(Some(NoteHeadOverride::Slash));
            }

            // Fill notes have no pitch (they're slash fills)
            if had_pitches {
                result.note_pitches.push(None);
            }
        }

        result.total_ticks = measure_ticks;
    }
}

/// Expand whole/half notes to quarter slashes for master rhythm chart style.
///
/// This converts sustained chord notation (diamonds) to rhythmic slashes.
/// Rests are preserved as-is. Triplet entries (eighths, etc.) are NOT expanded
/// and their tuplet_specs are preserved with remapped indices.
fn expand_to_quarters(result: &mut RhythmBuildResult) {
    let quarter_ticks = 480;
    let mut expanded = Vec::with_capacity(result.entries.len() * 4);
    let mut expanded_overrides = if result.head_type_overrides.is_empty() {
        Vec::new()
    } else {
        Vec::with_capacity(result.entries.len() * 4)
    };

    // Track index mapping: old_index -> new_start_index
    // Used to remap tuplet_specs after expansion
    let mut index_map: Vec<usize> = Vec::with_capacity(result.entries.len());

    for (i, entry) in result.entries.iter().enumerate() {
        // Record the new index for this original entry
        index_map.push(expanded.len());

        match entry {
            RhythmEntry::Note(dur) => {
                let ticks = dur.ticks();

                if ticks >= quarter_ticks * 2 {
                    // Expand to quarters
                    let num_quarters = ticks / quarter_ticks;
                    let original_override = result.head_type_overrides.get(i).copied().flatten();

                    for _ in 0..num_quarters {
                        expanded.push(RhythmEntry::Note(Duration::Quarter));
                        if !result.head_type_overrides.is_empty() {
                            expanded_overrides.push(original_override);
                        }
                    }

                    // Handle remaining ticks (e.g., dotted rhythms)
                    let remaining = ticks % quarter_ticks;
                    if remaining >= Duration::Eighth.ticks() {
                        expanded.push(RhythmEntry::Note(Duration::Eighth));
                        if !result.head_type_overrides.is_empty() {
                            expanded_overrides.push(original_override);
                        }
                    }
                } else {
                    // Quarter notes and shorter stay as-is
                    expanded.push(*entry);
                    if !result.head_type_overrides.is_empty() {
                        expanded_overrides
                            .push(result.head_type_overrides.get(i).copied().flatten());
                    }
                }
            }
            RhythmEntry::Rest(dur) => {
                // Keep rests as-is (don't expand sustained rests)
                expanded.push(RhythmEntry::Rest(*dur));
                if !result.head_type_overrides.is_empty() {
                    expanded_overrides.push(result.head_type_overrides.get(i).copied().flatten());
                }
            }
        }
    }

    result.entries = expanded;
    result.head_type_overrides = expanded_overrides;
    result.total_ticks = result.entries.iter().map(|e| e.duration().ticks()).sum();

    // Remap tuplet_specs indices using the index mapping
    // This preserves triplet brackets for entries that weren't expanded
    for spec in &mut result.tuplet_specs {
        // Remap start_idx
        if spec.start_idx < index_map.len() {
            spec.start_idx = index_map[spec.start_idx];
        }
        // Remap end_idx (exclusive) - use the mapped index of end_idx or expanded.len() if at end
        if spec.end_idx <= index_map.len() {
            spec.end_idx = if spec.end_idx == index_map.len() {
                result.entries.len()
            } else {
                index_map[spec.end_idx]
            };
        }
    }
}

/// Calculate measure duration in ticks based on time signature.
#[must_use]
pub fn calculate_measure_ticks(time_signature: (u8, u8)) -> i32 {
    let beat_ticks = match time_signature.1 {
        2 => 960, // Half note beat
        4 => 480, // Quarter note beat
        8 => 240, // Eighth note beat
        _ => 480, // Default to quarter
    };
    beat_ticks * i32::from(time_signature.0)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert keyflow LilySyntax to engraver Duration.
///
/// This is a standalone function that can be used without a ChartLayoutEngine instance.
/// It applies dotted and triplet modifiers to create the final Duration.
#[must_use]
pub fn lily_syntax_to_duration(lily: LilySyntax, dotted: bool, triplet: bool) -> Duration {
    // Convert LilySyntax to DurationKind via NoteValue
    let note_value = crate::core::NoteValue::from(lily);
    let kind = DurationKind::from(note_value);

    if triplet {
        if dotted {
            // Dotted triplet
            Duration {
                kind,
                dots: 1,
                tuplet: Some(TupletRatio::triplet()),
            }
        } else {
            Duration::triplet(kind)
        }
    } else if dotted {
        Duration::dotted(kind)
    } else {
        Duration::new(kind)
    }
}

/// Extract rhythm parts from a RhythmElement.
///
/// Returns (duration, is_rest, is_triplet)
fn extract_rhythm_parts(element: &RhythmElement) -> (Duration, bool, bool) {
    match element {
        RhythmElement::Chord(chord) => {
            if let Some((lily, dotted, triplet)) = chord.rhythm.lily_parts() {
                let dur = lily_syntax_to_duration(lily, dotted, triplet);
                (dur, false, triplet)
            } else if let ChordRhythm::Slashes { count, dotted, .. } = &chord.rhythm {
                // Slashes in explicit measures: each slash = 1 quarter beat
                // e.g., Dm7 /// = 3 quarter beats = DottedHalf
                let ticks = *count as i32 * 480;
                let dur = if *dotted {
                    // Dotted slashes: add half again (e.g., /. = dotted quarter)
                    ticks_to_duration(ticks + ticks / 2)
                } else {
                    ticks_to_duration(ticks)
                };
                (dur, false, false)
            } else {
                // For Default rhythm, use the chord's actual duration
                // Convert from MusicalDuration to notation Duration
                let dur = duration_from_musical(&chord.duration);
                (dur, false, false)
            }
        }
        RhythmElement::Rest(rest) => {
            if let Some((lily, dotted, triplet)) = rest.rhythm.lily_parts() {
                let dur = lily_syntax_to_duration(lily, dotted, triplet);
                (dur, true, triplet)
            } else {
                // For Default rhythm, use the rest's actual duration
                let dur = duration_from_musical(&rest.duration);
                (dur, true, false)
            }
        }
        RhythmElement::Space(space) => {
            // For spaces, use the actual duration
            // A space with 0 duration is just a marker and shouldn't add to rhythm
            let dur = duration_from_musical(&space.duration);
            // Check if this is a zero-duration space marker
            let is_zero = space.duration.measure == 0
                && space.duration.beat == 0
                && space.duration.subdivision == 0;
            if is_zero {
                // Zero-duration space - use a minimal duration that won't affect layout
                (Duration::new(DurationKind::SixtyFourth), true, false)
            } else {
                (dur, true, false)
            }
        }
    }
}

/// Convert a tick count to the closest notation Duration.
fn ticks_to_duration(ticks: i32) -> Duration {
    match ticks {
        0 => Duration::Quarter,
        t if t >= 1920 => Duration::Whole,
        t if t >= 1440 => Duration::DottedHalf,
        t if t >= 960 => Duration::Half,
        t if t >= 720 => Duration::DottedQuarter,
        t if t >= 480 => Duration::Quarter,
        t if t >= 360 => Duration::DottedEighth,
        t if t >= 240 => Duration::Eighth,
        t if t >= 120 => Duration::Sixteenth,
        _ => Duration::ThirtySecond,
    }
}

/// Convert a MusicalDuration to notation Duration.
///
/// This maps the musical duration (measures.beats.subdivisions) to the closest
/// notation Duration constant. Uses 4/4 time as default (480 ticks per beat).
fn duration_from_musical(md: &crate::time::MusicalDuration) -> Duration {
    use crate::time::TimeSignature;

    // Calculate total ticks: 480 ticks per beat in 4/4
    let time_sig = TimeSignature::common_time();
    let beats = md.to_beats(time_sig);
    let ticks = (beats * 480.0).round() as i32;

    // Map to Duration constants (480 = quarter, 960 = half, 1920 = whole)
    match ticks {
        0 => Duration::Quarter, // Fallback for zero-duration (shouldn't happen normally)
        t if t >= 1920 => Duration::Whole,
        t if t >= 1440 => Duration::DottedHalf,
        t if t >= 960 => Duration::Half,
        t if t >= 720 => Duration::DottedQuarter,
        t if t >= 480 => Duration::Quarter,
        t if t >= 360 => Duration::DottedEighth,
        t if t >= 240 => Duration::Eighth,
        t if t >= 120 => Duration::Sixteenth,
        _ => Duration::ThirtySecond,
    }
}

/// Check if a measure has explicit chord rhythms (Lily or Rest notation).
///
/// When chords have explicit rhythms like `r8t Ab9_8t r8t r4t F9_8t r2`,
/// we should render those rhythms instead of using slash notation.
///
/// Note: `Space` (s1, s2, etc.) does NOT count as explicit rhythm - it means
/// "fill this measure with automatic slashes".
#[must_use]
pub fn measure_has_explicit_chord_rhythm(measure: &Measure) -> bool {
    // Check rhythm_elements first (preferred - contains both chords and rests in order)
    if !measure.rhythm_elements.is_empty() {
        let has_real_rhythm = measure.rhythm_elements.iter().any(|elem| {
            match elem {
                RhythmElement::Chord(chord) => {
                    // Only Explicit durations count as explicit rhythm
                    // Skip pushed first chords (they spill back to previous measure)
                    let is_pushed_first = chord
                        .push_pull
                        .as_ref()
                        .is_some_and(|(is_push, _)| *is_push);
                    !is_pushed_first && chord.rhythm.has_lily_duration()
                }
                RhythmElement::Rest(_) => true, // Rests count as real rhythm
                RhythmElement::Space(_) => false, // Space triggers auto-fill
            }
        });
        if has_real_rhythm {
            return true;
        }
    }

    // Fallback: check chords for explicit rhythms
    measure.chords.iter().enumerate().any(|(idx, chord)| {
        let is_pushed_first = idx == 0
            && chord
                .push_pull
                .as_ref()
                .is_some_and(|(is_push, _)| *is_push);
        !is_pushed_first && (chord.rhythm.has_lily_duration() || chord.rhythm.is_rest())
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lily_syntax_to_duration_basic() {
        let dur = lily_syntax_to_duration(LilySyntax::Quarter, false, false);
        assert_eq!(dur.kind, DurationKind::Quarter);
        assert_eq!(dur.dots, 0);
        assert!(dur.tuplet.is_none());
    }

    #[test]
    fn test_lily_syntax_to_duration_dotted() {
        let dur = lily_syntax_to_duration(LilySyntax::Half, true, false);
        assert_eq!(dur.kind, DurationKind::Half);
        assert_eq!(dur.dots, 1);
        assert!(dur.tuplet.is_none());
    }

    #[test]
    fn test_lily_syntax_to_duration_triplet() {
        let dur = lily_syntax_to_duration(LilySyntax::Eighth, false, true);
        assert_eq!(dur.kind, DurationKind::Eighth);
        assert_eq!(dur.dots, 0);
        assert!(dur.tuplet.is_some());
    }

    #[test]
    fn test_lily_syntax_to_duration_all_note_values() {
        assert_eq!(
            lily_syntax_to_duration(LilySyntax::Whole, false, false).kind,
            DurationKind::Whole
        );
        assert_eq!(
            lily_syntax_to_duration(LilySyntax::Half, false, false).kind,
            DurationKind::Half
        );
        assert_eq!(
            lily_syntax_to_duration(LilySyntax::Quarter, false, false).kind,
            DurationKind::Quarter
        );
        assert_eq!(
            lily_syntax_to_duration(LilySyntax::Eighth, false, false).kind,
            DurationKind::Eighth
        );
        assert_eq!(
            lily_syntax_to_duration(LilySyntax::Sixteenth, false, false).kind,
            DurationKind::Sixteenth
        );
        assert_eq!(
            lily_syntax_to_duration(LilySyntax::ThirtySecond, false, false).kind,
            DurationKind::ThirtySecond
        );
    }

    #[test]
    fn test_calculate_measure_ticks() {
        // 4/4 time = 4 * 480 = 1920 ticks
        assert_eq!(calculate_measure_ticks((4, 4)), 1920);
        // 3/4 time = 3 * 480 = 1440 ticks
        assert_eq!(calculate_measure_ticks((3, 4)), 1440);
        // 6/8 time = 6 * 240 = 1440 ticks
        assert_eq!(calculate_measure_ticks((6, 8)), 1440);
        // 2/2 time = 2 * 960 = 1920 ticks
        assert_eq!(calculate_measure_ticks((2, 2)), 1920);
    }

    #[test]
    fn test_build_rhythm_result_helpers() {
        let result = RhythmBuildResult::default();
        assert!(result.is_empty());
        assert_eq!(result.len(), 0);
        assert!(!result.has_triplets());
    }

    #[test]
    fn test_fill_to_measure_4_4() {
        let config = RhythmBuildConfig {
            time_signature: (4, 4),
            use_stems: false,
            auto_rhythm_slashes: false,
            push_alters_rhythm: true,
        };

        // Start with 2 quarter notes (960 ticks)
        let mut result = RhythmBuildResult {
            entries: vec![
                RhythmEntry::Note(Duration::Quarter),
                RhythmEntry::Note(Duration::Quarter),
            ],
            total_ticks: 960,
            ..Default::default()
        };

        fill_to_measure(&mut result, &config);

        // Should have filled to 4 quarters
        assert_eq!(result.entries.len(), 4);
        assert_eq!(result.total_ticks, 1920);
    }
}
