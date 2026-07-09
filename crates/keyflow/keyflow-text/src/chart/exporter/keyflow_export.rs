//! Keyflow text export.
//!
//! This exporter writes idiomatic `.kf` syntax for imported charts.

use keyflow_proto::Note;
use keyflow_proto::chart::melody::Melody;
use keyflow_proto::chart::notations::{
    Dynamic, FiguredBass, Hairpin, HairpinKind, Placement, RepeatMark, StaffText,
};
use keyflow_proto::chart::types::{Measure, RhythmElement};
use keyflow_proto::key::ScaleMode;
use keyflow_proto::time::{MusicalDuration, MusicalPositionExt, TimeSignature};
use keyflow_proto::{Chart, ChordRhythm, SectionType};

#[must_use]
pub fn chart_to_keyflow(chart: &Chart) -> String {
    let mut out = String::new();

    if let Some(title) = chart.metadata.title.as_deref() {
        out.push_str(title);
        out.push('\n');
    }

    let mut metadata = Vec::new();
    if let Some(tempo) = &chart.tempo {
        metadata.push(format!("8th={}bpm", tempo.bpm as u32));
    }
    if let Some(ts) = chart
        .initial_time_signature
        .as_ref()
        .or(chart.time_signature.as_ref())
    {
        metadata.push(format!("{}/{}", ts.numerator, ts.denominator));
    }
    if let Some(key) = &chart.initial_key {
        metadata.push(key_to_syntax(key));
    }
    if !metadata.is_empty() {
        out.push_str(&metadata.join(" "));
        out.push('\n');
    }

    let mut pending_cross_section_repeat_prefix: Option<Vec<Measure>> = None;
    for section in &chart.sections {
        out.push('\n');
        let measures = expand_measures_without_repeat_symbols(
            section.measures(),
            pending_cross_section_repeat_prefix.as_deref(),
        );
        pending_cross_section_repeat_prefix = cross_section_repeat_prefix(section.measures());
        let section_chord_length = common_chord_length(&measures);
        out.push_str(&section_header(section, measures.len()));
        out.push('\n');
        if let Some(rhythm) = &section_chord_length {
            out.push_str("/ChordLength ");
            out.push_str(&chord_length_to_syntax(
                rhythm,
                measures
                    .first()
                    .map_or((4, 4), |measure| measure.time_signature),
            ));
            out.push('\n');
        }

        for (row_idx, row) in measures.chunks(4).enumerate() {
            if row_idx > 0 {
                out.push_str("    ");
            }

            for measure in row {
                write_measure(&mut out, measure, section_chord_length.as_ref());
            }
            out.push('|');
            out.push('\n');
        }
    }

    out
}

fn write_measure(out: &mut String, measure: &Measure, default_chord_length: Option<&ChordRhythm>) {
    out.push_str("| ");
    out.push_str(&measure_to_keyflow(measure, default_chord_length));
    out.push(' ');
}

fn key_to_syntax(key: &keyflow_proto::Key) -> String {
    let root = key.root().name();
    let clean = root.trim_start_matches('#').trim_start_matches('b');
    let prefix = if root.contains('b') { 'b' } else { '#' };
    if key.mode == ScaleMode::aeolian() {
        format!("{prefix}{clean}m")
    } else {
        format!("{prefix}{clean}")
    }
}

fn section_header(section: &keyflow_proto::ChartSection, count: usize) -> String {
    match &section.section.section_type {
        SectionType::CountIn => format!("count {count}"),
        SectionType::Opening => format!("opening {count}"),
        SectionType::Intro => format!("intro {count}"),
        SectionType::Verse => format!("vs {count}"),
        SectionType::Chorus => format!("ch {count}"),
        SectionType::Bridge => format!("br {count}"),
        SectionType::Outro => format!("out {count}"),
        SectionType::Pre(inner) if matches!(inner.as_ref(), SectionType::Chorus) => {
            format!("pre {count}")
        }
        SectionType::Post(inner) if matches!(inner.as_ref(), SectionType::Chorus) => {
            format!("post {count}")
        }
        SectionType::Custom(name) => format!("{name} {count}"),
        _ => format!("section {count}"),
    }
}

fn expand_measures_without_repeat_symbols(
    measures: &[Measure],
    repeat_prefix: Option<&[Measure]>,
) -> Vec<Measure> {
    let mut expanded = Vec::new();
    let mut repeat_start = 0usize;

    for (idx, measure) in measures.iter().enumerate() {
        if matches!(measure.start_repeat, RepeatMark::Forward) {
            repeat_start = idx;
        }

        expanded.push(clear_repeat_symbols(measure));

        if matches!(measure.end_repeat, RepeatMark::Backward) {
            let repeat_end = first_ending_start(measures, repeat_start, idx).unwrap_or(idx + 1);
            if let Some(repeat_prefix) = repeat_prefix {
                expanded.extend(repeat_prefix.iter().map(clear_repeat_symbols));
            }
            for repeated in &measures[repeat_start..repeat_end] {
                expanded.push(clear_repeat_symbols(repeated));
            }
            repeat_start = idx + 1;
        }
    }

    expanded
}

fn cross_section_repeat_prefix(measures: &[Measure]) -> Option<Vec<Measure>> {
    let repeat_start = measures
        .iter()
        .position(|measure| matches!(measure.start_repeat, RepeatMark::Forward))?;
    let has_repeat_end = measures
        .iter()
        .skip(repeat_start)
        .any(|measure| matches!(measure.end_repeat, RepeatMark::Backward));
    (!has_repeat_end).then(|| measures[repeat_start..].to_vec())
}

fn first_ending_start(
    measures: &[Measure],
    repeat_start: usize,
    repeat_end: usize,
) -> Option<usize> {
    measures[repeat_start..=repeat_end]
        .iter()
        .position(|measure| {
            measure
                .volta_start
                .as_ref()
                .map(|volta| volta.numbers.contains(&1))
                .unwrap_or(false)
        })
        .map(|offset| repeat_start + offset)
}

fn clear_repeat_symbols(measure: &Measure) -> Measure {
    let mut measure = measure.clone();
    measure.start_repeat = RepeatMark::None;
    measure.end_repeat = RepeatMark::None;
    measure.volta_start = None;
    measure
}

fn measure_to_keyflow(measure: &Measure, default_chord_length: Option<&ChordRhythm>) -> String {
    if !measure.melodies.is_empty() {
        let notation = measure_notation_to_keyflow(measure, default_chord_length);
        let melodies = measure
            .melodies
            .iter()
            .map(melody_to_syntax)
            .collect::<Vec<_>>()
            .join(" ");
        return format!("<< {notation} ; {melodies} >>");
    }

    measure_notation_to_keyflow(measure, default_chord_length)
}

fn measure_notation_to_keyflow(
    measure: &Measure,
    default_chord_length: Option<&ChordRhythm>,
) -> String {
    let mut parts = Vec::new();

    for text in &measure.staff_text {
        parts.push(staff_text_to_syntax(text));
    }

    for dynamic in &measure.dynamics {
        parts.push(dynamic.to_string());
    }

    for dynamic in &measure.classical_dynamics {
        parts.push(classical_dynamic_to_syntax(dynamic));
    }

    for hairpin in &measure.hairpins {
        parts.push(hairpin_to_syntax(hairpin));
    }

    if measure.chords.is_empty() {
        parts.extend(
            measure
                .figured_bass
                .iter()
                .map(figured_bass_to_standalone_syntax),
        );

        if !measure.rhythm_elements.is_empty() {
            parts.extend(
                measure
                    .rhythm_elements
                    .iter()
                    .map(|element| rhythm_element_to_syntax(element, measure.time_signature)),
            );
        }

        if !parts.is_empty() {
            return parts.join(" ");
        }

        "r1".to_string()
    } else if measure.rhythm_elements.iter().any(RhythmElement::is_rest) {
        parts.extend(
            measure
                .rhythm_elements
                .iter()
                .map(|element| rhythm_element_to_syntax(element, measure.time_signature)),
        );
        parts.join(" ")
    } else {
        let mut beat = 1u8;
        let suppress_full_measure_slashes = measure.chords.len() > 1;
        let fallback_durations =
            even_chord_duration_suffixes(measure.chords.len(), measure.time_signature);
        parts.extend(measure.chords.iter().enumerate().map(|(idx, chord)| {
            let mut token = chord_symbol_to_syntax(&chord.full_symbol);
            if let Some(figured_bass) = measure.figured_bass.iter().find(|item| {
                item.beat == beat && matches!(item.placement, Placement::Above | Placement::Below)
            }) {
                token.push('"');
                token.push_str(&escape_quoted(&figured_bass_rows_to_text(figured_bass)));
                token.push('"');
            }
            if suppress_full_measure_slashes {
                if let Some(duration) = fallback_durations.get(idx) {
                    token.push('_');
                    token.push_str(duration);
                }
                return token;
            }
            match chord.rhythm {
                ChordRhythm::Slashes {
                    count,
                    dotted,
                    tied,
                } => {
                    let matches_default =
                        default_chord_length.is_some_and(|default| *default == chord.rhythm);
                    let full_measure_slashes = count >= measure.time_signature.0;
                    if full_measure_slashes {
                        // MusicXML imports often represent a full-measure slash
                        // pattern as eight slash glyphs in 4/4. Keyflow's
                        // idiom is the bare chord for a full measure.
                    } else if !matches_default {
                        token.push(' ');
                        token.push_str(&"/".repeat(count as usize));
                        if dotted {
                            token.push('.');
                        }
                        if tied {
                            token.push('~');
                        }
                    }
                }
                ChordRhythm::Default => token.push_str(" /"),
                ChordRhythm::Explicit(_) => {}
            }
            let time_signature = TimeSignature::new(
                u32::from(measure.time_signature.0),
                u32::from(measure.time_signature.1),
            );
            beat = beat.saturating_add(chord.duration.to_beats(time_signature).round() as u8);
            token
        }));

        parts.join(" ")
    }
}

fn even_chord_duration_suffixes(chord_count: usize, time_signature: (u8, u8)) -> Vec<&'static str> {
    if time_signature == (6, 8) {
        return match chord_count {
            0 | 1 => vec![],
            2 => vec!["4.", "4."],
            3 => vec!["4", "4", "4"],
            4 => vec!["8.", "8.", "8.", "8."],
            _ => vec!["8"; chord_count],
        };
    }
    match chord_count {
        0 | 1 => vec![],
        2 => vec!["2", "2"],
        3 => vec!["4", "4", "2"],
        4 => vec!["4", "4", "4", "4"],
        _ => vec!["4"; chord_count],
    }
}

fn chord_symbol_to_syntax(symbol: &str) -> String {
    if is_plain_major_symbol(symbol) {
        format!("!{symbol}")
    } else {
        symbol.to_string()
    }
}

fn is_plain_major_symbol(symbol: &str) -> bool {
    if symbol.contains('/') {
        return false;
    }
    let mut chars = symbol.chars();
    let Some(root) = chars.next() else {
        return false;
    };
    if !matches!(root, 'A'..='G') {
        return false;
    }
    match chars.next() {
        None => true,
        Some('#' | 'b') => chars.next().is_none(),
        Some(_) => false,
    }
}

fn rhythm_element_to_syntax(element: &RhythmElement, time_signature: (u8, u8)) -> String {
    match element {
        RhythmElement::Chord(chord) => chord_to_syntax(chord, None, time_signature),
        RhythmElement::Rest(rest) => {
            if !rest.original_token.is_empty() {
                rest.original_token.clone()
            } else {
                format!(
                    "r{}",
                    duration_to_lily_syntax(
                        rest.duration,
                        TimeSignature::new(
                            u32::from(time_signature.0),
                            u32::from(time_signature.1),
                        )
                    )
                )
            }
        }
        RhythmElement::Space(space) => {
            if !space.original_token.is_empty() {
                space.original_token.clone()
            } else {
                format!(
                    "s{}",
                    duration_to_lily_syntax(
                        space.duration,
                        TimeSignature::new(
                            u32::from(time_signature.0),
                            u32::from(time_signature.1),
                        )
                    )
                )
            }
        }
    }
}

fn chord_to_syntax(
    chord: &keyflow_proto::chart::types::ChordInstance,
    default_chord_length: Option<&ChordRhythm>,
    time_signature: (u8, u8),
) -> String {
    let mut token = chord_symbol_to_syntax(&chord.full_symbol);
    match chord.rhythm {
        ChordRhythm::Slashes {
            count,
            dotted,
            tied,
        } => {
            let matches_default =
                default_chord_length.is_some_and(|default| *default == chord.rhythm);
            if !matches_default && count != time_signature.0 {
                token.push(' ');
                token.push_str(&"/".repeat(count as usize));
                if dotted {
                    token.push('.');
                }
                if tied {
                    token.push('~');
                }
            }
        }
        ChordRhythm::Default => token.push_str(" /"),
        ChordRhythm::Explicit(_) => {}
    }
    token
}

fn melody_to_syntax(melody: &Melody) -> String {
    melody.to_string().replace("m{", "m {")
}

fn duration_to_lily_syntax(duration: MusicalDuration, time_sig: TimeSignature) -> String {
    let beats = duration.to_beats(time_sig);
    for (value, base_beats) in [
        (1, f64::from(time_sig.denominator)),
        (2, f64::from(time_sig.denominator) / 2.0),
        (4, f64::from(time_sig.denominator) / 4.0),
        (8, f64::from(time_sig.denominator) / 8.0),
        (16, f64::from(time_sig.denominator) / 16.0),
        (32, f64::from(time_sig.denominator) / 32.0),
    ] {
        if (beats - base_beats).abs() < 0.001 {
            return value.to_string();
        }
        if (beats - base_beats * 1.5).abs() < 0.001 {
            return format!("{value}.");
        }
    }
    "1".to_string()
}

fn figured_bass_to_standalone_syntax(item: &FiguredBass) -> String {
    let prefix = match item.placement {
        Placement::Above => "^",
        Placement::Below => "",
    };
    format!(
        "{prefix}\"{}\"",
        escape_quoted(&figured_bass_rows_to_text(item))
    )
}

fn common_chord_length(measures: &[Measure]) -> Option<ChordRhythm> {
    let mut common: Option<ChordRhythm> = None;
    let mut chord_count = 0usize;

    for measure in measures {
        if !measure.melodies.is_empty() {
            return None;
        }
        for chord in &measure.chords {
            let ChordRhythm::Slashes { .. } = chord.rhythm else {
                return None;
            };
            chord_count += 1;
            if let Some(existing) = &common {
                if *existing != chord.rhythm {
                    return None;
                }
            } else {
                common = Some(chord.rhythm.clone());
            }
        }
    }

    (chord_count > 1)
        .then_some(common?)
        .filter(|_| chord_count > measures.len())
}

fn chord_length_to_syntax(rhythm: &ChordRhythm, time_signature: (u8, u8)) -> String {
    match rhythm {
        ChordRhythm::Slashes { count, dotted, .. } => {
            let slash_beats = f64::from(*count) * if *dotted { 1.5 } else { 1.0 };
            let dotted_quarter_beats = f64::from(time_signature.1) / 4.0 * 1.5;
            if (slash_beats - dotted_quarter_beats).abs() < 0.001 {
                return "4.".to_string();
            }
            let mut out = "/".repeat(*count as usize);
            if *dotted {
                out.push('.');
            }
            out
        }
        ChordRhythm::Default | ChordRhythm::Explicit(_) => "/".to_string(),
    }
}

fn figured_bass_rows_to_text(item: &FiguredBass) -> String {
    figured_bass_rows(item).join(" ")
}

fn figured_bass_rows(item: &FiguredBass) -> Vec<String> {
    let mut rows = Vec::new();
    for row in &item.rows {
        let mut parts = row.text.split_whitespace();
        if let Some(first) = parts.next() {
            let split_first = split_compacted_figured_row(first);
            rows.push(format!("{}{}", row.accidental, split_first[0]));
            rows.extend(split_first.into_iter().skip(1).map(str::to_string));
            rows.extend(parts.map(str::to_string));
        }
    }
    rows
}

fn split_compacted_figured_row(row: &str) -> Vec<&str> {
    let chars = row.chars().collect::<Vec<_>>();
    if chars.len() == 6
        && chars[0].is_ascii_digit()
        && chars[1] == '-'
        && chars[2].is_ascii_digit()
        && chars[3].is_ascii_digit()
        && chars[4] == '-'
        && chars[5].is_ascii_digit()
    {
        vec![&row[..3], &row[3..]]
    } else {
        vec![row]
    }
}

fn staff_text_to_syntax(text: &StaffText) -> String {
    let prefix = match text.placement {
        Placement::Above => "^",
        Placement::Below => "",
    };
    format!("{prefix}\"{}\"", escape_quoted(&text.text))
}

fn classical_dynamic_to_syntax(dynamic: &Dynamic) -> String {
    let mut text = format!("dyn {}", dynamic.level.as_str());
    if dynamic.beat != 1 {
        text.push('@');
        text.push_str(&dynamic.beat.to_string());
    }
    if dynamic.placement == Placement::Above {
        text.push_str(" above");
    }
    text
}

fn hairpin_to_syntax(hairpin: &Hairpin) -> String {
    let kind = match hairpin.kind {
        HairpinKind::Crescendo => "<",
        HairpinKind::Decrescendo => ">",
    };
    let mut text = format!(
        "hairpin {kind} {}..{}",
        hairpin.start_beat, hairpin.end_beat
    );
    if hairpin.placement == Placement::Above {
        text.push_str(" above");
    }
    text
}

fn escape_quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
