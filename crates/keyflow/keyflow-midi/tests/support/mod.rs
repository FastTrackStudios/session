use keyflow_midi::import::{MidiFile, normalize_chord_name};
use keyflow_midi::proto::chord::{
    Chord, ChordFamily, ChordQuality, DetectedChord, MidiNote as KeyflowMidiNote,
    detect_chords_from_midi_notes, midi_pitch_to_note_name,
};
use keyflow_midi::proto::sections::SectionType;

#[derive(Debug)]
struct MarkerMismatch {
    marker_index: usize,
    raw_marker_index: Option<usize>,
    marker_tick: u32,
    marker_position: String,
    marker_name: String,
    marker_kind: String,
    parsed_text_marker: String,
    parsed_section: Option<String>,
    parsed_marker_chord: Option<String>,
    midi_notes_at_position: Vec<String>,
    detected_tick: Option<i64>,
    detected_position: Option<String>,
    detected_name: Option<String>,
    detected_prev: Option<String>,
    detected_next: Option<String>,
    raw_events_same_tick: Vec<String>,
    raw_events_context: Vec<String>,
    reason: String,
}

pub fn assert_marker_chords_match(midi_relative_path: &str) {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let midi_path = crate_root.join(midi_relative_path);

    let bytes = std::fs::read(&midi_path)
        .unwrap_or_else(|e| panic!("Failed to read MIDI file '{}': {e}", midi_path.display()));
    let midi = MidiFile::parse(&bytes)
        .unwrap_or_else(|e| panic!("Failed to parse MIDI file '{}': {e}", midi_path.display()));

    let ppq = midi.ppq();
    let raw_markers = midi.markers();
    let markers = midi.chord_markers_absolute();
    assert!(
        !markers.is_empty(),
        "No chord markers found in '{}'",
        midi_path.display()
    );

    let all_notes = midi.all_notes();
    let raw_notes: Vec<KeyflowMidiNote> = all_notes
        .iter()
        .map(|n| {
            KeyflowMidiNote::new(
                n.pitch,
                n.start_tick as i64,
                (n.start_tick + n.duration_ticks) as i64,
                n.channel,
                n.velocity,
            )
        })
        .collect();
    let notes: Vec<KeyflowMidiNote> = raw_notes
        .iter()
        .filter_map(|n| quantize_note_for_detection(n, ppq))
        .collect();

    let min_chord_duration = (ppq / 8).max(1) as i64;
    let detected = detect_chords_from_midi_notes(&notes, min_chord_duration);
    assert!(
        !detected.is_empty(),
        "No chords were detected from MIDI notes in '{}'",
        midi_path.display()
    );

    // Require alignment tighter than a 32nd note and compare on a 16th-note grid.
    let tolerance = ((ppq / 8).saturating_sub(1)).max(1) as i64;
    let context_window = (ppq / 8).max(1);
    let mut mismatches = Vec::new();

    for (idx, marker) in markers.iter().enumerate() {
        let parsed_marker = parse_marker_text(&marker.chord_name);
        let raw_marker_index = raw_markers
            .iter()
            .position(|m| m.tick == marker.tick && m.text == marker.chord_name);
        let midi_notes_at_position = notes_at_tick(&midi, &all_notes, marker.tick, context_window);
        let local_detected = detect_chord_at_marker(&notes, marker.tick, ppq);
        let nearest_global = nearest_detected_chord(&detected, marker.tick, ppq);
        let nearest = local_detected.as_ref().or(nearest_global);

        if parsed_marker.marker_kind == "section" {
            mismatches.push(build_mismatch(
                &midi,
                raw_markers,
                &detected,
                idx,
                raw_marker_index,
                marker.tick,
                &marker.chord_name,
                &parsed_marker,
                midi_notes_at_position,
                nearest,
                "Marker is classified as a section, not a chord",
            ));
            continue;
        }

        let expected = match parsed_marker.parsed_chord.as_ref() {
            Some(chord) => chord,
            None => {
                mismatches.push(build_mismatch(
                    &midi,
                    raw_markers,
                    &detected,
                    idx,
                    raw_marker_index,
                    marker.tick,
                    &marker.chord_name,
                    &parsed_marker,
                    midi_notes_at_position,
                    nearest,
                    "Failed to parse marker text as chord",
                ));
                continue;
            }
        };

        let Some(actual) = nearest else {
            mismatches.push(build_mismatch(
                &midi,
                raw_markers,
                &detected,
                idx,
                raw_marker_index,
                marker.tick,
                &marker.chord_name,
                &parsed_marker,
                midi_notes_at_position,
                None,
                "No detected chords available near marker",
            ));
            continue;
        };

        let marker_tick = i64::from(marker.tick);
        let marker_quantized = quantize_to_sixteenth(marker_tick, ppq);
        let actual_quantized = quantize_to_sixteenth(actual.start_ppq, ppq);
        let tick_diff = (actual_quantized - marker_quantized).abs();
        if tick_diff > tolerance {
            mismatches.push(build_mismatch(
                &midi,
                raw_markers,
                &detected,
                idx,
                raw_marker_index,
                marker.tick,
                &marker.chord_name,
                &parsed_marker,
                midi_notes_at_position,
                Some(actual),
                &format!(
                    "No nearby detected chord within tolerance (diff={tick_diff}, tolerance={tolerance})"
                ),
            ));
            continue;
        }

        if !chords_equivalent(expected, &actual.chord) {
            mismatches.push(build_mismatch(
                &midi,
                raw_markers,
                &detected,
                idx,
                raw_marker_index,
                marker.tick,
                &marker.chord_name,
                &parsed_marker,
                midi_notes_at_position,
                Some(actual),
                &format!(
                    "Parsed marker chord '{}' does not match detected chord '{}'",
                    expected.normalized, actual.chord.normalized
                ),
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} marker mismatches in '{}':\n{}",
        mismatches.len(),
        midi_path.display(),
        format_mismatches(&mismatches)
    );
}

fn chords_equivalent(expected: &Chord, detected: &Chord) -> bool {
    if normalized_for_compare(&expected.normalized) == normalized_for_compare(&detected.normalized)
    {
        return true;
    }

    if diminished_family(expected).is_some() || diminished_family(detected).is_some() {
        return false;
    }

    // Treat plain triads as compatible with richer voicings/extensions
    // when the harmonic root/bass and basic quality are the same.
    if is_plain_triad(expected)
        && root_semitone(expected) == root_semitone(detected)
        && bass_semitone(expected) == bass_semitone(detected)
        && expected.quality == detected.quality
    {
        return true;
    }

    // Fixture MIDI can contain slash-chord marker text like Dm7/C while the
    // rendered notes only contain C as the bass note. Under recognition rules,
    // that is Dm/C: the seventh is not an upper chord tone unless it appears
    // above the bass too.
    if expected.family.is_some()
        && detected.family.is_none()
        && root_semitone(expected) == root_semitone(detected)
        && bass_semitone(expected) == bass_semitone(detected)
        && expected.quality == detected.quality
        && expected.additions == detected.additions
        && expected.alterations == detected.alterations
        && expected.extensions == detected.extensions
        && bass_matches_family_seventh(expected)
    {
        return true;
    }

    // Some hand-authored markers name a sparse upper structure from its
    // functional dominant root, while detection names the exact played upper
    // structure. Example: A11/G over the notes G-D-A-D-E is Dsus4add9/G after
    // removing the bass-only seventh.
    if expected.bass.is_some()
        && expected.extensions.eleventh.is_some()
        && bass_semitone(expected) == bass_semitone(detected)
        && absolute_pitch_classes(detected)
            .iter()
            .all(|pc| absolute_pitch_classes(expected).contains(pc))
    {
        return true;
    }

    if is_major_add9_over_third(expected)
        && is_root_position_minor7_sharp5(detected)
        && absolute_pitch_classes(expected) == absolute_pitch_classes(detected)
    {
        return true;
    }

    expected.pitch_classes() == detected.pitch_classes()
        && root_semitone(expected) == root_semitone(detected)
        && bass_semitone(expected) == bass_semitone(detected)
}

fn diminished_family(chord: &Chord) -> Option<ChordFamily> {
    match chord.family {
        Some(ChordFamily::HalfDiminished | ChordFamily::FullyDiminished) => chord.family,
        _ => None,
    }
}

fn is_major_add9_over_third(chord: &Chord) -> bool {
    let Some(root) = root_semitone(chord) else {
        return false;
    };
    let Some(bass) = bass_semitone(chord) else {
        return false;
    };

    chord.quality == ChordQuality::Major
        && chord.family.is_none()
        && chord
            .additions
            .contains(&keyflow_midi::proto::chord::ChordDegree::Ninth)
        && bass == (root + 4) % 12
}

fn is_root_position_minor7_sharp5(chord: &Chord) -> bool {
    chord.quality == ChordQuality::Minor
        && chord.family == Some(ChordFamily::Minor7)
        && chord.bass.is_none()
        && chord.alterations.len() == 1
        && chord.alterations[0].degree == keyflow_midi::proto::chord::ChordDegree::Fifth
}

fn bass_matches_family_seventh(chord: &Chord) -> bool {
    let Some(root) = root_semitone(chord) else {
        return false;
    };
    let Some(bass) = bass_semitone(chord) else {
        return false;
    };
    let Some(family) = chord.family else {
        return false;
    };

    let interval = match family {
        ChordFamily::Major7 | ChordFamily::MinorMajor7 => 11,
        ChordFamily::Dominant7 | ChordFamily::Minor7 | ChordFamily::HalfDiminished => 10,
        ChordFamily::FullyDiminished => 9,
    };

    bass == (root + interval) % 12
}

fn absolute_pitch_classes(chord: &Chord) -> Vec<u8> {
    let Some(root) = root_semitone(chord) else {
        return Vec::new();
    };

    let mut pitch_classes: Vec<u8> = chord
        .pitch_classes()
        .into_iter()
        .map(|pc| (root + pc) % 12)
        .collect();
    pitch_classes.sort_unstable();
    pitch_classes.dedup();
    pitch_classes
}

fn is_plain_triad(chord: &Chord) -> bool {
    matches!(
        chord.quality,
        ChordQuality::Major
            | ChordQuality::Minor
            | ChordQuality::Diminished
            | ChordQuality::Augmented
            | ChordQuality::Suspended(_)
            | ChordQuality::Power
    ) && chord.family.is_none()
        && chord.additions.is_empty()
        && chord.alterations.is_empty()
        && chord.extensions.ninth.is_none()
        && chord.extensions.eleventh.is_none()
        && chord.extensions.thirteenth.is_none()
}

fn normalized_for_compare(chord: &str) -> String {
    chord.replace("maj", "").replace("Maj", "")
}

fn root_semitone(chord: &Chord) -> Option<u8> {
    chord.root.resolved_note().map(|note| note.semitone)
}

fn bass_semitone(chord: &Chord) -> Option<u8> {
    chord
        .bass
        .as_ref()
        .and_then(|bass| bass.resolved_note())
        .map(|note| note.semitone)
}

fn format_mismatches(mismatches: &[MarkerMismatch]) -> String {
    let lines: Vec<String> = mismatches
        .iter()
        .take(50)
        .map(|m| {
            format!(
                "#{} raw={} tick={}\n  position={}\n  original_marker='{}'\n  parsed_text_marker='{}'\n  marker_kind={}\n  parsed_section={}\n  parsed_marker_chord={}\n  midi_notes_at_position={}\n  parsed_midi_chord_tick={:?}\n  parsed_midi_chord_position={}\n  parsed_midi_chord={}\n  prev_parsed_midi_chord={}\n  next_parsed_midi_chord={}\n  raw_midi_events_same_tick={}\n  raw_midi_events_context={}\n  reason={}",
                m.marker_index + 1,
                m.raw_marker_index
                    .map(|i| (i + 1).to_string())
                    .unwrap_or_else(|| "?".to_string()),
                m.marker_tick,
                m.marker_position,
                m.marker_name,
                m.parsed_text_marker,
                m.marker_kind,
                m.parsed_section
                    .as_deref()
                    .unwrap_or("None"),
                m.parsed_marker_chord
                    .as_deref()
                    .unwrap_or("None"),
                if m.midi_notes_at_position.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", m.midi_notes_at_position.join(", "))
                },
                m.detected_tick,
                m.detected_position.as_deref().unwrap_or("None"),
                m.detected_name.as_deref().unwrap_or("None"),
                m.detected_prev.as_deref().unwrap_or("None"),
                m.detected_next.as_deref().unwrap_or("None"),
                if m.raw_events_same_tick.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", m.raw_events_same_tick.join(", "))
                },
                if m.raw_events_context.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", m.raw_events_context.join(", "))
                },
                m.reason
            )
        })
        .collect();

    if mismatches.len() > 50 {
        format!(
            "{}\n... {} more mismatches omitted",
            lines.join("\n"),
            mismatches.len() - 50
        )
    } else {
        lines.join("\n")
    }
}

#[derive(Debug)]
struct ParsedMarker {
    normalized: String,
    marker_kind: String,
    parsed_text_marker: String,
    parsed_section: Option<String>,
    parsed_chord: Option<Chord>,
}

fn parse_marker_text(marker_text: &str) -> ParsedMarker {
    let normalized = normalize_chord_name(marker_text.trim());
    let parsed_section = parse_section_marker(marker_text);
    let parsed_chord = keyflow_text::api::parse::chord(&normalized).ok();

    let marker_kind = match (parsed_section.is_some(), parsed_chord.is_some()) {
        (true, true) => "ambiguous",
        (true, false) => "section",
        (false, true) => "chord",
        (false, false) => "unknown",
    }
    .to_string();

    let parsed_text_marker = match (&parsed_section, &parsed_chord) {
        (Some(section), Some(chord)) => format!("{section} | {}", chord.normalized),
        (Some(section), None) => section.clone(),
        (None, Some(chord)) => chord.normalized.clone(),
        (None, None) => normalized.clone(),
    };

    ParsedMarker {
        normalized,
        marker_kind,
        parsed_text_marker,
        parsed_section,
        parsed_chord,
    }
}

fn parse_section_marker(marker_text: &str) -> Option<String> {
    let trimmed = marker_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let upper = trimmed.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "SONGSTART" | "SONGEND" | "ENDING" | "END" | "COUNT-IN" | "COUNT IN"
    ) {
        return Some(format!("SpecialSection({upper})"));
    }

    if let Some(parsed) = SectionType::parse_with_measure_count(trimmed) {
        return Some(format!("{:?}", parsed.section_type));
    }

    let leading_token = trimmed.split_whitespace().next().unwrap_or(trimmed);
    SectionType::parse(leading_token)
        .ok()
        .map(|s| format!("{:?}", s))
}

fn notes_at_tick(
    midi: &MidiFile,
    notes: &[keyflow_midi::import::MidiNote],
    tick: u32,
    window: u32,
) -> Vec<String> {
    let mut active: Vec<_> = notes
        .iter()
        .filter(|n| {
            let note_end = n.start_tick.saturating_add(n.duration_ticks);
            n.start_tick <= tick && tick < note_end
        })
        .collect();

    active.sort_by_key(|n| n.pitch);
    if !active.is_empty() {
        return active
            .iter()
            .take(24)
            .map(|n| {
                let start_pos = format_position(midi.tick_to_absolute_measure(n.start_tick));
                let end_pos = format_position(
                    midi.tick_to_absolute_measure(n.start_tick.saturating_add(n.duration_ticks)),
                );
                format!(
                    "{}(p{} ch{} v{} {}..{} @ {}->{})",
                    midi_pitch_to_note_name(n.pitch),
                    n.pitch,
                    n.channel,
                    n.velocity,
                    n.start_tick,
                    n.start_tick.saturating_add(n.duration_ticks),
                    start_pos,
                    end_pos
                )
            })
            .collect();
    }

    let mut nearby: Vec<_> = notes
        .iter()
        .filter_map(|n| {
            let note_end = n.start_tick.saturating_add(n.duration_ticks);
            let dist_start = i64::from(n.start_tick).abs_diff(i64::from(tick));
            let dist_end = i64::from(note_end).abs_diff(i64::from(tick));
            let dist = dist_start.min(dist_end);
            (dist <= u64::from(window)).then_some((dist, n))
        })
        .collect();
    nearby.sort_by_key(|(dist, n)| (*dist, n.pitch));

    nearby
        .iter()
        .take(24)
        .map(|(dist, n)| {
            let start_pos = format_position(midi.tick_to_absolute_measure(n.start_tick));
            let end_pos = format_position(
                midi.tick_to_absolute_measure(n.start_tick.saturating_add(n.duration_ticks)),
            );
            format!(
                "~{}:{}(p{} ch{} v{} {}..{} @ {}->{})",
                dist,
                midi_pitch_to_note_name(n.pitch),
                n.pitch,
                n.channel,
                n.velocity,
                n.start_tick,
                n.start_tick.saturating_add(n.duration_ticks),
                start_pos,
                end_pos
            )
        })
        .collect()
}

fn nearest_detected_chord(
    detected: &[DetectedChord],
    marker_tick: u32,
    ppq: u32,
) -> Option<&DetectedChord> {
    let marker_tick = i64::from(marker_tick);
    let marker_quantized = quantize_to_sixteenth(marker_tick, ppq);

    // Prefer a chord that is active at the quantized marker position.
    let mut active_at_marker = detected.iter().filter(|chord| {
        let start_q = quantize_to_sixteenth(chord.start_ppq, ppq);
        let end_q = quantize_to_sixteenth(chord.end_ppq, ppq).max(start_q + 1);
        start_q <= marker_quantized && marker_quantized < end_q
    });

    if let Some(chord) = active_at_marker.next() {
        return Some(chord);
    }

    detected.iter().min_by_key(|chord| {
        let start_q = quantize_to_sixteenth(chord.start_ppq, ppq);
        (start_q - marker_quantized).abs()
    })
}

fn quantize_to_sixteenth(tick: i64, ppq: u32) -> i64 {
    let grid = (ppq / 4).max(1) as i64;
    ((tick + grid / 2) / grid) * grid
}

fn quantize_note_for_detection(note: &KeyflowMidiNote, ppq: u32) -> Option<KeyflowMidiNote> {
    let start = quantize_to_sixteenth(note.start_ppq, ppq);
    let mut end = quantize_to_sixteenth(note.end_ppq, ppq);
    if end <= start {
        end = start + 1;
    }
    (end > start).then_some(KeyflowMidiNote::new(
        note.pitch,
        start,
        end,
        note.channel,
        note.velocity,
    ))
}

fn detect_chord_at_marker(
    notes: &[KeyflowMidiNote],
    marker_tick: u32,
    ppq: u32,
) -> Option<DetectedChord> {
    let marker = i64::from(marker_tick);
    // Small forward probe: treat notes that end just after the marker as ended,
    // and notes that start just after the marker as active.
    let boundary_tolerance = ((ppq / 8).saturating_sub(1)).max(1) as i64;
    let probe_tick = marker + boundary_tolerance;

    let active_notes: Vec<KeyflowMidiNote> = notes
        .iter()
        .filter(|n| n.start_ppq <= probe_tick && n.end_ppq > probe_tick)
        .copied()
        .collect();

    if active_notes.len() < 2 {
        return None;
    }

    let mut detected = detect_chords_from_midi_notes(&active_notes, 1);
    detected
        .drain(..)
        .min_by_key(|chord| (chord.start_ppq - marker).abs())
}

#[allow(clippy::too_many_arguments)]
fn build_mismatch(
    midi: &MidiFile,
    raw_markers: &[keyflow_midi::import::MarkerEvent],
    detected: &[DetectedChord],
    marker_index: usize,
    raw_marker_index: Option<usize>,
    marker_tick: u32,
    marker_name: &str,
    parsed_marker: &ParsedMarker,
    midi_notes_at_position: Vec<String>,
    nearest: Option<&DetectedChord>,
    reason: &str,
) -> MarkerMismatch {
    let marker_position = format_position(midi.tick_to_absolute_measure(marker_tick));
    let detected_position = nearest
        .and_then(|c| u32::try_from(c.start_ppq).ok())
        .map(|tick| format_position(midi.tick_to_absolute_measure(tick)));
    let detected_prev = nearest
        .and_then(|c| previous_detected_chord(detected, c.start_ppq))
        .map(|c| format_detected(midi, c));
    let detected_next = nearest
        .and_then(|c| next_detected_chord(detected, c.start_ppq))
        .map(|c| format_detected(midi, c));
    let raw_events_same_tick = raw_markers
        .iter()
        .filter(|m| m.tick == marker_tick)
        .map(format_raw_marker_event)
        .collect();
    let raw_events_context = raw_marker_context(raw_markers, raw_marker_index, marker_tick, 1);

    MarkerMismatch {
        marker_index,
        raw_marker_index,
        marker_tick,
        marker_position,
        marker_name: marker_name.to_string(),
        marker_kind: parsed_marker.marker_kind.clone(),
        parsed_text_marker: parsed_marker.parsed_text_marker.clone(),
        parsed_section: parsed_marker.parsed_section.clone(),
        parsed_marker_chord: parsed_marker
            .parsed_chord
            .as_ref()
            .map(|c| c.normalized.clone()),
        midi_notes_at_position,
        detected_tick: nearest.map(|c| c.start_ppq),
        detected_position,
        detected_name: nearest.map(|c| c.chord.normalized.clone()),
        detected_prev,
        detected_next,
        raw_events_same_tick,
        raw_events_context,
        reason: format!("{} (normalized='{}')", reason, parsed_marker.normalized),
    }
}

fn previous_detected_chord(detected: &[DetectedChord], tick: i64) -> Option<&DetectedChord> {
    detected.iter().rev().find(|c| c.start_ppq < tick)
}

fn next_detected_chord(detected: &[DetectedChord], tick: i64) -> Option<&DetectedChord> {
    detected.iter().find(|c| c.start_ppq > tick)
}

fn format_detected(midi: &MidiFile, chord: &DetectedChord) -> String {
    let pos = u32::try_from(chord.start_ppq)
        .ok()
        .map(|t| format_position(midi.tick_to_absolute_measure(t)))
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "{} @ {} (tick {})",
        chord.chord.normalized, pos, chord.start_ppq
    )
}

fn format_raw_marker_event(event: &keyflow_midi::import::MarkerEvent) -> String {
    format!("{:?}:'{}'@{}", event.marker_type, event.text, event.tick)
}

fn raw_marker_context(
    raw_markers: &[keyflow_midi::import::MarkerEvent],
    raw_marker_index: Option<usize>,
    marker_tick: u32,
    radius: usize,
) -> Vec<String> {
    if let Some(index) = raw_marker_index {
        let start = index.saturating_sub(radius);
        let end = (index + radius + 1).min(raw_markers.len());
        return raw_markers[start..end]
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let absolute = start + i + 1;
                format!("#{} {}", absolute, format_raw_marker_event(m))
            })
            .collect();
    }

    raw_markers
        .iter()
        .filter(|m| m.tick == marker_tick)
        .map(format_raw_marker_event)
        .collect()
}

fn format_position(pos: keyflow_midi::import::MusicalPosition) -> String {
    format!("{}.{}.{}", pos.measure, pos.beat + 1, pos.subdivision)
}
