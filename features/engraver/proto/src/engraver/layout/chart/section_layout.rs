//! Section layout utilities for chart rendering.
//!
//! This module provides functions for section labeling, theming,
//! and consecutive section lettering.

use crate::sections::SectionType;
use std::collections::HashMap;

use crate::engraver::layout::tlayout::{RehearsalMarkStyle, rehearsal_themes};

/// Get theme for section type.
///
/// Maps section types to visual styles using Tailwind-based colors:
/// - Intro: Orange 400 (warm start)
/// - Verse: Emerald 400 (fresh, natural)
/// - Chorus: Blue 500 (strong, memorable)
/// - Bridge: Violet 400 (contrast, transitional)
/// - Outro: Amber 400 (warm conclusion)
/// - Instrumental: Orange 200 (lighter, related to intro)
/// - Interlude: Yellow 400 (bright pause)
/// - Pre-*/Post-*: Lighter shade (200) of parent section
/// - Hits/Breakdown: Slate 400 (neutral)
/// - Custom (Solo, etc.): Slate 200 with border
#[must_use]
pub fn get_section_theme(section_type: &SectionType) -> RehearsalMarkStyle {
    match section_type {
        // Main sections - distinct colors for each section type
        SectionType::Intro => rehearsal_themes::intro(),
        SectionType::Verse => rehearsal_themes::verse(),
        SectionType::Chorus => rehearsal_themes::chorus(),
        SectionType::Bridge => rehearsal_themes::bridge(),
        SectionType::Outro => rehearsal_themes::outro(),
        SectionType::Instrumental => rehearsal_themes::instrumental(),
        SectionType::Solo => rehearsal_themes::solo(),
        SectionType::Interlude => rehearsal_themes::interlude(),
        SectionType::Vamp => rehearsal_themes::interlude(), // Vamp uses interlude styling (similar transitional role)

        // Pre/Post sections - lighter versions of their parent section
        SectionType::Pre(inner) | SectionType::Post(inner) => {
            match inner.as_ref() {
                SectionType::Verse => rehearsal_themes::pre_verse(),
                SectionType::Chorus => rehearsal_themes::pre_chorus(),
                SectionType::Bridge => rehearsal_themes::pre_bridge(),
                // Default to light for other Pre/Post combinations
                _ => rehearsal_themes::light(),
            }
        }

        // Utility sections - neutral colors
        SectionType::CountIn | SectionType::Opening | SectionType::End => {
            rehearsal_themes::outline()
        }
        SectionType::Hits | SectionType::Breakdown => rehearsal_themes::breakdown(),

        // Custom sections (Solo, etc.) - slate with border
        SectionType::Custom(_) => rehearsal_themes::custom(),
    }
}

/// Compute section letters for consecutive repeats of the same section type.
///
/// When sections of the same type appear consecutively (e.g., Interlude Interlude Interlude),
/// they get lettered A, B, C, etc. If a different section type appears in between,
/// the lettering sequence resets.
///
/// # Examples
///
/// - `VS VS CH VS` becomes `VS 1 A, VS 1 B, CH, VS 2` (letters only for consecutive)
/// - `INT INT INT INT` becomes `INT A, INT B, INT C, INT D`
#[must_use]
pub fn compute_section_letters(sections: &[crate::ChartSection]) -> HashMap<usize, char> {
    let mut letters: HashMap<usize, char> = HashMap::new();

    // Track consecutive runs of the same section type
    // We need to do two passes:
    // 1. Find all consecutive runs
    // 2. Assign letters to runs with 2+ sections

    // Group sections by consecutive runs
    let mut runs: Vec<(String, Vec<usize>)> = Vec::new();

    for (idx, chart_section) in sections.iter().enumerate() {
        let section_type = &chart_section.section.section_type;

        // Skip compact sections (count-in) - they don't get letters
        if section_type.is_compact() {
            continue;
        }

        // Skip sections that should not be rendered (End sections)
        if !section_type.should_render() {
            continue;
        }

        // Skip non-numbered section types (Intro, Outro, Solo, etc.)
        // These show their full name + comment, so letters are redundant
        if !section_type.should_number() {
            continue;
        }

        // Get a key for the section type (ignoring number)
        let type_key = section_type.key();

        // Check if this continues the current run
        if let Some((last_key, indices)) = runs.last_mut()
            && *last_key == type_key
        {
            indices.push(idx);
            continue;
        }

        // Start a new run
        runs.push((type_key, vec![idx]));
    }

    // Assign letters to runs with 2+ sections
    for (_, indices) in runs {
        if indices.len() >= 2 {
            for (i, idx) in indices.iter().enumerate() {
                // A = 0, B = 1, C = 2, etc.
                // Support up to Z (26 letters)
                if i < 26 {
                    let letter = (b'A' + i as u8) as char;
                    letters.insert(*idx, letter);
                }
            }
        }
    }

    letters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_section_theme_intro_outro() {
        // Intro uses Orange 400, Outro uses Amber 400 (distinct warm tones)
        let intro_theme = get_section_theme(&SectionType::Intro);
        let outro_theme = get_section_theme(&SectionType::Outro);

        // They should have different colors (Orange 400 vs Amber 400)
        assert_ne!(intro_theme.background_color, outro_theme.background_color);
    }

    #[test]
    fn test_get_section_theme_verse_chorus_different() {
        // Verse and Chorus should have different themes
        let verse_theme = get_section_theme(&SectionType::Verse);
        let chorus_theme = get_section_theme(&SectionType::Chorus);

        assert_ne!(verse_theme.background_color, chorus_theme.background_color);
    }

    #[test]
    fn test_get_section_theme_pre_post_chorus() {
        // Pre(Chorus) and Post(Chorus) should use Blue 200 (lighter variant of Chorus Blue 500)
        let pre_chorus_theme = get_section_theme(&SectionType::Pre(Box::new(SectionType::Chorus)));
        let post_chorus_theme =
            get_section_theme(&SectionType::Post(Box::new(SectionType::Chorus)));
        let chorus_theme = get_section_theme(&SectionType::Chorus);

        // Pre/Post share the same lighter color
        assert_eq!(
            pre_chorus_theme.background_color,
            post_chorus_theme.background_color
        );
        // But differ from full Chorus (Blue 200 vs Blue 500)
        assert_ne!(
            pre_chorus_theme.background_color,
            chorus_theme.background_color
        );
    }
}
