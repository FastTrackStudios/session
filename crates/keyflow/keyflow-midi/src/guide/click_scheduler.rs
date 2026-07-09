//! Click subdivision scheduling — pure beat math in quarter-note space.
//!
//! Ported from the legacy FTS Guide plugin's `audio/trigger_scheduler.rs`,
//! stripped of sample-space calculations. Works entirely in quarter-note positions.

use keyflow_proto::TimeSignature;
use keyflow_proto::guide::{
    config::ClickConfig,
    event::{ClickEvent, ClickType},
    midi_map,
};

/// Schedules click events within a time range.
pub struct ClickScheduler;

impl ClickScheduler {
    /// Schedule click events within a quarter-note range for one measure.
    ///
    /// `start_quarters` and `end_quarters` are absolute positions. Events are placed
    /// at every beat/subdivision boundary that falls within `[start, end)`.
    ///
    /// The `measure_start_quarters` anchors the grid so subdivisions align to the measure.
    pub fn schedule(
        start_quarters: f64,
        end_quarters: f64,
        measure_start_quarters: f64,
        time_signature: &TimeSignature,
        config: &ClickConfig,
    ) -> Vec<ClickEvent> {
        if !config.beat_enabled
            && !config.accent_enabled
            && !config.eighth_enabled
            && !config.sixteenth_enabled
            && !config.triplet_enabled
        {
            return Vec::new();
        }

        let beat_unit = 4.0 / time_signature.denominator() as f64;
        let beats_per_measure = time_signature.numerator() as usize;
        let measure_length = beat_unit * beats_per_measure as f64;

        // Subdivision intervals in quarter notes
        let eighth_interval = 0.5;
        let sixteenth_interval = 0.25;
        let triplet_interval = beat_unit / 3.0;

        let mut events = Vec::new();
        let tolerance = 1e-9;

        // Schedule beats
        if config.beat_enabled || config.accent_enabled {
            for beat_idx in 0..beats_per_measure {
                let pos = measure_start_quarters + beat_idx as f64 * beat_unit;
                if pos >= start_quarters - tolerance && pos < end_quarters - tolerance {
                    let is_accent = beat_idx == 0 && config.accent_enabled;
                    let click_type = if is_accent {
                        ClickType::Accent
                    } else {
                        ClickType::Beat
                    };
                    // Only emit beat if beat_enabled, or accent if accent_enabled on beat 1
                    if (is_accent && config.accent_enabled) || (!is_accent && config.beat_enabled) {
                        events.push(ClickEvent {
                            click_type,
                            position_quarters: pos,
                            midi_note: if is_accent {
                                midi_map::CLICK_ACCENT
                            } else {
                                midi_map::CLICK_BEAT
                            },
                            velocity: if is_accent { 127 } else { 100 },
                        });
                    }
                }
            }
        }

        // Schedule eighths (between beats only — skip positions that coincide with beats)
        if config.eighth_enabled {
            let mut pos = measure_start_quarters;
            let measure_end = measure_start_quarters + measure_length;
            while pos < measure_end + tolerance {
                if pos >= start_quarters - tolerance
                    && pos < end_quarters - tolerance
                    && !is_on_beat_grid(pos, measure_start_quarters, beat_unit, tolerance)
                {
                    events.push(ClickEvent {
                        click_type: ClickType::Eighth,
                        position_quarters: pos,
                        midi_note: midi_map::CLICK_EIGHTH,
                        velocity: 80,
                    });
                }
                pos += eighth_interval;
            }
        }

        // Schedule sixteenths (skip positions that coincide with beats or eighths)
        if config.sixteenth_enabled {
            let mut pos = measure_start_quarters;
            let measure_end = measure_start_quarters + measure_length;
            while pos < measure_end + tolerance {
                if pos >= start_quarters - tolerance
                    && pos < end_quarters - tolerance
                    && !is_on_beat_grid(pos, measure_start_quarters, beat_unit, tolerance)
                    && !is_on_subdivision_grid(
                        pos,
                        measure_start_quarters,
                        eighth_interval,
                        tolerance,
                    )
                {
                    events.push(ClickEvent {
                        click_type: ClickType::Sixteenth,
                        position_quarters: pos,
                        midi_note: midi_map::CLICK_SIXTEENTH,
                        velocity: 60,
                    });
                }
                pos += sixteenth_interval;
            }
        }

        // Schedule triplets (skip positions that coincide with beats)
        if config.triplet_enabled {
            for beat_idx in 0..beats_per_measure {
                let beat_start = measure_start_quarters + beat_idx as f64 * beat_unit;
                for sub in 0..3 {
                    let pos = beat_start + sub as f64 * triplet_interval;
                    if pos >= start_quarters - tolerance && pos < end_quarters - tolerance {
                        // Skip beat positions (sub == 0 coincides with the beat)
                        if sub != 0 {
                            events.push(ClickEvent {
                                click_type: ClickType::Triplet,
                                position_quarters: pos,
                                midi_note: midi_map::CLICK_TRIPLET,
                                velocity: 70,
                            });
                        }
                    }
                }
            }
        }

        // Sort by position, then by priority (Accent < Beat < Eighth < Sixteenth < Triplet)
        events.sort_by(|a, b| {
            a.position_quarters
                .partial_cmp(&b.position_quarters)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    click_type_priority(a.click_type).cmp(&click_type_priority(b.click_type))
                })
        });

        // Deduplicate: if two events are at the same position, keep only the highest priority
        events.dedup_by(|b, a| (a.position_quarters - b.position_quarters).abs() < tolerance);

        events
    }
}

/// Check if a position falls on the beat grid.
fn is_on_beat_grid(pos: f64, measure_start: f64, beat_unit: f64, tolerance: f64) -> bool {
    let offset = pos - measure_start;
    let remainder = offset % beat_unit;
    remainder < tolerance || (beat_unit - remainder) < tolerance
}

/// Check if a position falls on a subdivision grid.
fn is_on_subdivision_grid(pos: f64, measure_start: f64, interval: f64, tolerance: f64) -> bool {
    let offset = pos - measure_start;
    let remainder = offset % interval;
    remainder < tolerance || (interval - remainder) < tolerance
}

/// Lower number = higher priority for dedup ordering.
fn click_type_priority(ct: ClickType) -> u8 {
    match ct {
        ClickType::Accent => 0,
        ClickType::Beat => 1,
        ClickType::Eighth => 2,
        ClickType::Sixteenth => 3,
        ClickType::Triplet => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(beat: bool, eighth: bool, sixteenth: bool, triplet: bool) -> ClickConfig {
        ClickConfig {
            beat_enabled: beat,
            eighth_enabled: eighth,
            sixteenth_enabled: sixteenth,
            triplet_enabled: triplet,
            accent_enabled: true,
        }
    }

    fn ts(num: u32, den: u32) -> TimeSignature {
        TimeSignature::new(num, den)
    }

    #[test]
    fn test_beats_4_4() {
        let config = make_config(true, false, false, false);
        let events = ClickScheduler::schedule(0.0, 4.0, 0.0, &ts(4, 4), &config);

        assert_eq!(events.len(), 4);
        // Beat 1 is accent
        assert_eq!(events[0].click_type, ClickType::Accent);
        assert!((events[0].position_quarters - 0.0).abs() < 1e-9);
        // Beats 2-4 are regular
        assert_eq!(events[1].click_type, ClickType::Beat);
        assert!((events[1].position_quarters - 1.0).abs() < 1e-9);
        assert!((events[2].position_quarters - 2.0).abs() < 1e-9);
        assert!((events[3].position_quarters - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_eighths_4_4() {
        let config = make_config(true, true, false, false);
        let events = ClickScheduler::schedule(0.0, 4.0, 0.0, &ts(4, 4), &config);

        // 4 beats + 4 eighths (at 0.5, 1.5, 2.5, 3.5)
        assert_eq!(events.len(), 8);

        let eighth_positions: Vec<f64> = events
            .iter()
            .filter(|e| e.click_type == ClickType::Eighth)
            .map(|e| e.position_quarters)
            .collect();
        assert_eq!(eighth_positions.len(), 4);
        assert!((eighth_positions[0] - 0.5).abs() < 1e-9);
        assert!((eighth_positions[1] - 1.5).abs() < 1e-9);
        assert!((eighth_positions[2] - 2.5).abs() < 1e-9);
        assert!((eighth_positions[3] - 3.5).abs() < 1e-9);
    }

    #[test]
    fn test_triplets_4_4() {
        let config = make_config(true, false, false, true);
        let events = ClickScheduler::schedule(0.0, 4.0, 0.0, &ts(4, 4), &config);

        // 4 beats + 8 triplet off-beats (2 per beat)
        assert_eq!(events.len(), 12);

        let triplet_events: Vec<&ClickEvent> = events
            .iter()
            .filter(|e| e.click_type == ClickType::Triplet)
            .collect();
        assert_eq!(triplet_events.len(), 8);
        // First beat triplets at 1/3 and 2/3
        let t = 1.0 / 3.0;
        assert!((triplet_events[0].position_quarters - t).abs() < 1e-9);
        assert!((triplet_events[1].position_quarters - 2.0 * t).abs() < 1e-9);
    }

    #[test]
    fn test_waltz_3_4() {
        let config = make_config(true, false, false, false);
        let events = ClickScheduler::schedule(0.0, 3.0, 0.0, &ts(3, 4), &config);

        assert_eq!(events.len(), 3);
        assert!((events[0].position_quarters - 0.0).abs() < 1e-9);
        assert!((events[1].position_quarters - 1.0).abs() < 1e-9);
        assert!((events[2].position_quarters - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_6_8_time() {
        // 6/8: beat unit = 4/8 = 0.5 quarter notes, 6 beats
        let config = make_config(true, false, false, false);
        let ts_6_8 = ts(6, 8);
        let measure_length = 6.0 * 0.5; // 3.0 quarter notes
        let events = ClickScheduler::schedule(0.0, measure_length, 0.0, &ts_6_8, &config);

        assert_eq!(events.len(), 6);
        assert!((events[0].position_quarters - 0.0).abs() < 1e-9);
        assert!((events[1].position_quarters - 0.5).abs() < 1e-9);
        assert!((events[2].position_quarters - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_no_events_when_all_disabled() {
        let config = ClickConfig {
            beat_enabled: false,
            eighth_enabled: false,
            sixteenth_enabled: false,
            triplet_enabled: false,
            accent_enabled: false,
        };
        let events = ClickScheduler::schedule(0.0, 4.0, 0.0, &ts(4, 4), &config);
        assert!(events.is_empty());
    }

    #[test]
    fn test_accent_only_no_beat() {
        let config = ClickConfig {
            beat_enabled: false,
            eighth_enabled: false,
            sixteenth_enabled: false,
            triplet_enabled: false,
            accent_enabled: true,
        };
        let events = ClickScheduler::schedule(0.0, 4.0, 0.0, &ts(4, 4), &config);
        // Only accent on beat 1
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].click_type, ClickType::Accent);
    }

    // ── Edge case tests ─────────────────────────────────────────────────────

    #[test]
    fn test_zero_length_region() {
        let config = make_config(true, false, false, false);
        let events = ClickScheduler::schedule(4.0, 4.0, 4.0, &ts(4, 4), &config);
        // Zero-length region produces no events
        assert!(events.is_empty());
    }

    #[test]
    fn test_1_4_time() {
        // 1/4: single beat per measure
        let config = make_config(true, false, false, false);
        let events = ClickScheduler::schedule(0.0, 1.0, 0.0, &ts(1, 4), &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].click_type, ClickType::Accent);
    }

    #[test]
    fn test_sixteenths_4_4() {
        let config = make_config(true, false, true, false);
        let events = ClickScheduler::schedule(0.0, 4.0, 0.0, &ts(4, 4), &config);

        let sixteenth_positions: Vec<f64> = events
            .iter()
            .filter(|e| e.click_type == ClickType::Sixteenth)
            .map(|e| e.position_quarters)
            .collect();
        // Sixteenths at 0.25, 0.75, 1.25, 1.75, 2.25, 2.75, 3.25, 3.75
        // (skipping beat positions at 0, 1, 2, 3 and eighth positions at 0.5, 1.5, 2.5, 3.5)
        assert_eq!(sixteenth_positions.len(), 8);
        assert!((sixteenth_positions[0] - 0.25).abs() < 1e-9);
        assert!((sixteenth_positions[1] - 0.75).abs() < 1e-9);
    }

    #[test]
    fn test_partial_measure_region() {
        // Only the second half of a 4/4 measure
        let config = make_config(true, false, false, false);
        let events = ClickScheduler::schedule(2.0, 4.0, 0.0, &ts(4, 4), &config);
        // Beats 3 and 4 (positions 2.0 and 3.0)
        assert_eq!(events.len(), 2);
        assert!((events[0].position_quarters - 2.0).abs() < 1e-9);
        assert!((events[1].position_quarters - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_midi_notes_correct() {
        let config = make_config(true, true, false, false);
        let events = ClickScheduler::schedule(0.0, 2.0, 0.0, &ts(4, 4), &config);

        let accent = events
            .iter()
            .find(|e| e.click_type == ClickType::Accent)
            .unwrap();
        assert_eq!(accent.midi_note, 60); // CLICK_ACCENT

        let beat = events
            .iter()
            .find(|e| e.click_type == ClickType::Beat)
            .unwrap();
        assert_eq!(beat.midi_note, 61); // CLICK_BEAT

        let eighth = events
            .iter()
            .find(|e| e.click_type == ClickType::Eighth)
            .unwrap();
        assert_eq!(eighth.midi_note, 62); // CLICK_EIGHTH
    }

    #[test]
    fn test_velocity_hierarchy() {
        // Accent > Beat > Eighth > Triplet > Sixteenth
        let config = ClickConfig {
            beat_enabled: true,
            eighth_enabled: true,
            sixteenth_enabled: true,
            triplet_enabled: true,
            accent_enabled: true,
        };
        let events = ClickScheduler::schedule(0.0, 4.0, 0.0, &ts(4, 4), &config);

        let accent_vel = events
            .iter()
            .find(|e| e.click_type == ClickType::Accent)
            .unwrap()
            .velocity;
        let beat_vel = events
            .iter()
            .find(|e| e.click_type == ClickType::Beat)
            .unwrap()
            .velocity;
        let eighth_vel = events
            .iter()
            .find(|e| e.click_type == ClickType::Eighth)
            .unwrap()
            .velocity;

        assert!(accent_vel > beat_vel);
        assert!(beat_vel > eighth_vel);
    }
}
