//! ChordPro export - convert Chart to ChordPro format text

use keyflow_proto::chart::{Track, TrackType};
use keyflow_proto::chord::{
    ChordProChunk, ChordProDirective, ChordProDocument, ChordProLine, ChordProSection,
};
use keyflow_proto::Chart;

/// Export a Chart to ChordPro format
///
/// # Examples
///
/// Converts a Keyflow chart with chords and lyrics tracks into
/// ChordPro notation with inline chords.
pub fn chart_to_chordpro(chart: &Chart) -> Result<String, String> {
    let doc = convert_chart_to_chordpro_doc(chart)?;
    Ok(chordpro_document_to_text(&doc))
}

/// Convert Chart to ChordProDocument
fn convert_chart_to_chordpro_doc(chart: &Chart) -> Result<ChordProDocument, String> {
    let mut doc = ChordProDocument::new();

    // Add metadata
    let metadata = &chart.metadata;
    if let Some(title) = &metadata.title {
        doc.directives.push(ChordProDirective::title(title));
    }
    if let Some(artist) = &metadata.artist {
        doc.directives.push(ChordProDirective::artist(artist));
    }

    // Convert sections
    for chart_section in &chart.sections {
        let mut cp_section = ChordProSection::new();

        // Set label from section type (e.g., "Verse", "Chorus")
        if let Some(label) = section_label(&chart_section.section) {
            cp_section.label = Some(label);
        }

        // Collect chords and lyrics from tracks
        let chords_track = chart_section
            .tracks
            .iter()
            .find(|t| t.track_type == TrackType::Chords);
        let lyrics_track = chart_section
            .tracks
            .iter()
            .find(|t| t.track_type == TrackType::Lyrics);

        // Build lines by interleaving chords and lyrics
        if let (Some(chords), Some(lyrics)) = (chords_track, lyrics_track) {
            if let Some(lyric_line) = &lyrics.lyrics {
                let chords_text = format_chords_from_track(chords);
                let line = interleave_chords_and_lyrics(&chords_text, &lyric_line.full_text());
                cp_section.lines.push(line);
            }
        } else if let Some(chords) = chords_track {
            // Chords only, no lyrics
            let line = chords_track_to_line(chords);
            cp_section.lines.push(line);
        } else if let Some(lyrics) = lyrics_track {
            // Lyrics only, no chords
            if let Some(lyric_line) = &lyrics.lyrics {
                let chunks = vec![ChordProChunk {
                    chord: None,
                    text: lyric_line.full_text(),
                }];
                cp_section.lines.push(ChordProLine::Lyric(chunks));
            }
        }

        if !cp_section.lines.is_empty() {
            doc.sections.push(cp_section);
        }
    }

    Ok(doc)
}

/// Get a label for a section (Verse 1, Chorus, etc.)
fn section_label(section: &keyflow_proto::Section) -> Option<String> {
    use keyflow_proto::SectionType;

    match &section.section_type {
        SectionType::Verse => {
            if let Some(num) = section.number {
                Some(format!("Verse {}", num))
            } else {
                Some("Verse".to_string())
            }
        }
        SectionType::Chorus => Some("Chorus".to_string()),
        SectionType::Bridge => Some("Bridge".to_string()),
        SectionType::Pre(inner) if matches!(inner.as_ref(), SectionType::Chorus) => {
            Some("Pre-Chorus".to_string())
        }
        SectionType::Intro => Some("Intro".to_string()),
        SectionType::Outro => Some("Outro".to_string()),
        SectionType::Custom(name) => Some(name.clone()),
        _ => None,
    }
}

/// Format chords from a track as a space-separated string
fn format_chords_from_track(track: &Track) -> String {
    track
        .measures
        .iter()
        .flat_map(|m| m.chords.iter())
        .map(|chord| chord.full_symbol.clone())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a chords track to a ChordProLine
fn chords_track_to_line(track: &Track) -> ChordProLine {
    let chunks: Vec<ChordProChunk> = track
        .measures
        .iter()
        .flat_map(|m| m.chords.iter())
        .map(|chord| ChordProChunk {
            chord: Some(chord.full_symbol.clone()),
            text: String::new(),
        })
        .collect();

    ChordProLine::ChordOnly(chunks)
}

/// Interleave chords with lyrics text
///
/// Simple strategy: align chord boundaries with word boundaries in lyrics
fn interleave_chords_and_lyrics(chords: &str, lyrics: &str) -> ChordProLine {
    let chord_list: Vec<&str> = chords.split_whitespace().collect();
    let word_list: Vec<&str> = lyrics.split_whitespace().collect();

    let mut chunks = Vec::new();

    for (i, word) in word_list.iter().enumerate() {
        let chord = chord_list.get(i).copied();
        chunks.push(ChordProChunk {
            chord: chord.map(|s| s.to_string()),
            text: if i < word_list.len() - 1 {
                format!("{} ", word)
            } else {
                word.to_string()
            },
        });
    }

    ChordProLine::Lyric(chunks)
}

/// Convert ChordProDocument to text
fn chordpro_document_to_text(doc: &ChordProDocument) -> String {
    let mut output = String::new();

    // Write directives
    for directive in &doc.directives {
        output.push_str(&format!("{{{}: {}}}\n", directive.name, directive.value));
    }

    if !doc.directives.is_empty() {
        output.push('\n');
    }

    // Write sections
    for section in &doc.sections {
        if let Some(label) = &section.label {
            output.push_str(&format!("[{}]\n", label));
        }

        for line in &section.lines {
            output.push_str(&line_to_text(line));
            output.push('\n');
        }

        output.push('\n');
    }

    output.trim().to_string() + "\n"
}

/// Convert a single ChordProLine to text
fn line_to_text(line: &ChordProLine) -> String {
    match line {
        ChordProLine::Lyric(chunks) => chunks
            .iter()
            .map(|chunk| {
                if let Some(chord) = &chunk.chord {
                    format!("[{}]{}", chord, chunk.text)
                } else {
                    chunk.text.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        ChordProLine::ChordOnly(chunks) => chunks
            .iter()
            .filter_map(|chunk| chunk.chord.as_deref())
            .collect::<Vec<_>>()
            .join(" "),
        ChordProLine::Comment(text) => format!("# {}", text),
        ChordProLine::Empty => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chordpro_document_to_text() {
        let mut doc = ChordProDocument::new();
        doc.directives
            .push(ChordProDirective::title("Amazing Grace"));

        let mut section = ChordProSection::new();
        section.label = Some("Verse".to_string());
        section.lines.push(ChordProLine::Lyric(vec![
            ChordProChunk::with_chord("C", "Amazing "),
            ChordProChunk::with_chord("G", "grace"),
        ]));
        doc.sections.push(section);

        let text = chordpro_document_to_text(&doc);
        assert!(text.contains("{title: Amazing Grace}"));
        assert!(text.contains("[Verse]"));
        assert!(text.contains("[C]"));
    }
}
