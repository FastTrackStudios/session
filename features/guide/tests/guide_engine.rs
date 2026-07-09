//! Unit tests for the ported count-in logic plus scheduling / render
//! smoke tests. (The legacy crate only had REAPER integration tests;
//! these pin down the behavior of the verbatim-ported pattern code.)

use session_guide::count_in::{CountInCalculator, CountInPattern};
use session_guide::{
    AudioSample, BlockClock, CueEvent, CueSchedule, GuideConfig, GuideEngine, GuideSection,
    GuideSongTiming, ScheduleOptions, tts_cue_key,
};

// ─── CountInCalculator ──────────────────────────────────────────────────

#[test]
fn calculator_rounds_to_nearest_measure_and_clamps() {
    // 2 measures of 4/4 = 8 quarters
    assert_eq!(
        CountInCalculator::calculate_count_in_measures(0.0, 8.0, 4.0),
        2
    );
    // Slightly off distances round to the nearest measure
    assert_eq!(
        CountInCalculator::calculate_count_in_measures(0.0, 7.9, 4.0),
        2
    );
    assert_eq!(
        CountInCalculator::calculate_count_in_measures(0.0, 6.1, 4.0),
        2
    );
    // Clamped to 1..=8
    assert_eq!(
        CountInCalculator::calculate_count_in_measures(0.0, 0.5, 4.0),
        1
    );
    assert_eq!(
        CountInCalculator::calculate_count_in_measures(0.0, 400.0, 4.0),
        8
    );
}

// ─── CountInPattern ─────────────────────────────────────────────────────

fn pattern_for(total_measures: i32, num: i32, den: i32) -> Vec<Vec<Option<i32>>> {
    (0..total_measures)
        .map(|m| {
            (1..=num)
                .map(|b| CountInPattern::should_count(m, b, total_measures, num, den, true, true))
                .collect()
        })
        .collect()
}

#[test]
fn one_measure_4_4_counts_all_beats() {
    assert_eq!(
        pattern_for(1, 4, 4),
        vec![vec![Some(1), Some(2), Some(3), Some(4)]]
    );
}

#[test]
fn two_measures_4_4_half_then_full() {
    // Measure 1: "1 _ 2 _", measure 2: full count
    assert_eq!(
        pattern_for(2, 4, 4),
        vec![
            vec![Some(1), None, Some(2), None],
            vec![Some(1), Some(2), Some(3), Some(4)],
        ]
    );
}

#[test]
fn four_measures_4_4_extended_pattern() {
    // Measures 1-2: beat 1 only (numbered 1, 2); measure 3: half; measure 4: full
    assert_eq!(
        pattern_for(4, 4, 4),
        vec![
            vec![Some(1), None, None, None],
            vec![Some(2), None, None, None],
            vec![Some(1), None, Some(2), None],
            vec![Some(1), Some(2), Some(3), Some(4)],
        ]
    );
}

#[test]
fn one_measure_6_8_counts_all_beats() {
    // 6/8 is NOT split (numerator not > 6, denominator not 16)
    assert_eq!(
        pattern_for(1, 6, 8),
        vec![vec![Some(1), Some(2), Some(3), Some(4), Some(5), Some(6)]]
    );
}

#[test]
fn one_measure_9_8_splits_into_groups_ending_with_4() {
    // 9/8: last group is beats 6-9 counted 1-4; earlier group (beats 1-5)
    // counted 1-5 when full_count_odd_time is on.
    assert_eq!(
        pattern_for(1, 9, 8),
        vec![vec![
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
        ]]
    );
}

#[test]
fn one_measure_9_8_without_full_count_falls_back_to_standard_count() {
    // Legacy behavior (ported verbatim): when full_count_odd_time is off,
    // calculate_odd_time_count returns None for early-group beats, and
    // should_count then falls through to the STANDARD full count — the
    // early beats still count 1..=9 rather than being silent.
    let row: Vec<Option<i32>> = (1..=9)
        .map(|b| CountInPattern::should_count(0, b, 1, 9, 8, true, false))
        .collect();
    assert_eq!(
        row,
        vec![
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(1),
            Some(2),
            Some(3),
            Some(4)
        ]
    );
}

#[test]
fn sixteenth_denominator_single_measure_has_no_counts() {
    assert_eq!(pattern_for(1, 7, 16), vec![vec![None; 7]]);
}

#[test]
fn sixteenth_denominator_multi_measure_counts_down_measures() {
    // 3 measures of 7/16 with offset_by_one: beat 1 of each measure,
    // numbered total - index - 1 = 2, 1, 0(dropped)
    let p = pattern_for(3, 7, 16);
    assert_eq!(p[0][0], Some(2));
    assert_eq!(p[1][0], Some(1));
    assert_eq!(p[2][0], None); // count 0 is out of range 1..=8
    for row in &p {
        assert!(row[1..].iter().all(|c| c.is_none()));
    }
}

// ─── CueSchedule ────────────────────────────────────────────────────────

fn section(start: f64, end: f64, name: &str, type_name: &str) -> GuideSection {
    GuideSection {
        start_seconds: start,
        end_seconds: end,
        name: name.to_string(),
        count_in_position: None,
        song_end_position: None,
        is_first_section: false,
        section_type_name: type_name.to_string(),
        section_number: None,
        spoken_note: None,
    }
}

#[test]
fn schedule_one_measure_count_in_at_120_bpm() {
    // 120 bpm 4/4: beat = 0.5 s, measure = 2 s. Count-in marker 2 s before
    // the section → 1 measure → counts "1 2 3 4" at 8.0, 8.5, 9.0, 9.5.
    let mut first = section(10.0, 20.0, "Verse 1", "Verse");
    first.is_first_section = true;
    first.count_in_position = Some(8.0);

    let timing = GuideSongTiming {
        tempo_bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
    };
    let options = ScheduleOptions {
        guide_replace_beat1: false,
        extend_songend_count: false,
        ..Default::default()
    };
    let schedule = CueSchedule::build(&[first], &timing, &options);

    let counts: Vec<(f64, usize)> = schedule
        .cues
        .iter()
        .filter_map(|c| match &c.event {
            CueEvent::Count { index } => Some((c.time_seconds, *index)),
            _ => None,
        })
        .collect();
    assert_eq!(counts, vec![(8.0, 0), (8.5, 1), (9.0, 2), (9.5, 3)]);

    // Guide announcement one measure (4 beats) before the section start.
    let guides: Vec<&f64> = schedule
        .cues
        .iter()
        .filter_map(|c| match &c.event {
            CueEvent::Guide { .. } => Some(&c.time_seconds),
            _ => None,
        })
        .collect();
    assert_eq!(guides, vec![&8.0]);
}

#[test]
fn guide_replaces_beat_one() {
    let mut first = section(10.0, 20.0, "Verse 1", "Verse");
    first.is_first_section = true;
    first.count_in_position = Some(8.0);

    let timing = GuideSongTiming {
        tempo_bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
    };
    let options = ScheduleOptions {
        guide_replace_beat1: true,
        extend_songend_count: false,
        ..Default::default()
    };
    let schedule = CueSchedule::build(&[first], &timing, &options);

    // The count at 8.0 coincides with the guide cue and is dropped.
    let counts: Vec<f64> = schedule
        .cues
        .iter()
        .filter_map(|c| match &c.event {
            CueEvent::Count { .. } => Some(c.time_seconds),
            _ => None,
        })
        .collect();
    assert_eq!(counts, vec![8.5, 9.0, 9.5]);
}

#[test]
fn songend_gets_two_measure_count_out_and_ending_guide() {
    let mut last = section(20.0, 30.0, "Outro", "Outro");
    last.song_end_position = Some(30.0);

    let timing = GuideSongTiming {
        tempo_bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
    };
    let options = ScheduleOptions {
        guide_replace_beat1: false,
        ..Default::default()
    };
    let schedule = CueSchedule::build(&[last], &timing, &options);

    // 2-measure pattern into 30.0 starts at 26.0: "1 _ 2 _ | 1 2 3 4"
    let counts: Vec<(f64, usize)> = schedule
        .cues
        .iter()
        .filter_map(|c| match &c.event {
            CueEvent::Count { index } => Some((c.time_seconds, *index)),
            _ => None,
        })
        .collect();
    assert_eq!(
        counts,
        vec![
            (26.0, 0),
            (27.0, 1),
            (28.0, 0),
            (28.5, 1),
            (29.0, 2),
            (29.5, 3),
        ]
    );

    // "Ending" announcement at the count-out start.
    assert!(schedule.cues.iter().any(|c| matches!(
        &c.event,
        CueEvent::Guide { keys } if c.time_seconds == 26.0 && keys.contains(&"Ending_None".to_string())
    )));
}

#[test]
fn push_speak_inserts_sorted_tts_cue() {
    let mut schedule = CueSchedule::default();
    schedule.push_speak(5.0, "Bridge in 2");
    schedule.push_speak(1.0, "Chorus");
    assert_eq!(schedule.cues[0].time_seconds, 1.0);
    assert_eq!(
        schedule.cues[1].event,
        CueEvent::Guide {
            keys: vec![tts_cue_key("Bridge in 2")]
        }
    );
}

// ─── Engine render smoke test ───────────────────────────────────────────

/// A 1-frame impulse sample makes trigger positions directly observable.
fn impulse(sample_rate: u32) -> AudioSample {
    AudioSample::mono(vec![1.0], sample_rate)
}

#[test]
fn engine_renders_clicks_and_cues_sample_accurately() {
    const SR: f64 = 1000.0; // 1 kHz keeps offsets human-readable
    let mut engine = GuideEngine::new(GuideConfig {
        enable_measure_accent: false,
        ..Default::default()
    });
    engine.bank_mut().beat = Some(impulse(SR as u32));
    engine.bank_mut().counts[0] = Some(impulse(SR as u32));
    engine
        .bank_mut()
        .insert_guide("Verse_1", impulse(SR as u32));

    // One cue schedule: count "1" at 0.25 s, guide "Verse_1" at 0.75 s.
    let mut first = section(1.0, 2.0, "Verse 1", "Verse");
    first.section_number = Some(1);
    let mut schedule = CueSchedule::default();
    schedule.cues.push(session_guide::ScheduledCue {
        time_seconds: 0.25,
        event: CueEvent::Count { index: 0 },
    });
    schedule.cues.push(session_guide::ScheduledCue {
        time_seconds: 0.75,
        event: CueEvent::Guide {
            keys: vec!["Verse_1".to_string()],
        },
    });
    engine.set_schedule(schedule);

    // 120 bpm → beat every 0.5 s → every 500 samples at 1 kHz.
    // Render 1 second in two 500-frame blocks.
    let mut click = vec![0.0f32; 1000];
    let mut count = vec![0.0f32; 1000];
    let mut guide = vec![0.0f32; 1000];
    for block in 0..2 {
        let start = block * 500;
        let clock = BlockClock {
            playing: true,
            pos_seconds: start as f64 / SR,
            pos_beats: (start as f64 / SR) * 2.0, // 120 bpm = 2 quarters/s
            tempo_bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            sample_rate: SR,
        };
        let (cl, cr) = (&mut click[start..start + 500], &mut vec![0.0f32; 500]);
        let (nl, nr) = (&mut count[start..start + 500], &mut vec![0.0f32; 500]);
        let (gl, gr) = (&mut guide[start..start + 500], &mut vec![0.0f32; 500]);
        let mut buses = session_guide::GuideBuses {
            click_l: cl,
            click_r: cr,
            count_l: nl,
            count_r: nr,
            guide_l: gl,
            guide_r: gr,
        };
        engine.render(&mut buses, &clock);
    }

    // Beat clicks at samples 0 and 500 (0.0 s and 0.5 s), nowhere else.
    let click_hits: Vec<usize> = click
        .iter()
        .enumerate()
        .filter(|(_, v)| **v != 0.0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(click_hits, vec![0, 500]);

    // Count "1" at sample 250.
    let count_hits: Vec<usize> = count
        .iter()
        .enumerate()
        .filter(|(_, v)| **v != 0.0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(count_hits, vec![250]);

    // Guide at sample 750.
    let guide_hits: Vec<usize> = guide
        .iter()
        .enumerate()
        .filter(|(_, v)| **v != 0.0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(guide_hits, vec![750]);
}

#[test]
fn engine_stops_triggering_when_not_playing() {
    const SR: f64 = 1000.0;
    let mut engine = GuideEngine::default();
    engine.bank_mut().beat = Some(impulse(SR as u32));
    engine.bank_mut().measure_accent = Some(impulse(SR as u32));

    let mut l = vec![0.0f32; 500];
    let mut r = vec![0.0f32; 500];
    let clock = BlockClock {
        playing: false,
        pos_seconds: 0.0,
        pos_beats: 0.0,
        tempo_bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
        sample_rate: SR,
    };
    engine.render_stereo(&mut l, &mut r, &clock);
    assert!(l.iter().all(|v| *v == 0.0));
}

// ─── SampleBank::synthesize_defaults ────────────────────────────────────

/// Peak absolute value across all channels of a sample.
fn peak(sample: &session_guide::AudioSample) -> f32 {
    sample
        .data
        .iter()
        .flat_map(|ch| ch.iter())
        .fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn synthesize_defaults_fills_all_slots_non_silent_and_bounded() {
    const SR: u32 = 48_000;
    let mut engine = GuideEngine::default();
    engine.bank_mut().synthesize_defaults(SR);
    let bank = engine.bank();

    let named = [
        ("beat", bank.beat.as_ref()),
        ("eighth", bank.eighth.as_ref()),
        ("sixteenth", bank.sixteenth.as_ref()),
        ("triplet", bank.triplet.as_ref()),
        ("accent", bank.measure_accent.as_ref()),
    ];
    for (name, slot) in named {
        let sample = slot.unwrap_or_else(|| panic!("{name} not synthesized"));
        let p = peak(sample);
        assert!(p > 0.05, "{name} is silent (peak {p})");
        assert!(p <= 1.0, "{name} exceeds full scale (peak {p})");
        assert_eq!(sample.sample_rate, SR);
        // Bounded length: every placeholder is well under half a second.
        assert!(sample.frames() < SR as usize / 2, "{name} too long");
    }
    for (i, slot) in bank.counts.iter().enumerate() {
        let sample = slot.as_ref().unwrap_or_else(|| panic!("count {i} missing"));
        let p = peak(sample);
        assert!(p > 0.05 && p <= 1.0, "count {i} peak {p} out of range");
    }
    // Section-guide chimes cover the standard keys.
    for key in ["Verse_1", "Chorus_None", "Pre Chorus_2", "Ending_None"] {
        let sample = &bank.guides[key];
        let p = peak(sample);
        assert!(p > 0.05 && p <= 1.0, "guide {key} peak {p} out of range");
        assert!(sample.frames() < SR as usize, "guide {key} too long");
    }
}

#[test]
fn synthesize_defaults_never_overwrites_loaded_samples() {
    const SR: u32 = 48_000;
    let mut engine = GuideEngine::default();
    engine.bank_mut().beat = Some(impulse(SR));
    engine.bank_mut().counts[0] = Some(impulse(SR));
    engine.bank_mut().insert_guide("Chorus_None", impulse(SR));
    engine.bank_mut().synthesize_defaults(SR);

    let bank = engine.bank();
    assert_eq!(bank.beat.as_ref().unwrap().frames(), 1);
    assert_eq!(bank.counts[0].as_ref().unwrap().frames(), 1);
    assert_eq!(bank.guides["Chorus_None"].frames(), 1);
    // ...while empty slots were still filled.
    assert!(bank.measure_accent.is_some());
    assert!(bank.counts[1].is_some());
    assert!(bank.guides.contains_key("Verse_1"));
}

#[test]
fn synthesized_bank_renders_audible_guide() {
    // End-to-end: a zero-asset engine + demo-like schedule produces
    // non-silent output through render_stereo.
    const SR: f64 = 48_000.0;
    let mut engine = GuideEngine::default();
    engine.bank_mut().synthesize_defaults(SR as u32);

    let mut first = section(4.0, 12.0, "Verse 1", "Verse");
    first.is_first_section = true;
    first.count_in_position = Some(0.0);
    first.section_number = Some(1);
    engine.set_sections(
        &[first],
        &GuideSongTiming {
            tempo_bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
        },
    );

    let mut l = vec![0.0f32; 4096];
    let mut r = vec![0.0f32; 4096];
    let clock = BlockClock {
        playing: true,
        pos_seconds: 0.0,
        pos_beats: 0.0,
        tempo_bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
        sample_rate: SR,
    };
    engine.render_stereo(&mut l, &mut r, &clock);
    let p = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(p > 0.05, "synthesized guide rendered silence (peak {p})");
    assert!(p <= 2.0, "synthesized guide clipped wildly (peak {p})");
}

#[test]
fn measure_accent_replaces_beat_one() {
    const SR: f64 = 1000.0;
    let mut engine = GuideEngine::default();
    // Distinguish accent (amplitude 2.0) from beat (1.0).
    engine.bank_mut().beat = Some(impulse(SR as u32));
    engine.bank_mut().measure_accent = Some(AudioSample::mono(vec![2.0], SR as u32));

    // Render one 4/4 measure at 120 bpm = 2 s = 2000 samples.
    let mut l = vec![0.0f32; 2000];
    let mut r = vec![0.0f32; 2000];
    let clock = BlockClock {
        playing: true,
        pos_seconds: 0.0,
        pos_beats: 0.0,
        tempo_bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
        sample_rate: SR,
    };
    engine.render_stereo(&mut l, &mut r, &clock);

    assert_eq!(l[0], 2.0); // accent on beat 1
    assert_eq!(l[500], 1.0); // plain beats 2-4
    assert_eq!(l[1000], 1.0);
    assert_eq!(l[1500], 1.0);
}
