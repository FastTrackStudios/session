//! Section cue scheduling — generates section announcement events at count-in start.

use keyflow_proto::SectionType;
use keyflow_proto::guide::{config::GuideConfig, event::SectionCueEvent, midi_map};

/// Schedules section cue events during count-in regions.
pub struct SectionCueScheduler;

impl SectionCueScheduler {
    /// Generate a section cue event at the start of the count-in region.
    ///
    /// Returns `None` if the guide is disabled or the section type has no MIDI mapping.
    pub fn schedule(
        count_in_start_quarters: f64,
        section_type: &SectionType,
        section_number: Option<u32>,
        config: &GuideConfig,
    ) -> Option<SectionCueEvent> {
        if !config.enabled {
            return None;
        }

        let midi_note = midi_map::section_type_midi_note(section_type)?;

        Some(SectionCueEvent {
            section_type: section_type.clone(),
            section_number,
            position_quarters: count_in_start_quarters,
            midi_note,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_verse() {
        let config = GuideConfig {
            enabled: true,
            replace_beat_one: true,
        };
        let event =
            SectionCueScheduler::schedule(0.0, &SectionType::Verse, Some(1), &config).unwrap();
        assert_eq!(event.midi_note, 84);
        assert_eq!(event.section_number, Some(1));
        assert!((event.position_quarters - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_schedule_disabled() {
        let config = GuideConfig {
            enabled: false,
            replace_beat_one: true,
        };
        assert!(SectionCueScheduler::schedule(0.0, &SectionType::Verse, None, &config).is_none());
    }

    #[test]
    fn test_schedule_unsupported_section() {
        let config = GuideConfig {
            enabled: true,
            replace_beat_one: true,
        };
        // CountIn has no MIDI mapping
        assert!(SectionCueScheduler::schedule(0.0, &SectionType::CountIn, None, &config).is_none());
    }
}
