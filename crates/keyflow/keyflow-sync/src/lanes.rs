//! Minimal reader for the lane-based `.kf` format.
//!
//! A `.kf` is a header line + metadata line, then named `name { ... }` lane
//! blocks that all index into a shared `sections {}` spine:
//!
//! ```text
//! Keep on Finding More - John Allan
//! 118bpm 4/4 #C
//! sections { IN · VS1 · CH · VS1 · OUTRO }
//! rhythm   { IN  1 6m7 5 4   VS1 1 6m7 5 4 ... }
//! lyrics   { VS1  [1]More than I ask... }
//! sync     { ... }            ; generated
//! ```
//!
//! This reader extracts what the audio pipeline needs (tempo + per-section
//! lyrics, expanded along the spine into playback order). Chart rendering still
//! goes through the full keyflow parser; this is the lightweight projection the
//! alignment / MIDI / Reaper export consume.

use std::collections::BTreeMap;

use crate::error::{Result, SyncError};
use crate::pipeline::SectionLyrics;

#[derive(Debug, Clone)]
pub struct LaneChart {
    pub title: String,
    pub bpm: f32,
    pub key: Option<String>,
    pub time_sig: (u8, u8),
    /// Section labels in playback order (the spine).
    pub spine: Vec<String>,
    /// label → raw chord line(s) from the `rhythm` lane.
    pub rhythm: BTreeMap<String, String>,
    /// label → ChordPro lyric lines (verbatim, chords intact) from `lyrics`.
    pub lyrics: BTreeMap<String, Vec<String>>,
}

impl LaneChart {
    /// Expand the spine into one [`SectionLyrics`] per playback occurrence that
    /// carries lyrics. `section` is the spine index, so repeated sections (a
    /// chorus sung 4×) align independently against their own audio.
    pub fn playback_lyrics(&self) -> Vec<SectionLyrics> {
        let mut out = Vec::new();
        for (i, label) in self.spine.iter().enumerate() {
            if let Some(lines) = self.lyrics.get(label) {
                let text = lines
                    .iter()
                    .map(|l| strip_chords(l))
                    .collect::<Vec<_>>()
                    .join(" ");
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    out.push(SectionLyrics {
                        section: i as u32,
                        text,
                    });
                }
            }
        }
        out
    }

    /// Spine index → label, for labelling output.
    pub fn label_at(&self, spine_index: usize) -> Option<&str> {
        self.spine.get(spine_index).map(|s| s.as_str())
    }
}

/// Remove `[chord]` markers from a ChordPro line, leaving the lyric text.
pub fn strip_chords(line: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for ch in line.chars() {
        match ch {
            '[' => depth += 1,
            ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Parse the lane-format `.kf`.
pub fn parse(text: &str) -> Result<LaneChart> {
    // Strip `;` line comments and blank-out, but keep structure.
    let lines: Vec<&str> = text.lines().collect();
    let mut meaningful = lines.iter().filter(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with(';')
    });

    let title = meaningful
        .next()
        .ok_or_else(|| SyncError::Sidecar("empty chart".into()))?
        .trim()
        .to_string();
    let meta = meaningful
        .next()
        .ok_or_else(|| SyncError::Sidecar("missing metadata line".into()))?;
    let (bpm, key, time_sig) = parse_meta(meta);

    // Re-join (sans comments) and pull out brace blocks by name.
    let body: String = lines
        .iter()
        .map(|l| match l.find(';') {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let spine = match extract_block(&body, "sections") {
        Some(s) => s
            .split(|c: char| c == '·' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect(),
        None => Vec::new(),
    };

    let rhythm = extract_block(&body, "rhythm")
        .map(parse_labelled_lines)
        .unwrap_or_default();

    let lyrics = extract_block(&body, "lyrics")
        .map(parse_lyrics_block)
        .unwrap_or_default();

    Ok(LaneChart {
        title,
        bpm,
        key,
        time_sig,
        spine,
        rhythm,
        lyrics,
    })
}

fn parse_meta(meta: &str) -> (f32, Option<String>, (u8, u8)) {
    let mut bpm = 120.0;
    let mut key = None;
    let mut ts = (4u8, 4u8);
    for tok in meta.split_whitespace() {
        if let Some(b) = tok.strip_suffix("bpm").or_else(|| tok.strip_suffix("BPM")) {
            if let Ok(v) = b.parse() {
                bpm = v;
            }
        } else if let Some(k) = tok.strip_prefix('#') {
            key = Some(k.to_string());
        } else if let Some((n, d)) = tok.split_once('/') {
            if let (Ok(n), Ok(d)) = (n.parse(), d.parse()) {
                ts = (n, d);
            }
        }
    }
    (bpm, key, ts)
}

/// Find `name { ... }` and return the brace contents (handles one level).
fn extract_block<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let mut search = 0;
    while let Some(rel) = body[search..].find(name) {
        let at = search + rel;
        // must be a whole word followed (after ws) by `{`
        let before_ok = at == 0 || !body.as_bytes()[at - 1].is_ascii_alphanumeric();
        let after = &body[at + name.len()..];
        let after_trim = after.trim_start();
        if before_ok && after_trim.starts_with('{') {
            let open = body.len() - after_trim.len(); // index of '{'
            let inner_start = open + 1;
            // find matching close
            let mut depth = 1;
            for (i, c) in body[inner_start..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&body[inner_start..inner_start + i]);
                        }
                    }
                    _ => {}
                }
            }
            return Some(&body[inner_start..]);
        }
        search = at + name.len();
    }
    None
}

/// `LABEL rest...` per line → map label → rest.
fn parse_labelled_lines(block: &str) -> BTreeMap<String, String> {
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

/// Lyrics block: a bare label line starts a section; following lines (until the
/// next bare label) are its ChordPro lines. A "bare label" is a line with no
/// space and no `[` that matches a section-label shape (letters/digits).
fn parse_lyrics_block(block: &str) -> BTreeMap<String, Vec<String>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Keep on Finding More - John Allan\n\
        118bpm 4/4 #C\n\
        sections { IN · VS1 · CH · VS1 · OUTRO }\n\
        rhythm {\n  IN  1 6m7 5 4\n  VS1 1 6m7 5 4\n  CH 4 1 5 (6 5)\n}\n\
        lyrics {\n  VS1\n    [1]More than I ask, i[6m7]magine\n    You're [5]better\n  CH\n    [4]You are the [1]maker\n}\n";

    #[test]
    fn parses_header_and_spine() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.title, "Keep on Finding More - John Allan");
        assert_eq!(c.bpm, 118.0);
        assert_eq!(c.key.as_deref(), Some("C"));
        assert_eq!(c.time_sig, (4, 4));
        assert_eq!(c.spine, ["IN", "VS1", "CH", "VS1", "OUTRO"]);
    }

    #[test]
    fn extracts_lyrics_and_strips_chords() {
        let c = parse(SAMPLE).unwrap();
        assert!(c.lyrics.contains_key("VS1"));
        assert_eq!(
            strip_chords("[1]More than I ask, i[6m7]magine"),
            "More than I ask, imagine"
        );
    }

    #[test]
    fn expands_spine_into_playback_lyrics() {
        let c = parse(SAMPLE).unwrap();
        let pl = c.playback_lyrics();
        // VS1 appears at spine 1 and 3, CH at 2 → three lyric-bearing sections.
        assert_eq!(pl.len(), 3);
        assert_eq!(pl[0].section, 1); // first VS1
        assert_eq!(pl[1].section, 2); // CH
        assert_eq!(pl[2].section, 3); // repeated VS1
        assert!(pl[0].text.starts_with("More than I ask"));
    }
}
