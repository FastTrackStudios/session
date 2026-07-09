//! MusicXML → Keyflow Chart importer.
//!
//! Wraps the third-party `musicxml` crate (full MusicXML 4.0 support, .musicxml
//! and .mxl) and translates the parsed score into a [`keyflow_proto::chart::Chart`].
//!
//! Coverage so far (Lord of the Fight baseline):
//! - Metadata (title, composer, lyricist, arranger, copyright)
//! - Per-measure time signature
//! - Barline style + start/end repeat marks
//! - Volta (1st/2nd endings) start markers
//! - Hairpin spanners (within a single measure)
//! - Classical dynamics (mp/mf/f/ff/…)
//! - Staff text directions (free-form `<words>`, boxed rehearsal marks)
//! - Stacked figured-bass numerals (`<words>` like "4-3 / 2-1")
//! - `<harmony>` to `ChordInstance`, including vertical bass arrangements
//! - Measure-local harmony placement, including sub-beat offsets
//! - MusicXML measure numbers preserved on imported measures
//! - Simple tie flags on melody notes
//!
//! Intentionally limited for now:
//! - Direction `<offset>` is ignored for staff text and rehearsal marks; Tom
//!   Brooks charts use those offsets as visual nudges rather than playback time.
//! - Multi-measure / cross-system hairpins and voltas are not carried across
//!   system breaks yet.

mod convert;

use std::path::Path;

use keyflow_proto::chart::Chart;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("musicxml parse error: {0}")]
    Parse(String),
}

/// Parse a `.musicxml` or `.mxl` file and translate to a keyflow [`Chart`].
pub fn import_file(path: impl AsRef<Path>) -> Result<Chart, ImportError> {
    let path_str = path
        .as_ref()
        .to_str()
        .ok_or_else(|| ImportError::Parse("path is not valid UTF-8".into()))?;
    let score = musicxml::read_score_partwise(path_str).map_err(ImportError::Parse)?;
    Ok(convert::chart_from_score(&score))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("..");
        p.push("examples/png-project-charts");
        p.push(name);
        p
    }

    #[test]
    fn imports_lord_of_the_fight_metadata() {
        let chart = import_file(fixture("02 LORD OF THE FIGHT Master RS.musicxml"))
            .expect("musicxml import should succeed");
        // Visible chart title comes from the biggest credit-words on page 1
        // (Tom Brooks puts "Lord of the Fight" there); <movement-title>
        // "WHO'S THERE, GOD'S THERE" is the file's working subtitle.
        assert_eq!(chart.metadata.title.as_deref(), Some("Lord of the Fight"));
        assert_eq!(
            chart.metadata.subtitle.as_deref(),
            Some("WHO'S THERE, GOD'S THERE")
        );
        assert_eq!(chart.metadata.composer.as_deref(), Some("TONY KENOLY"));

        // Lord of the Fight is in E major (4 sharps), bass clef.
        let key = chart
            .metadata
            .title
            .as_deref()
            .and(chart.initial_key.as_ref());
        assert!(key.is_some(), "expected initial_key to be set");
        assert_eq!(
            chart.initial_clef,
            Some(keyflow_proto::chart::ChartClef::Bass)
        );
    }

    #[test]
    fn imports_lord_of_the_fight_measures() {
        use keyflow_proto::chart::notations::RepeatMark;
        use keyflow_proto::chart::types::Measure;

        let chart = import_file(fixture("02 LORD OF THE FIGHT Master RS.musicxml"))
            .expect("musicxml import should succeed");
        assert!(
            chart.sections.len() >= 3,
            "Lord of the Fight is split into multiple sections (intro, verses, choruses, bridge…), got {}",
            chart.sections.len()
        );

        // Collect every measure across every section/track.
        let measures: Vec<&Measure> = chart
            .sections
            .iter()
            .flat_map(|s| s.tracks.iter())
            .flat_map(|t| t.measures.iter())
            .collect();
        assert!(
            measures.len() >= 30,
            "expected many measures, got {}",
            measures.len()
        );

        let backward_repeats = measures
            .iter()
            .filter(|m| matches!(m.end_repeat, RepeatMark::Backward))
            .count();
        assert!(
            backward_repeats >= 1,
            "expected at least one backward repeat"
        );
        let by_source_number = |n: u32| -> &Measure {
            measures
                .iter()
                .copied()
                .find(|m| m.source_measure_number == Some(n))
                .unwrap_or_else(|| panic!("missing source measure {n}"))
        };
        assert!(
            matches!(by_source_number(19).start_repeat, RepeatMark::Forward),
            "m19 should start the VS1b/VS2 repeat"
        );
        assert!(
            matches!(by_source_number(45).end_repeat, RepeatMark::Backward),
            "m45 should close the VS1b/VS2 repeat"
        );
        let first_ending = by_source_number(42)
            .volta_start
            .as_ref()
            .expect("m42 should infer first-ending volta from MusicXML text");
        assert_eq!(first_ending.numbers, vec![1]);
        assert_eq!(
            first_ending.length_measures, 4,
            "first ending should span m42 through m45"
        );

        let staff_text_count: usize = measures.iter().map(|m| m.staff_text.len()).sum();
        assert!(
            staff_text_count >= 5,
            "expected several staff-text directions, got {staff_text_count}"
        );

        let figured_bass_count: usize = measures.iter().map(|m| m.figured_bass.len()).sum();
        assert!(
            figured_bass_count >= 1,
            "expected at least one figured-bass annotation, got {figured_bass_count}"
        );
        let m31_figured_bass = by_source_number(31)
            .figured_bass
            .first()
            .expect("m31 should import the #4-3 / 2-1 words as figured bass");
        assert_eq!(
            m31_figured_bass.source_default_x,
            Some(136.0),
            "figured-bass default-x should be preserved for source-relative alignment"
        );

        let chord_count: usize = measures.iter().map(|m| m.chords.len()).sum();
        assert!(
            chord_count >= 20,
            "expected many chords from harmony, got {chord_count}"
        );

        let vertical_bass = measures
            .iter()
            .flat_map(|m| m.chords.iter())
            .filter(|c| c.parsed.bass_vertical)
            .count();
        assert!(
            vertical_bass >= 1,
            "expected at least one vertical-bass chord, got {vertical_bass}"
        );

        // Copyright line should be lifted to metadata, not staff-text.
        let any_copyright_inline = measures
            .iter()
            .flat_map(|m| m.staff_text.iter())
            .any(|t| t.text.contains("Tom Brooks Music") || t.text.contains('©'));
        assert!(
            !any_copyright_inline,
            "copyright text should be moved out of inline staff_text"
        );
        assert!(
            chart.metadata.copyright.is_some(),
            "expected metadata.copyright to be populated"
        );
    }

    #[test]
    fn lord_of_the_fight_keyflow_roundtrip_preserves_semantics_without_annotations() {
        use keyflow_proto::chart::types::Measure;

        let direct = import_file(fixture("02 LORD OF THE FIGHT Master RS.musicxml"))
            .expect("musicxml import should succeed");
        let kf = keyflow_text::chart::exporter::chart_to_keyflow(&direct);
        let roundtripped =
            keyflow_text::chart::parse_chart(&kf).expect("exported keyflow should parse");

        let direct_measures: Vec<&Measure> = direct
            .sections
            .iter()
            .flat_map(|s| s.tracks.iter())
            .flat_map(|t| t.measures.iter())
            .collect();
        let roundtrip_measures: Vec<&Measure> = roundtripped
            .sections
            .iter()
            .flat_map(|s| s.tracks.iter())
            .flat_map(|t| t.measures.iter())
            .collect();

        assert_eq!(
            roundtrip_measures.len(),
            111,
            "exported Lord of the Fight should parse as the fully expanded chart without hidden measure annotations"
        );

        let direct_m31 = direct_measures
            .iter()
            .position(|m| m.source_measure_number == Some(31))
            .expect("missing direct source measure 31");
        let roundtrip_m31 = &roundtrip_measures[direct_m31];
        assert_eq!(
            roundtrip_m31.chords[0].full_symbol, "A",
            "source m31 should still land on the same expanded measure after export/import"
        );
        let figured = roundtrip_m31
            .figured_bass
            .first()
            .expect("m31 figured bass should roundtrip through chord-attached quoted text");
        let figure_text = figured
            .rows
            .iter()
            .map(|row| format!("{}{}", row.accidental, row.text))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(figure_text, "#4-3 2-1");
    }

    #[test]
    fn presence_keyflow_export_expands_cross_section_repeat() {
        let direct = import_file(fixture("04 PRESENCE Master RS.musicxml"))
            .expect("musicxml import should succeed");
        let kf = keyflow_text::chart::exporter::chart_to_keyflow(&direct);
        let roundtripped =
            keyflow_text::chart::parse_chart(&kf).expect("exported keyflow should parse");

        let measure_count: usize = roundtripped
            .sections
            .iter()
            .map(|section| section.measures().len())
            .sum();
        assert_eq!(
            measure_count, 69,
            "Presence has a repeat start in the verse section and the repeat end in the chorus section; the exported no-repeat .kf should include the second verse pass"
        );

        let flattened = roundtripped
            .sections
            .iter()
            .flat_map(|section| section.measures())
            .collect::<Vec<_>>();
        let second_pass = &flattened[33..41];
        let second_pass_chords = second_pass
            .iter()
            .map(|measure| {
                measure
                    .chords
                    .iter()
                    .map(|chord| chord.full_symbol.as_str())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            second_pass_chords,
            vec![
                vec!["E", "Eb/G#", "A"],
                vec!["B", "C#m"],
                vec!["A", "E/G#"],
                vec!["Bsus4", "E/G#"],
                vec!["A", "E/G#"],
                vec!["B", "C#m"],
                vec!["F#m7"],
                vec!["E"],
            ],
            "plain major chords in the repeated verse pass should not recall previous B11/E extensions"
        );
    }

    /// Detailed Lord of the Fight structural assertions. The user's reference
    /// charts use absolute XML measure numbers — count-in is xml measures 1
    /// & 2, the first musical measure is xml measure 3. We assert by
    /// `source_measure_number` so the test stays meaningful no matter how
    /// the importer slices sections.
    #[test]
    fn lord_of_the_fight_structure() {
        use keyflow_proto::chart::types::Measure;
        use keyflow_proto::sections::SectionType;

        let chart = import_file(fixture("02 LORD OF THE FIGHT Master RS.musicxml"))
            .expect("musicxml import should succeed");
        assert_eq!(
            chart.metadata.tempo,
            Some(168),
            "BPM credit should import the visible eighth-note tempo"
        );
        assert_eq!(
            chart.tempo.map(|tempo| tempo.bpm as u32),
            Some(168),
            "chart tempo should prefer the visible BPM credit over playback sound tempo"
        );

        let measures: Vec<&Measure> = chart
            .sections
            .iter()
            .flat_map(|s| s.tracks.iter())
            .flat_map(|t| t.measures.iter())
            .collect();

        // ── Count-in ────────────────────────────────────────────────────
        // First section is the auto-detected count-in: xml measures 1 & 2.
        let count_in_section = &chart.sections[0];
        assert_eq!(count_in_section.section.section_type, SectionType::CountIn);
        let ci_measures = count_in_section.measures();
        assert_eq!(
            ci_measures.len(),
            2,
            "count-in should be exactly 2 measures"
        );
        assert_eq!(ci_measures[0].source_measure_number, Some(1));
        assert_eq!(ci_measures[1].source_measure_number, Some(2));

        let verse_repeat = chart
            .sections
            .iter()
            .find(|section| {
                section
                    .section
                    .name
                    .as_deref()
                    .is_some_and(|name| name.contains("VS 1b") && name.contains("VS 2"))
            })
            .expect("expected VS 1b / VS 2 repeated verse section");
        let verse_repeat_measures = verse_repeat.measures();
        assert_eq!(
            verse_repeat_measures
                .first()
                .and_then(|measure| measure.source_measure_number),
            Some(19),
            "VS 1b / VS 2 repeat section should start at m19"
        );
        assert_eq!(
            verse_repeat
                .section
                .metadata
                .get("repeat_pass.labels")
                .map(String::as_str),
            Some("VS 1b\nVS 2"),
            "repeat-pass instruction labels should stay attached to the repeated verse section"
        );
        assert_eq!(
            verse_repeat
                .section
                .metadata
                .get("repeat_pass.VS_1b.instruction")
                .map(String::as_str),
            Some("Groove starts to build -- Bass loco -- Drums enter")
        );
        assert_eq!(
            verse_repeat
                .section
                .metadata
                .get("repeat_pass.VS_2.instruction")
                .map(String::as_str),
            Some("Full Groove now > > >")
        );
        assert!(
            verse_repeat
                .measures()
                .iter()
                .flat_map(|measure| measure.staff_text.iter())
                .all(|text| !text.text.contains("Groove starts to build")
                    && !text.text.contains("Full Groove now")),
            "repeat-pass instruction text should be owned by the section, not loose measure text"
        );

        let chorus_repeat = chart
            .sections
            .iter()
            .find(|section| {
                section
                    .section
                    .name
                    .as_deref()
                    .is_some_and(|name| name.contains("CH 1") && name.contains("CH 2"))
            })
            .expect("expected CH 1 / CH 2 repeated chorus section");
        let chorus_repeat_measures = chorus_repeat.measures();
        assert_eq!(
            chorus_repeat_measures
                .first()
                .and_then(|measure| measure.source_measure_number),
            Some(27),
            "CH 1 / CH 2 repeat section should start at m27"
        );
        assert_eq!(
            chorus_repeat.measures().len(),
            21,
            "CH 1 / CH 2 should include m27 through first-ending m45 plus m46-m47"
        );
        assert_eq!(
            chorus_repeat
                .section
                .metadata
                .get("repeat_pass.CH_1.instruction")
                .map(String::as_str),
            Some("STOP GROOVE!"),
            "CH 1 instruction should attach to the chorus repeat section"
        );
        assert_eq!(
            chorus_repeat
                .section
                .metadata
                .get("repeat_pass.CH_2.instruction")
                .map(String::as_str),
            Some("KEEP DRIVING!"),
            "CH 2 instruction should attach to the chorus repeat section"
        );
        assert!(
            chart
                .sections
                .iter()
                .all(|section| section.section.name.as_deref() != Some("CH 1 = STOP GROOVE!")),
            "CH 1 = STOP GROOVE! is an instruction inside the repeat, not a section split"
        );

        // First musical measure is xml measure 3 — verify by finding it.
        let m_by_num = |n: u32| -> &Measure {
            measures
                .iter()
                .copied()
                .find(|m| m.source_measure_number == Some(n))
                .unwrap_or_else(|| panic!("no measure with source number {n}"))
        };
        let first_musical = measures
            .iter()
            .find(|m| m.source_measure_number == Some(3))
            .expect("expected xml measure 3 to exist");
        assert_eq!(first_musical.source_measure_number, Some(3));
        let first_ending = m_by_num(42)
            .volta_start
            .as_ref()
            .expect("m42 should start the first ending");
        assert_eq!(first_ending.numbers, vec![1]);
        assert_eq!(
            first_ending.length_measures, 4,
            "first ending should span m42 through m45"
        );
        let second_ending = m_by_num(46)
            .volta_start
            .as_ref()
            .expect("m46 should infer the second ending from MusicXML text");
        assert_eq!(second_ending.numbers, vec![2]);
        assert_eq!(
            second_ending.length_measures, 2,
            "second ending should span m46 through m47, then land on bridge at m48"
        );
        assert!(
            m_by_num(47).volta_start.is_none(),
            "m47 is inside the second ending and should not start another volta"
        );
        let bridge = chart
            .sections
            .iter()
            .find(|section| section.section.section_type == SectionType::Bridge)
            .expect("expected bridge section");
        assert_eq!(
            bridge
                .measures()
                .first()
                .and_then(|measure| measure.source_measure_number),
            Some(48),
            "bridge should land on m48 after the second ending"
        );

        let beat_at = |c: &keyflow_proto::ChordInstance| -> f64 {
            c.position.beats() as f64 + c.position.subdivisions() as f64 / 1000.0 + 1.0
        };
        let approx = |a: f64, b: f64| (a - b).abs() < 0.05;
        let assert_chord_at = |measure_num: u32, beat: f64, want_sym: &str| {
            let m = m_by_num(measure_num);
            let found = m
                .chords
                .iter()
                .find(|c| c.full_symbol == want_sym && approx(beat_at(c), beat));
            assert!(
                found.is_some(),
                "expected {want_sym} at position {measure_num}.{beat}; got {:?}",
                m.chords
                    .iter()
                    .map(|c| format!("{}@{:.2}", c.full_symbol, beat_at(c)))
                    .collect::<Vec<_>>()
            );
        };

        // ── Measure 6 (the F#-G#-A-B 6/8 figure) ────────────────────────
        // The first chord (beat 1) carries forward from measure 5's
        // F#min7; the three explicit harmonies in xml measure 6 are
        // G#min7 at beat 2.5, Amaj7 at beat 4, B at beat 5.5.
        let m5 = m_by_num(5);
        assert!(
            m5.chords.is_empty(),
            "m5 late F#m7 harmony should only carry into m6, not render before the barline: {:?}",
            m5.chords
                .iter()
                .map(|c| format!("{}@{:.2}", c.full_symbol, beat_at(c)))
                .collect::<Vec<_>>()
        );

        let m6 = m_by_num(6);
        let symbols: Vec<&str> = m6.chords.iter().map(|c| c.full_symbol.as_str()).collect();
        let beats: Vec<f64> = m6.chords.iter().map(beat_at).collect();
        assert_eq!(
            m6.chords.len(),
            4,
            "measure 6 should have 4 chord positions, got {symbols:?} at {beats:?}"
        );
        let assert_chord = |idx: usize, want_sym: &str, want_beat: f64| {
            let got = &m6.chords[idx];
            assert_eq!(
                got.full_symbol, want_sym,
                "m6 chord {idx} symbol: want {want_sym}, got {} (all: {symbols:?})",
                got.full_symbol
            );
            let b = beat_at(got);
            assert!(
                approx(b, want_beat),
                "m6 chord {idx} ({want_sym}) beat: want {want_beat}, got {b} (all: {beats:?})"
            );
        };
        assert_chord(0, "F#m7", 1.0);
        assert_chord(1, "G#m7", 2.5);
        assert_chord(2, "Amaj7", 4.0);
        assert_chord(3, "B", 5.5);
        assert_chord_at(6, 1.0, "F#m7");
        assert_chord_at(6, 5.5, "B");
        assert_eq!(m6.source_measure_width, Some(266.0));

        let m6_notes: Vec<&keyflow_proto::chart::melody::MelodyNote> = m6
            .melodies
            .iter()
            .flat_map(|mel| mel.notes.iter())
            .collect();
        let mut note_beat = 1.0;
        let beat_unit_scale = f64::from(m6.time_signature.1) / 4.0;
        let note_starts = m6_notes
            .iter()
            .map(|note| {
                let start = note_beat;
                note_beat += note.duration_beats() * beat_unit_scale;
                (start, note)
            })
            .collect::<Vec<_>>();
        assert_eq!(note_starts.len(), 4, "m6 should have four melody attacks");
        for ((want_beat, want_pitch, want_extra), (got_beat, note)) in [
            (1.0, "F#", ("C#".to_string(), Some(4))),
            (2.5, "G#", ("D#".to_string(), Some(4))),
            (4.0, "A", ("E".to_string(), Some(4))),
            (5.5, "B", ("F#".to_string(), Some(4))),
        ]
        .into_iter()
        .zip(note_starts.iter())
        {
            assert!(
                approx(want_beat, *got_beat),
                "m6 melody attack should start at beat {want_beat}, got {got_beat}"
            );
            assert_eq!(note.pitch, want_pitch, "m6 note at beat {want_beat}");
            assert!(
                note.extra_pitches.contains(&want_extra),
                "m6 note at beat {want_beat} should include stacked pitch {:?}, got {:?}",
                want_extra,
                note.extra_pitches
            );
        }

        // ── Measures 7-10: vertical-slash chord progression ─────────────
        // m7: C#m @ beat 1, B/C# @ beat 4
        // m8: A/C# @ beat 1, G#min7/C# @ beat 4
        // m9: C#m @ beat 1, B/C# @ beat 4
        // m10: A/C# @ beat 1, G#min7/C# @ beat 4
        // (User said m7-m9 share the same shape; m10 differs in melody.)
        for (n, want_a, want_b) in [
            (7u32, "C#m", "B/C#"),
            (8, "A/C#", "G#m7/C#"),
            (9, "C#m", "B/C#"),
            (10, "A/C#", "G#m7"),
        ] {
            let m = m_by_num(n);
            let syms: Vec<&str> = m.chords.iter().map(|c| c.full_symbol.as_str()).collect();
            assert_eq!(
                m.chords.len(),
                2,
                "measure {n} should have 2 chords, got {syms:?}"
            );
            assert_eq!(m.chords[0].full_symbol, want_a, "m{n} beat-1 chord");
            assert_eq!(m.chords[1].full_symbol, want_b, "m{n} beat-4 chord");
            assert!(
                approx(beat_at(&m.chords[0]), 1.0),
                "m{n} first chord should be at beat 1, got {:.2}",
                beat_at(&m.chords[0])
            );
            assert!(
                approx(beat_at(&m.chords[1]), 4.0),
                "m{n} second chord should be at beat 4, got {:.2}",
                beat_at(&m.chords[1])
            );
        }
        // The two B/C#-family chords ride a vertical bass.
        let m7 = m_by_num(7);
        assert!(
            m7.chords[1].parsed.bass_vertical,
            "m7 B/C# should be vertical-bass slash"
        );

        // ── Melody ties across m7..m9 + m10 dotted-quarter pair ─────────
        // Each of m7, m8, m9 holds a single dotted-half melody note that
        // ties into the next measure. m10 has C# dotted-quarter then G#
        // dotted-quarter with NO tie out.
        fn melody_notes(m: &Measure) -> Vec<&keyflow_proto::chart::melody::MelodyNote> {
            m.melodies.iter().flat_map(|mel| mel.notes.iter()).collect()
        }
        for n in [7u32, 8, 9] {
            let m = m_by_num(n);
            let notes = melody_notes(m);
            assert_eq!(
                notes.len(),
                1,
                "m{n} should have exactly one melody note (dotted half), got {}",
                notes.len()
            );
            let note = notes[0];
            assert_eq!(note.duration, 2, "m{n} note should be a half");
            assert!(note.dotted, "m{n} note should be dotted");
            assert!(note.tie_start, "m{n} note should tie INTO m{}", n + 1);
        }
        // m10: C# dotted-quarter (tied stop from m9) then G# dotted-quarter
        // with no outgoing tie.
        let m10 = m_by_num(10);
        let m10_notes = melody_notes(m10);
        assert_eq!(m10_notes.len(), 2, "m10 should have 2 melody notes");
        assert_eq!(m10_notes[0].pitch, "C#", "m10 note 1 pitch");
        assert_eq!(m10_notes[0].duration, 4, "m10 note 1 should be a quarter");
        assert!(m10_notes[0].dotted, "m10 note 1 should be dotted");
        assert!(m10_notes[0].tie_stop, "m10 note 1 should be tied FROM m9");
        assert_eq!(m10_notes[1].pitch, "G#", "m10 note 2 pitch");
        assert_eq!(m10_notes[1].duration, 4, "m10 note 2 should be a quarter");
        assert!(m10_notes[1].dotted, "m10 note 2 should be dotted");
        assert!(!m10_notes[1].tie_start, "m10 note 2 should NOT tie out");

        // ── Written rests vs explicit rhythm-slash regions ─────────────
        // m3 and m4 contain real MusicXML rests and should stay rests. The
        // VS1 slash region begins at m11 via <measure-style><slash>.
        for n in [3u32, 4] {
            let m = m_by_num(n);
            assert!(
                m.chords.is_empty(),
                "m{n} is a written-rest measure and should not be backfilled with slash chords"
            );
            let notes = melody_notes(m);
            assert_eq!(notes.len(), 1, "m{n} should carry one written rest");
            assert_eq!(notes[0].pitch, "r", "m{n} note should be a rest");
        }

        use keyflow_proto::chord::ChordRhythm;
        for n in [11u32, 12, 13, 14, 15] {
            let m = m_by_num(n);
            assert!(
                m.melodies.iter().all(|mel| mel.notes.is_empty()),
                "m{n} should not have any rendered melody notes (rest-only got through)"
            );
            assert!(
                !m.chords.is_empty(),
                "m{n} should carry at least one slash-rhythm chord, got none"
            );
            match m.chords[0].rhythm {
                ChordRhythm::Slashes { count, dotted, .. } => {
                    assert_eq!(count, 2, "m{n} slash count: want 2 (6/8 dotted pair)");
                    assert!(dotted, "m{n} slashes must be dotted (compound 6/8)");
                }
                ref other => panic!("m{n} rhythm should be Slashes, got {other:?}"),
            }
        }
        let m16 = m_by_num(16);
        assert!(
            m16.melodies.iter().all(|mel| mel.notes.is_empty()),
            "m16 should not have any rendered melody notes"
        );
        let m16_slash_counts = m16
            .chords
            .iter()
            .map(|chord| match chord.rhythm {
                ChordRhythm::Slashes { count, dotted, .. } => {
                    (chord.full_symbol.as_str(), count, dotted)
                }
                ref other => panic!(
                    "m16 chord {} rhythm should be Slashes, got {other:?}",
                    chord.full_symbol
                ),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            m16_slash_counts,
            vec![("F#m7", 1, true), ("G#m7", 1, true)],
            "m16 has two half-measure harmonies and should render as /. /., not /. /. /. /."
        );
        let m17 = m_by_num(17);
        assert!(
            m17.chords.iter().any(|c| c.full_symbol == "C#m"),
            "m17 should leave slash mode and show C#m"
        );
    }
}
