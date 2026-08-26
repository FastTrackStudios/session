#![cfg(feature = "midi-import")]
//! Test 026: RPP Import — Vienna (Just Friends)
//!
//! Parses a Reaper `.rpp` project file, converts the CHORDS track into a
//! `MidiFile`, generates Keyflow chart text, and optionally renders to PDF.

use std::fs;
use std::path::PathBuf;

use dawfile_reaper::{MidiEventType, MidiSourceEvent};
use engraver::export::pdf::PdfSerializer;
use engraver::export::svg::{SvgExportConfig, SvgSerializer};
use engraver::fonts::ChartFontBundle;
use engraver::import::{
    generate_chart_text, MarkerEvent, MarkerType, MidiChartConfig, MidiFile, MidiNote, MidiTrack,
    TempoEvent, TimeSignatureEvent,
};
use engraver::layout::chart::{ChartLayoutConfig, LayoutMode};
use engraver::style::MStyle;
use keyflow::chord::{detect_chords_from_midi_notes, MidiNote as KeyflowMidiNote};

fn output_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/output");
    fs::create_dir_all(&dir).expect("create output dir");
    dir
}

fn load_rpp() -> dawfile_reaper::ReaperProject {
    let rpp_text = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vienna_couch.rpp"
    ))
    .expect("read RPP fixture");
    dawfile_reaper::parse_project_text(&rpp_text).expect("parse RPP")
}

/// Convert an RPP project into a `MidiFile` suitable for the chart generator.
fn rpp_to_midi_file(project: &dawfile_reaper::ReaperProject) -> MidiFile {
    // --- Tempo ---
    let bpm = project
        .tempo_envelope
        .as_ref()
        .map(|te| te.default_tempo)
        .unwrap_or(120.0);
    let uspqn = (60_000_000.0 / bpm) as u32;
    let tempo_map = vec![TempoEvent {
        tick: 0,
        microseconds_per_quarter: uspqn,
    }];

    // --- Time signature ---
    let (num, den) = project
        .tempo_envelope
        .as_ref()
        .map(|te| te.default_time_signature)
        .unwrap_or((4, 4));
    let time_signatures = vec![TimeSignatureEvent {
        tick: 0,
        numerator: num as u8,
        denominator: den as u8,
    }];

    // --- Find CHORDS track and its item ---
    let chords_track = project
        .tracks
        .iter()
        .find(|t| t.name == "CHORDS")
        .expect("CHORDS track not found");

    let chords_item = chords_track
        .items
        .first()
        .expect("CHORDS track has no items");

    let midi_source = chords_item
        .takes
        .iter()
        .filter_map(|take| take.source.as_ref())
        .find_map(|src| src.midi_data.as_ref())
        .expect("no MIDI source in CHORDS track");

    let ppq = midi_source.ticks_per_qn;

    // The MIDI item starts at a position on the timeline (in seconds).
    // All MIDI ticks are relative to the item start, so we must offset them
    // to get absolute timeline positions that align with section markers.
    let item_offset_ticks = (chords_item.position * ppq as f64 * bpm / 60.0).round() as u32;

    // --- Extract notes and chord names from event_stream (preserves interleaved order) ---
    let mut notes: Vec<MidiNote> = Vec::new();
    let mut chord_markers: Vec<MarkerEvent> = Vec::new();
    let mut local_tick: u32 = 0;

    // Track note-on events: (pitch, channel) -> (local_tick, velocity)
    let mut pending_notes: std::collections::HashMap<(u8, u8), (u32, u8)> =
        std::collections::HashMap::new();

    for evt in &midi_source.event_stream {
        local_tick += evt.delta_ticks();
        let timeline_tick = local_tick + item_offset_ticks;

        match evt {
            MidiSourceEvent::Extended(ext) => {
                // fields: ["0", "0", "0", "0", "6", "Gm"]
                // Chord name is at index 5
                if ext.fields.len() > 5 {
                    let chord_name = ext.fields[5].clone();
                    chord_markers.push(MarkerEvent {
                        tick: timeline_tick,
                        text: chord_name,
                        marker_type: MarkerType::CuePoint,
                    });
                }
            }
            MidiSourceEvent::Midi(midi_evt) => {
                let channel = midi_evt.channel();
                match midi_evt.event_type() {
                    MidiEventType::NoteOn => {
                        let pitch = midi_evt.bytes[1];
                        let velocity = midi_evt.bytes[2];
                        if velocity > 0 {
                            pending_notes.insert((pitch, channel), (timeline_tick, velocity));
                        } else {
                            // velocity 0 note-on = note-off
                            if let Some((start, vel)) = pending_notes.remove(&(pitch, channel)) {
                                notes.push(MidiNote {
                                    pitch,
                                    velocity: vel,
                                    start_tick: start,
                                    duration_ticks: timeline_tick.saturating_sub(start),
                                    channel,
                                });
                            }
                        }
                    }
                    MidiEventType::NoteOff => {
                        let pitch = midi_evt.bytes[1];
                        if let Some((start, vel)) = pending_notes.remove(&(pitch, channel)) {
                            notes.push(MidiNote {
                                pitch,
                                velocity: vel,
                                start_tick: start,
                                duration_ticks: timeline_tick.saturating_sub(start),
                                channel,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    notes.sort_by_key(|n| (n.start_tick, n.pitch));

    let midi_track = MidiTrack {
        index: 0,
        name: Some("CHORDS".to_string()),
        notes,
        channel: Some(0),
    };

    // --- Extract LINES track for melody data ---
    let mut tracks = vec![midi_track];
    let mut track_names: Vec<Option<String>> = vec![Some("CHORDS".to_string())];

    if let Some(lines_track) = project.tracks.iter().find(|t| t.name == "LINES") {
        let mut line_notes: Vec<MidiNote> = Vec::new();

        // LINES track may have multiple items (one per section)
        for lines_item in &lines_track.items {
            let lines_item_offset = (lines_item.position * ppq as f64 * bpm / 60.0).round() as u32;

            if let Some(lines_midi) = lines_item
                .takes
                .iter()
                .filter_map(|take| take.source.as_ref())
                .find_map(|src| src.midi_data.as_ref())
            {
                let mut local_tick: u32 = 0;
                let mut pending: std::collections::HashMap<(u8, u8), (u32, u8)> =
                    std::collections::HashMap::new();

                for evt in &lines_midi.event_stream {
                    local_tick += evt.delta_ticks();
                    let timeline_tick = local_tick + lines_item_offset;

                    if let MidiSourceEvent::Midi(midi_evt) = evt {
                        let channel = midi_evt.channel();
                        match midi_evt.event_type() {
                            MidiEventType::NoteOn => {
                                let pitch = midi_evt.bytes[1];
                                let velocity = midi_evt.bytes[2];
                                if velocity > 0 {
                                    pending.insert((pitch, channel), (timeline_tick, velocity));
                                } else if let Some((start, vel)) = pending.remove(&(pitch, channel))
                                {
                                    line_notes.push(MidiNote {
                                        pitch,
                                        velocity: vel,
                                        start_tick: start,
                                        duration_ticks: timeline_tick.saturating_sub(start),
                                        channel: 1, // Use channel 1 for LINES
                                    });
                                }
                            }
                            MidiEventType::NoteOff => {
                                let pitch = midi_evt.bytes[1];
                                if let Some((start, vel)) = pending.remove(&(pitch, channel)) {
                                    line_notes.push(MidiNote {
                                        pitch,
                                        velocity: vel,
                                        start_tick: start,
                                        duration_ticks: timeline_tick.saturating_sub(start),
                                        channel: 1,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        line_notes.sort_by_key(|n| (n.start_tick, n.pitch));

        if !line_notes.is_empty() {
            tracks.push(MidiTrack {
                index: 1,
                name: Some("LINES".to_string()),
                notes: line_notes,
                channel: Some(1),
            });
            track_names.push(Some("LINES".to_string()));
        }
    }

    // --- Section markers (time-based → tick-based) ---
    let seconds_to_tick = |secs: f64| -> u32 { (secs * ppq as f64 * bpm / 60.0).round() as u32 };

    // Use all markers and regions (regions have section names like "Intro", "VS 1", etc.)
    let section_markers: Vec<MarkerEvent> = project
        .markers_regions
        .all
        .iter()
        .filter(|m| !m.name.is_empty())
        .map(|m| MarkerEvent {
            tick: seconds_to_tick(m.position),
            text: m.name.clone(),
            marker_type: MarkerType::Marker,
        })
        .collect();

    // Combine section markers and chord markers
    let mut all_markers = section_markers;
    all_markers.extend(chord_markers);
    all_markers.sort_by_key(|m| m.tick);

    // Extract swing from CFGEDIT field (index 11 = swing ratio)
    let swing = midi_source
        .cfg_edit
        .as_ref()
        .and_then(|fields| fields.get(11))
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|&v| (v - 0.5).abs() > 0.01); // None if straight

    MidiFile::from_parts(
        ppq,
        tracks,
        tempo_map,
        time_signatures,
        all_markers,
        track_names,
        swing,
    )
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_parse_vienna_rpp() {
    let project = load_rpp();

    // Verify basic project properties
    assert!(
        project.tempo_envelope.is_some(),
        "should have tempo envelope"
    );
    let te = project.tempo_envelope.as_ref().unwrap();
    assert!(
        (te.default_tempo - 140.0).abs() < 0.01,
        "tempo should be 140, got {}",
        te.default_tempo
    );
    assert_eq!(te.default_time_signature, (4, 4));

    // Verify tracks exist
    assert!(!project.tracks.is_empty(), "should have tracks");
    let chords = project.tracks.iter().find(|t| t.name == "CHORDS");
    assert!(chords.is_some(), "should have a CHORDS track");

    // Verify markers (sections are stored as regions in RPP)
    let marker_names: Vec<&str> = project
        .markers_regions
        .all
        .iter()
        .filter(|m| !m.name.is_empty())
        .map(|m| m.name.as_str())
        .collect();
    assert!(
        marker_names.contains(&"Intro"),
        "markers should include Intro"
    );
    assert!(
        marker_names.contains(&"VS 1"),
        "markers should include VS 1"
    );
    assert!(
        marker_names.contains(&"CH 1"),
        "markers should include CH 1"
    );
    assert!(
        marker_names.contains(&"Ending"),
        "markers should include Ending"
    );

    println!("Markers: {:?}", marker_names);
    println!(
        "Track names: {:?}",
        project
            .tracks
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_vienna_rpp_to_midi_file() {
    let project = load_rpp();
    let midi = rpp_to_midi_file(&project);

    assert_eq!(midi.ppq(), 960);
    assert!(
        midi.swing().is_some(),
        "should have swing value from CFGEDIT"
    );
    assert!(
        (midi.swing().unwrap() - 0.6667).abs() < 0.01,
        "swing should be triplet"
    );
    assert!((midi.initial_tempo() - 140.0).abs() < 0.01);
    assert_eq!(midi.initial_time_signature(), (4, 4));
    assert!(!midi.tracks().is_empty());

    let notes = midi.track_notes(0).expect("should have track 0 notes");
    assert!(!notes.is_empty(), "CHORDS track should have notes");
    println!("Total notes: {}", notes.len());

    let markers = midi.markers();
    assert!(!markers.is_empty(), "should have markers");
    println!("Total markers: {}", markers.len());

    // Verify chord markers are present
    let chord_markers: Vec<&MarkerEvent> = markers
        .iter()
        .filter(|m| m.marker_type == MarkerType::CuePoint)
        .collect();
    assert!(
        !chord_markers.is_empty(),
        "should have chord markers from extended events"
    );
    println!(
        "First 5 chord markers: {:?}",
        chord_markers
            .iter()
            .take(5)
            .map(|m| format!("{}@{}", m.text, m.tick))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_vienna_rpp_to_chart_text() {
    let project = load_rpp();
    let midi = rpp_to_midi_file(&project);

    let config = MidiChartConfig {
        key_root: Some("Bb".to_string()),
        title: Some("Vienna - Just Friends\nTranscribed By: Cody Wright".to_string()),
        swing: midi.swing(),
        ..Default::default()
    };
    let chart_text = generate_chart_text(&midi, &config);

    assert!(!chart_text.is_empty(), "chart text should not be empty");
    println!("{}", chart_text);

    // Write to output file
    let out = output_dir().join("vienna.kf");
    fs::write(&out, &chart_text).expect("write vienna.kf");
    println!("Wrote {}", out.display());
}

#[test]
fn test_vienna_rpp_to_pdf() {
    let project = load_rpp();
    let midi = rpp_to_midi_file(&project);

    let config = MidiChartConfig {
        key_root: Some("Bb".to_string()),
        title: Some("Vienna - Just Friends\nTranscribed By: Cody Wright".to_string()),
        swing: midi.swing(),
        ..Default::default()
    };
    let chart_text = generate_chart_text(&midi, &config);

    // Parse chart text into a Chart
    let chart = keyflow::text::chart::parse_chart(&chart_text).expect("parse chart text");

    // Layout engine
    let font_bundle = ChartFontBundle::new().expect("load fonts");
    let style: &'static MStyle = Box::leak(Box::new(MStyle::new()));
    let engine = font_bundle.create_layout_engine(style);

    let layout_config = ChartLayoutConfig::master_rhythm().with_page_offsets(true);
    let mode = LayoutMode::paginated_a4();
    let result = engine.layout_chart_with_config(&chart, &mode, &layout_config);

    // Export SVG pages
    let mut svg_pages = Vec::with_capacity(result.pages.len());
    for page in &result.pages {
        let svg_config =
            SvgExportConfig::for_page(page.x_offset, page.y_offset, page.width, page.height)
                .with_embedded_font("Bravura", font_bundle.symbol_font_data().as_ref().clone())
                .with_embedded_font(
                    "MuseJazzText",
                    font_bundle.text_font_data().as_ref().clone(),
                )
                .with_embedded_font("FreeSans", font_bundle.aux_font_data().as_ref().clone());
        let mut serializer = SvgSerializer::new(svg_config);
        svg_pages.push(serializer.serialize(&result.scene));
    }

    // SVG → PDF
    let pdf_bytes = PdfSerializer::serialize_from_svg(
        &svg_pages,
        &[
            ("Bravura", font_bundle.symbol_font_data().as_slice()),
            ("MuseJazzText", font_bundle.text_font_data().as_slice()),
            ("FreeSans", font_bundle.aux_font_data().as_slice()),
        ],
    )
    .expect("serialize PDF");

    let out = output_dir().join("vienna.pdf");
    fs::write(&out, &pdf_bytes).expect("write vienna.pdf");
    println!(
        "Wrote {} ({} pages, {} bytes)",
        out.display(),
        result.pages.len(),
        pdf_bytes.len()
    );
}

/// Diagnostic: test chord detection on specific voicings from the RPP.
#[test]
fn test_chord_detection_voicings() {
    fn detect(pitches: &[u8], label: &str) -> Vec<String> {
        let notes: Vec<KeyflowMidiNote> = pitches
            .iter()
            .map(|&p| KeyflowMidiNote::new(p, 0, 960, 0, 96))
            .collect();
        let chords = detect_chords_from_midi_notes(&notes, 100);
        let names: Vec<String> = chords.iter().map(|c| c.chord.to_string()).collect();
        println!("{label}: {pitches:?} -> {names:?}");
        names
    }

    // D4 G4 A4 C5 E5 — 5-note voicing of D-G-A-C-E. With no F# present this is
    // not literally a D11 (which needs the major 3rd); the detector names it by
    // its exact pitch set as Am7/D (A-C-E-G over a D bass), an equally valid and
    // arguably more accurate spelling of the same notes. Accept either reading.
    let d11_5 = detect(&[62, 67, 69, 72, 76], "D11 (5-note)");
    assert!(
        d11_5
            .iter()
            .any(|s| s.contains("D11") || s.contains("D7sus4") || s.contains("Am7/D")),
        "D11 5-note, got {d11_5:?}"
    );

    // D11 (VS 2: D4 G4 A4 C5) — 4-note voicing, currently D7sus4
    let d11_4 = detect(&[62, 67, 69, 72], "D11 (4-note)");
    println!("  4-note detected as: {:?}", d11_4);

    // Eb11/Ab (Ab3 Db4 Eb4 Bb4) — sus4add9 with Ab root (Eb7sus4/Ab with Eb root)
    let eb11 = detect(&[56, 61, 63, 70], "Eb11/Ab");
    assert!(
        eb11.iter().any(|s| s.contains("sus4")),
        "Eb11/Ab should be sus4-based, got {:?}",
        eb11
    );
    println!("  Eb11/Ab detected as: {:?}", eb11);

    // Gbmaj (Gb3 Bb3 Db4)
    let gb = detect(&[54, 58, 61], "Gbmaj");
    println!("  Gbmaj detected as: {:?}", gb);

    // Overlap: Am/C notes + Gb notes (C4 E4 A4 + Gb3 Bb3 Db4)
    let overlap = detect(&[48, 52, 57, 54, 58, 61], "Am/C+Gb overlap");
    println!("  Overlap detected as: {:?}", overlap);
}
