//! MusicXML-to-MIDI harmony comparison harness.
//!
//! This test is ignored by default because it depends on the local, git-ignored
//! Wikifonia corpus under `reference-data/`. It compares MusicXML `<harmony>`
//! symbols against Keyflow's detected chords from MuseScore-exported MIDI.

use std::path::{Path, PathBuf};

use keyflow_midi::import::{MidiFile, normalize_chord_name};
use keyflow_midi::proto::chord::{
    Chord, DetectedChord, MidiNote as KeyflowMidiNote, detect_chords_from_midi_notes_with_spelling,
};
use keyflow_midi::proto::key::KeySpelling;
use keyflow_midi::proto::primitives::MusicalNote;

const SAMPLE_LIMIT: usize = 25;

#[test]
#[ignore = "local reference-data corpus diagnostic"]
fn wikifonia_musicxml_harmony_matches_musescore_midi_detection() {
    let corpus = corpus_root();
    let xml_dir = corpus.join("xml");
    let midi_dir = corpus.join("midi");

    if !xml_dir.exists() || !midi_dir.exists() {
        eprintln!(
            "missing local Wikifonia corpus at {}; skipping",
            corpus.display()
        );
        return;
    }

    let mut checked = 0usize;
    let mut total_expected = 0usize;
    let mut total_matched = 0usize;
    let mut mismatches = Vec::new();

    for xml_path in collect_files(&xml_dir, "mxl")
        .into_iter()
        .take(SAMPLE_LIMIT)
    {
        let rel = xml_path.strip_prefix(&xml_dir).unwrap();
        let midi_path = midi_dir.join(rel).with_extension("mid");
        if !midi_path.exists() {
            continue;
        }

        let report = compare_pair(&xml_path, &midi_path);
        checked += 1;
        total_expected += report.expected_count;
        total_matched += report.matched_count;
        mismatches.extend(report.mismatches);
    }

    assert!(checked > 0, "no converted MusicXML/MIDI pairs found");

    let match_rate = if total_expected == 0 {
        0.0
    } else {
        total_matched as f64 / total_expected as f64
    };

    eprintln!(
        "MusicXML/MIDI harmony report: checked={checked}, matched={total_matched}/{total_expected} ({:.1}%).\n{}",
        match_rate * 100.0,
        mismatches
            .iter()
            .take(80)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn compare_pair(xml_path: &Path, midi_path: &Path) -> PairReport {
    let xml = read_mxl_score_xml(xml_path);
    let expected = parse_musicxml_harmonies(&xml);
    if expected.is_empty() {
        return PairReport::default();
    }

    let bytes = std::fs::read(midi_path).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", midi_path.display());
    });
    let midi = MidiFile::parse(&bytes).unwrap_or_else(|err| {
        panic!("failed to parse {}: {err}", midi_path.display());
    });
    let notes = harmony_channel_notes(&midi);
    let spelling = infer_spelling_context(&expected);
    let min_duration = (midi.ppq() / 8).max(1) as i64;
    let detected =
        detect_chords_from_midi_notes_with_spelling(&notes, min_duration, spelling.as_ref());

    let mut report = PairReport::default();

    for harmony in expected {
        if !has_harmony_midi_evidence(&notes, harmony.tick, midi.ppq()) {
            continue;
        }

        report.expected_count += 1;

        let Some(actual) = nearest_detected(&detected, harmony.tick, midi.ppq()) else {
            report.mismatches.push(format!(
                "{} tick={} expected={} actual=None",
                xml_path.display(),
                harmony.tick,
                harmony.symbol
            ));
            continue;
        };

        if chords_match(&harmony.chord, &actual.chord) {
            report.matched_count += 1;
        } else {
            report.mismatches.push(format!(
                "{} tick={} expected={} actual={} actual_tick={}",
                xml_path.display(),
                harmony.tick,
                harmony.chord.normalized,
                actual.chord.normalized,
                actual.start_ppq
            ));
        }
    }

    report
}

#[test]
#[ignore = "local reference-data corpus diagnostic"]
fn misty_musicxml_harmony_sample_report() {
    let xml_path = corpus_root().join("xml/Errol Garner, Johnny Burne - Misty.mxl");
    let midi_path = corpus_root().join("midi/Errol Garner, Johnny Burne - Misty.mid");
    assert!(xml_path.exists(), "missing {}", xml_path.display());
    assert!(midi_path.exists(), "missing {}", midi_path.display());

    let xml = read_mxl_score_xml(&xml_path);
    let expected = parse_musicxml_harmonies(&xml);
    let bytes = std::fs::read(&midi_path).expect("read Misty MIDI");
    let midi = MidiFile::parse(&bytes).expect("parse Misty MIDI");
    let notes = harmony_channel_notes(&midi);
    let spelling = infer_spelling_context(&expected);
    let detected = detect_chords_from_midi_notes_with_spelling(
        &notes,
        (midi.ppq() / 8).max(1) as i64,
        spelling.as_ref(),
    );

    eprintln!(
        "Misty: expected_harmonies={} harmony_channel_notes={} detected={}",
        expected.len(),
        notes.len(),
        detected.len()
    );
    eprintln!("tick\twritten\tdetected\tactual_tick");
    for harmony in expected.iter().take(36) {
        let actual = nearest_detected(&detected, harmony.tick, midi.ppq());
        eprintln!(
            "{}\t{}\t{}\t{}",
            harmony.tick,
            harmony.chord.normalized,
            actual
                .map(|chord| chord.chord.normalized.as_str())
                .unwrap_or("None"),
            actual
                .map(|chord| chord.start_ppq.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
    }
}

fn infer_spelling_context(expected: &[ExpectedHarmony]) -> Option<KeySpelling> {
    let mut flats = 0usize;
    let mut sharps = 0usize;

    for harmony in expected {
        if let Some(root) = harmony.chord.root.resolved_note() {
            flats += root.name.matches('b').count();
            sharps += root.name.matches('#').count();
        }
        if let Some(bass) = harmony
            .chord
            .bass
            .as_ref()
            .and_then(|bass| bass.resolved_note())
        {
            flats += bass.name.matches('b').count();
            sharps += bass.name.matches('#').count();
        }
    }

    let root_name = if flats > sharps {
        "F"
    } else if sharps > flats {
        "G"
    } else {
        "C"
    };

    MusicalNote::from_string(root_name).map(|root| KeySpelling::major(&root))
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference-data/wikifonia")
}

fn collect_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some(extension) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn read_mxl_score_xml(path: &Path) -> String {
    let output = std::process::Command::new("unzip")
        .arg("-p")
        .arg(path)
        .arg("musicXML.xml")
        .output()
        .unwrap_or_else(|err| panic!("failed to run unzip for {}: {err}", path.display()));
    assert!(
        output.status.success(),
        "failed to unzip {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|err| panic!("{} is not utf-8 MusicXML: {err}", path.display()))
}

fn parse_musicxml_harmonies(xml: &str) -> Vec<ExpectedHarmony> {
    let xml = strip_doctype(xml);
    let doc = roxmltree::Document::parse(&xml).expect("invalid MusicXML");
    let mut divisions = 1u32;
    let mut measures = Vec::new();

    for node in doc.descendants().filter(|n| n.has_tag_name("measure")) {
        let mut cursor = 0u32;
        let mut measure_end = 0u32;
        let mut harmonies = Vec::new();
        let mut repeat_forward = false;
        let mut repeat_backward = false;
        let mut endings = Vec::new();

        for child in node.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "attributes" => {
                    if let Some(value) = child
                        .children()
                        .find(|n| n.has_tag_name("divisions"))
                        .and_then(|n| n.text())
                        .and_then(|s| s.trim().parse::<u32>().ok())
                    {
                        divisions = value.max(1);
                    }
                }
                "note" | "forward" => {
                    if !child.children().any(|n| n.has_tag_name("chord")) {
                        cursor = cursor.saturating_add(duration_ticks(child, divisions));
                        measure_end = measure_end.max(cursor);
                    }
                }
                "backup" => {
                    cursor = cursor.saturating_sub(duration_ticks(child, divisions));
                }
                "harmony" => {
                    if let Some(symbol) = harmony_symbol(child)
                        && let Ok(chord) = keyflow_text::api::parse::chord(&symbol)
                    {
                        harmonies.push(MeasureHarmony {
                            offset: cursor,
                            symbol,
                            chord,
                        });
                    }
                }
                "barline" => {
                    for ending in child.children().filter(|n| n.has_tag_name("ending")) {
                        if let Some(number) = ending.attribute("number") {
                            endings.extend(
                                number
                                    .split(',')
                                    .map(str::trim)
                                    .filter(|number| !number.is_empty())
                                    .map(str::to_string),
                            );
                        }
                    }
                    for repeat in child.children().filter(|n| n.has_tag_name("repeat")) {
                        match repeat.attribute("direction") {
                            Some("forward") => repeat_forward = true,
                            Some("backward") => repeat_backward = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        measures.push(MeasureTimeline {
            duration: measure_end,
            harmonies,
            repeat_forward,
            repeat_backward,
            endings,
        });
    }

    expand_measure_timelines(&measures)
}

fn expand_measure_timelines(measures: &[MeasureTimeline]) -> Vec<ExpectedHarmony> {
    let mut expanded = Vec::new();
    let mut tick = 0u32;
    let mut repeat_start = 0usize;
    let mut index = 0usize;
    let mut repeated_once = vec![false; measures.len()];

    while index < measures.len() {
        let measure = &measures[index];

        if measure.endings.iter().any(|ending| ending == "1") && repeated_once[index] {
            index += 1;
            continue;
        }

        append_measure_harmonies(&mut expanded, measure, tick);
        tick = tick.saturating_add(measure.duration);

        if measure.repeat_forward {
            repeat_start = index;
        }

        if measure.repeat_backward && !repeated_once[index] {
            repeated_once[index] = true;
            index = repeat_start;
        } else {
            index += 1;
        }
    }

    expanded
}

fn append_measure_harmonies(
    expanded: &mut Vec<ExpectedHarmony>,
    measure: &MeasureTimeline,
    measure_tick: u32,
) {
    for harmony in &measure.harmonies {
        expanded.push(ExpectedHarmony {
            tick: measure_tick.saturating_add(harmony.offset),
            symbol: harmony.symbol.clone(),
            chord: harmony.chord.clone(),
        });
    }
}

#[derive(Clone)]
struct MeasureTimeline {
    duration: u32,
    harmonies: Vec<MeasureHarmony>,
    repeat_forward: bool,
    repeat_backward: bool,
    endings: Vec<String>,
}

#[derive(Clone)]
struct MeasureHarmony {
    offset: u32,
    symbol: String,
    chord: Chord,
}

fn strip_doctype(xml: &str) -> String {
    let Some(start) = xml.find("<!DOCTYPE") else {
        return xml.to_string();
    };
    let Some(end) = xml[start..].find('>') else {
        return xml.to_string();
    };

    let mut stripped = String::with_capacity(xml.len());
    stripped.push_str(&xml[..start]);
    stripped.push_str(&xml[start + end + 1..]);
    stripped
}

fn duration_ticks(node: roxmltree::Node<'_, '_>, divisions: u32) -> u32 {
    let duration = node
        .children()
        .find(|n| n.has_tag_name("duration"))
        .and_then(|n| n.text())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    duration.saturating_mul(480) / divisions.max(1)
}

fn harmony_symbol(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let root = node.children().find(|n| n.has_tag_name("root"))?;
    let root_step = child_text(root, "root-step")?;
    let root_alter = child_text(root, "root-alter").and_then(alter_suffix);
    let kind = node.children().find(|n| n.has_tag_name("kind"));
    let kind_text = kind.and_then(|n| n.attribute("text")).unwrap_or("");
    let kind_name = kind
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let bass = node.children().find(|n| n.has_tag_name("bass"));

    let mut symbol = format!(
        "{}{}{}",
        root_step.trim(),
        root_alter.unwrap_or_default(),
        if kind_text.is_empty() {
            musicxml_kind_suffix(kind_name)
        } else {
            kind_text
        }
    );

    if let Some(bass) = bass
        && let Some(step) = child_text(bass, "bass-step")
    {
        let alter = child_text(bass, "bass-alter").and_then(alter_suffix);
        symbol.push('/');
        symbol.push_str(step.trim());
        symbol.push_str(alter.unwrap_or_default());
    }

    Some(normalize_chord_name(&symbol))
}

fn child_text<'a>(node: roxmltree::Node<'a, 'a>, tag: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
}

fn alter_suffix(value: &str) -> Option<&'static str> {
    match value.trim() {
        "1" => Some("#"),
        "-1" => Some("b"),
        _ => None,
    }
}

fn musicxml_kind_suffix(kind: &str) -> &'static str {
    match kind {
        "" | "major" => "",
        "minor" => "m",
        "dominant" => "7",
        "dominant-ninth" => "9",
        "dominant-11th" => "11",
        "dominant-13th" => "13",
        "major-seventh" => "maj7",
        "major-sixth" => "6",
        "major-ninth" => "maj9",
        "major-11th" => "maj11",
        "major-13th" => "maj13",
        "minor-sixth" => "m6",
        "minor-seventh" => "m7",
        "minor-ninth" => "m9",
        "minor-11th" => "m11",
        "minor-13th" => "m13",
        "diminished" => "dim",
        "augmented" => "+",
        "augmented-seventh" => "+7",
        "half-diminished" => "m7b5",
        "diminished-seventh" => "dim7",
        "suspended-fourth" => "sus4",
        "suspended-second" => "sus2",
        _ => "",
    }
}

fn harmony_channel_notes(midi: &MidiFile) -> Vec<KeyflowMidiNote> {
    let harmony_channel = midi
        .tracks()
        .iter()
        .flat_map(|track| &track.notes)
        .fold(
            std::collections::BTreeMap::<u8, Vec<&keyflow_midi::import::MidiNote>>::new(),
            |mut channels, note| {
                channels.entry(note.channel).or_default().push(note);
                channels
            },
        )
        .into_iter()
        .max_by_key(|(_channel, notes)| harmony_channel_score(notes))
        .map(|(channel, _notes)| channel)
        .unwrap_or(0);

    midi.tracks()
        .iter()
        .flat_map(|track| &track.notes)
        .filter(|note| note.channel == harmony_channel)
        .map(|note| {
            KeyflowMidiNote::new(
                note.pitch,
                note.start_tick as i64,
                note.start_tick.saturating_add(note.duration_ticks) as i64,
                note.channel,
                note.velocity,
            )
        })
        .collect()
}

fn harmony_channel_score(notes: &[&keyflow_midi::import::MidiNote]) -> (usize, usize, u32) {
    let mut starts = std::collections::BTreeMap::<u32, usize>::new();
    for note in notes {
        *starts.entry(note.start_tick).or_default() += 1;
    }

    let chord_starts = starts
        .values()
        .filter(|simultaneous| **simultaneous >= 3)
        .count();
    let stacked_notes = starts
        .values()
        .filter(|simultaneous| **simultaneous >= 3)
        .sum();
    let max_duration = notes
        .iter()
        .map(|note| note.duration_ticks)
        .max()
        .unwrap_or(0);

    (chord_starts, stacked_notes, max_duration)
}

fn nearest_detected(
    detected: &[DetectedChord],
    expected_tick: u32,
    ppq: u32,
) -> Option<&DetectedChord> {
    let expected = i64::from(expected_tick);
    let tolerance = i64::from(ppq / 2);

    if let Some(active) = detected
        .iter()
        .find(|chord| chord.start_ppq <= expected && expected < chord.end_ppq)
    {
        return Some(active);
    }

    detected
        .iter()
        .min_by_key(|chord| (chord.start_ppq - expected).abs())
        .filter(|chord| (chord.start_ppq - expected).abs() <= tolerance)
}

fn has_harmony_midi_evidence(notes: &[KeyflowMidiNote], expected_tick: u32, ppq: u32) -> bool {
    let expected = i64::from(expected_tick);
    let tolerance = i64::from(ppq / 2);

    let nearby_start_count = notes
        .iter()
        .filter(|note| (note.start_ppq - expected).abs() <= tolerance)
        .count();
    nearby_start_count >= 2
}

fn chords_match(expected: &Chord, actual: &Chord) -> bool {
    if diminished_family(expected).is_some() || diminished_family(actual).is_some() {
        return expected.normalized == actual.normalized;
    }

    expected.normalized == actual.normalized
        || (expected.root.resolved_note().map(|n| n.semitone)
            == actual.root.resolved_note().map(|n| n.semitone)
            && expected.pitch_classes() == actual.pitch_classes())
}

fn diminished_family(chord: &Chord) -> Option<keyflow_midi::proto::chord::ChordFamily> {
    match chord.family {
        Some(
            keyflow_midi::proto::chord::ChordFamily::HalfDiminished
            | keyflow_midi::proto::chord::ChordFamily::FullyDiminished,
        ) => chord.family,
        _ => None,
    }
}

#[derive(Debug)]
struct ExpectedHarmony {
    tick: u32,
    symbol: String,
    chord: Chord,
}

#[derive(Default)]
struct PairReport {
    expected_count: usize,
    matched_count: usize,
    mismatches: Vec<String>,
}
