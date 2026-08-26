//! ChordPro → Keyflow `Chart` integration.
//!
//! When a `.kf` document contains a `--- chordpro ---` block, the keyflow
//! block carries rhythm + section structure (the source of truth for
//! timing) and the ChordPro block carries lyrics + chord-over-lyric
//! placement. This module merges the two: top-level ChordPro directives
//! enrich `Chart` metadata; ChordPro environment blocks (`{soc}` /
//! `{sov}` / …) are matched to keyflow `ChartSection`s by `SectionType`
//! and lyric lines are attached as `Track::lyrics`. Each ChordPro document
//! acts as its own lyric layer, so multiple blocks can hold separate
//! singers, parts, translations, or sync granularities over the same
//! keyflow rhythm chart.
//!
//! The match policy is deliberately conservative — we never overwrite
//! data that the keyflow block already supplied.

use keyflow_chordpro::{DirectiveKind, Document as KcDocument, Environment as KcEnv, Line};
use std::collections::HashMap;

use keyflow_proto::chart::{
    Chart, LyricLine, LyricSourceFormat, LyricSyllable, LyricSyncLevel, Track,
};
use keyflow_proto::sections::SectionType;

/// Merge a parsed ChordPro document into an existing `Chart`.
///
/// - Top-level ChordPro metadata (title, artist, key, tempo, time, capo,
///   composer, copyright, year, …) is merged into `chart.metadata` /
///   `chart.tempo` / `chart.current_key` / `chart.time_signature` when
///   those fields are not already set by the keyflow block.
/// - Each ChordPro environment is matched to a keyflow `ChartSection` of
///   the corresponding `SectionType` (in source order). Lyric lines under
///   the environment become a `Track::lyrics(LyricLine)` on that section.
///
/// Returns the number of lyric lines that were attached (mostly useful
/// for tests).
pub fn merge_chordpro_into_chart(chart: &mut Chart, kc: &KcDocument) -> usize {
    merge_metadata(chart, kc);

    // Group ChordPro lines into (section, label, lyric_lines) blocks. We
    // ignore implicit (no-environment) lines for now — those are
    // typically lead-in directives, not lyric content.
    let mut blocks: Vec<(SectionType, Option<String>, Vec<LyricLine>)> = Vec::new();
    let mut current_env: Option<KcEnv> = None;
    let mut current_label: Option<String> = None;
    let mut current_lines: Vec<LyricLine> = Vec::new();

    for line in &kc.lines {
        match line {
            Line::Directive(d) => match &d.kind {
                DirectiveKind::StartOfEnvironment { env, label } => {
                    flush_block(
                        &mut blocks,
                        &mut current_env,
                        &mut current_label,
                        &mut current_lines,
                    );
                    current_env = Some(*env);
                    current_label = label.clone();
                }
                DirectiveKind::EndOfEnvironment { .. } => {
                    flush_block(
                        &mut blocks,
                        &mut current_env,
                        &mut current_label,
                        &mut current_lines,
                    );
                }
                _ => {}
            },
            Line::Lyric { chunks, .. } => {
                if current_env.is_some() && lyric_chunks_have_text(chunks) {
                    current_lines.push(lyric_line_from_chunks(chunks));
                }
            }
            Line::HashComment { .. } | Line::Empty { .. } => {}
        }
    }
    flush_block(
        &mut blocks,
        &mut current_env,
        &mut current_label,
        &mut current_lines,
    );

    // Per-SectionType cursor for this ChordPro document. A single ChordPro
    // document maps verse 1 / verse 2 / ... to successive matching keyflow
    // sections. A second ChordPro document starts its own cursor, which lets
    // multiple lyric layers (translations, singers, parts) attach to the same
    // master rhythm chart.
    let mut cursors: HashMap<SectionType, usize> = HashMap::new();
    let mut attached = 0usize;
    for (target_type, label, lines) in blocks {
        let cursor = cursors.entry(target_type.clone()).or_insert(0);
        let Some((section_idx, section)) = chart
            .sections
            .iter_mut()
            .enumerate()
            .skip(*cursor)
            .find(|(_, s)| section_types_match(&s.section.section_type, &target_type))
        else {
            // No matching keyflow section. Drop the lyric block silently —
            // a future revision can surface this as a diagnostic.
            continue;
        };
        *cursor = section_idx + 1;

        // Concatenate every chordpro line into a single LyricLine so the
        // existing chord-syllable aligner has one logical track to work
        // with. Multi-line chordpro within an environment becomes a
        // space-joined sequence of syllables (mirrors how the existing
        // lyrics track handles multi-line content).
        let mut merged = LyricLine::empty()
            .with_source_format(LyricSourceFormat::ChordPro)
            .with_sync_level(LyricSyncLevel::Line);
        for (i, mut l) in lines.into_iter().enumerate() {
            if i > 0 && !merged.syllables.is_empty() {
                // Mark a soft word boundary by marking the next syllable
                // as word-initial.
                if let Some(last) = merged.syllables.last_mut() {
                    last.hyphen_after = false;
                }
            }
            if let Some(last) = l.syllables.last_mut() {
                last.line_break_after = true;
            }
            merged.syllables.extend(l.syllables);
            attached += 1;
        }
        if merged.syllables.is_empty() {
            attached += 1;
        }
        let track_name = label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(name) = track_name.as_ref() {
            merged = merged.with_label(name.clone());
            apply_label_metadata(&mut merged, name);
        }

        let mut track = Track::lyrics(merged);
        if let Some(name) = track_name {
            track = track.with_name(name);
        }
        section.tracks.push(track);
    }
    attached
}

fn flush_block(
    blocks: &mut Vec<(SectionType, Option<String>, Vec<LyricLine>)>,
    env: &mut Option<KcEnv>,
    label: &mut Option<String>,
    lines: &mut Vec<LyricLine>,
) {
    let Some(e) = env.take() else {
        lines.clear();
        *label = None;
        return;
    };
    let target_type = match e {
        KcEnv::Verse => SectionType::Verse,
        KcEnv::Chorus => SectionType::Chorus,
        KcEnv::Bridge => SectionType::Bridge,
        KcEnv::Section => label
            .as_deref()
            .and_then(section_type_from_label)
            .unwrap_or(SectionType::Custom("Section".to_string())),
        KcEnv::Tab | KcEnv::Grid => {
            lines.clear();
            *label = None;
            return; // Not a lyric environment
        }
    };
    let lyric_lines = std::mem::take(lines);
    let lbl = label.take();
    blocks.push((target_type, lbl, lyric_lines));
}

fn section_type_from_label(label: &str) -> Option<SectionType> {
    let first = label
        .split_whitespace()
        .find(|word| !word.contains('=') && !word.starts_with('('))?
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
    SectionType::parse(first).ok()
}

fn lyric_chunks_have_text(chunks: &[keyflow_chordpro::ChordChunk]) -> bool {
    chunks.iter().any(|chunk| !chunk.text.trim().is_empty())
}

fn section_types_match(chart_type: &SectionType, chordpro_type: &SectionType) -> bool {
    chart_type == chordpro_type
        || matches!(
            (chart_type, chordpro_type),
            (SectionType::Instrumental, SectionType::Interlude)
                | (SectionType::Interlude, SectionType::Instrumental)
        )
}

/// Build a `LyricLine` from a sequence of ChordPro chord/lyric chunks.
fn lyric_line_from_chunks(chunks: &[keyflow_chordpro::ChordChunk]) -> LyricLine {
    let mut syllables: Vec<LyricSyllable> = Vec::new();
    let mut previous_chunk_ended_with_whitespace = false;
    for chunk in chunks {
        let chord_str = chunk.chord.clone();
        let continues_previous_word = !chunk.text.starts_with(char::is_whitespace)
            && !previous_chunk_ended_with_whitespace
            && !syllables.is_empty();
        // Split text into whitespace-separated syllables. The first
        // syllable in a chunk inherits the chunk's chord (via the
        // chord-syllable aligner's expected shape).
        let mut first_in_chunk = true;
        for word in chunk.text.split_whitespace() {
            if word.is_empty() {
                continue;
            }
            if word.contains('-') {
                let parts: Vec<&str> = word.split('-').collect();
                for (i, part) in parts.iter().enumerate() {
                    if part.is_empty() {
                        continue;
                    }
                    let hyphen_after = i < parts.len() - 1;
                    syllables.push(LyricSyllable {
                        text: part.to_string(),
                        hyphen_after,
                        chord: if first_in_chunk {
                            chord_str.clone()
                        } else {
                            None
                        },
                        chord_attachment: None,
                        measure_index: 0,
                        beat: 0.0,
                        word_initial: i == 0 && !continues_previous_word,
                        line_break_after: false,
                    });
                    first_in_chunk = false;
                }
            } else {
                syllables.push(LyricSyllable {
                    text: word.to_string(),
                    hyphen_after: false,
                    chord: if first_in_chunk {
                        chord_str.clone()
                    } else {
                        None
                    },
                    chord_attachment: None,
                    measure_index: 0,
                    beat: 0.0,
                    word_initial: !continues_previous_word || !first_in_chunk,
                    line_break_after: false,
                });
                first_in_chunk = false;
            }
        }
        // If the chunk had a chord but no text (e.g. trailing `[C]` on a
        // line), still emit an empty-text syllable so the chord isn't
        // dropped. The aligner handles zero-text syllables as held chords.
        if first_in_chunk && chord_str.is_some() {
            syllables.push(LyricSyllable {
                text: String::new(),
                hyphen_after: false,
                chord: chord_str,
                chord_attachment: None,
                measure_index: 0,
                beat: 0.0,
                word_initial: true,
                line_break_after: false,
            });
        }
        if let Some(last) = chunk.text.chars().last() {
            previous_chunk_ended_with_whitespace = last.is_whitespace();
        }
    }
    LyricLine::new(syllables)
        .with_source_format(LyricSourceFormat::ChordPro)
        .with_sync_level(LyricSyncLevel::Line)
}

fn apply_label_metadata(line: &mut LyricLine, label: &str) {
    let mut remaining_words = Vec::new();
    for word in label.split_whitespace() {
        if let Some(value) = word.strip_prefix("singer=") {
            if !value.is_empty() {
                line.singer = Some(value.to_string());
            }
        } else if let Some(value) = word.strip_prefix("part=") {
            if !value.is_empty() {
                line.part = Some(value.to_string());
            }
        } else if let Some(value) = word.strip_prefix("sync=") {
            if let Some(level) = parse_sync_level(value) {
                line.sync_level = level;
            }
        } else {
            remaining_words.push(word);
        }
    }

    if line.part.is_none() && !remaining_words.is_empty() {
        line.part = Some(remaining_words.join(" "));
    }
}

fn parse_sync_level(value: &str) -> Option<LyricSyncLevel> {
    match value.to_ascii_lowercase().as_str() {
        "section" | "per-section" | "per_section" => Some(LyricSyncLevel::Section),
        "slide" | "slides" => Some(LyricSyncLevel::Slide),
        "line" | "lines" => Some(LyricSyncLevel::Line),
        "word" | "words" => Some(LyricSyncLevel::Word),
        "syllable" | "syllables" => Some(LyricSyncLevel::Syllable),
        _ => None,
    }
}

/// Apply title/artist/key/tempo/time-signature/etc. from ChordPro
/// directives to the `Chart`, but only when those fields are missing.
fn merge_metadata(chart: &mut Chart, kc: &KcDocument) {
    for d in kc.directives() {
        match &d.kind {
            DirectiveKind::Title(s) if chart.metadata.title.is_none() => {
                chart.metadata.title = Some(s.clone());
            }
            DirectiveKind::Subtitle(s) if chart.metadata.subtitle.is_none() => {
                chart.metadata.subtitle = Some(s.clone());
            }
            DirectiveKind::Meta(m) => apply_meta_item(chart, &m.item, &m.value),
            _ => {}
        }
    }
}

fn apply_meta_item(chart: &mut Chart, item: &str, value: &str) {
    match item {
        "artist" if chart.metadata.artist.is_none() => {
            chart.metadata.artist = Some(value.to_string());
        }
        "composer" if chart.metadata.composer.is_none() => {
            chart.metadata.composer = Some(value.to_string());
        }
        "copyright" if chart.metadata.copyright.is_none() => {
            chart.metadata.copyright = Some(value.to_string());
        }
        "year" if chart.metadata.year.is_none() => {
            if let Ok(y) = value.parse::<u16>() {
                chart.metadata.year = Some(y);
            }
        }
        "key" if chart.current_key.is_none() => {
            if let Ok(k) = keyflow_proto::Key::parse(value) {
                chart.current_key = Some(k.clone());
                if chart.initial_key.is_none() {
                    chart.initial_key = Some(k);
                }
            }
        }
        "tempo" if chart.tempo.is_none() => {
            if let Ok(bpm) = value.parse::<f64>() {
                chart.tempo = Some(keyflow_proto::Tempo::from_bpm(bpm));
            }
        }
        "time" if chart.time_signature.is_none() => {
            if let Some((n, d)) = value.split_once('/') {
                if let (Ok(num), Ok(den)) = (n.trim().parse::<u32>(), d.trim().parse::<u32>()) {
                    let ts = keyflow_proto::TimeSignature::new(num, den);
                    chart.time_signature = Some(ts);
                    if chart.initial_time_signature.is_none() {
                        chart.initial_time_signature = Some(ts);
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::parse_chart;
    use keyflow_proto::chart::TrackType;

    fn has_lyrics(section: &keyflow_proto::chart::ChartSection) -> bool {
        section
            .tracks
            .iter()
            .any(|t| t.track_type == TrackType::Lyrics)
    }

    #[test]
    fn merges_metadata_when_keyflow_omits_it() {
        let mut chart = parse_chart("VS 1: | 1 4 5 1 |\n").expect("parse");
        let kc = keyflow_chordpro::parse("{title: Hello}\n{artist: Trad}\n").unwrap();
        let _ = merge_chordpro_into_chart(&mut chart, &kc);
        assert_eq!(chart.metadata.title.as_deref(), Some("Hello"));
        assert_eq!(chart.metadata.artist.as_deref(), Some("Trad"));
    }

    #[test]
    fn keyflow_metadata_is_not_overwritten() {
        // Keyflow block already declares a title via metadata; chordpro should
        // NOT overwrite it.
        let mut chart = parse_chart("VS 1: | 1 |\n").expect("parse");
        chart.metadata.title = Some("Original".to_string());
        let kc = keyflow_chordpro::parse("{title: Replacement}\n").unwrap();
        merge_chordpro_into_chart(&mut chart, &kc);
        assert_eq!(chart.metadata.title.as_deref(), Some("Original"));
    }

    #[test]
    fn end_to_end_parse_document_routes_chordpro_block() {
        use crate::chart::parse_document;
        let doc_text = "\
--- keyflow ---
{title: Twinkle}
120bpm 4/4 #C

VS 1: | 1 4 5 1 |
CH 1: | 4 5 1 1 |

--- chordpro ---
{artist: Trad}
{sov}
[C]Twinkle, [F]little [C]star
{eov}
{soc}
[F]How I [C]wonder
{eoc}
";
        let (chart, _doc) = parse_document(doc_text).expect("parse_document");
        // Title from keyflow (first match wins).
        assert!(chart.metadata.title.as_deref().is_some());
        // Artist from chordpro (keyflow didn't supply one).
        assert_eq!(chart.metadata.artist.as_deref(), Some("Trad"));

        // Both sections got lyrics tracks.
        let vs = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Verse)
            .unwrap();
        let ch = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Chorus)
            .unwrap();
        assert!(vs.tracks.iter().any(|t| t.track_type == TrackType::Lyrics));
        assert!(ch.tracks.iter().any(|t| t.track_type == TrackType::Lyrics));

        // Verify chord attachment landed on the right syllables in the verse.
        let lyrics = vs
            .tracks
            .iter()
            .find(|t| t.track_type == TrackType::Lyrics)
            .and_then(|t| t.lyrics.as_ref())
            .unwrap();
        let with_chords: Vec<_> = lyrics
            .syllables
            .iter()
            .filter(|s| s.chord.is_some())
            .map(|s| (s.text.clone(), s.chord.clone().unwrap()))
            .collect();
        assert!(
            with_chords.iter().any(|(t, c)| t == "Twinkle," && c == "C"),
            "expected Twinkle,->C, got {:?}",
            with_chords
        );
        assert!(with_chords.iter().any(|(t, c)| t == "little" && c == "F"));
    }

    #[test]
    fn lyrics_attach_to_matching_section_type() {
        // One Verse and one Chorus in keyflow; chordpro provides lyrics for
        // both via `{soc}` / `{sov}`.
        let kf = "\
VS 1: | 1 4 5 1 |
CH 1: | 4 5 1 1 |
";
        let mut chart = parse_chart(kf).expect("parse");
        let chordpro = "\
{sov}
[C]Hello [F]there
{eov}
{soc}
[F]Big [C]chorus
{eoc}
";
        let kc = keyflow_chordpro::parse(chordpro).unwrap();
        let attached = merge_chordpro_into_chart(&mut chart, &kc);
        assert!(
            attached >= 2,
            "expected at least two lyric lines attached, got {}",
            attached
        );

        // Verify each section has a lyrics track now.
        let vs = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Verse)
            .unwrap();
        let ch = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Chorus)
            .unwrap();
        assert!(has_lyrics(vs));
        assert!(has_lyrics(ch));
    }

    #[test]
    fn multiple_chordpro_blocks_attach_as_parallel_lyric_layers() {
        use crate::chart::parse_document;

        let doc_text = "\
--- keyflow ---
VS 1: | 1 4 |

--- chordpro ---
{sov: singer=lead part=Lead sync=words}
[C]Lead [F]line
{eov}

--- chordpro ---
{sov: singer=harmony part=Harmony sync=slides}
[C]Harmony [F]line
{eov}
";
        let (chart, _doc) = parse_document(doc_text).expect("parse_document");
        let verse = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Verse)
            .unwrap();
        let lyric_tracks: Vec<_> = verse
            .tracks
            .iter()
            .filter(|t| t.track_type == TrackType::Lyrics)
            .collect();

        assert_eq!(lyric_tracks.len(), 2);
        let lead = lyric_tracks[0].lyrics.as_ref().unwrap();
        assert_eq!(lead.source_format, LyricSourceFormat::ChordPro);
        assert_eq!(lead.sync_level, LyricSyncLevel::Word);
        assert_eq!(lead.singer.as_deref(), Some("lead"));
        assert_eq!(lead.part.as_deref(), Some("Lead"));

        let harmony = lyric_tracks[1].lyrics.as_ref().unwrap();
        assert_eq!(harmony.sync_level, LyricSyncLevel::Slide);
        assert_eq!(harmony.singer.as_deref(), Some("harmony"));
        assert_eq!(harmony.part.as_deref(), Some("Harmony"));
    }

    #[test]
    fn chordpro_line_sync_preserves_source_lines() {
        use crate::chart::parse_document;

        let doc_text = "\
--- keyflow ---
VS 1: | 1 4 |

--- chordpro ---
{sov: Verse 1 sync=lines}
[C]First [F]line
[G]Second [C]line
{eov}
";
        let (chart, _doc) = parse_document(doc_text).expect("parse_document");
        let verse = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Verse)
            .unwrap();
        let lyrics = verse
            .tracks
            .iter()
            .find(|t| t.track_type == TrackType::Lyrics)
            .and_then(|t| t.lyrics.as_ref())
            .unwrap();

        assert_eq!(lyrics.sync_level, LyricSyncLevel::Line);
        let lines = lyrics.derive_segments(LyricSyncLevel::Line);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "First line");
        assert_eq!(lines[1].text, "Second line");
    }

    #[test]
    fn chordpro_chords_inside_words_keep_word_text_intact() {
        let line = lyric_line_from_chunks(
            &keyflow_chordpro::parse("[G]Worthy of every so[C/G]ng")
                .unwrap()
                .lines
                .iter()
                .find_map(|line| match line {
                    Line::Lyric { chunks, .. } => Some(chunks.clone()),
                    _ => None,
                })
                .unwrap(),
        );

        let words = line.derive_segments(LyricSyncLevel::Word);
        assert!(words.iter().any(|word| word.text == "song"));
        assert!(line
            .syllables
            .iter()
            .any(|syl| syl.text == "ng" && syl.chord.as_deref() == Some("C/G")));
    }

    #[test]
    fn chordpro_chord_only_sections_attach_blank_lyric_tracks() {
        use crate::chart::parse_document;

        let doc_text = "\
--- keyflow ---
Intro
1 4

Interlude
4 4

--- chordpro ---
Intro:
[G] [C]

Interlude (2x):
[C] [D] [G]
";
        let (chart, _doc) = parse_document(doc_text).expect("parse_document");
        let intro = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Intro)
            .unwrap();
        let interlude = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Interlude)
            .unwrap();

        for section in [intro, interlude] {
            let lyrics = section
                .tracks
                .iter()
                .find(|t| t.track_type == TrackType::Lyrics)
                .and_then(|t| t.lyrics.as_ref())
                .unwrap();
            assert!(lyrics.syllables.is_empty());
            assert_eq!(lyrics.sync_level, LyricSyncLevel::Line);
        }
    }

    #[test]
    fn build_my_life_example_parses_with_line_sync() {
        use crate::chart::parse_document;

        let input = include_str!("../../../../examples/build_my_life.kf");
        let (chart, _doc) = parse_document(input).expect("parse_document");
        let verse = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Verse)
            .unwrap();
        let lyrics = verse
            .tracks
            .iter()
            .find(|t| t.track_type == TrackType::Lyrics)
            .and_then(|t| t.lyrics.as_ref())
            .unwrap();

        assert_eq!(chart.metadata.title.as_deref(), Some("Build My Life"));
        assert_eq!(chart.metadata.artist.as_deref(), Some("Housefires"));
        assert_eq!(lyrics.sync_level, LyricSyncLevel::Line);

        let intro = chart
            .sections
            .iter()
            .find(|s| s.section.section_type == SectionType::Intro)
            .unwrap();
        let intro_lyrics = intro
            .tracks
            .iter()
            .find(|t| t.track_type == TrackType::Lyrics)
            .and_then(|t| t.lyrics.as_ref())
            .unwrap();
        assert!(intro_lyrics.syllables.is_empty());

        let lines = lyrics.derive_segments(LyricSyncLevel::Line);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].text.contains("Worthy of every song"));
        assert!(lines[3].text.contains("We live for You"));
    }
}
