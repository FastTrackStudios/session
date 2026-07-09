//! Chord layout utilities for chart rendering.
//!
//! This module provides functions for chord symbol positioning,
//! collision detection, and conversion to harmony parameters.

use crate::engraver::layout::segment::SegmentType;
use crate::engraver::layout::tlayout::{BassArrangement, HarmonyParams, HarmonyStyle, parse_chord};
use crate::engraver::notation::MeasureScene;

/// Extract ChordRest segment x-positions from a MeasureScene (in spatiums).
///
/// Returns a vector of x-positions for segments containing chord/rest notation.
/// These positions are used for placing chord symbols above the staff.
#[must_use]
pub fn get_chord_rest_positions(measure_scene: &MeasureScene) -> Vec<f64> {
    measure_scene
        .segments
        .iter()
        .filter(|seg| seg.seg_type.contains(SegmentType::CHORD_REST))
        .map(|seg| seg.x)
        .collect()
}

/// Convert a Keyflow ChordInstance to engraver HarmonyParams.
///
/// Uses the chord's `full_symbol` string and parses it into HarmonyParams,
/// then applies the provided harmony style.
#[must_use]
pub fn chord_to_harmony_params(
    chord: &crate::chart::types::ChordInstance,
    harmony_style: &HarmonyStyle,
) -> HarmonyParams {
    let mut params = parse_chord(&chord.full_symbol);
    // A floating slash-bass keeps its inherited harmony in `full_symbol`
    // (e.g. Bb/D) but should print verbatim (`/D`). Render the override text
    // as a single baseline segment with no quality/extension/bass.
    if let Some(text) = &chord.display_override {
        params.root = text.clone();
        params.root_accidental = String::new();
        params.quality = String::new();
        params.extension = String::new();
        params.alterations = Vec::new();
        params.bass = None;
        params.bass_accidental = String::new();
    }
    params.style = harmony_style.clone();
    if chord.parsed.bass_vertical {
        params.bass_arrangement = BassArrangement::Vertical;
    }
    params
}

/// Check if a chord symbol should be visible (not a space or rest placeholder).
#[must_use]
pub fn is_visible_chord(full_symbol: &str) -> bool {
    !full_symbol.is_empty() && full_symbol != "s" && full_symbol != "r"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_visible_chord_regular() {
        assert!(is_visible_chord("C"));
        assert!(is_visible_chord("Am7"));
        assert!(is_visible_chord("F#dim"));
        assert!(is_visible_chord("Bb/D"));
    }

    #[test]
    fn test_is_visible_chord_hidden() {
        assert!(!is_visible_chord(""));
        assert!(!is_visible_chord("s")); // space
        assert!(!is_visible_chord("r")); // rest
    }
}
