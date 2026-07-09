//! Translate parsed MusicXML into a keyflow [`Chart`].
//!
//! The mapping is intentionally close to MuseScore's importexport/musicxml
//! reader (`importexport/musicxml/internal/`) — same naming, same primary
//! routing of elements per measure.
//!
//! Element coverage (Lord of the Fight baseline):
//! - `<movement-title>` + `<identification>` → `SongMetadata`
//! - `<attributes><time>` → `Measure.time_signature`
//! - `<harmony>`        → empty (placeholder); chord-text conversion TBD
//! - `<barline>`        → `Measure.end_barline`, `end_repeat`, `start_repeat`
//! - `<barline><ending>` (start) → `Measure.volta_start`
//! - `<direction><wedge>` → `Measure.hairpins`
//! - `<direction><dynamics>` → `Measure.classical_dynamics`
//! - `<direction><words>` (numeric N-N) → `Measure.figured_bass`
//! - `<direction><words>` (other)        → `Measure.staff_text`
//! - `<direction><rehearsal>`            → `Measure.staff_text` (boxed)
//!
//! Harmony→ChordInstance conversion (with `bass_vertical` from
//! `<bass arrangement="vertical">`) is the next layer; the wiring exists but
//! ChordInstance construction needs the keyflow chord tokenizer fed the
//! reassembled symbol string. Left as a TODO so this importer compiles and
//! exercises the new chart-level fields end to end.

use musicxml::datatypes::{
    BackwardForward, BarStyle as XmlBarStyle, HarmonyArrangement, StartStop, StartStopDiscontinue,
    WedgeType,
};
use musicxml::elements::{
    Barline, Direction, DirectionTypeContents, MeasureElement, MeasureStyleContents, PartElement,
    ScorePartwise, Time, Words,
};

use keyflow_proto::chart::notations::{
    BarlineStyle, Dynamic, DynamicLevel, FiguredBass, FiguredBassRow, Hairpin, HairpinKind,
    Placement, RepeatMark, StaffText, Volta,
};
use keyflow_proto::chart::types::Measure;
use keyflow_proto::chart::{Chart, ChartClef, ChartSection};
use keyflow_proto::metadata::SongMetadata;
use keyflow_proto::sections::section::Section;
use keyflow_proto::sections::section_type::SectionType;
use keyflow_proto::time::Tempo;

/// Top-level entry: rebuild a [`Chart`] from a parsed [`ScorePartwise`].
pub fn chart_from_score(score: &ScorePartwise) -> Chart {
    let mut chart = Chart::new();
    chart.metadata = metadata_from_score(score);
    if let Some(bpm) = chart.metadata.tempo {
        chart.tempo = Some(Tempo::from_bpm(f64::from(bpm)));
    }

    // Pull initial clef + key signature from the first `<attributes>` block.
    let (clef, key) = extract_initial_clef_and_key(score);
    chart.initial_clef = clef;
    chart.initial_key = key.clone();
    chart.current_key = key;

    let mut measures = collect_measures(score);
    extract_footers(&mut measures, &mut chart);

    // Lift the first measure's time signature onto the chart so the engraver
    // renders the right prefix (treble + key + time) at system start. Without
    // this, the chart defaults to 4/4 even when every measure carries 6/8.
    if let Some(first) = measures.first() {
        let (n, d) = first.time_signature;
        let ts = keyflow_proto::time::TimeSignature {
            numerator: n as u32,
            denominator: d as u32,
        };
        chart.initial_time_signature = Some(ts);
        chart.time_signature = Some(ts);
    }

    let count_in_measures = detect_count_in_measure_count(&measures);
    for chart_section in split_into_sections(measures, count_in_measures) {
        chart.sections.push(chart_section);
    }
    chart
}

/// Heuristic: scan the first few measures for tell-tale count-in text
/// directions (`"Click-Count"`, `"***2 Bars***"`, `"Click"`, etc.) and
/// return how many of them are count-in. Returns 0 if none look like a
/// count-in.
fn detect_count_in_measure_count(measures: &[Measure]) -> usize {
    let mut count = 0;
    for m in measures.iter().take(4) {
        let looks_like_countin = m.staff_text.iter().any(|t| {
            let lower = t.text.to_ascii_lowercase();
            lower.contains("click-count")
                || lower.contains("click count")
                || lower == "click"
                || lower.contains("bars*") // matches "***2 Bars***"
                || lower.starts_with("count")
        });
        if looks_like_countin {
            count = (count + 1).max(
                m.staff_text
                    .iter()
                    .filter_map(|t| {
                        // "***2 Bars***" → 2
                        let txt = t.text.replace('*', "").trim().to_string();
                        let mut digits = String::new();
                        for c in txt.chars() {
                            if c.is_ascii_digit() {
                                digits.push(c);
                            } else if !digits.is_empty() {
                                break;
                            }
                        }
                        digits.parse::<usize>().ok()
                    })
                    .max()
                    .unwrap_or(1),
            );
            // Don't keep walking past the count-in run; stop at the first
            // measure WITHOUT count-in text after we've started.
        } else if count > 0 {
            break;
        }
    }
    count.min(measures.len())
}

/// Pull copyright-like lines out of measure staff_text and stash on metadata.
fn extract_footers(measures: &mut [Measure], chart: &mut Chart) {
    for m in measures.iter_mut() {
        m.staff_text.retain(|t| {
            if is_copyright_line(&t.text) {
                if chart.metadata.copyright.is_none() {
                    chart.metadata.copyright = Some(t.text.clone());
                }
                false
            } else {
                true
            }
        });
    }
}

fn is_copyright_line(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    s.contains('©')
        || lower.contains("copyright")
        || lower.contains("all rights reserved")
        || lower.starts_with("this arrangement")
        || lower.contains("ccli")
}

/// Walk the measure list and start a new ChartSection at every measure that
/// carries a section-marker label. The first `count_in_measures` go into a
/// `CountIn` section; everything between the count-in and the first explicit
/// marker becomes `Opening` (groove builds, fade-ins, "the CLIMB"). When the
/// first explicit marker matches the auto-opened section type, we retag in
/// place rather than emitting a duplicate.
fn split_into_sections(measures: Vec<Measure>, count_in_measures: usize) -> Vec<ChartSection> {
    if measures.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<ChartSection> = Vec::new();
    let mut current_section: Section;
    let mut current_measures: Vec<Measure> = Vec::new();
    let mut current_is_explicit = false;

    // Slice off the count-in prelude into its own CountIn section.
    let mut iter = measures.into_iter();
    if count_in_measures > 0 {
        let mut count_in_acc = Vec::with_capacity(count_in_measures);
        for _ in 0..count_in_measures {
            if let Some(m) = iter.next() {
                count_in_acc.push(m);
            }
        }
        if !count_in_acc.is_empty() {
            out.push(
                ChartSection::new(Section::new(SectionType::CountIn)).with_measures(count_in_acc),
            );
        }
        current_section = Section::new(SectionType::Opening);
    } else {
        current_section = Section::new(SectionType::Intro);
    }

    for mut m in iter {
        let label = take_section_label(&mut m);
        if let Some((sec, raw_label)) = label {
            // Don't bother pushing a separator if the only thing in the
            // current section is the implicit Intro AND its type matches
            // the new marker — just retag in place.
            let same_as_auto =
                !current_is_explicit && current_section.section_type == sec.section_type;
            if same_as_auto {
                current_section.number = sec.number;
                current_section.split_letter = sec.split_letter;
                current_section.name = Some(raw_label);
                current_is_explicit = true;
            } else {
                if !current_measures.is_empty() {
                    let finished_section =
                        std::mem::replace(&mut current_section, Section::new(SectionType::Intro));
                    out.push(
                        ChartSection::new(finished_section)
                            .with_measures(std::mem::take(&mut current_measures)),
                    );
                }
                current_section = sec;
                current_section.name = Some(raw_label);
                current_is_explicit = true;
            }
        }
        attach_repeat_pass_instructions(&mut current_section, &mut m);
        current_measures.push(m);
    }
    if !current_measures.is_empty() {
        out.push(ChartSection::new(current_section).with_measures(current_measures));
    }
    out
}

fn attach_repeat_pass_instructions(section: &mut Section, measure: &mut Measure) {
    let mut labels = Vec::new();
    measure.staff_text.retain(|text| {
        let Some((label, instruction)) = parse_repeat_pass_instruction(&text.text) else {
            return true;
        };
        let Some(label_section) = parse_section_label(&label) else {
            return true;
        };
        if label_section.section_type != section.section_type {
            return true;
        }

        let key_label = repeat_pass_metadata_label(&label);
        section.set_metadata(
            format!("repeat_pass.{key_label}.instruction"),
            instruction.trim(),
        );
        labels.push(label);
        false
    });

    if !labels.is_empty() {
        let existing = section
            .metadata
            .get("repeat_pass.labels")
            .map(String::as_str)
            .unwrap_or("");
        let mut merged = existing
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        for label in labels {
            if !merged.iter().any(|existing| existing == &label) {
                merged.push(label);
            }
        }
        section.set_metadata("repeat_pass.labels", merged.join("\n"));
    }
}

fn parse_repeat_pass_instruction(text: &str) -> Option<(String, String)> {
    let (label, instruction) = text.split_once('=')?;
    let label = label.trim();
    let instruction = instruction.trim();
    if label.is_empty() || instruction.is_empty() {
        return None;
    }
    Some((label.to_string(), instruction.to_string()))
}

fn repeat_pass_metadata_label(label: &str) -> String {
    label
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c)
            } else if c.is_whitespace() {
                Some('_')
            } else {
                None
            }
        })
        .collect()
}

/// If a measure's staff_text contains a section-marker label, remove it from
/// the measure (so it doesn't double-render as inline text) and return the
/// parsed Section + original raw label string.
///
/// Heuristic: boxed entries (from `<rehearsal>`) are treated as section
/// markers whenever they parse; unboxed `<words>` only count if their first
/// line matches a *known* section abbreviation. That keeps prose like
/// "Click-Count" or "Cresc. <…" from being promoted to section pills.
fn take_section_label(m: &mut Measure) -> Option<(Section, String)> {
    let idx = m.staff_text.iter().position(|t| {
        let parsed = parse_section_label(&t.text);
        match parsed {
            Some(sec) => t.boxed || is_well_known_section(&sec.section_type),
            None => false,
        }
    })?;
    let entry = m.staff_text.remove(idx);
    parse_section_label(&entry.text).map(|sec| (sec, entry.text))
}

fn is_well_known_section(t: &SectionType) -> bool {
    !matches!(t, SectionType::Custom(_))
}

/// Recognise common section-marker text. Supports:
///   - boxed rehearsal marks like `INTRO`, `BR`, `TAG`, `END`
///   - numbered labels like `VS 1`, `CH 2`, `VERSE 1a`, `CHORUS 2`
///   - multi-line "CH 1\nCH 2" — first line wins, full text becomes the name
fn parse_section_label(text: &str) -> Option<Section> {
    let first = text.lines().next()?.trim();
    if first.is_empty() {
        return None;
    }
    // Strip a trailing letter (a/b/c) for split sections and a trailing number.
    let mut letters = String::new();
    let mut number: Option<u32> = None;
    let mut split: Option<char> = None;

    let mut chars = first.chars().peekable();
    // Letters (and spaces).
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            letters.push(c.to_ascii_uppercase());
            chars.next();
        } else if c == ' ' && !letters.is_empty() {
            chars.next();
        } else {
            break;
        }
    }
    // Optional number.
    let mut num_str = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            num_str.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if !num_str.is_empty() {
        number = num_str.parse().ok();
    }
    // Optional split letter (single lowercase after the number).
    if let Some(&c) = chars.peek() {
        if c.is_ascii_lowercase() {
            split = Some(c);
            chars.next();
        }
    }
    let trailing: String = chars.collect();
    if !valid_section_label_trailing(&trailing) {
        return None;
    }

    let section_type = match letters.as_str() {
        "" => return None,
        "INTRO" => SectionType::Intro,
        "VS" | "V" | "VERSE" => SectionType::Verse,
        "CH" | "C" | "CHORUS" => SectionType::Chorus,
        "BR" | "BRIDGE" => SectionType::Bridge,
        "OUTRO" | "O" => SectionType::Outro,
        "END" | "ENDING" => SectionType::End,
        "TAG" | "TAGS" => SectionType::Custom("Tag".to_string()),
        "SOLO" => SectionType::Solo,
        "BREAKDOWN" | "BD" => SectionType::Breakdown,
        "INST" | "INSTRUMENTAL" => SectionType::Instrumental,
        "INTERLUDE" => SectionType::Interlude,
        "VAMP" => SectionType::Vamp,
        "HITS" => SectionType::Hits,
        // Anything else that "looks like" a label (short, all-caps, no
        // sentence punctuation) becomes a Custom section. This catches
        // things like "ROARING GROOVE!" / "STOP" used as repeat markers.
        other if looks_like_label(other) => SectionType::Custom(other.to_string()),
        _ => return None,
    };

    let mut sec = Section::new(section_type);
    sec.number = number;
    sec.split_letter = split;
    Some(sec)
}

fn valid_section_label_trailing(trailing: &str) -> bool {
    let trimmed = trailing.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.contains('=') {
        return false;
    }

    let mut letters = String::new();
    for c in trimmed.chars() {
        if c.is_ascii_alphabetic() {
            letters.push(c.to_ascii_uppercase());
        } else if c == ' ' && !letters.is_empty() {
            continue;
        } else {
            break;
        }
    }

    matches!(
        letters.as_str(),
        "INTRO"
            | "VS"
            | "V"
            | "VERSE"
            | "CH"
            | "C"
            | "CHORUS"
            | "BR"
            | "BRIDGE"
            | "OUTRO"
            | "O"
            | "END"
            | "ENDING"
            | "TAG"
            | "TAGS"
            | "SOLO"
            | "BREAKDOWN"
            | "BD"
            | "INST"
            | "INSTRUMENTAL"
            | "INTERLUDE"
            | "VAMP"
            | "HITS"
    )
}

/// Heuristic: a "label-like" string is short, all-uppercase, and contains no
/// lowercase letters or sentence-style punctuation. Used to identify
/// repeat-coda markers that aren't well-known section names.
fn looks_like_label(s: &str) -> bool {
    if s.len() > 24 {
        return false;
    }
    let mut letters = 0;
    for c in s.chars() {
        if c.is_ascii_lowercase() {
            return false;
        }
        if matches!(c, '.' | ',' | ':' | ';' | '?') {
            return false;
        }
        if c.is_ascii_alphabetic() {
            letters += 1;
        }
    }
    letters >= 2
}

fn metadata_from_score(score: &ScorePartwise) -> SongMetadata {
    let mut meta = SongMetadata::new();
    let contents = &score.content;

    // Title: the *largest* credit-words on page 1 is what musicians read as
    // the title. `<movement-title>` is what Finale/Sibelius treat as the
    // file's working title and often holds a subtitle — fall back to it.
    let title_from_credits = pick_title_from_credits(score);
    if let Some(t) = title_from_credits {
        meta.title = Some(strip_outer_quotes(&t).to_string());
    } else if let Some(title) = &contents.movement_title {
        let s = decode_xml_entities(title.content.trim());
        if !s.is_empty() {
            meta.title = Some(strip_outer_quotes(&s).to_string());
        }
    }

    // Subtitle: prefer credit-type="subtitle" if present, else movement-title
    // when it's distinct from the chosen title.
    if let Some(sub) = pick_credit_with_type(score, "subtitle") {
        meta.subtitle = Some(sub);
    } else if let Some(mt) = &contents.movement_title {
        let s = decode_xml_entities(mt.content.trim());
        if !s.is_empty() && meta.title.as_deref() != Some(s.as_str()) {
            meta.subtitle = Some(strip_outer_quotes(&s).to_string());
        }
    }

    if let Some(ident) = &contents.identification {
        for creator in &ident.content.creator {
            let role = creator
                .attributes
                .r#type
                .as_ref()
                .map(|t| t.as_str().to_ascii_lowercase())
                .unwrap_or_default();
            let value = decode_xml_entities(creator.content.trim());
            if value.is_empty() {
                continue;
            }
            match role.as_str() {
                "composer" => {
                    meta.composer.get_or_insert(value);
                }
                "lyricist" => {
                    meta.lyricist.get_or_insert(value);
                }
                "arranger" => {
                    meta.arranger.get_or_insert(value);
                }
                _ => {}
            }
        }
        if let Some(rights) = ident.content.rights.first() {
            let s = decode_xml_entities(rights.content.trim());
            if !s.is_empty() {
                meta.copyright = Some(s);
            }
        }
    }
    if let Some(bpm) = pick_bpm_from_credits(score) {
        meta.tempo = Some(bpm);
    }
    meta
}

fn pick_bpm_from_credits(score: &ScorePartwise) -> Option<u32> {
    for credit in &score.content.credit {
        let page = credit.attributes.page.as_ref().map(|p| **p).unwrap_or(1);
        if page != 1 {
            continue;
        }
        for cw in credit_words_iter(credit) {
            if let Some(bpm) = parse_bpm_credit(&decode_xml_entities(cw.content.trim())) {
                return Some(bpm);
            }
        }
    }
    None
}

fn parse_bpm_credit(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    let bpm_idx = lower.find("bpm")?;
    let after_bpm = &text[bpm_idx + 3..];
    let tempo_region = after_bpm
        .split_once('=')
        .map(|(_, rhs)| rhs)
        .unwrap_or(after_bpm);
    let digits = tempo_region
        .split(|c: char| !c.is_ascii_digit())
        .rfind(|part| !part.is_empty())?;
    digits.parse().ok()
}

/// Walk every `<credit>` block on page 1 and pick the `<credit-words>`
/// content with the largest decimal `font-size`. That's almost universally
/// the chart's title in Finale / Sibelius / MuseScore exports.
fn pick_title_from_credits(score: &ScorePartwise) -> Option<String> {
    let mut best: Option<(f32, String)> = None;
    for credit in &score.content.credit {
        let page = credit.attributes.page.as_ref().map(|p| **p).unwrap_or(1);
        if page != 1 {
            continue;
        }
        for cw in credit_words_iter(credit) {
            let size = font_size_pt(cw);
            let text_decoded = decode_xml_entities(cw.content.trim());
            let text = text_decoded.as_str();
            if text.is_empty() {
                continue;
            }
            let score = size + if is_bold(cw) { 4.0 } else { 0.0 };
            match &best {
                Some((s, _)) if *s >= score => {}
                _ => best = Some((score, text.to_string())),
            }
        }
    }
    best.map(|(_, t)| t)
}

fn pick_credit_with_type(score: &ScorePartwise, ty: &str) -> Option<String> {
    for credit in &score.content.credit {
        let has_type = credit
            .content
            .credit_type
            .iter()
            .any(|ct| ct.content.eq_ignore_ascii_case(ty));
        if !has_type {
            continue;
        }
        for cw in credit_words_iter(credit) {
            let text_decoded = decode_xml_entities(cw.content.trim());
            let text = text_decoded.as_str();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn credit_words_iter(
    credit: &musicxml::elements::Credit,
) -> impl Iterator<Item = &musicxml::elements::CreditWords> {
    use musicxml::elements::CreditSubcontents;
    let text = match &credit.content.credit {
        CreditSubcontents::Text(t) => Some(t),
        _ => None,
    };
    text.into_iter().flat_map(|t| {
        t.credit_words
            .iter()
            .chain(t.additional.iter().filter_map(|a| a.credit_words.as_ref()))
    })
}

fn font_size_pt(cw: &musicxml::elements::CreditWords) -> f32 {
    use musicxml::datatypes::FontSize;
    match cw.attributes.font_size.as_ref() {
        Some(FontSize::Decimal(v)) => *v,
        // CSS keyword sizes get a rough ordering so they still compare.
        Some(FontSize::Css(_)) => 12.0,
        None => 10.0,
    }
}

fn is_bold(cw: &musicxml::elements::CreditWords) -> bool {
    matches!(
        cw.attributes.font_weight,
        Some(musicxml::datatypes::FontWeight::Bold)
    )
}

/// Walk parts/measures for the first `<attributes>` block and extract its
/// `<clef>` + `<key>`. Returns `(initial_clef, initial_key)` when found.
fn extract_initial_clef_and_key(
    score: &ScorePartwise,
) -> (Option<ChartClef>, Option<keyflow_proto::key::Key>) {
    for part in &score.content.part {
        for el in &part.content {
            if let PartElement::Measure(m) = el {
                for me in &m.content {
                    if let MeasureElement::Attributes(attrs) = me {
                        let clef = attrs.content.clef.first().and_then(clef_from_xml);
                        let key = attrs.content.key.first().and_then(key_from_xml);
                        if clef.is_some() || key.is_some() {
                            return (clef, key);
                        }
                    }
                }
            }
        }
    }
    (None, None)
}

fn clef_from_xml(c: &musicxml::elements::Clef) -> Option<ChartClef> {
    use musicxml::datatypes::ClefSign;
    let sign = &c.content.sign.content;
    let line = c.content.line.as_ref().map(|l| *l.content as i32);
    Some(match sign {
        ClefSign::G => ChartClef::Treble,
        ClefSign::F => ChartClef::Bass,
        ClefSign::C => match line {
            Some(3) => ChartClef::Alto,
            Some(4) => ChartClef::Tenor,
            _ => ChartClef::Alto,
        },
        ClefSign::Percussion => ChartClef::Percussion,
        _ => return None,
    })
}

fn key_from_xml(k: &musicxml::elements::Key) -> Option<keyflow_proto::key::Key> {
    use keyflow_proto::key::Key as KfKey;
    use keyflow_proto::primitives::MusicalNote;
    use musicxml::datatypes::Mode as XmlMode;
    use musicxml::elements::KeyContents;

    let explicit = match &k.content {
        KeyContents::Explicit(e) => e,
        _ => return None,
    };
    let fifths = *explicit.fifths.content as i32;
    let is_minor = matches!(
        explicit.mode.as_ref().map(|m| &m.content),
        Some(XmlMode::Minor) | Some(XmlMode::Aeolian)
    );
    let tonic = tonic_for_fifths(fifths, is_minor)?;
    let root = MusicalNote::from_string(tonic)?;
    Some(if is_minor {
        KfKey::minor(root)
    } else {
        KfKey::major(root)
    })
}

/// Map a `<fifths>` count to the tonic note name for a major or minor key,
/// following the circle of fifths. `fifths` is positive for sharps, negative
/// for flats; range is roughly -7..=7 in real-world scores.
fn tonic_for_fifths(fifths: i32, is_minor: bool) -> Option<&'static str> {
    if is_minor {
        Some(match fifths {
            -7 => "Ab",
            -6 => "Eb",
            -5 => "Bb",
            -4 => "F",
            -3 => "C",
            -2 => "G",
            -1 => "D",
            0 => "A",
            1 => "E",
            2 => "B",
            3 => "F#",
            4 => "C#",
            5 => "G#",
            6 => "D#",
            7 => "A#",
            _ => return None,
        })
    } else {
        Some(match fifths {
            -7 => "Cb",
            -6 => "Gb",
            -5 => "Db",
            -4 => "Ab",
            -3 => "Eb",
            -2 => "Bb",
            -1 => "F",
            0 => "C",
            1 => "G",
            2 => "D",
            3 => "A",
            4 => "E",
            5 => "B",
            6 => "F#",
            7 => "C#",
            _ => return None,
        })
    }
}

fn strip_outer_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && *bytes.last().unwrap() == b'"' {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Measure walk
// ─────────────────────────────────────────────────────────────────────────────

fn collect_measures(score: &ScorePartwise) -> Vec<Measure> {
    let mut out: Vec<Measure> = Vec::new();
    let mut time_sig = (4u8, 4u8);
    let mut divisions: u32 = 1;
    let mut open_wedges: std::collections::HashMap<u8, OpenWedge> =
        std::collections::HashMap::new();
    let mut open_voltas: std::collections::HashMap<String, OpenVolta> =
        std::collections::HashMap::new();
    let mut previous_chord: Option<ChordInstance> = None;
    let mut slash_notation_active = false;

    for part in &score.content.part {
        for el in &part.content {
            if let PartElement::Measure(m) = el {
                let (measure_uses_slashes, next_slash_state) =
                    measure_slash_application(m, slash_notation_active);
                measure_from_xml(
                    m,
                    &mut out,
                    &mut time_sig,
                    &mut divisions,
                    &mut open_wedges,
                    &mut open_voltas,
                    &mut previous_chord,
                    measure_uses_slashes,
                );
                close_open_voltas_at_new_section(&mut out, &mut open_voltas);
                slash_notation_active = next_slash_state;
            }
        }
    }

    // Anything still here is a dangling spanner — drop silently.
    open_wedges.clear();
    open_voltas.clear();

    out
}

fn close_open_voltas_at_new_section(
    measures: &mut [Measure],
    open_voltas: &mut std::collections::HashMap<String, OpenVolta>,
) {
    let Some(current_idx) = measures.len().checked_sub(1) else {
        return;
    };
    if !measure_has_section_label(&measures[current_idx]) {
        return;
    }

    let to_close = open_voltas
        .iter()
        .filter(|&(_key, open)| open.measure_idx < current_idx)
        .map(|(key, _open)| key.clone())
        .collect::<Vec<_>>();

    for key in to_close {
        if let Some(open) = open_voltas.remove(&key) {
            let length = current_idx
                .saturating_sub(open.measure_idx)
                .max(1)
                .min(usize::from(u16::MAX)) as u16;
            if let Some(start_measure) = measures.get_mut(open.measure_idx) {
                if let Some(volta) = &mut start_measure.volta_start {
                    volta.length_measures = length;
                } else {
                    start_measure.volta_start = Some(Volta {
                        numbers: open.numbers,
                        label: String::new(),
                        length_measures: length,
                    });
                }
            }
        }
    }
}

fn measure_has_section_label(measure: &Measure) -> bool {
    measure
        .staff_text
        .iter()
        .any(|text| parse_section_label(&text.text).is_some())
}

/// Open wedge spanner — the start measure index, start beat, kind, placement
/// are remembered until the matching `<wedge type="stop">` fires.
struct OpenWedge {
    measure_idx: usize,
    beat: u8,
    kind: HairpinKind,
    placement: Placement,
}

/// Open volta — the start measure index + numbers until the matching
/// `<ending type="stop">` fires.
struct OpenVolta {
    measure_idx: usize,
    numbers: Vec<u8>,
}

fn measure_from_xml(
    xml: &musicxml::elements::Measure,
    measures: &mut Vec<Measure>,
    running_time_sig: &mut (u8, u8),
    divisions: &mut u32,
    open_wedges: &mut std::collections::HashMap<u8, OpenWedge>,
    open_voltas: &mut std::collections::HashMap<String, OpenVolta>,
    previous_chord: &mut Option<ChordInstance>,
    slash_notation_active: bool,
) {
    let mut m = Measure::new();
    m.time_signature = *running_time_sig;
    m.source_measure_number = xml.attributes.number.0.parse::<u32>().ok();
    m.source_measure_width = xml.attributes.width.as_ref().map(|w| w.0);

    // Snapshot the chord that was ringing as we entered this measure — used
    // below to seed beat 1 when the bar starts late (no harmony at tick 0).
    // Capturing here (before we ingest any harmonies in this measure) means
    // a measure's own chords never overwrite the carry-forward source.
    let inbound_chord: Option<ChordInstance> = previous_chord.clone();

    let mut volta_numbers: Option<Vec<u8>> = None;
    let measure_idx = measures.len();
    let mut tick: i64 = 0;
    let mut melody_acc: Vec<keyflow_proto::chart::melody::MelodyNote> = Vec::new();
    let note_source_positions = note_source_positions(&xml.content);

    for el in &xml.content {
        match el {
            MeasureElement::Attributes(attrs) => {
                if let Some(d) = attrs.content.divisions.as_ref() {
                    let v: u32 = d.content.0.max(1);
                    *divisions = v;
                }
                if let Some(time) = attrs.content.time.first() {
                    if let Some(ts) = parse_time_signature(time) {
                        *running_time_sig = ts;
                        m.time_signature = ts;
                    }
                }
            }
            MeasureElement::Note(n) => {
                if is_chord_note(n) {
                    // `<note><chord/>` — share previous stem, add to its
                    // extra_pitches so the engraver can stack the head.
                    if let Some(prev) = melody_acc.last_mut() {
                        if let Some((p, oct)) = melody_note_pitch(n) {
                            prev.extra_pitches.push((p, Some(oct)));
                            prev.extra_pitch_modifiers.push(Default::default());
                        }
                    }
                } else if let Some(note) = melody_note_from_xml(n, *divisions) {
                    melody_acc.push(note);
                }
                tick += note_duration_ticks(n);
            }
            MeasureElement::Backup(b) => {
                tick -= *b.content.duration.content as i64;
                if tick < 0 {
                    tick = 0;
                }
            }
            MeasureElement::Forward(f) => {
                tick += *f.content.duration.content as i64;
            }
            MeasureElement::Direction(dir) => {
                let direction_tick = tick + direction_offset(dir);
                let beat = tick_to_beat(direction_tick, *divisions, m.time_signature.0);
                let source_default_x = source_x_from_tick(direction_tick, &note_source_positions);
                ingest_direction(
                    dir,
                    beat,
                    source_default_x,
                    measure_idx,
                    &mut m,
                    measures,
                    open_wedges,
                );
            }
            MeasureElement::Barline(bar) => {
                ingest_barline(
                    bar,
                    measure_idx,
                    &mut m,
                    measures,
                    &mut volta_numbers,
                    open_voltas,
                );
            }
            MeasureElement::Harmony(h) => {
                if let Some(mut chord) = chord_from_harmony(h, &m.time_signature) {
                    chord.position = position_from_tick(
                        tick + harmony_offset(h).max(0),
                        *divisions,
                        m.time_signature,
                    );
                    *previous_chord = Some(chord.clone());
                    if !is_end_measure_carry_harmony(h, &chord.position, m.time_signature) {
                        m.chords.push(chord);
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(nums) = volta_numbers {
        m.volta_start = Some(Volta {
            numbers: nums.clone(),
            label: String::new(),
            length_measures: 1,
        });
    }
    if m.volta_start.is_none() {
        if let Some(nums) = inferred_ending_numbers_from_staff_text(&m) {
            m.volta_start = Some(Volta {
                numbers: nums.clone(),
                label: String::new(),
                length_measures: 1,
            });
            open_voltas.insert(
                String::new(),
                OpenVolta {
                    measure_idx,
                    numbers: nums,
                },
            );
        }
    }

    let any_pitched = melody_acc.iter().any(|n| n.pitch != "r");
    if any_pitched || (!melody_acc.is_empty() && !slash_notation_active) {
        m.melodies
            .push(keyflow_proto::chart::melody::Melody::with_notes(melody_acc));
    }

    // Empty-measure slash fill: when a measure carries no <harmony>, treat
    // it as a continuation of the previous chord. We re-emit the previous
    // chord's symbol with the current meter's slash rhythm; the renderer's
    // hide_repeated_chords suppresses the symbol redraw so the measure
    // shows as bare rhythm slashes — matching how a chart actually reads.
    //
    // When the first harmony of the measure starts AFTER beat 1 we also
    // prepend a carry-forward chord at beat 1, so the bar reads as a full
    // progression rather than implicitly starting late.
    let first_at_beat_one = m
        .chords
        .first()
        .is_some_and(|c| c.position.beats() == 0 && c.position.subdivisions() == 0);
    if (slash_notation_active && m.chords.is_empty())
        || (!m.chords.is_empty() && !first_at_beat_one)
    {
        if let Some(prev) = inbound_chord.as_ref() {
            let mut filler = prev.clone();
            if m.chords.is_empty() {
                filler.rhythm = default_slash_rhythm(m.time_signature);
            }
            filler.position = AbsolutePosition::at_beginning();
            m.chords.insert(0, filler);
        }
    }

    if slash_notation_active {
        apply_slash_rhythm_spans(&mut m);
    }

    measures.push(m);
}

fn inferred_ending_numbers_from_staff_text(measure: &Measure) -> Option<Vec<u8>> {
    measure.staff_text.iter().find_map(|text| {
        let lower = text.text.to_ascii_lowercase();
        if lower.contains("1st ending") || lower.contains("first ending") {
            Some(vec![1])
        } else if lower.contains("2nd ending") || lower.contains("second ending") {
            Some(vec![2])
        } else {
            None
        }
    })
}

fn apply_slash_rhythm_spans(measure: &mut Measure) {
    if measure.chords.is_empty() {
        return;
    }

    let measure_end = u32::from(measure.time_signature.0) * 1000;
    let starts = measure
        .chords
        .iter()
        .map(|chord| chord_position_units(&chord.position).min(measure_end))
        .collect::<Vec<_>>();

    for idx in 0..measure.chords.len() {
        let start = starts[idx];
        let end = starts
            .iter()
            .copied()
            .skip(idx + 1)
            .find(|next| *next > start)
            .unwrap_or(measure_end);
        let span_units = end.saturating_sub(start).max(1);
        let rhythm = slash_rhythm_for_position_span(span_units, measure.time_signature);
        tracing::debug!(
            target: "keyflow_musicxml::slash",
            source_measure = ?measure.source_measure_number,
            chord_idx = idx,
            symbol = %measure.chords[idx].full_symbol,
            start_units = start,
            end_units = end,
            span_units,
            time_signature = ?measure.time_signature,
            rhythm = ?rhythm,
            "[musicxml-slash] assigned chord slash span"
        );
        measure.chords[idx].rhythm = rhythm;
    }
}

fn chord_position_units(position: &AbsolutePosition) -> u32 {
    position.beats() * 1000 + position.subdivisions()
}

fn measure_slash_application(
    xml: &musicxml::elements::Measure,
    currently_active: bool,
) -> (bool, bool) {
    let mut applies = currently_active;
    let mut next_active = currently_active;
    for el in &xml.content {
        let MeasureElement::Attributes(attrs) = el else {
            continue;
        };
        for style in &attrs.content.measure_style {
            let MeasureStyleContents::Slash(slash) = &style.content else {
                continue;
            };
            match slash.attributes.r#type {
                StartStop::Start => {
                    applies = true;
                    next_active = true;
                }
                StartStop::Stop => {
                    applies = false;
                    next_active = false;
                }
            };
        }
    }
    (applies, next_active)
}

/// True when this note carries a `<chord/>` child — i.e. it's a polyphonic
/// continuation of the previous note (octave doubling, double-stop, etc.).
/// The caller decides how to attach it to the previous melody note.
fn is_chord_note(n: &musicxml::elements::Note) -> bool {
    use musicxml::elements::NoteType;
    match &n.content.info {
        NoteType::Normal(info) => info.chord.is_some(),
        NoteType::Cue(info) => info.chord.is_some(),
        NoteType::Grace(_) => false,
    }
}

/// Extract just the pitch (name + octave) from a `<note>`, ignoring
/// duration/timing. Used for chord-note continuations where we only
/// need to know the additional pitch to stack on the previous head.
fn melody_note_pitch(n: &musicxml::elements::Note) -> Option<(String, u8)> {
    use musicxml::elements::{AudibleType, NoteType};
    let audible = match &n.content.info {
        NoteType::Normal(info) => &info.audible,
        NoteType::Cue(info) => &info.audible,
        NoteType::Grace(_) => return None,
    };
    match audible {
        AudibleType::Pitch(p) => {
            let name = note_name(
                &p.content.step.content,
                p.content.alter.as_ref().map(|a| a.content.0),
            );
            let octave = (*p.content.octave.content).min(9);
            Some((name, octave))
        }
        _ => None,
    }
}

/// Convert a `<note>` to a keyflow [`MelodyNote`]. Returns `None` for
/// grace notes and zero-duration notes. Chord-note continuations are
/// handled at the caller (see `is_chord_note`).
fn melody_note_from_xml(
    n: &musicxml::elements::Note,
    divisions: u32,
) -> Option<keyflow_proto::chart::melody::MelodyNote> {
    use musicxml::elements::{AudibleType, NoteType};

    let (audible, dur_ticks) = match &n.content.info {
        NoteType::Normal(info) => (&info.audible, *info.duration.content),
        NoteType::Cue(info) => (&info.audible, *info.duration.content),
        NoteType::Grace(_) => return None,
    };
    if dur_ticks == 0 {
        return None;
    }

    let mut note = match audible {
        AudibleType::Pitch(p) => {
            let pitch_str = note_name(
                &p.content.step.content,
                p.content.alter.as_ref().map(|a| a.content.0),
            );
            let octave: u8 = (*p.content.octave.content).min(9);
            keyflow_proto::chart::melody::MelodyNote::new(pitch_str, 4).with_octave(octave)
        }
        AudibleType::Rest(_) => keyflow_proto::chart::melody::MelodyNote::rest(4),
        AudibleType::Unpitched(_) => {
            // Render unpitched as a midline rhythm placeholder.
            keyflow_proto::chart::melody::MelodyNote::new("r", 4)
        }
    };

    let (duration, dotted) = ticks_to_lily_duration(dur_ticks, divisions);
    note.duration = duration;
    note.dotted = dotted;

    // Tie info: a single note can carry both <tie type="start"/> and
    // <tie type="stop"/> (mid-chain of a sustained pitch). Honor both flags
    // independently so callers can model the whole chain.
    let info_ties: &[musicxml::elements::Tie] = match &n.content.info {
        NoteType::Normal(info) => &info.tie,
        NoteType::Cue(_) | NoteType::Grace(_) => &[],
    };
    for tie in info_ties {
        match tie.attributes.r#type {
            musicxml::datatypes::StartStop::Start => note.tie_start = true,
            musicxml::datatypes::StartStop::Stop => note.tie_stop = true,
        }
    }

    Some(note)
}

/// Map raw tick count + divisions-per-quarter to a LilyPond-style duration
/// number (1 = whole, 2 = half, 4 = quarter, 8 = eighth …) plus a dotted
/// flag. Falls back to quarter when nothing reasonable matches.
fn ticks_to_lily_duration(ticks: u32, divisions: u32) -> (u8, bool) {
    if divisions == 0 {
        return (4, false);
    }
    // Express ticks as a fraction of a whole note (whole = 4 * divisions).
    let whole = (divisions as u64) * 4;
    let ratio = (ticks as u64 * 64) / whole.max(1); // 64ths
    // Look up the closest entry in a small table. Each row is
    // (64ths, lily_dur, dotted).
    let table: &[(u64, u8, bool)] = &[
        (96, 1, true),  // dotted whole
        (64, 1, false), // whole
        (48, 2, true),  // dotted half
        (32, 2, false), // half
        (24, 4, true),  // dotted quarter
        (16, 4, false), // quarter
        (12, 8, true),  // dotted eighth
        (8, 8, false),  // eighth
        (6, 16, true),  // dotted sixteenth
        (4, 16, false), // sixteenth
        (3, 32, true),  // dotted 32nd
        (2, 32, false), // 32nd
        (1, 64, false), // 64th
    ];
    let mut best = (4u8, false);
    let mut best_err = u64::MAX;
    for (sixtyfourths, lily, dotted) in table {
        let err = ratio.abs_diff(*sixtyfourths);
        if err < best_err {
            best_err = err;
            best = (*lily, *dotted);
        }
    }
    best
}

/// Returns the duration this note consumes in ticks. Chord-notes (subsequent
/// notes attached to a previous stem) and grace notes contribute 0.
fn note_duration_ticks(n: &musicxml::elements::Note) -> i64 {
    use musicxml::elements::NoteType;
    match &n.content.info {
        NoteType::Normal(info) => {
            if info.chord.is_some() {
                0
            } else {
                *info.duration.content as i64
            }
        }
        NoteType::Cue(info) => {
            if info.chord.is_some() {
                0
            } else {
                *info.duration.content as i64
            }
        }
        NoteType::Grace(_) => 0,
    }
}

fn note_source_positions(content: &[MeasureElement]) -> Vec<(i64, f64)> {
    let mut tick = 0_i64;
    let mut positions = Vec::new();

    for el in content {
        match el {
            MeasureElement::Note(n) => {
                if !is_chord_note(n) {
                    if let Some(default_x) = n.attributes.default_x.as_ref().map(|x| x.0) {
                        positions.push((tick, default_x));
                    }
                }
                tick += note_duration_ticks(n);
            }
            MeasureElement::Backup(b) => {
                tick = (tick - *b.content.duration.content as i64).max(0);
            }
            MeasureElement::Forward(f) => {
                tick += *f.content.duration.content as i64;
            }
            _ => {}
        }
    }

    positions
}

fn source_x_from_tick(tick: i64, note_positions: &[(i64, f64)]) -> Option<f64> {
    if note_positions.is_empty() {
        return None;
    }

    let tick = tick.max(0);
    if tick <= note_positions[0].0 {
        return Some(note_positions[0].1);
    }

    for window in note_positions.windows(2) {
        let (left_tick, left_x) = window[0];
        let (right_tick, right_x) = window[1];
        if tick <= right_tick {
            let span = right_tick - left_tick;
            if span <= 0 {
                return Some(right_x);
            }
            let ratio = (tick - left_tick) as f64 / span as f64;
            return Some(left_x + (right_x - left_x) * ratio);
        }
    }

    note_positions.last().map(|(_, x)| *x)
}

fn direction_offset(dir: &Direction) -> i64 {
    dir.content
        .offset
        .as_ref()
        .map(|o| o.content.0 as i64)
        .unwrap_or(0)
}

fn harmony_offset(harmony: &Harmony) -> i64 {
    harmony
        .content
        .offset
        .as_ref()
        .map(|o| o.content.0 as i64)
        .unwrap_or(0)
}

fn is_end_measure_carry_harmony(
    harmony: &Harmony,
    position: &AbsolutePosition,
    time_sig: (u8, u8),
) -> bool {
    harmony_offset(harmony) > 0
        && position.subdivisions() == 0
        && position.beats() >= time_sig.0.saturating_sub(1) as u32
}

fn tick_to_beat(tick: i64, divisions: u32, beats_per_measure: u8) -> u8 {
    if divisions == 0 {
        return 1;
    }
    let beat_index = tick / divisions as i64;
    let clamped = beat_index.max(0).min((beats_per_measure as i64) - 1) as u8;
    clamped + 1
}

/// Convert a measure-local tick (musicxml `divisions`-per-quarter units) into a
/// measure-local position. The position's `beats` is 0-based for the meter
/// denominator (so 6/8 has beats 0..6) and `subdivisions` uses 1000 per beat —
/// e.g. tick=6 in 6/8 with divisions=8 → beat=1, subdivisions=500 (display "2.5").
fn position_from_tick(tick: i64, divisions: u32, time_sig: (u8, u8)) -> AbsolutePosition {
    use keyflow_proto::time::MusicalDuration;
    let denom = time_sig.1.max(1) as i32;
    let ticks_per_beat = (divisions as i32 * 4 / denom).max(1);
    let t = tick.max(0) as i32;
    let beats = t / ticks_per_beat;
    let frac_ticks = t % ticks_per_beat;
    let subdivisions = (frac_ticks * 1000 / ticks_per_beat).clamp(0, 999);
    AbsolutePosition::new(MusicalDuration::new(0, beats, subdivisions), 0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Time signature
// ─────────────────────────────────────────────────────────────────────────────

fn parse_time_signature(time: &Time) -> Option<(u8, u8)> {
    let pair = time.content.beats.first()?;
    let beats: u8 = pair.beats.content.parse().ok()?;
    let beat_type: u8 = pair.beat_type.content.parse().ok()?;
    Some((beats, beat_type))
}

// ─────────────────────────────────────────────────────────────────────────────
// Barline + endings
// ─────────────────────────────────────────────────────────────────────────────

fn ingest_barline(
    bar: &Barline,
    measure_idx: usize,
    measure: &mut Measure,
    measures: &mut [Measure],
    volta_numbers: &mut Option<Vec<u8>>,
    open_voltas: &mut std::collections::HashMap<String, OpenVolta>,
) {
    use musicxml::datatypes::RightLeftMiddle;

    let location = bar.attributes.location.as_ref();

    if let Some(style) = bar.content.bar_style.as_ref().map(|bs| &bs.content) {
        let mapped = match style {
            XmlBarStyle::LightHeavy => BarlineStyle::LightHeavy,
            XmlBarStyle::HeavyLight => BarlineStyle::HeavyLight,
            XmlBarStyle::LightLight => BarlineStyle::LightLight,
            XmlBarStyle::HeavyHeavy => BarlineStyle::HeavyHeavy,
            XmlBarStyle::Dashed | XmlBarStyle::Dotted => BarlineStyle::Dashed,
            XmlBarStyle::None => BarlineStyle::None,
            _ => BarlineStyle::Normal,
        };
        // Default location is "right" (closing barline).
        if matches!(location, Some(&RightLeftMiddle::Left)) {
            // Style on the left barline doesn't have a dedicated field —
            // ignored for now (StartRepeat geometry kicks in via Repeat).
        } else {
            measure.end_barline = mapped;
        }
    }

    if let Some(rep) = &bar.content.repeat {
        let mark = match rep.attributes.direction {
            BackwardForward::Forward => RepeatMark::Forward,
            BackwardForward::Backward => RepeatMark::Backward,
        };
        match rep.attributes.direction {
            BackwardForward::Forward => measure.start_repeat = mark,
            BackwardForward::Backward => measure.end_repeat = mark,
        }
    }

    if let Some(ending) = &bar.content.ending {
        let key = ending.attributes.number.0.clone();
        match ending.attributes.r#type {
            StartStopDiscontinue::Start => {
                let nums = parse_ending_numbers(&ending.attributes.number.0);
                if !nums.is_empty() {
                    *volta_numbers = Some(nums.clone());
                }
                open_voltas.insert(
                    key,
                    OpenVolta {
                        measure_idx,
                        numbers: nums,
                    },
                );
            }
            StartStopDiscontinue::Stop | StartStopDiscontinue::Discontinue => {
                if let Some(open) = open_voltas.remove(&key) {
                    let length = measure_idx.saturating_sub(open.measure_idx) as u16 + 1;
                    if let Some(start_measure) = measures.get_mut(open.measure_idx) {
                        if let Some(volta) = &mut start_measure.volta_start {
                            volta.length_measures = length;
                        } else {
                            start_measure.volta_start = Some(Volta {
                                numbers: open.numbers,
                                label: String::new(),
                                length_measures: length,
                            });
                        }
                    }
                }
            }
        }
    }
}

fn parse_ending_numbers(s: &str) -> Vec<u8> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|t| t.trim().parse::<u8>().ok())
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Directions (dynamics / wedge / words / rehearsal)
// ─────────────────────────────────────────────────────────────────────────────

fn ingest_direction(
    dir: &Direction,
    beat: u8,
    direction_source_default_x: Option<f64>,
    measure_idx: usize,
    measure: &mut Measure,
    measures: &mut [Measure],
    open_wedges: &mut std::collections::HashMap<u8, OpenWedge>,
) {
    let placement = direction_placement(dir);

    for dt in &dir.content.direction_type {
        match &dt.content {
            DirectionTypeContents::Dynamics(items) => {
                for d in items {
                    if let Some(level) = dynamic_level_from_xml(d) {
                        measure.classical_dynamics.push(Dynamic {
                            level,
                            beat,
                            placement,
                        });
                    }
                }
            }
            DirectionTypeContents::Wedge(w) => {
                ingest_wedge(
                    w,
                    beat,
                    placement,
                    measure_idx,
                    measure,
                    measures,
                    open_wedges,
                );
            }
            DirectionTypeContents::Words(words) => {
                for w in words {
                    ingest_words(w, beat, placement, direction_source_default_x, measure);
                }
            }
            DirectionTypeContents::Rehearsal(items) => {
                for r in items {
                    let text = decode_xml_entities(r.content.trim());
                    if !text.is_empty() {
                        measure.staff_text.push(StaffText {
                            text,
                            beat,
                            placement,
                            source_default_x: direction_source_default_x,
                            boxed: true,
                            bold: true,
                            italic: false,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn direction_placement(dir: &Direction) -> Placement {
    use musicxml::datatypes::AboveBelow;
    match dir.attributes.placement {
        Some(AboveBelow::Above) => Placement::Above,
        _ => Placement::Below,
    }
}

fn words_placement(w: &Words) -> Option<Placement> {
    let y = w.attributes.default_y.as_ref()?.0;
    if y >= 0.0 {
        Some(Placement::Above)
    } else {
        Some(Placement::Below)
    }
}

fn dynamic_level_from_xml(d: &musicxml::elements::Dynamics) -> Option<DynamicLevel> {
    use musicxml::elements::DynamicsType;
    for entry in &d.content {
        let level = match entry {
            DynamicsType::P(_) => Some(DynamicLevel::P),
            DynamicsType::Pp(_) => Some(DynamicLevel::Pp),
            DynamicsType::Ppp(_) => Some(DynamicLevel::Ppp),
            DynamicsType::Mp(_) => Some(DynamicLevel::Mp),
            DynamicsType::Mf(_) => Some(DynamicLevel::Mf),
            DynamicsType::F(_) => Some(DynamicLevel::F),
            DynamicsType::Ff(_) => Some(DynamicLevel::Ff),
            DynamicsType::Fff(_) => Some(DynamicLevel::Fff),
            DynamicsType::Sf(_) => Some(DynamicLevel::Sf),
            DynamicsType::Sfz(_) => Some(DynamicLevel::Sfz),
            DynamicsType::Fp(_) => Some(DynamicLevel::Fp),
            _ => None,
        };
        if level.is_some() {
            return level;
        }
    }
    None
}

fn ingest_wedge(
    w: &musicxml::elements::Wedge,
    beat: u8,
    placement: Placement,
    measure_idx: usize,
    measure: &mut Measure,
    measures: &mut [Measure],
    open_wedges: &mut std::collections::HashMap<u8, OpenWedge>,
) {
    let number: u8 = w.attributes.number.as_ref().map(|n| **n).unwrap_or(1);

    match w.attributes.r#type {
        WedgeType::Crescendo => {
            open_wedges.insert(
                number,
                OpenWedge {
                    measure_idx,
                    beat,
                    kind: HairpinKind::Crescendo,
                    placement,
                },
            );
        }
        WedgeType::Diminuendo => {
            open_wedges.insert(
                number,
                OpenWedge {
                    measure_idx,
                    beat,
                    kind: HairpinKind::Decrescendo,
                    placement,
                },
            );
        }
        WedgeType::Stop => {
            if let Some(open) = open_wedges.remove(&number) {
                let end_offset = measure_idx.saturating_sub(open.measure_idx) as u16;
                let hairpin = Hairpin {
                    kind: open.kind,
                    start_beat: open.beat,
                    end_measure_offset: end_offset,
                    end_beat: beat,
                    placement: open.placement,
                };
                if open.measure_idx == measure_idx {
                    measure.hairpins.push(hairpin);
                } else if let Some(start_measure) = measures.get_mut(open.measure_idx) {
                    start_measure.hairpins.push(hairpin);
                }
            }
        }
        _ => {}
    }
}

/// Decode the five predefined XML entities + numeric character references.
/// The `musicxml` crate's hand-rolled parser does not unescape these, so any
/// text we ingest still contains literal `&lt;`, `&gt;`, `&amp;`, `&quot;`,
/// `&apos;`, `&#NNN;`, `&#xHH;` — decode them here at every text boundary.
fn decode_xml_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(end) = bytes[i + 1..].iter().position(|&b| b == b';') {
                let entity = &s[i + 1..i + 1 + end];
                let decoded: Option<char> = match entity {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    e if e.starts_with("#x") || e.starts_with("#X") => {
                        u32::from_str_radix(&e[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                if let Some(c) = decoded {
                    out.push(c);
                    i += end + 2;
                    continue;
                }
            }
        }
        // Push current byte safely (advance one UTF-8 char).
        let ch_len = s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn ingest_words(
    w: &Words,
    beat: u8,
    placement: Placement,
    direction_source_default_x: Option<f64>,
    measure: &mut Measure,
) {
    let raw_decoded = decode_xml_entities(w.content.trim());
    let raw = raw_decoded.as_str();
    if raw.is_empty() {
        return;
    }

    // Multi-line figured-bass-style numerals like "4-3 / 2-1" or "#4-3 / 2-1".
    // Each line becomes a row; the trailing dash separator is preserved.
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.iter().all(|l| looks_like_figured_bass_row(l)) {
        let rows: Vec<FiguredBassRow> = lines
            .iter()
            .map(|l| {
                let (acc, body) = split_leading_accidental(l);
                FiguredBassRow {
                    accidental: acc.to_string(),
                    text: body.to_string(),
                }
            })
            .collect();
        let placement = words_placement(w).unwrap_or(placement);
        measure.figured_bass.push(FiguredBass {
            rows,
            beat,
            placement,
            source_default_x: w
                .attributes
                .default_x
                .as_ref()
                .map(|x| x.0)
                .or(direction_source_default_x),
            source_relative_x: w.attributes.relative_x.as_ref().map(|x| x.0),
        });
        return;
    }

    measure.staff_text.push(StaffText {
        text: raw.to_string(),
        beat,
        placement,
        source_default_x: w
            .attributes
            .default_x
            .as_ref()
            .map(|x| x.0)
            .or(direction_source_default_x),
        boxed: false,
        bold: matches!(
            w.attributes.font_weight,
            Some(musicxml::datatypes::FontWeight::Bold)
        ),
        italic: matches!(
            w.attributes.font_style,
            Some(musicxml::datatypes::FontStyle::Italic)
        ),
    });
}

fn looks_like_figured_bass_row(s: &str) -> bool {
    let s = strip_leading_accidental(s).trim();
    if s.is_empty() {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_digit() || c == '-' || c == ' ')
}

fn strip_leading_accidental(s: &str) -> &str {
    let s = s.trim_start();
    let leading = s.chars().next();
    match leading {
        Some('#' | 'b' | '♭' | '♯' | '♮') => {
            &s[s.char_indices().nth(1).map(|(i, _)| i).unwrap_or(s.len())..]
        }
        _ => s,
    }
}

fn split_leading_accidental(s: &str) -> (&str, &str) {
    let s = s.trim();
    if let Some(c) = s.chars().next() {
        if matches!(c, '#' | 'b' | '♭' | '♯' | '♮') {
            let idx = s.char_indices().nth(1).map(|(i, _)| i).unwrap_or(s.len());
            return (&s[..idx], s[idx..].trim_start());
        }
    }
    ("", s)
}

// ─────────────────────────────────────────────────────────────────────────────
// Harmony → ChordInstance
// ─────────────────────────────────────────────────────────────────────────────

use keyflow_proto::chart::types::ChordInstance;
use keyflow_proto::chord::{Chord, ChordRhythm};
use keyflow_proto::primitives::RootNotation;
use keyflow_proto::time::{AbsolutePosition, MusicalDuration};
use keyflow_syntax::parsing::Lexer;
use musicxml::datatypes::{KindValue, Step};
use musicxml::elements::Harmony;

fn chord_from_harmony(h: &Harmony, time_sig: &(u8, u8)) -> Option<ChordInstance> {
    let sub = h.content.harmony.first()?;
    let root = sub.root.as_ref()?;
    let root_str = note_name(
        &root.content.root_step.content,
        root.content.root_alter.as_ref().map(|a| a.content.0),
    );

    let kind_suffix = kind_to_suffix(&sub.kind);

    let (bass_str, bass_vertical) = match &sub.bass {
        Some(b) => {
            let name = note_name(
                &b.content.bass_step.content,
                b.content.bass_alter.as_ref().map(|a| a.content.0),
            );
            let is_vertical =
                matches!(b.attributes.arrangement, Some(HarmonyArrangement::Vertical));
            (Some(name), is_vertical)
        }
        None => (None, false),
    };

    let mut symbol = format!("{root_str}{kind_suffix}");
    if let Some(bass) = &bass_str {
        symbol.push('/');
        symbol.push_str(bass);
    }

    let mut lexer = Lexer::new(symbol.clone());
    let tokens = lexer.tokenize();
    let mut parsed = Chord::parse(&tokens).ok()?;
    parsed.bass_vertical = bass_vertical;

    let root_notation = RootNotation::from_string(&root_str)?;
    let rhythm = default_slash_rhythm(*time_sig);

    Some(ChordInstance::new(
        root_notation,
        symbol.clone(),
        parsed,
        rhythm,
        symbol,
        MusicalDuration::new(0, 0, 0),
        AbsolutePosition::at_beginning(),
    ))
}

/// Pick the default slash rhythm for a whole measure given its time signature.
///
/// `ChordRhythm::Slashes { count, dotted }` only carries *one* dotted flag
/// for the whole chord, so we collapse the per-pulse pattern from
/// [`slash_pulses_for_meter`] to a single `(count, dotted)` pair. For
/// mixed /8 meters that means we lose the "/. /. /" idiom — the engraver
/// renders `num` undotted eighth slashes instead. The richer per-pulse
/// pattern is preserved in [`slash_pulses_for_meter`] for an engraver
/// pass that reads it directly.
fn default_slash_rhythm((num, den): (u8, u8)) -> ChordRhythm {
    let pulses = slash_pulses_for_meter(num, den);
    slash_rhythm_from_pulses(&pulses)
}

fn slash_rhythm_for_position_span(span_units: u32, (_num, den): (u8, u8)) -> ChordRhythm {
    let beat_units = ((span_units + 500) / 1000).clamp(1, u32::from(u8::MAX)) as u8;
    let pulses = slash_pulses_for_meter(beat_units, den);
    slash_rhythm_from_pulses(&pulses)
}

fn slash_rhythm_from_pulses(pulses: &[(u8, bool)]) -> ChordRhythm {
    let (count, dotted) = if pulses.iter().all(|(_, d)| *d) && !pulses.is_empty() {
        // All pulses are dotted (pure compound) — collapse to count=num_pulses, dotted=true.
        (pulses.len() as u8, true)
    } else {
        // Mixed or simple — count = total eighths, undotted, so the
        // engraver fills the bar with eighth slashes (always correct,
        // even if not idiomatic).
        let eighths: u8 = pulses.iter().map(|(e, _)| *e).sum();
        (eighths, false)
    };
    ChordRhythm::Slashes {
        count,
        dotted,
        tied: false,
    }
}

/// Compute the idiomatic slash pulse pattern for a measure-level rhythm
/// fill, given its time signature.
///
/// Each entry is `(duration_in_eighths, dotted)`:
/// - Simple meters (/4, /2): one undotted slash per beat unit.
/// - Pure compound (/8, num divisible by 3): one dotted-quarter slash per
///   triplet group. 6/8 → `[(3,t), (3,t)]`, 9/8 → three, 12/8 → four.
/// - Mixed /8 (5/8, 7/8, 11/8): greedy group-of-three from the left with
///   the remainder as a final undotted quarter or eighth slash.
///   5/8 → `[(3,t), (2,f)]`, 7/8 → `[(3,t), (3,t), (1,f)]`,
///   11/8 → `[(3,t), (3,t), (3,t), (2,f)]`.
///
/// Returns empty for unsupported denominators (e.g. 16ths).
pub(crate) fn slash_pulses_for_meter(num: u8, den: u8) -> Vec<(u8, bool)> {
    if num == 0 {
        return Vec::new();
    }
    match den {
        4 => {
            // 4/4, 3/4, etc. — one quarter slash per beat (2 eighths each).
            std::iter::repeat_n((2u8, false), num as usize).collect()
        }
        2 => {
            // 2/2, 3/2 — one half slash per beat (4 eighths each).
            std::iter::repeat_n((4u8, false), num as usize).collect()
        }
        8 => {
            // /8 meters: greedy group-of-three.
            let mut pulses = Vec::new();
            let mut remaining = num;
            while remaining >= 3 {
                pulses.push((3u8, true));
                remaining -= 3;
            }
            match remaining {
                0 => {}
                1 => pulses.push((1, false)), // eighth slash
                2 => pulses.push((2, false)), // quarter slash
                _ => unreachable!(),
            }
            pulses
        }
        _ => Vec::new(),
    }
}

fn note_name(step: &Step, alter: Option<i16>) -> String {
    let letter = match step {
        Step::A => "A",
        Step::B => "B",
        Step::C => "C",
        Step::D => "D",
        Step::E => "E",
        Step::F => "F",
        Step::G => "G",
    };
    let accidental = match alter {
        Some(a) if a >= 1 => "#",
        Some(a) if a <= -1 => "b",
        _ => "",
    };
    format!("{letter}{accidental}")
}

/// Map MusicXML `<kind>` (including its optional `text=""` attribute) to a
/// suffix that the keyflow chord tokenizer understands. When the XML
/// provides explicit `text`, we trust it (e.g. `"min7"`, `"7"`); otherwise we
/// fall back to a mapping of the `KindValue` enum.
fn kind_to_suffix(kind: &musicxml::elements::Kind) -> String {
    // Some exporters write `text="7" use-symbols="yes"` with kind=major-seventh
    // — the "7" is meant to be rendered as the triangle-7 glyph and stands for
    // maj7. The bare text is ambiguous (dominant 7 vs major 7), so fall back
    // to the KindValue suffix whenever symbol mode is on.
    let use_symbols = matches!(
        kind.attributes.use_symbols,
        Some(musicxml::datatypes::YesNo::Yes)
    );
    if !use_symbols {
        if let Some(text) = &kind.attributes.text {
            let t = text.0.trim();
            if !t.is_empty() {
                return normalise_kind_text(t);
            }
        }
    }
    match kind.content {
        KindValue::Major => String::new(),
        KindValue::Minor => "m".to_string(),
        KindValue::Augmented => "aug".to_string(),
        KindValue::Diminished => "dim".to_string(),
        KindValue::Dominant => "7".to_string(),
        KindValue::MajorSeventh => "maj7".to_string(),
        KindValue::MinorSeventh => "m7".to_string(),
        KindValue::DiminishedSeventh => "dim7".to_string(),
        KindValue::AugmentedSeventh => "aug7".to_string(),
        KindValue::HalfDiminished => "m7b5".to_string(),
        KindValue::MajorMinor => "mMaj7".to_string(),
        KindValue::MajorSixth => "6".to_string(),
        KindValue::MinorSixth => "m6".to_string(),
        KindValue::DominantNinth => "9".to_string(),
        KindValue::MajorNinth => "maj9".to_string(),
        KindValue::MinorNinth => "m9".to_string(),
        KindValue::Dominant11th => "11".to_string(),
        KindValue::Major11th => "maj11".to_string(),
        KindValue::Minor11th => "m11".to_string(),
        KindValue::Dominant13th => "13".to_string(),
        KindValue::Major13th => "maj13".to_string(),
        KindValue::Minor13th => "m13".to_string(),
        KindValue::SuspendedFourth => "sus4".to_string(),
        KindValue::SuspendedSecond => "sus2".to_string(),
        KindValue::Power => "5".to_string(),
        _ => String::new(),
    }
}

fn normalise_kind_text(t: &str) -> String {
    // Trim and adapt a handful of common Finale/Sibelius exports to the
    // tokens keyflow's chord parser expects.
    match t {
        "min" => "m".to_string(),
        "min7" => "m7".to_string(),
        "min9" => "m9".to_string(),
        "min11" => "m11".to_string(),
        "min13" => "m13".to_string(),
        "maj" => String::new(),
        "Maj" => String::new(),
        "MAJ" => String::new(),
        // Empty (= use_symbols suppresses text) → leave the bare suffix to
        // the KindValue fallback path.
        "" => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod meter_tests {
    use super::slash_pulses_for_meter;

    #[test]
    fn simple_meters_one_quarter_slash_per_beat() {
        assert_eq!(slash_pulses_for_meter(4, 4), vec![(2, false); 4]);
        assert_eq!(slash_pulses_for_meter(3, 4), vec![(2, false); 3]);
        assert_eq!(slash_pulses_for_meter(2, 4), vec![(2, false); 2]);
    }

    #[test]
    fn pure_compound_one_dotted_quarter_per_group() {
        assert_eq!(slash_pulses_for_meter(6, 8), vec![(3, true); 2]);
        assert_eq!(slash_pulses_for_meter(9, 8), vec![(3, true); 3]);
        assert_eq!(slash_pulses_for_meter(12, 8), vec![(3, true); 4]);
    }

    #[test]
    fn mixed_eight_meters_group_3_then_remainder() {
        // 5/8 = 3+2 → /. /
        assert_eq!(slash_pulses_for_meter(5, 8), vec![(3, true), (2, false)]);
        // 7/8 = 3+3+1 → /. /. /(eighth)
        assert_eq!(
            slash_pulses_for_meter(7, 8),
            vec![(3, true), (3, true), (1, false)]
        );
        // 11/8 = 3+3+3+2 → /. /. /. /
        assert_eq!(
            slash_pulses_for_meter(11, 8),
            vec![(3, true), (3, true), (3, true), (2, false)]
        );
    }

    #[test]
    fn half_meters_use_half_slashes() {
        assert_eq!(slash_pulses_for_meter(2, 2), vec![(4, false); 2]);
    }
}
