//! Per-measure layout: builds a [`MeasureScene`] from a parsed
//! [`Measure`](crate::chart::types::Measure) + its surrounding
//! [`LayoutContext`]. Includes count-in measure rendering and the
//! rhythm-expansion helpers used by `auto_rhythm_slashes`.

use crate::engraver::layout::tlayout::{ClefType, NoteHeadType};
use crate::engraver::notation::{Duration, MeasureBuilder, MeasureScene, RhythmEntry};
use tracing::debug;

use super::rhythm_builder::{self, NoteHeadOverride, RhythmBuildConfig, RhythmSource};
use super::{ChartLayoutEngine, MeasureLayoutParams};

impl ChartLayoutEngine {
    /// Layout a single measure, returning the full MeasureScene with segment positions.
    ///
    /// If `melody_data` is provided, it contains preprocessed melody segments that account
    /// for spillover across measure boundaries. Otherwise, falls back to the measure's
    /// raw melody data (legacy behavior, doesn't handle cross-measure melodies).
    ///
    /// If `spillbacks` is provided, it contains chords from the next measure that push
    /// back across the barline and need to be rendered in this measure.
    pub(super) fn layout_measure(&self, params: MeasureLayoutParams<'_>) -> MeasureScene {
        let MeasureLayoutParams {
            measure,
            melody_data,
            spillbacks,
            measure_width,
            include_clef,
            include_time_sig,
            time_signature,
            time_sig_color,
            clef,
            ctx,
            id_base,
            is_boundary,
        } = params;
        // Check if this measure has melodies (from preprocessed data or raw measure)
        let has_melodies =
            melody_data.map_or_else(|| !measure.melodies.is_empty(), |data| data.has_content());

        // Calculate measure duration in ticks based on time signature
        // Quarter note = 480 ticks, so beat duration depends on denominator
        let beat_ticks = match time_signature.1 {
            2 => 960, // Half note beat
            4 => 480, // Quarter note beat
            8 => 240, // Eighth note beat
            _ => 480, // Default to quarter
        };
        let _measure_ticks = beat_ticks * time_signature.0 as i32;
        let _beats_per_measure = time_signature.0 as f64;

        // Check if the measure has explicit chord rhythms (Lily, Rest, Space notation)
        // Staccato chords also produce explicit-style rhythm (eighth hit + rests)
        // so they need to preserve the Note/Rest distinction in the entries.
        let has_staccato_chords = measure.chords.iter().any(|c| {
            c.commands
                .iter()
                .any(|cmd| matches!(cmd, crate::chart::commands::Command::Staccato))
        });
        let has_explicit_chord_rhythm =
            rhythm_builder::measure_has_explicit_chord_rhythm(measure) || has_staccato_chords;

        // Check if there are triplet spillbacks that need rhythmic processing
        let has_triplet_spillbacks = spillbacks.is_some_and(|spills| {
            spills
                .iter()
                .any(|s| s.push_base == crate::chord::PushPullBase::Triplet)
        });

        // Check if there are internal triplet pushes (pushed chords within this measure
        // that DON'T spill back to the previous measure). A push at beat 0 spills back,
        // so we only count pushes that occur after beat 0.
        // push_pull is Option<(bool, PushPullAmount)> where .1.base is the timing base
        // NOTE: Skip "s" (space) placeholder chords added by post-processor - they don't
        // represent actual musical content and shouldn't affect beat position calculation.
        let has_internal_triplet_push = {
            let mut cumulative_beats = 0usize;
            let mut found_internal = false;
            for chord in &measure.chords {
                // Skip space placeholders - they're not real chords
                if chord.full_symbol == "s" {
                    continue;
                }
                let is_triplet_push = chord.push_pull.as_ref().is_some_and(|(is_push, amount)| {
                    *is_push && amount.base == crate::chord::PushPullBase::Triplet
                });
                // Only count as internal if we've accumulated some beats (not at beat 0)
                if is_triplet_push && cumulative_beats > 0 {
                    found_internal = true;
                    break;
                }
                // Add this chord's beat count
                let chord_beats = match &chord.rhythm {
                    crate::chord::ChordRhythm::Slashes { count, .. } => *count as usize,
                    _ => 1,
                };
                cumulative_beats += chord_beats;
            }
            found_internal
        };

        // Determine the rhythm source
        // PRIORITY ORDER:
        // 1. ExplicitRhythm - when measure has explicit notation (r8t, _8t, etc.), always use it
        //    even if there are triplet pushes (the explicit notation already encodes the rhythm)
        // 2. SlashNotation with triplet rhythm - when triplet pushes need to alter the rhythm
        // 3. MelodyData - when melody dictates the rhythm
        // 4. SlashNotation (default) - simple slash notation
        let needs_triplet_rhythm =
            (has_triplet_spillbacks || has_internal_triplet_push) && self.config.push_alters_rhythm;

        debug!(
            "[rhythm-source-select] has_spillbacks={} has_internal={} needs_triplet={} has_explicit={}",
            has_triplet_spillbacks,
            has_internal_triplet_push,
            needs_triplet_rhythm,
            has_explicit_chord_rhythm
        );

        let source = if has_explicit_chord_rhythm {
            // Explicit rhythm takes priority - it already encodes the full rhythm including rests
            RhythmSource::ExplicitRhythm {
                elements: &measure.rhythm_elements,
                spillbacks,
            }
        } else if needs_triplet_rhythm {
            // Use SlashNotation with triplet generation for measures without explicit rhythm
            RhythmSource::SlashNotation {
                chords: &measure.chords,
                spillbacks,
            }
        } else if let Some(data) = melody_data {
            RhythmSource::MelodyData(data)
        } else {
            RhythmSource::SlashNotation {
                chords: &measure.chords,
                spillbacks,
            }
        };

        let config = RhythmBuildConfig {
            time_signature,
            use_stems: self.config.use_stems,
            auto_rhythm_slashes: false, // Applied separately below for finer control
            push_alters_rhythm: self.config.push_alters_rhythm,
        };

        // Build rhythm using the unified pipeline
        let rhythm_result = rhythm_builder::build_rhythm(source, &config);

        // Convert head type overrides from NoteHeadOverride to NoteHeadType
        let head_type_overrides: Vec<Option<NoteHeadType>> = rhythm_result
            .head_type_overrides
            .iter()
            .map(|opt| {
                opt.map(|o| match o {
                    NoteHeadOverride::Default => NoteHeadType::Normal,
                    NoteHeadOverride::Slash => NoteHeadType::Slash,
                })
            })
            .collect();

        // Capture spillback positions and note pitches from rhythm result
        let spillback_positions = rhythm_result.spillback_positions.clone();
        let note_pitches = rhythm_result.note_pitches.clone();
        let note_pitch_stacks = rhythm_result.note_pitch_stacks.clone();

        // Convert RhythmBuildResult to the format expected by MeasureBuilder
        let (rhythm_entries, full_rhythm, _rhythm_ticks, tuplet_specs, internal_push_positions) =
            if has_explicit_chord_rhythm || melody_data.is_some() {
                // Use entries directly for explicit rhythms and melody data
                (
                    Some(rhythm_result.entries),
                    Vec::<Duration>::new(),
                    rhythm_result.total_ticks,
                    rhythm_result.tuplet_specs,
                    rhythm_result.internal_push_positions,
                )
            } else {
                // Convert entries to Duration vec for slash notation
                let rhythm: Vec<Duration> = rhythm_result
                    .entries
                    .iter()
                    .map(RhythmEntry::duration)
                    .collect();
                (
                    None,
                    rhythm,
                    rhythm_result.total_ticks,
                    rhythm_result.tuplet_specs,
                    rhythm_result.internal_push_positions,
                )
            };

        // Convert measure width from points to spatiums for justification
        let width_spatiums = measure_width / self.config.spatium;

        // Calculate number of rhythm segments (for chord min width calculation)
        let num_segments = if let Some(ref entries) = rhythm_entries {
            entries.len()
        } else {
            full_rhythm.len()
        };

        // Compute minimum segment widths based on actual chord symbol layout bounds
        // This ensures segments are wide enough to prevent chord symbol collisions
        let chord_min_widths =
            self.compute_chord_min_widths(measure, num_segments, measure_width, ctx, is_boundary);

        // Apply auto_rhythm_slashes: expand whole/half notes to quarter slashes
        // This makes master rhythm charts easier to read by showing consistent quarter slashes
        // instead of whole note diamonds for sustained chords
        // Track whether we did auto-expansion (those slashes should be stemless)
        let auto_expanded =
            self.config.auto_rhythm_slashes && !has_melodies && !has_explicit_chord_rhythm;
        let full_rhythm = if auto_expanded {
            self.expand_rhythm_to_quarters(full_rhythm)
        } else {
            full_rhythm
        };

        // Build the measure based on whether we have explicit rhythm entries or not
        let mut builder = MeasureBuilder::new()
            .id_base(id_base)
            .justify_to(width_spatiums)
            .no_barlines() // Barlines handled by chart_layout
            .segment_min_widths(chord_min_widths);

        // Set the rhythm using either entries (for explicit rhythms) or rhythm (for slash fills)
        if let Some(entries) = rhythm_entries {
            // Expand half/whole note entries to quarter slashes when auto_rhythm_slashes is enabled.
            // This converts diamond noteheads to slash noteheads for master rhythm chart style.
            // Rests are preserved as-is (expand_entries_to_quarters keeps them unchanged).
            let entries = if self.config.auto_rhythm_slashes && !has_melodies {
                self.expand_entries_to_quarters(entries)
            } else {
                entries
            };
            builder = builder.entries(entries);
        } else {
            builder = builder.rhythm(full_rhythm);
        }

        // Apply head type overrides for mixed melody/slash notation
        if !head_type_overrides.is_empty() {
            builder = builder.head_type_overrides(head_type_overrides);
        }

        // Apply per-note pitch information for melody rendering
        if !note_pitches.is_empty() {
            builder = builder.note_pitches(note_pitches);
        }
        if note_pitch_stacks.iter().any(|s| !s.is_empty()) {
            builder = builder.note_pitch_stacks(note_pitch_stacks);
        }

        // Use rhythmic (slash) notation when:
        // - We have explicit chord rhythms (these should render as slashes with stems)
        // - We don't have melodies (pure slash notation)
        if has_explicit_chord_rhythm || !has_melodies {
            builder = builder.rhythmic();

            // Stemless behavior:
            // 1. When use_stems is false and no triplets - all stemless via builder.stemless()
            // 2. When auto_rhythm_slashes is true - use auto_stemless() which makes
            //    plain quarters stemless but keeps tuplet notes with stems
            // 3. Otherwise - use auto_stemless() for per-note computation
            let has_triplets = !tuplet_specs.is_empty();

            if !self.config.use_stems && !has_explicit_chord_rhythm && !has_triplets {
                // Pure stemless mode - no triplets, just make everything stemless
                builder = builder.stemless();
            } else if self.config.auto_rhythm_slashes && !has_melodies {
                // Auto-rhythm slashes mode - use per-note auto_stemless()
                // This makes plain quarters stemless but keeps triplet notes with stems
                builder = builder.auto_stemless();
            }
            // Otherwise, compute_auto_stemless() runs automatically in build()
        }

        if include_clef {
            builder = builder.clef(ClefType::Treble);
        }

        if include_time_sig {
            builder = builder.time_signature(time_signature.0, time_signature.1);
            if let Some(color) = time_sig_color {
                builder = builder.time_signature_color(color);
            }
        } else {
            builder = builder.time_signature_meta(crate::engraver::notation::TimeSignature::new(
                time_signature.0,
                time_signature.1,
            ));
        }

        // Apply tuplet specifications for triplet groups
        for spec in &tuplet_specs {
            builder = builder.tuplet_group(spec.start_idx, spec.end_idx, spec.ratio);
        }

        let mut result = builder.build(ctx);

        // Set internal push positions for chord rendering
        result.internal_push_positions = internal_push_positions;
        // Set spillback positions for placing spillback chords at correct triplet positions
        result.spillback_positions = spillback_positions;

        // Add ties for cross-barline notes (configurable — lead-sheet styles
        // sometimes prefer the second piece to render as a fresh attack).
        if self.config.draw_melody_barline_ties
            && let Some(data) = melody_data
        {
            self.add_melody_ties(&mut result, data, ctx, clef);
        }

        // Rhythm-slash ties (chord-side, independent of melody ties).
        self.add_slash_ties(&mut result, measure, ctx);

        result
    }

    /// Expand whole/half note durations to quarter notes for auto_rhythm_slashes.
    ///
    /// This converts sustained chord notation (diamonds) to rhythmic slashes,
    /// which is standard for master rhythm charts.
    fn expand_rhythm_to_quarters(&self, rhythm: Vec<Duration>) -> Vec<Duration> {
        let mut expanded = Vec::with_capacity(rhythm.len() * 4);
        for dur in rhythm {
            let ticks = dur.ticks();
            let quarter_ticks = Duration::Quarter.ticks(); // 480

            if ticks >= quarter_ticks * 2 {
                // Whole note (1920 ticks) -> 4 quarters
                // Half note (960 ticks) -> 2 quarters
                // Dotted half (1440 ticks) -> 3 quarters
                let num_quarters = ticks / quarter_ticks;
                for _ in 0..num_quarters {
                    expanded.push(Duration::Quarter);
                }
                // Handle remaining ticks (e.g., dotted rhythms may have fractional beats)
                let remaining = ticks % quarter_ticks;
                if remaining > 0 {
                    // For now, just add an eighth if there's a half-beat remainder
                    if remaining >= Duration::Eighth.ticks() {
                        expanded.push(Duration::Eighth);
                    }
                }
            } else {
                // Quarter notes and shorter stay as-is
                expanded.push(dur);
            }
        }
        expanded
    }

    /// Expand rhythm entries (notes/rests) to quarters for auto_rhythm_slashes.
    fn expand_entries_to_quarters(&self, entries: Vec<RhythmEntry>) -> Vec<RhythmEntry> {
        let mut expanded = Vec::with_capacity(entries.len() * 4);
        for entry in entries {
            match entry {
                RhythmEntry::Note(dur) => {
                    let ticks = dur.ticks();
                    let quarter_ticks = Duration::Quarter.ticks();

                    if ticks >= quarter_ticks * 2 {
                        let num_quarters = ticks / quarter_ticks;
                        for _ in 0..num_quarters {
                            expanded.push(RhythmEntry::Note(Duration::Quarter));
                        }
                        let remaining = ticks % quarter_ticks;
                        if remaining >= Duration::Eighth.ticks() {
                            expanded.push(RhythmEntry::Note(Duration::Eighth));
                        }
                    } else {
                        expanded.push(RhythmEntry::Note(dur));
                    }
                }
                RhythmEntry::Rest(dur) => {
                    // Keep rests as-is (don't expand sustained rests to quarter slashes)
                    expanded.push(RhythmEntry::Rest(dur));
                }
            }
        }
        expanded
    }
}
