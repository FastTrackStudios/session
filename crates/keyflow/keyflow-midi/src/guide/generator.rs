//! Top-level guide event generator — combines click, count-in, and section cue scheduling.

use keyflow_proto::guide::{
    config::{ClickConfig, CountInConfig, GuideConfig},
    event::{ClickType, CountEvent, GuideEvent},
    midi_map,
};
use keyflow_proto::{SectionType, TimeSignature};

use super::click_scheduler::ClickScheduler;
use super::count_in_pattern::CountInPattern;
use super::section_cue_scheduler::SectionCueScheduler;

/// Top-level coordinator that generates all guide events for a count-in region.
pub struct GuideGenerator;

impl GuideGenerator {
    /// Generate all guide events for a count-in region.
    ///
    /// Combines click, count-in, and section cue events into a single sorted list.
    /// The region spans from `count_in_start_quarters` to `section_start_quarters`.
    pub fn generate(
        section_start_quarters: f64,
        count_in_start_quarters: f64,
        time_sig: &TimeSignature,
        _tempo: f64, // Reserved for future use (e.g., velocity scaling by tempo)
        section_type: &SectionType,
        section_number: Option<u32>,
        click_config: &ClickConfig,
        count_in_config: &CountInConfig,
        guide_config: &GuideConfig,
    ) -> Vec<GuideEvent> {
        let beat_unit = 4.0 / time_sig.denominator() as f64;
        let quarters_per_measure = beat_unit * time_sig.numerator() as f64;

        let total_measures = CountInPattern::calculate_measures(
            count_in_start_quarters,
            section_start_quarters,
            quarters_per_measure,
        );

        let mut events: Vec<GuideEvent> = Vec::new();

        // Generate click and count events for each measure
        for measure_idx in 0..total_measures {
            let measure_start = count_in_start_quarters + measure_idx as f64 * quarters_per_measure;
            let measure_end = measure_start + quarters_per_measure;

            // Click events for this measure
            let click_events = ClickScheduler::schedule(
                measure_start,
                measure_end,
                measure_start,
                time_sig,
                click_config,
            );

            for click in click_events {
                // If guide replaces beat 1 and this is beat 1 of the first measure,
                // and we have a section cue, suppress the click
                if guide_config.replace_beat_one
                    && guide_config.enabled
                    && measure_idx == 0
                    && (click.click_type == ClickType::Accent
                        || click.click_type == ClickType::Beat)
                    && (click.position_quarters - measure_start).abs() < 1e-9
                    && midi_map::section_type_midi_note(section_type).is_some()
                {
                    continue; // Section cue replaces this click
                }

                events.push(GuideEvent::Click(click));
            }

            // Count-in events for this measure
            if count_in_config.enabled {
                for beat in 1..=time_sig.numerator() as usize {
                    if let Some(count_number) = CountInPattern::should_count(
                        measure_idx,
                        beat,
                        total_measures,
                        time_sig.numerator(),
                        time_sig.denominator(),
                        count_in_config.offset_by_one,
                        count_in_config.full_count_odd_time,
                    ) {
                        let position = measure_start + (beat - 1) as f64 * beat_unit;
                        if let Some(midi_note) = midi_map::count_midi_note(count_number) {
                            events.push(GuideEvent::Count(CountEvent {
                                count_number,
                                position_quarters: position,
                                midi_note,
                            }));
                        }
                    }
                }
            }
        }

        // Section cue event
        if let Some(cue) = SectionCueScheduler::schedule(
            count_in_start_quarters,
            section_type,
            section_number,
            guide_config,
        ) {
            events.push(GuideEvent::SectionCue(cue));
        }

        // Sort all events by position
        events.sort_by(|a, b| {
            event_position(a)
                .partial_cmp(&event_position(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        events
    }
}

impl GuideGenerator {
    /// Generate all guide events for a complete section: count-in + body.
    ///
    /// - Count-in region (count_in_start → section_start): clicks + counts + section cue
    ///   (delegates to [`Self::generate()`])
    /// - Section body (section_start → section_end): clicks only, except beat 1
    ///   of the first body measure gets the section cue (replacing count "1")
    ///   or count "1" if no cue mapping exists.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_section(
        section_start_quarters: f64,
        section_end_quarters: f64,
        count_in_start_quarters: f64,
        time_sig: &TimeSignature,
        tempo: f64,
        section_type: &SectionType,
        section_number: Option<u32>,
        click_config: &ClickConfig,
        count_in_config: &CountInConfig,
        guide_config: &GuideConfig,
    ) -> Vec<GuideEvent> {
        // 1. Count-in region — reuse existing generate()
        let mut events = Self::generate(
            section_start_quarters,
            count_in_start_quarters,
            time_sig,
            tempo,
            section_type,
            section_number,
            click_config,
            count_in_config,
            guide_config,
        );

        // 2. Section body — continuous clicks from section_start to section_end
        let beat_unit = 4.0 / time_sig.denominator() as f64;
        let quarters_per_measure = beat_unit * time_sig.numerator() as f64;

        if section_end_quarters <= section_start_quarters || quarters_per_measure <= 0.0 {
            return events;
        }

        let total_body_measures = ((section_end_quarters - section_start_quarters)
            / quarters_per_measure)
            .ceil() as usize;

        let has_section_cue = guide_config.enabled
            && guide_config.replace_beat_one
            && midi_map::section_type_midi_note(section_type).is_some();

        for measure_idx in 0..total_body_measures {
            let measure_start = section_start_quarters + measure_idx as f64 * quarters_per_measure;
            // Clamp the last measure to section_end to avoid overshooting
            let measure_end = (measure_start + quarters_per_measure).min(section_end_quarters);

            let click_events = ClickScheduler::schedule(
                measure_start,
                measure_end,
                measure_start,
                time_sig,
                click_config,
            );

            let is_first_measure = measure_idx == 0;

            for click in click_events {
                let is_beat_one = (click.position_quarters - measure_start).abs() < 1e-9
                    && (click.click_type == ClickType::Accent
                        || click.click_type == ClickType::Beat);

                if is_first_measure && is_beat_one && has_section_cue {
                    // Section cue replaces beat 1 on the first body measure
                    continue;
                }

                events.push(GuideEvent::Click(click));
            }

            // First body measure, beat 1: emit section cue or count "1"
            if is_first_measure {
                if has_section_cue {
                    if let Some(cue) = SectionCueScheduler::schedule(
                        section_start_quarters,
                        section_type,
                        section_number,
                        guide_config,
                    ) {
                        events.push(GuideEvent::SectionCue(cue));
                    }
                } else if count_in_config.enabled {
                    // No section cue mapping — emit count "1" as fallback
                    if let Some(midi_note) = midi_map::count_midi_note(1) {
                        events.push(GuideEvent::Count(CountEvent {
                            count_number: 1,
                            position_quarters: section_start_quarters,
                            midi_note,
                        }));
                    }
                }
            }
        }

        // Sort all events by position
        events.sort_by(|a, b| {
            event_position(a)
                .partial_cmp(&event_position(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        events
    }
}

/// Extract the quarter-note position from any guide event.
fn event_position(event: &GuideEvent) -> f64 {
    match event {
        GuideEvent::Click(e) => e.position_quarters,
        GuideEvent::Count(e) => e.position_quarters,
        GuideEvent::SectionCue(e) => e.position_quarters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_configs() -> (ClickConfig, CountInConfig, GuideConfig) {
        (
            ClickConfig::default(),
            CountInConfig::default(),
            GuideConfig::default(),
        )
    }

    fn ts(num: u32, den: u32) -> TimeSignature {
        TimeSignature::new(num, den)
    }

    #[test]
    fn test_generate_1_measure_4_4() {
        let (click, count, guide) = default_configs();
        let events = GuideGenerator::generate(
            4.0, // section starts at beat 4
            0.0, // count-in starts at beat 0
            &ts(4, 4),
            120.0,
            &SectionType::Verse,
            Some(1),
            &click,
            &count,
            &guide,
        );

        // Should have: section cue + clicks (beat 1 replaced by guide) + counts
        let cue_count = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::SectionCue(_)))
            .count();
        let click_count = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::Click(_)))
            .count();
        let count_count = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::Count(_)))
            .count();

        assert_eq!(cue_count, 1, "should have 1 section cue");
        // Beat 1 is replaced by guide, so 3 beat clicks remain
        assert_eq!(
            click_count, 3,
            "should have 3 click events (beat 1 replaced)"
        );
        assert_eq!(count_count, 4, "should have 4 count events");
    }

    #[test]
    fn test_generate_no_guide_keeps_beat_1() {
        let click = ClickConfig::default();
        let count = CountInConfig::default();
        let guide = GuideConfig {
            enabled: false,
            replace_beat_one: true,
        };

        let events = GuideGenerator::generate(
            4.0,
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Verse,
            Some(1),
            &click,
            &count,
            &guide,
        );

        let click_count = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::Click(_)))
            .count();
        // No guide replacement, so all 4 beats present (1 accent + 3 beats)
        assert_eq!(click_count, 4);

        // No section cue
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, GuideEvent::SectionCue(_)))
        );
    }

    #[test]
    fn test_events_are_sorted_by_position() {
        let (click, count, guide) = default_configs();
        let events = GuideGenerator::generate(
            8.0,
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Chorus,
            Some(1),
            &click,
            &count,
            &guide,
        );

        for window in events.windows(2) {
            let pos_a = event_position(&window[0]);
            let pos_b = event_position(&window[1]);
            assert!(
                pos_a <= pos_b + 1e-9,
                "events should be sorted: {} <= {}",
                pos_a,
                pos_b
            );
        }
    }

    // ── generate_section() tests ────────────────────────────────────────────

    #[test]
    fn test_section_body_has_clicks_on_every_beat() {
        let (click, count, guide) = default_configs();
        // 1-measure count-in (qn 0–4), 2-measure body (qn 4–12)
        let events = GuideGenerator::generate_section(
            4.0,  // section_start
            12.0, // section_end (2 measures of 4/4)
            0.0,  // count_in_start
            &ts(4, 4),
            120.0,
            &SectionType::Verse,
            Some(1),
            &click,
            &count,
            &guide,
        );

        // Body clicks: measure 1 (qn 4–8) has 3 beats (beat 1 replaced by cue) + measure 2 (qn 8–12) has 4 beats = 7
        let body_clicks: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::Click(c) if c.position_quarters >= 4.0 - 1e-9))
            .collect();
        assert_eq!(
            body_clicks.len(),
            7,
            "body should have 7 click events (3 + 4)"
        );
    }

    #[test]
    fn test_section_body_beat_1_replaced_by_cue() {
        let (click, count, guide) = default_configs();
        let events = GuideGenerator::generate_section(
            4.0,
            8.0, // 1 body measure
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Chorus,
            Some(1),
            &click,
            &count,
            &guide,
        );

        // There should be a section cue at position 4.0 (section start = body beat 1)
        let body_cues: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::SectionCue(c) if (c.position_quarters - 4.0).abs() < 1e-9))
            .collect();
        assert_eq!(body_cues.len(), 1, "should have section cue at body beat 1");

        // No click at position 4.0 (replaced by cue)
        let click_at_4: Vec<_> = events
            .iter()
            .filter(
                |e| matches!(e, GuideEvent::Click(c) if (c.position_quarters - 4.0).abs() < 1e-9),
            )
            .collect();
        assert!(
            click_at_4.is_empty(),
            "beat 1 click should be replaced by section cue"
        );
    }

    #[test]
    fn test_section_body_no_guide_fallback_count_1() {
        let click = ClickConfig::default();
        let count = CountInConfig::default();
        let guide = GuideConfig {
            enabled: false,
            replace_beat_one: true,
        };

        let events = GuideGenerator::generate_section(
            4.0,
            8.0,
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Verse,
            Some(1),
            &click,
            &count,
            &guide,
        );

        // No section cue at body start (guide disabled)
        let body_cues: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::SectionCue(c) if (c.position_quarters - 4.0).abs() < 1e-9))
            .collect();
        assert!(body_cues.is_empty(), "no section cue when guide disabled");

        // Count "1" at position 4.0 as fallback
        let count_at_4: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::Count(c) if (c.position_quarters - 4.0).abs() < 1e-9 && c.count_number == 1))
            .collect();
        assert_eq!(
            count_at_4.len(),
            1,
            "should have count '1' fallback at body beat 1"
        );
    }

    #[test]
    fn test_section_body_multi_measure_event_count() {
        let click = ClickConfig::default();
        let count = CountInConfig {
            enabled: false, // disable count-in to simplify counting
            ..CountInConfig::default()
        };
        let guide = GuideConfig {
            enabled: false,
            replace_beat_one: false,
        };

        // 4 measures of 4/4 body, no count-in events
        let events = GuideGenerator::generate_section(
            4.0,
            20.0, // 4 measures × 4 qn = 16 qn
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Verse,
            Some(1),
            &click,
            &count,
            &guide,
        );

        // Count-in: 4 clicks (no counts, no guide)
        // Body: 4 measures × 4 beats = 16 clicks, plus 1 count "1" fallback
        // But count is disabled, so no fallback count either
        let body_clicks: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GuideEvent::Click(c) if c.position_quarters >= 4.0 - 1e-9))
            .collect();
        assert_eq!(
            body_clicks.len(),
            16,
            "4 body measures × 4 beats = 16 clicks"
        );
    }

    #[test]
    fn test_section_events_are_sorted() {
        let (click, count, guide) = default_configs();
        let events = GuideGenerator::generate_section(
            4.0,
            12.0,
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Chorus,
            Some(1),
            &click,
            &count,
            &guide,
        );

        for window in events.windows(2) {
            let pos_a = event_position(&window[0]);
            let pos_b = event_position(&window[1]);
            assert!(
                pos_a <= pos_b + 1e-9,
                "events should be sorted: {} <= {}",
                pos_a,
                pos_b
            );
        }
    }

    #[test]
    fn test_section_zero_length_body() {
        let (click, count, guide) = default_configs();
        // section_start == section_end → no body events
        let events_section = GuideGenerator::generate_section(
            4.0,
            4.0, // zero-length body
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Verse,
            Some(1),
            &click,
            &count,
            &guide,
        );
        let events_countin = GuideGenerator::generate(
            4.0,
            0.0,
            &ts(4, 4),
            120.0,
            &SectionType::Verse,
            Some(1),
            &click,
            &count,
            &guide,
        );
        // With zero body, generate_section should produce the same events as generate
        assert_eq!(events_section.len(), events_countin.len());
    }
}
