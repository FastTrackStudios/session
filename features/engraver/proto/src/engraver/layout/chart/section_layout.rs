//! Section layout utilities for chart rendering.
//!
//! This module provides functions for section labeling, theming,
//! and consecutive section lettering.

use crate::sections::SectionType;
use std::collections::HashMap;

use crate::Chart;
use crate::engraver::layout::tlayout::{RehearsalMarkStyle, rehearsal_themes};
use crate::engraver::ui::capsule_label::format_rehearsal_label_with_letter;

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
        SectionType::Refrain => rehearsal_themes::chorus(), // Refrain is a recurring hook — chorus-family styling
        SectionType::Turnaround => rehearsal_themes::interlude(), // Turnaround — transitional link styling

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

/// The full margin label the chart engraver draws for a section, given the
/// consecutive-repeat letter from [`compute_section_letters`].
///
/// This is the single source of truth for a section's label text: both the
/// layout renderer ([`super::ChartLayoutEngine::create_section_label`], via
/// its `label_override`) and [`chart_section_timeline`] call it, so the drawn
/// chart and any timeline/progress-bar consumer can never disagree about what
/// a section is called. It reproduces the exact abbreviation/number/letter
/// assembly the capsule uses (e.g. `"CH 2 A"`, `"VS 2"`, `"PRE-CH"`,
/// `"INTRO"`, `"INST"`, `"REF"`); the section's *comment* is drawn separately
/// below the capsule and is intentionally NOT part of this string.
#[must_use]
pub fn section_label(
    section_type: &SectionType,
    number: Option<u32>,
    letter: Option<char>,
) -> String {
    format_rehearsal_label_with_letter(
        &section_type.full_name(),
        &section_type.abbreviation(),
        number,
        letter,
    )
}

/// A section's engraved label plus its span on the real (count-in-excluded)
/// measure timeline.
///
/// Produced by [`chart_section_timeline`]. The [`label`](Self::label) is the
/// exact text the chart engraver prints for the section (see [`section_label`]),
/// so a progress bar or section navigator shows the same names the chart does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionSpan {
    /// The full label exactly as the engraved chart shows it (e.g. `"CH 2 A"`,
    /// `"VS 2"`, `"PRE-CH"`, `"INTRO"`, `"INST"`, `"REF"`).
    pub label: String,
    /// The section's musical type.
    pub section_type: SectionType,
    /// 0-based measure index where this section starts, measured on the REAL
    /// timeline (count-in excluded — real music starts at measure 0).
    pub start_measure: usize,
    /// Number of measures in the section.
    pub measure_count: usize,
    /// True for the leading count-in section. When set, `start_measure` /
    /// `measure_count` describe the count-in itself (before real measure 0) and
    /// the count-in does not advance the real measure counter. Consumers
    /// usually skip it.
    pub is_count_in: bool,
}

/// Walk a parsed chart's sections in order and produce their labels + measure
/// spans, using the SAME labeling the engraver draws.
///
/// The label for each section is [`section_label`] — the identical
/// abbreviation/number/letter assembly the chart capsule uses, with the
/// consecutive-repeat letters coming from [`compute_section_letters`] — so the
/// timeline never drifts from what the chart shows.
///
/// Measure numbering mirrors the paginated chart layout
/// ([`super::ChartLayoutEngine`]): the leading count-in (a *compact* section)
/// is emitted with `is_count_in = true` and does NOT advance the real measure
/// counter, so the first real section starts at measure 0. `End` sections are
/// likewise emitted but excluded from the real-measure numbering (they don't
/// advance the counter, matching the layout skipping them). Every other
/// section's `start_measure` is the running sum of the preceding real
/// sections' `measure_count`s, where `measure_count = section.measures().len()`.
///
/// Pure and deterministic; performs no I/O.
#[must_use]
pub fn chart_section_timeline(chart: &Chart) -> Vec<SectionSpan> {
    // Consecutive-repeat letters, keyed by the section's index within
    // `chart.sections` — exactly the key the renderer looks them up by.
    let letters = compute_section_letters(&chart.sections);

    let mut spans = Vec::with_capacity(chart.sections.len());
    let mut real_measure: usize = 0;

    for (idx, chart_section) in chart.sections.iter().enumerate() {
        let section = &chart_section.section;
        let section_type = section.section_type.clone();
        let measure_count = chart_section.measures().len();
        let letter = letters.get(&idx).copied();
        let label = section_label(&section_type, section.number, letter);

        // Compact = count-in: describe it before real measure 0, don't advance.
        if section_type.is_compact() {
            spans.push(SectionSpan {
                label,
                section_type,
                start_measure: 0,
                measure_count,
                is_count_in: true,
            });
            continue;
        }

        // End sections are included but excluded from real-measure numbering
        // (the paginated layout skips them without advancing the counter).
        let is_end = matches!(section_type, SectionType::End);
        spans.push(SectionSpan {
            label,
            section_type,
            start_measure: real_measure,
            measure_count,
            is_count_in: false,
        });
        if !is_end {
            real_measure += measure_count;
        }
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CHART: &str = "Praise - Elevation Worship
#A 127bpm 4/4
Count 2
In 4
Refrain 8
VS 8
VS
PRE 2
CH 8
VS
VS
PRE
CH
CH
Interlude \"Breakdown\" 8
BR \"Down\" 8
BR \"Build\"
CH
CH
CH
INST \"Guitar Lead\" 8
Refrain
Refrain
";

    #[test]
    fn test_chart_section_timeline_sample() {
        let chart = keyflow::parse(SAMPLE_CHART).expect("sample chart parses");
        let timeline = chart_section_timeline(&chart);

        // Exact labels + spans the UI will show, in chart order.
        let got: Vec<(String, usize, usize, bool)> = timeline
            .iter()
            .map(|s| {
                (
                    s.label.clone(),
                    s.start_measure,
                    s.measure_count,
                    s.is_count_in,
                )
            })
            .collect();

        // (label, start_measure, measure_count, is_count_in)
        let expected: Vec<(&str, usize, usize, bool)> = vec![
            ("COUNT", 0, 2, true), // leading count-in, flagged + skipped by real numbering
            ("INTRO", 0, 4, false),
            ("REF", 4, 8, false), // Refrain isn't numbered/lettered (should_number == false)
            ("VS 1 A", 12, 8, false),
            ("VS 1 B", 20, 8, false),
            ("PRE-CH", 28, 2, false),
            ("CH 1", 30, 8, false),
            ("VS 2 A", 38, 8, false),
            ("VS 2 B", 46, 8, false),
            ("PRE-CH", 54, 2, false),
            ("CH 2 A", 56, 8, false),
            ("CH 2 B", 64, 8, false),
            ("INT", 72, 8, false),
            ("BR A", 80, 8, false),
            ("BR B", 88, 8, false),
            ("CH 3 A", 96, 8, false),
            ("CH 3 B", 104, 8, false),
            ("CH 3 C", 112, 8, false),
            ("INST", 120, 8, false),
            ("REF", 128, 8, false), // Refrain: no number/letter even when repeated
            ("REF", 136, 8, false),
        ];
        let expected: Vec<(String, usize, usize, bool)> = expected
            .into_iter()
            .map(|(l, s, c, ci)| (l.to_string(), s, c, ci))
            .collect();

        assert_eq!(got, expected, "\ngot: {got:#?}");
    }

    #[test]
    fn test_chart_section_timeline_count_in_flagged() {
        let chart = keyflow::parse(SAMPLE_CHART).expect("sample chart parses");
        let timeline = chart_section_timeline(&chart);

        // Exactly one count-in, and it's the leading section.
        assert!(timeline[0].is_count_in);
        assert_eq!(timeline.iter().filter(|s| s.is_count_in).count(), 1);

        // Real music starts at measure 0 (count-in excluded from numbering).
        let first_real = timeline.iter().find(|s| !s.is_count_in).unwrap();
        assert_eq!(first_real.start_measure, 0);
    }

    #[test]
    fn test_section_label_matches_format() {
        // The shared helper reproduces the capsule assembly for the tricky cases.
        assert_eq!(
            section_label(&SectionType::Chorus, Some(2), Some('A')),
            "CH 2 A"
        );
        assert_eq!(section_label(&SectionType::Verse, Some(2), None), "VS 2");
        assert_eq!(
            section_label(&SectionType::Pre(Box::new(SectionType::Chorus)), None, None),
            "PRE-CH"
        );
        // Intro/Outro/Breakdown/Hits render their full name uppercased.
        assert_eq!(section_label(&SectionType::Intro, None, None), "INTRO");
    }

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
