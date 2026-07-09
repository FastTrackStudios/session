//! Transpile the **block-lane** `.kf` format into classic keyflow section text,
//! so the main parser consumes it and produces a full `Chart` graph.
//!
//! Block format (readable, single-file, no `--- ---` delimiters):
//! ```text
//! Keep on Finding More - John Allan
//! 118bpm 4/4 #C
//! sections { IN · VS1 · CH · VS1 · OUTRO }
//! rhythm   { IN  1 6m7 5 4   VS1 1 6m7 5 4   CH 4 1 5 (6 5) }
//! lyrics   { VS1  [1]More than I ask... }
//! sync     { ... }            ; ignored here (timing sidecar)
//! ```
//!
//! Each spine entry becomes a classic section (its label is the header), with
//! the label's `rhythm` chord line(s) and, where present, an inline `[lyrics]`
//! ChordPro block — both of which the classic section parser already handles.
//! The `sync` lane is intentionally dropped (it's per-syllable audio timing,
//! not chart content).

use std::collections::BTreeMap;

/// True if the input uses the block-lane format (a top-level `sections {`/
/// `rhythm {`/`lyrics {` declaration), rather than classic section syntax.
pub fn is_block_format(input: &str) -> bool {
    input.lines().any(|l| {
        let t = l.trim_start();
        for name in ["sections", "rhythm", "lyrics"] {
            if let Some(rest) = t.strip_prefix(name) {
                if rest.trim_start().starts_with('{') {
                    return true;
                }
            }
        }
        false
    })
}

/// Transpile block-lane text into classic keyflow text.
pub fn transpile(input: &str) -> Result<String, String> {
    // Strip `;` line comments for block extraction (keep line structure).
    let stripped: String = input
        .lines()
        .map(|l| match l.find(';') {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Header = the first two meaningful lines (they precede every block).
    let mut hdr = input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(';'));
    let title = hdr.next().unwrap_or("Untitled");
    let meta = hdr.next().unwrap_or("120bpm 4/4");

    let spine: Vec<String> = extract_block(&stripped, "sections")
        .map(|s| {
            s.split(|c: char| c == '·' || c.is_whitespace())
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let rhythm = extract_block(&stripped, "rhythm")
        .map(parse_labelled)
        .unwrap_or_default();
    let lyrics = extract_block(&stripped, "lyrics")
        .map(parse_lyrics)
        .unwrap_or_default();

    if spine.is_empty() {
        return Err("block format: empty or missing `sections {}` spine".into());
    }

    let mut out = String::new();
    out.push_str(title);
    out.push('\n');
    out.push_str(meta);
    out.push_str("\n\n");
    for label in &spine {
        // Bracket the label so the classic parser treats it as a distinct
        // Custom section header (VS1 ≠ VS2; repeats stay separate sections).
        out.push('[');
        out.push_str(label);
        out.push(']');
        out.push('\n');
        if let Some(r) = rhythm.get(label) {
            out.push_str(r.trim());
            out.push('\n');
        }
        if let Some(ls) = lyrics.get(label) {
            if !ls.is_empty() {
                out.push_str("[lyrics]\n");
                for l in ls {
                    out.push_str(l);
                    out.push('\n');
                }
            }
        }
        out.push('\n');
    }
    Ok(out)
}

/// Contents between the braces of `name { ... }` (one nesting level).
fn extract_block<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let mut search = 0;
    while let Some(rel) = body[search..].find(name) {
        let at = search + rel;
        let before_ok = at == 0 || !body.as_bytes()[at - 1].is_ascii_alphanumeric();
        let after = &body[at + name.len()..];
        let after_trim = after.trim_start();
        if before_ok && after_trim.starts_with('{') {
            let open = body.len() - after_trim.len();
            let inner = open + 1;
            let mut depth = 1;
            for (i, c) in body[inner..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&body[inner..inner + i]);
                        }
                    }
                    _ => {}
                }
            }
            return Some(&body[inner..]);
        }
        search = at + name.len();
    }
    None
}

/// `LABEL rest...` per line → map label → rest.
fn parse_labelled(block: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((label, rest)) = line.split_once(char::is_whitespace) {
            out.insert(label.to_string(), rest.trim().to_string());
        } else {
            out.insert(line.to_string(), String::new());
        }
    }
    out
}

/// Lyrics block: a bare label line starts a section; following lines are its
/// ChordPro lines until the next bare label.
fn parse_lyrics(block: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_label = !trimmed.contains('[')
            && !trimmed.contains(char::is_whitespace)
            && trimmed.chars().all(|c| c.is_ascii_alphanumeric());
        if is_label {
            current = Some(trimmed.to_string());
            out.entry(trimmed.to_string()).or_default();
        } else if let Some(label) = &current {
            out.get_mut(label).unwrap().push(trimmed.to_string());
        }
    }
    out
}

// ── voice lanes (melody + per-note lyrics, per vocalist) ────────────────────

use keyflow_proto::chart::melody::Melody;
use keyflow_proto::chart::track::Track;
use keyflow_proto::chart::types::ChartSection;
use keyflow_proto::sections::section_type::SectionType;

struct Voice {
    name: String,
    color: Option<String>,
    /// label → (melody line, optional words line)
    sections: Vec<(String, String, Option<String>)>,
}

/// Parse every `voice <name> [color=<c>] { ... }` block from the source.
fn parse_voices(src: &str) -> Vec<Voice> {
    let stripped: String = src
        .lines()
        .map(|l| match l.find(';') {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut voices = Vec::new();
    let mut search = 0;
    while let Some(rel) = stripped[search..].find("voice") {
        let at = search + rel;
        let before_ok = at == 0 || !stripped.as_bytes()[at - 1].is_ascii_alphanumeric();
        // header runs from after "voice" to the `{`
        let rest = &stripped[at + "voice".len()..];
        let Some(brace_rel) = rest.find('{') else {
            break;
        };
        if !before_ok
            || !rest[..brace_rel]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            search = at + "voice".len();
            continue;
        }
        let header = rest[..brace_rel].trim();
        let (name, color) = parse_voice_header(header);
        // braces content
        let inner_start = at + "voice".len() + brace_rel + 1;
        let mut depth = 1;
        let mut end = inner_start;
        for (i, c) in stripped[inner_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = inner_start + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        voices.push(Voice {
            name,
            color,
            sections: parse_voice_sections(&stripped[inner_start..end]),
        });
        search = end + 1;
    }
    voices
}

fn parse_voice_header(header: &str) -> (String, Option<String>) {
    let mut name = String::new();
    let mut color = None;
    for tok in header.split_whitespace() {
        if let Some(c) = tok.strip_prefix("color=") {
            color = Some(c.to_string());
        } else if name.is_empty() {
            name = tok.to_string();
        }
    }
    if name.is_empty() {
        name = "voice".into();
    }
    (name, color)
}

/// Inside a voice: a bare label starts a section; `m:` / `w:` lines fill it.
fn parse_voice_sections(block: &str) -> Vec<(String, String, Option<String>)> {
    let mut out: Vec<(String, String, Option<String>)> = Vec::new();
    for line in block.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix("m:") {
            if let Some(last) = out.last_mut() {
                last.1 = rest.trim().to_string();
            }
        } else if let Some(rest) = t.strip_prefix("w:") {
            if let Some(last) = out.last_mut() {
                last.2 = Some(rest.trim().to_string());
            }
        } else if !t.contains(char::is_whitespace) && t.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push((t.to_string(), String::new(), None));
        }
    }
    out
}

/// Zip an ABC/LilyPond-style `w:` syllable stream onto the melody's notes:
/// one syllable per non-rest note; `_` (or `*`/`__`) is a melisma continuation
/// (note carries no new syllable); `--`/`-` are word-internal hyphen separators
/// that consume no note.
fn assign_lyrics(melody: &mut Melody, words: &str) {
    let tokens: Vec<&str> = words.split_whitespace().collect();
    let mut notes = melody.notes.iter_mut().filter(|n| !n.is_rest());
    for tok in tokens {
        if tok == "--" || tok == "-" {
            continue; // hyphen separator: no note consumed
        }
        let Some(note) = notes.next() else { break };
        note.lyric = match tok {
            "_" | "*" | "__" => None, // melisma / held / skip
            s => Some(s.to_string()),
        };
    }
}

/// Parse the source's `voice` lanes and attach each as a named, colored melody
/// track (notes carrying lyrics) to every section matching its label.
pub fn attach_voices(sections: &mut [ChartSection], src: &str) {
    for v in parse_voices(src) {
        for (label, mline, wline) in &v.sections {
            if mline.trim().is_empty() {
                continue;
            }
            let Ok(mut melody) = Melody::parse(mline) else {
                continue;
            };
            if let Some(w) = wline {
                assign_lyrics(&mut melody, w);
            }
            let mut track = Track::melody(melody).with_name(v.name.clone());
            if let Some(c) = &v.color {
                track = track.with_color(c.clone());
            }
            for sec in sections.iter_mut() {
                if matches!(&sec.section.section_type, SectionType::Custom(n) if n == label) {
                    sec.tracks.push(track.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Title - Artist\n118bpm 4/4 #C\n\
        sections { IN · VS1 · VS1 }\n\
        rhythm { IN 1 6m7 5 4\n VS1 1 6m7 5 4 }\n\
        lyrics { VS1\n [1]More than I ask\n }\n\
        sync { [1:VS1] 1.0 2.0 }\n";

    #[test]
    fn detects_block_format() {
        assert!(is_block_format(SAMPLE));
        assert!(!is_block_format("VS\n1 6m7 5 4"));
    }

    #[test]
    fn parses_voice_header_and_sections() {
        let src = "T\n120bpm 4/4 #C\nsections { VS1 }\n\
            voice lead color=#E2574C {\n  VS1\n    m: C D E\n    w: More than dream\n}\n";
        let voices = parse_voices(src);
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].name, "lead");
        assert_eq!(voices[0].color.as_deref(), Some("#E2574C"));
        let (label, m, w) = &voices[0].sections[0];
        assert_eq!(label, "VS1");
        assert_eq!(m, "C D E");
        assert_eq!(w.as_deref(), Some("More than dream"));
    }

    #[test]
    fn assigns_one_syllable_per_note_skipping_hyphens() {
        let mut melody = Melody::parse("C D E F").unwrap();
        assign_lyrics(&mut melody, "i -- ma -- gine more");
        let got: Vec<_> = melody.notes.iter().map(|n| n.lyric.clone()).collect();
        assert_eq!(
            got,
            vec![
                Some("i".into()),
                Some("ma".into()),
                Some("gine".into()),
                Some("more".into())
            ]
        );
    }

    #[test]
    fn melisma_underscore_holds_previous_syllable() {
        let mut melody = Melody::parse("B A G").unwrap();
        assign_lyrics(&mut melody, "dream _ _");
        let got: Vec<_> = melody.notes.iter().map(|n| n.lyric.clone()).collect();
        assert_eq!(got, vec![Some("dream".into()), None, None]);
    }

    #[test]
    fn voice_lanes_attach_as_colored_melody_tracks_with_lyrics() {
        let src = "Demo\n120bpm 4/4 #C\n\
            sections { VS1 · CH }\n\
            rhythm { VS1 1 4 5 1   CH 4 1 5 1 }\n\
            voice lead color=#E2574C {\n  CH\n    m: G A B C B A G\n    w: You are the maker dream _ _\n}\n\
            voice bgv color=#4C82E2 {\n  CH\n    m: E E E E\n    w: oh oh oh oh\n}\n";
        let chart = crate::chart::parse_chart(src).unwrap();
        let ch = chart
            .sections
            .iter()
            .find(|s| matches!(&s.section.section_type, SectionType::Custom(n) if n == "CH"))
            .expect("CH section");
        let lead = ch
            .tracks
            .iter()
            .find(|t| t.name.as_deref() == Some("lead"))
            .expect("lead voice track");
        assert_eq!(lead.color.as_deref(), Some("#E2574C"));
        let lyrics: Vec<_> = lead
            .melody
            .as_ref()
            .expect("lead melody")
            .notes
            .iter()
            .map(|n| n.lyric.as_deref())
            .collect();
        assert_eq!(
            lyrics,
            vec![
                Some("You"),
                Some("are"),
                Some("the"),
                Some("maker"),
                Some("dream"),
                None, // melisma: "dream" held over these 2 notes
                None,
            ]
        );
        let bgv = ch
            .tracks
            .iter()
            .find(|t| t.name.as_deref() == Some("bgv"))
            .expect("bgv voice track");
        assert_eq!(bgv.color.as_deref(), Some("#4C82E2"));
    }

    #[test]
    fn transpiles_spine_with_repeats_and_lyrics() {
        let out = transpile(SAMPLE).unwrap();
        assert!(out.starts_with("Title - Artist\n118bpm 4/4 #C\n"));
        // IN once + VS1 twice (spine order) = 3 bracketed section headers.
        assert_eq!(out.matches("[IN]").count(), 1);
        assert_eq!(out.matches("[VS1]").count(), 2);
        // both VS1 occurrences carry lyrics; IN does not.
        assert_eq!(out.matches("[lyrics]").count(), 2);
        assert!(out.contains("[1]More than I ask"));
        assert!(!out.contains("sync")); // sync lane dropped
    }
}
