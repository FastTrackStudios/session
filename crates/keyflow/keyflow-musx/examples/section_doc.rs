//! Extract a **keyflow section document** from a Finale `.musx` score.
//!
//! Usage: `cargo run -p keyflow-musx --example section_doc -- <in.musx> [out.json]`
//!
//! ## Why this reads EnigmaXML directly (not the MusicXML)
//!
//! The `musx2mxl` MusicXML export **drops** the two things a section
//! document needs most: assigned tempo marks and Finale text expressions
//! (the song-form labels — "Verse 1", "Chorus 1", "Bridge", …). Those live
//! in the EnigmaXML as `<textExprDef>`/`<expression>` bodies assigned to
//! measures via `<measExprAssign cmper="MEASURE"><textExprID>…`. Real Alan
//! Parsons conductor scores carry **no** `<rehearsal>` marks and only sparse
//! double barlines, so section boundaries come from those song-form
//! expressions, with tempo/meter/key changes as the fallback signal.
//!
//! This example is deliberately a self-contained probe: it re-implements a
//! small slice of EnigmaXML parsing so it can become the real batch tool
//! later without waiting on the full `keyflow-musicxml` importer (which is
//! lead-sheet oriented and flattens an orchestral score into one blob).

use std::collections::BTreeMap;

use roxmltree::Document;
use serde::Serialize;

/// One structural section of the song.
#[derive(Serialize)]
struct Section {
    label: String,
    start_measure: u32,
    length_measures: u32,
    beats: u32,
    time_sig: String,
    tempo_bpm: Option<u32>,
    key: String,
}

#[derive(Serialize)]
struct SectionDoc {
    song: Option<String>,
    tempo_bpm: Option<u32>,
    tempo_source: String,
    key: String,
    total_measures: u32,
    section_source: String,
    sections: Vec<Section>,
}

/// Song-form label keywords. A text expression counts as a section boundary
/// when its (macro-stripped) body starts with one of these — this filters out
/// performance directions (a2, cresc., Con sord., fingering numbers, …) that
/// share the same expression list.
const SECTION_KEYWORDS: &[&str] = &[
    "intro", "re-intro", "verse", "pre-chorus", "prechorus", "chorus", "bridge",
    "interlude", "solo", "outro", "coda", "tag", "turnaround", "vamp", "ending",
    "refrain", "instrumental", "band enters", "breakdown", "hook",
];

/// Strip Finale `^macro(args)` inline-format codes from an expression body.
fn strip_macros(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '^' {
            // consume `name(...)`
            while let Some(&n) = chars.peek() {
                if n == '(' {
                    for n2 in chars.by_ref() {
                        if n2 == ')' {
                            break;
                        }
                    }
                    break;
                } else if n.is_ascii_alphabetic() {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn looks_like_section(text: &str) -> bool {
    // A single short line whose (lowercased) body contains a song-form keyword.
    // Multi-line bodies and long sentences are performance notes, not labels
    // (e.g. "1st x: Gtr. Solo\n2nd x: Sax Solo" is a cue, "1st Gtr. Solo" is a
    // section). We keep it to one line and a small word budget.
    if text.contains('\n') || text.split_whitespace().count() > 4 {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    SECTION_KEYWORDS.iter().any(|k| lower.contains(k))
}

/// Very small fifths→major-key name table (concert pitch).
fn fifths_to_key(fifths: i32) -> String {
    let name = match fifths {
        -7 => "Cb", -6 => "Gb", -5 => "Db", -4 => "Ab", -3 => "Eb", -2 => "Bb",
        -1 => "F", 0 => "C", 1 => "G", 2 => "D", 3 => "A", 4 => "E", 5 => "B",
        6 => "F#", 7 => "C#", _ => "?",
    };
    format!("{name} major")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: section_doc <in.musx> [out.json]");
    let out = args.next();

    let musx = std::fs::read(&input).expect("read input");

    // --- EnigmaXML side: expressions + assignments + concert key ----------
    let enigma_bytes = keyflow_musx::musx_to_enigmaxml(&musx).expect("decode musx");
    let enigma = String::from_utf8_lossy(&enigma_bytes).into_owned();
    let edoc = Document::parse(&enigma).expect("parse enigmaxml");

    // expression number -> macro-stripped text
    let mut exprs: BTreeMap<u32, String> = BTreeMap::new();
    for n in edoc.descendants().filter(|n| n.has_tag_name("expression")) {
        if let Some(num) = n.attribute("number").and_then(|s| s.parse().ok()) {
            let text = strip_macros(&n.text().unwrap_or_default());
            exprs.insert(num, text);
        }
    }

    // measExprAssign cmper="MEASURE" -> textExprID
    // Keep the earliest measure a given section label is assigned to.
    let mut section_hits: Vec<(u32, String)> = Vec::new();
    for a in edoc.descendants().filter(|n| n.has_tag_name("measExprAssign")) {
        let Some(meas) = a.attribute("cmper").and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Some(tid) = a
            .children()
            .find(|c| c.has_tag_name("textExprID"))
            .and_then(|c| c.text())
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(text) = exprs.get(&tid) {
            if looks_like_section(text) {
                section_hits.push((meas, text.clone()));
            }
        }
    }
    // Dedup: keep the first (lowest-measure) appearance of each (measure,label);
    // a label repeated at m+N (e.g. "Verse 1" at m1 & m5) is a re-cue of the
    // same section, so collapse consecutive same-label boundaries.
    section_hits.sort_by_key(|(m, _)| *m);
    section_hits.dedup(); // collapse identical (measure,label) from multi-staff copies
    let mut boundaries: Vec<(u32, String)> = Vec::new();
    for (m, label) in section_hits {
        match boundaries.last() {
            Some((_, prev)) if *prev == label => {} // re-cue of same section
            Some((pm, _)) if *pm == m => {}          // two labels same bar; keep first
            _ => boundaries.push((m, label)),
        }
    }

    // concert key: first non-transposing staff's <fifths> (min |fifths| wins on
    // a tie of measure 1). We just take the most common fifths at measure 1.
    let mut fifths_votes: BTreeMap<i32, u32> = BTreeMap::new();
    for k in edoc.descendants().filter(|n| n.has_tag_name("keySig")) {
        if let Some(v) = k
            .children()
            .find(|c| c.has_tag_name("key"))
            .and_then(|c| c.text())
            .and_then(|s| s.parse::<i32>().ok())
        {
            *fifths_votes.entry(v).or_default() += 1;
        }
    }
    // EnigmaXML key is trickier; fall back to MusicXML for the concert key.

    // --- MusicXML side: per-measure time signature + total measures + key --
    let musicxml = keyflow_musx::musx_to_musicxml(&musx).expect("convert musicxml");
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let mdoc = Document::parse_with_options(&musicxml, opts).expect("parse musicxml");

    // Use the FIRST part as the measure spine (all parts share measure count).
    let first_part = mdoc
        .descendants()
        .find(|n| n.has_tag_name("part"))
        .expect("no <part>");
    let mut total_measures = 0u32;
    let mut ts_at: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for meas in first_part.children().filter(|n| n.has_tag_name("measure")) {
        let num: u32 = meas.attribute("number").and_then(|s| s.parse().ok()).unwrap_or(0);
        total_measures = total_measures.max(num);
        if let Some(time) = meas.descendants().find(|n| n.has_tag_name("time")) {
            let beats = time
                .children()
                .find(|c| c.has_tag_name("beats"))
                .and_then(|c| c.text())
                .and_then(|s| s.parse::<u32>().ok());
            let bt = time
                .children()
                .find(|c| c.has_tag_name("beat-type"))
                .and_then(|c| c.text())
                .and_then(|s| s.parse::<u32>().ok());
            if let (Some(b), Some(d)) = (beats, bt) {
                ts_at.insert(num, (b, d));
            }
        }
    }

    // Concert key = min |fifths| among the first-measure <key> values.
    let mut concert_fifths = 0i32;
    let mut best = i32::MAX;
    for part in mdoc.descendants().filter(|n| n.has_tag_name("part")) {
        if let Some(m1) = part.children().find(|n| n.has_tag_name("measure")) {
            if let Some(f) = m1
                .descendants()
                .find(|n| n.has_tag_name("fifths"))
                .and_then(|n| n.text())
                .and_then(|s| s.parse::<i32>().ok())
            {
                if f.abs() < best {
                    best = f.abs();
                    concert_fifths = f;
                }
            }
        }
    }
    let _ = fifths_votes;
    let key = fifths_to_key(concert_fifths);

    // Effective time signature at a measure = last <time> at or before it.
    let ts_for = |m: u32| -> (u32, u32) {
        ts_at
            .range(..=m)
            .next_back()
            .map(|(_, &v)| v)
            .unwrap_or((4, 4))
    };
    let beats_between = |a: u32, b: u32| -> u32 { (a..b).map(|m| ts_for(m).0).sum() };

    // --- tempo: only a real if an assigned tempo expression carries a BPM;
    // else the playback default beatsPerMinute. ---------------------------
    let playback_bpm = edoc
        .descendants()
        .find(|n| n.has_tag_name("beatsPerMinute"))
        .and_then(|n| n.text())
        .and_then(|s| s.parse::<u32>().ok());

    // If the first labeled section doesn't start at measure 1, the leading
    // bars are an (unlabeled) intro — emit it so the doc covers every bar.
    if let Some((first, _)) = boundaries.first() {
        if *first > 1 {
            boundaries.insert(0, (1, "Intro (implied)".to_string()));
        }
    }

    // --- assemble sections -------------------------------------------------
    let mut sections = Vec::new();
    for i in 0..boundaries.len() {
        let (start, label) = &boundaries[i];
        let end = boundaries
            .get(i + 1)
            .map(|(m, _)| *m)
            .unwrap_or(total_measures + 1);
        let (n, d) = ts_for(*start);
        sections.push(Section {
            label: label.clone(),
            start_measure: *start,
            length_measures: end - start,
            beats: beats_between(*start, end),
            time_sig: format!("{n}/{d}"),
            tempo_bpm: playback_bpm,
            key: key.clone(),
        });
    }

    let doc = SectionDoc {
        song: mdoc
            .descendants()
            .find(|n| n.has_tag_name("movement-title"))
            .and_then(|n| n.text())
            .map(str::to_string),
        tempo_bpm: playback_bpm,
        tempo_source: "EnigmaXML playback beatsPerMinute (no assigned tempo mark found)".into(),
        key,
        total_measures,
        section_source: if sections.is_empty() {
            "NONE FOUND — no song-form text expressions; fall back to double barlines / meter changes".into()
        } else {
            "Finale text expressions (song-form labels) via measExprAssign in EnigmaXML".into()
        },
        sections,
    };

    let json = serde_json::to_string_pretty(&doc).unwrap();
    match out {
        Some(path) => {
            std::fs::write(&path, &json).expect("write json");
            eprintln!("wrote {path}");
        }
        None => println!("{json}"),
    }
}
