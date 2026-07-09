//! Post-processing for parsed charts
//!
//! Handles section numbering, push/pull adjustments, and position calculation.

use super::ChartParser;
use crate::chart::types::{ChordInstance, RhythmElement};
use crate::chord::{ChordRhythm, PushPullAmount};
use crate::primitives::RootNotation;
use crate::sections::SectionNumberer;
use crate::time::{
    AbsolutePosition, MusicalDuration, MusicalPosition, MusicalPositionExt, TimeSignature,
    TimeSignatureExt,
};

// region:    --- Post Processing

impl<'a> ChartParser<'a> {
    /// Phase 3: Post-processing
    pub(super) fn post_process(&mut self) {
        // Auto-number sections using batch method for retroactive split letter assignment
        let mut numberer = SectionNumberer::new();
        let mut sections_to_number: Vec<crate::sections::Section> =
            self.sections.iter().map(|cs| cs.section.clone()).collect();

        numberer.number_sections(&mut sections_to_number);

        // Apply the numbering back to the chart sections
        for (i, section) in sections_to_number.iter().enumerate() {
            self.sections[i].section.number = section.number;
            self.sections[i].section.split_letter = section.split_letter;
        }

        // Set ending key
        self.ending_key = self.current_key.clone();

        self.resolve_melody_octaves();

        // Calculate absolute positions for all elements BEFORE applying duration adjustments.
        // This ensures positions are based on original rhythm slots, not adjusted durations.
        // The position calculation handles push/pull by adjusting the sounding position
        // (e.g., a pushed chord sounds earlier than its written position).
        self.calculate_absolute_positions();

        // Apply push/pull timing adjustments to durations AFTER calculating positions.
        // This modifies how long each chord sounds (push: shorten current, lengthen previous)
        // but doesn't affect the calculated sounding positions.
        self.apply_push_pull_adjustments();

        // Generate rhythm slashes for empty beats in each measure
        self.generate_rhythm_slashes();

        // Compute chord-syllable alignments for sections with both chords and lyrics
        self.compute_alignments();

        // Note: Template recall is performed at *parse time*, not in post-processing:
        //   - Section-level recall: `parser/sections.rs` calls
        //     `self.templates.recall_transposed(&section_type, current_key)` when
        //     a section header is followed by empty content (and a prior
        //     same-type section has been stored as a template).
        //   - Melody-variable recall: `$name` tokens are expanded inline by
        //     `parser/chords.rs` via `self.melody_variables.get(name)`.
        // No post-processing pass is required.
    }

    fn resolve_melody_octaves(&mut self) {
        for section in &mut self.sections {
            for measure in section.measures_mut() {
                for melody in &mut measure.melodies {
                    melody.resolve_absolute_octaves();
                }
            }
        }
    }

    /// Compute chord-syllable alignments for sections that have both chord and lyrics tracks
    pub(super) fn compute_alignments(&mut self) {
        use crate::chart::chord_syllable_alignment::ChordSyllableAligner;
        use crate::chart::track::TrackType;

        for section in &mut self.sections {
            // Check if this section has both a chord track and a lyrics track
            let has_chords = section
                .tracks
                .iter()
                .any(|t| t.track_type == TrackType::Chords && !t.measures.is_empty());
            let has_lyrics = section.tracks.iter().any(|t| {
                t.track_type == TrackType::Lyrics
                    && t.lyrics.as_ref().is_some_and(|l| !l.syllables.is_empty())
            });

            if !has_chords || !has_lyrics {
                continue;
            }

            // Collect all chords from the chord track
            let chords: Vec<_> = section
                .measures()
                .iter()
                .flat_map(|m| m.chords.iter())
                .filter(|c| c.full_symbol != "s") // Skip space placeholders
                .cloned()
                .collect();

            // Get lyrics from the lyrics track
            let syllables = section
                .lyrics_track()
                .and_then(|t| t.lyrics.as_ref())
                .map(|l| &l.syllables);

            if let Some(syllables) = syllables {
                if let Ok(alignment) = ChordSyllableAligner::align(&chords, syllables) {
                    section.alignment = Some(alignment);
                }
            }
        }
    }

    /// Apply push/pull timing adjustments
    /// Push: lengthen current chord, shorten previous chord by same amount
    ///       If no previous chord, insert a space (tacet) before
    /// Pull: lengthen current chord, shorten next chord by same amount
    ///       If no next chord, insert a space after
    /// This keeps the total duration constant
    /// Works across measure boundaries
    pub(super) fn apply_push_pull_adjustments(&mut self) {
        use crate::chord::{Chord as ChordStruct, ChordQuality, LilySyntax};

        for section in &mut self.sections {
            // Get time signature from first measure or default (before we borrow mutably)
            let time_sig = if !section.measures().is_empty() {
                TimeSignature::new(
                    section.measures()[0].time_signature.0 as u32,
                    section.measures()[0].time_signature.1 as u32,
                )
            } else {
                TimeSignature::new(4, 4)
            };

            // We need to work with indices to insert new chords
            // First pass: identify chords that need space inserted
            let mut insertions: Vec<(usize, usize, ChordInstance)> = Vec::new(); // (measure_idx, chord_idx, space_chord)

            for (measure_idx, measure) in section.measures().iter().enumerate() {
                for (chord_idx, chord) in measure.chords.iter().enumerate() {
                    if let Some((is_push, amount)) = chord.push_pull {
                        let _adjustment = Self::push_pull_to_duration(amount);

                        // Check if this is the first chord overall
                        let is_first = measure_idx == 0 && chord_idx == 0;

                        if is_push && is_first {
                            // Need to insert space before
                            // Initial space has 0 duration, will be filled by push adjustment
                            let space_duration = MusicalDuration::new(0, 0, 0); // DAW uses i32
                            let note = crate::primitives::MusicalNote::from_string("C").unwrap();
                            let root_notation = RootNotation::from_note_name(note);
                            let space_chord = ChordInstance::new(
                                root_notation.clone(),
                                "s".to_string(), // Space symbol
                                ChordStruct::new(root_notation, ChordQuality::Major),
                                ChordRhythm::space(LilySyntax::Whole, false, false, None),
                                "s".to_string(),
                                space_duration,
                                AbsolutePosition::at_beginning(),
                            );
                            insertions.push((measure_idx, chord_idx, space_chord));
                        }
                    }
                }
            }

            // Apply insertions (in reverse order to maintain indices)
            for (measure_idx, chord_idx, space_chord) in insertions.into_iter().rev() {
                section.measures_mut()[measure_idx]
                    .chords
                    .insert(chord_idx, space_chord);
            }

            // Second pass: flatten all chords and apply duration adjustments
            let mut all_chords: Vec<&mut ChordInstance> = Vec::new();
            for measure in section.measures_mut() {
                for chord in &mut measure.chords {
                    all_chords.push(chord);
                }
            }

            // Apply adjustments across all chords
            let mut i = 0;
            while i < all_chords.len() {
                if let Some((is_push, amount)) = all_chords[i].push_pull {
                    let adjustment = Self::push_pull_to_duration(amount);

                    if is_push && i > 0 {
                        // Push: shorten current chord, lengthen previous chord (or space) by same amount
                        // This makes the current chord play earlier by "stealing" time from itself
                        let prev_duration = all_chords[i - 1].duration.to_beats(time_sig);
                        let curr_duration = all_chords[i].duration.to_beats(time_sig);

                        let new_prev = prev_duration + adjustment; // Previous gets longer (space fills in)
                        let new_curr = (curr_duration - adjustment).max(0.0); // Current gets shorter

                        all_chords[i - 1].duration =
                            MusicalDuration::from_beats(new_prev, time_sig);
                        all_chords[i].duration = MusicalDuration::from_beats(new_curr, time_sig);
                    } else if !is_push && i + 1 < all_chords.len() {
                        // Pull: lengthen current chord, shorten next chord
                        let curr_duration = all_chords[i].duration.to_beats(time_sig);
                        let next_duration = all_chords[i + 1].duration.to_beats(time_sig);

                        let new_curr = curr_duration + adjustment;
                        let new_next = (next_duration - adjustment).max(0.0);

                        all_chords[i].duration = MusicalDuration::from_beats(new_curr, time_sig);
                        all_chords[i + 1].duration =
                            MusicalDuration::from_beats(new_next, time_sig);
                    }
                }
                i += 1;
            }

            // Sync rhythm_elements with updated chord durations
            // We update the chord durations in rhythm_elements while preserving Space and Rest elements
            // Match by chord symbol since space insertions can shift indices
            for measure in section.measures_mut() {
                for element in &mut measure.rhythm_elements {
                    if let RhythmElement::Chord(chord_el) = element {
                        // Find the matching chord by symbol (skip spaces which have symbol "s")
                        if let Some(matching_chord) = measure
                            .chords
                            .iter()
                            .find(|c| c.full_symbol == chord_el.full_symbol)
                        {
                            chord_el.duration = matching_chord.duration;
                        }
                    }
                }
            }
        }
    }

    /// Convert push/pull amount to beat adjustment
    pub(super) fn push_pull_to_duration(amount: PushPullAmount) -> f64 {
        // Use the new to_beats method which handles triplets and tuplets
        amount.to_beats()
    }

    /// Calculate absolute positions for all elements in the chart
    /// This accumulates durations as we traverse sections and measures
    /// IMPORTANT: Iterates rhythm_elements (not just chords) to include rests and spaces
    pub(super) fn calculate_absolute_positions(&mut self) {
        use crate::chart::types::RhythmElement;

        // Start at position 0.0.0
        let mut current_position = MusicalDuration::new(0, 0, 0); // DAW uses i32
        let mut current_time_sig = self.time_signature.unwrap_or(TimeSignature::common_time());

        for (section_idx, section) in self.sections.iter_mut().enumerate() {
            for measure in section.measures_mut() {
                // Update time signature if this measure has one
                if measure.time_signature
                    != (
                        current_time_sig.numerator as u8,
                        current_time_sig.denominator as u8,
                    )
                {
                    current_time_sig = TimeSignature::new(
                        measure.time_signature.0 as u32,
                        measure.time_signature.1 as u32,
                    );
                }

                let beats_per_measure = current_time_sig.numerator as f64;

                // Assign positions to ALL rhythm elements (chords, rests, spaces)
                for element in &mut measure.rhythm_elements {
                    let base_position_beats = current_position.to_beats(current_time_sig);

                    match element {
                        RhythmElement::Chord(chord) => {
                            // Calculate position adjustment for push/pull
                            let position_adjustment =
                                if let Some((is_push, amount)) = chord.push_pull {
                                    let adjustment_beats = Self::push_pull_to_duration(amount);
                                    if is_push {
                                        -adjustment_beats
                                    } else {
                                        adjustment_beats
                                    }
                                } else {
                                    0.0
                                };

                            let adjusted_position_beats = base_position_beats + position_adjustment;
                            let base_measure_num =
                                (base_position_beats / beats_per_measure).floor() as i32;
                            let adjusted_measure_num =
                                (adjusted_position_beats / beats_per_measure).floor() as i32;

                            // Calculate the absolute position, handling positions in previous measure
                            let adjusted_position = if adjusted_measure_num < base_measure_num {
                                let position_in_prev_measure = adjusted_position_beats
                                    - (adjusted_measure_num as f64 * beats_per_measure);
                                let position_in_prev_measure = if position_in_prev_measure < 0.0 {
                                    beats_per_measure + position_in_prev_measure
                                } else {
                                    position_in_prev_measure
                                };
                                let total_beats = (adjusted_measure_num.max(0) as f64
                                    * beats_per_measure)
                                    + position_in_prev_measure.max(0.0);
                                let measure = (total_beats / beats_per_measure).floor() as i32;
                                let beat = (total_beats % beats_per_measure).floor() as i32;
                                let subdivision =
                                    (((total_beats % beats_per_measure) % 1.0) * 1000.0).round()
                                        as i32;
                                MusicalPosition::try_new(measure, beat, subdivision.clamp(0, 999))
                                    .unwrap_or_else(|_| MusicalPosition::start())
                            } else {
                                let measure = (adjusted_position_beats.max(0.0) / beats_per_measure)
                                    .floor() as i32;
                                let beat = (adjusted_position_beats.max(0.0) % beats_per_measure)
                                    .floor() as i32;
                                let subdivision =
                                    (((adjusted_position_beats.max(0.0) % beats_per_measure) % 1.0)
                                        * 1000.0)
                                        .round() as i32;
                                MusicalPosition::try_new(measure, beat, subdivision.clamp(0, 999))
                                    .unwrap_or_else(|_| MusicalPosition::start())
                            };
                            chord.position = AbsolutePosition::new(adjusted_position, section_idx);

                            // Advance position by chord duration
                            let duration_beats = chord.duration.to_beats(current_time_sig);
                            let new_position_beats = base_position_beats + duration_beats;
                            current_position =
                                MusicalDuration::from_beats(new_position_beats, current_time_sig);
                        }
                        RhythmElement::Rest(rest) => {
                            // Rests get their position set (for rendering) but also advance the timeline
                            let measure = (base_position_beats / beats_per_measure).floor() as i32;
                            let beat = (base_position_beats % beats_per_measure).floor() as i32;
                            let subdivision = ((base_position_beats % 1.0) * 1000.0).round() as i32;
                            let musical_pos =
                                MusicalPosition::try_new(measure, beat, subdivision.clamp(0, 999))
                                    .unwrap_or_else(|_| MusicalPosition::start());
                            rest.position = AbsolutePosition::new(musical_pos, section_idx);

                            // Advance position by rest duration
                            let duration_beats = rest.duration.to_beats(current_time_sig);
                            let new_position_beats = base_position_beats + duration_beats;
                            current_position =
                                MusicalDuration::from_beats(new_position_beats, current_time_sig);
                        }
                        RhythmElement::Space(space) => {
                            // Spaces also advance the timeline (invisible duration)
                            let measure = (base_position_beats / beats_per_measure).floor() as i32;
                            let beat = (base_position_beats % beats_per_measure).floor() as i32;
                            let subdivision = ((base_position_beats % 1.0) * 1000.0).round() as i32;
                            let musical_pos =
                                MusicalPosition::try_new(measure, beat, subdivision.clamp(0, 999))
                                    .unwrap_or_else(|_| MusicalPosition::start());
                            space.position = AbsolutePosition::new(musical_pos, section_idx);

                            // Advance position by space duration
                            let duration_beats = space.duration.to_beats(current_time_sig);
                            let new_position_beats = base_position_beats + duration_beats;
                            current_position =
                                MusicalDuration::from_beats(new_position_beats, current_time_sig);
                        }
                    }
                }

                // Sync chords vec from rhythm_elements for backward compatibility
                measure.chords = measure
                    .rhythm_elements
                    .iter()
                    .filter_map(|el| match el {
                        RhythmElement::Chord(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect();
            }
        }

        // Update key changes with their actual positions
        // (They should be at the same position as the chord they precede)
        for _key_change in &mut self.key_changes {
            // Find the measure/chord where this key change occurs
            // For now, we'll keep the positions as they were set during parsing
            // In a more sophisticated implementation, we'd look up the exact position
        }
    }
}

// endregion: --- Post Processing
