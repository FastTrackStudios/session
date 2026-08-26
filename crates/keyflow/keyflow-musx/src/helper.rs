//! Pure translation helpers — a faithful port of `musx2mxl/helper.py`.
//!
//! These map Finale's internal encodings (EVPU durations, engraver-font glyph
//! codes, key-signature integers, chord-suffix strings, …) onto the vocabulary
//! MusicXML expects. No XML is touched here.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

const SHARPS_AND_FLATS: [&str; 7] = ["F", "C", "G", "D", "A", "E", "B"];

/// Map of duration flags to note types, most-significant bit first.
const FLAG_TO_TYPE: [(i64, &str); 14] = [
    (32768, "maxima"),
    (16384, "long"),
    (8192, "breve"),
    (4096, "whole"),
    (2048, "half"),
    (1024, "quarter"),
    (512, "eighth"),
    (256, "16th"),
    (128, "32nd"),
    (64, "64th"),
    (32, "128th"),
    (16, "256th"),
    (8, "512th"),
    (4, "1024th"),
];

fn map_bar_line_type(finale: &str) -> Option<&'static str> {
    Some(match finale {
        "none" => "none",
        "normal" => "regular",
        "double" => "light-light",
        "final" => "light-heavy",
        "solid" => "heavy",
        "dash" => "dashed",
        "partial" => "tick",
        _ => return None,
    })
}

fn engraver_note_type(c: char) -> Option<&'static str> {
    Some(match c {
        'x' => "16th",
        'e' => "eighth",
        'q' => "quarter",
        'h' => "half",
        _ => return None,
    })
}

/// engraver char → (articulation tag, optional `type` attribute)
fn engraver_articulation(code: i64) -> Option<(&'static str, Option<&'static str>)> {
    Some(match code {
        62 => ("accent", None),
        94 => ("strong-accent", Some("up")),
        118 => ("strong-accent", Some("down")),
        46 => ("staccato", None),
        95 => ("tenuto", None),
        248 => ("detached-legato", None),
        224 => ("staccatissimo", None),
        -1 => ("spiccato", None),
        -2 => ("scoop", None),
        103 => ("plop", None),
        -5 => ("doit", None),
        -4 => ("falloff", None),
        44 => ("breath-mark", None),
        34 => ("caesura", None),
        -8 => ("stress", None),
        -9 => ("unstress", None),
        -10 => ("soft-accent", None),
        _ => return None,
    })
}

fn engraver_dynamic(code: u32) -> Option<&'static str> {
    Some(match code {
        112 => "p",
        185 => "pp",
        184 => "ppp",
        175 => "pppp",
        102 => "f",
        196 => "ff",
        236 => "fff",
        235 => "ffff",
        80 => "mp",
        70 => "mf",
        83 => "sf",
        130 => "sfp",
        182 => "sfpp",
        234 => "fp",
        167 => "sfz",
        141 => "sffz",
        90 => "fz",
        _ => return None,
    })
}

/// engraver char → (clef sign, octave change)
fn engraver_clef(code: i64) -> Option<(&'static str, i64)> {
    Some(match code {
        38 => ("G", 0),
        63 => ("F", 0),
        66 => ("C", 0),
        86 => ("G", -1),
        116 => ("F", -1),
        160 => ("G", 1),
        139 => ("percussion", 0),
        214 => ("percussion", 0),
        230 => ("F", 1),
        57424 => ("G", 0),
        57425 => ("G", -2),
        57426 => ("G", -1),
        57427 => ("G", 1),
        57428 => ("G", 2),
        57429 => ("G", -1),
        57430 => ("G", 0),
        57431 => ("G", 0),
        57432 => ("G", 0),
        57433 => ("G", 0),
        57434 => ("G", 0),
        57435 => ("G", 0),
        57436 => ("C", 0),
        57437 => ("C", -1),
        57438 => ("C", 0),
        57439 => ("C", 0),
        57440 => ("C", 0),
        57441 => ("C", 0),
        57442 => ("F", 0),
        57443 => ("F", -2),
        57444 => ("F", -1),
        57445 => ("F", 1),
        57446 => ("F", 2),
        57447 => ("F", 0),
        57448 => ("F", 0),
        57449 => ("percussion", 0),
        57450 => ("percussion", 0),
        57451 => ("percussion", 0),
        57452 => ("percussion", 0),
        61478 => ("F", 0),
        61503 => ("F", 0),
        _ => return None,
    })
}

// -------------------------------------------------------------------------
// Instrument UUID table (instruments.json)
// -------------------------------------------------------------------------

#[derive(Deserialize)]
struct InstrumentEntry {
    name: String,
    sound_id: String,
}

static INST_UUID_MAP: LazyLock<HashMap<String, InstrumentEntry>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("instruments.json"))
        .expect("bundled instruments.json is valid")
});

// -------------------------------------------------------------------------
// Chord-suffix parsing
// -------------------------------------------------------------------------

/// A single altered/added chord degree.
#[derive(Debug, Clone)]
pub struct Degree {
    pub degree_value: i64,
    pub degree_alter: i64,
    pub degree_type: String,
}

/// The decomposed chord suffix consumed by `handle_chords`.
#[derive(Debug, Clone)]
pub struct ChordSuffix {
    pub kind: String,
    pub use_symbols: String,
    pub parentheses_degrees: String,
    pub text: String,
    pub degrees: Vec<Degree>,
}

const DEGREE_PATTERN: &str =
    r"(?P<type>add|omit|alt|sus|maj7)?(?P<alter>[+\-b#])?(?P<value>[2-79]|11|13)?";

fn degrees_pattern() -> String {
    format!(
        r"(?P<parentheses_open>\(|\{{|\[)?(?P<degrees>(?:{DEGREE_PATTERN})+)(?P<parentheses_closed>\)|\}}|\])?"
    )
}

/// Ordered list of (kind, anchored regex) — insertion order matters, as the
/// first matching pattern wins (mirrors the Python dict iteration order).
static CHORD_PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    let dp = degrees_pattern();
    let make = |kind: &'static str, body: &str| {
        (
            kind,
            Regex::new(&format!(r"^(?:{body})(?:{dp})?$")).expect("valid chord regex"),
        )
    };
    vec![
        make("augmented-seventh", r"(?P<kind>aug7|\+7|7\+)"),
        make("augmented", r"(?P<kind>aug|\+|\+5)"),
        make("diminished-seventh", r"(?P<kind>(?:'|`|dim|°|o)7)"),
        make("diminished", r"(?P<kind>'|`|dim|°|o)"),
        make(
            "half-diminished",
            r"(?P<kind>(?:min|mi|m|-|−)7\(?[b\-−]?5\)?|ø7)",
        ),
        make("suspended-fourth", r"(?P<kind>7?sus4?)"),
        make("suspended-second", r"(?P<kind>7?sus2)"),
        make("dominant", r"(?P<kind>7)"),
        make("dominant-ninth", r"(?P<kind>9)"),
        make("dominant-11th", r"(?P<kind>11)"),
        make("dominant-13th", r"(?P<kind>13)"),
        make("major-sixth", r"(?P<kind>(?:maj|ma|Δ)?6)"),
        make("major-seventh", r"(?P<kind>(?:maj|ma|Δ)7)"),
        make("major-ninth", r"(?P<kind>(?:maj|ma|Δ)9)"),
        make("major-11th", r"(?P<kind>(?:maj|ma|Δ)11)"),
        make("major-13th", r"(?P<kind>(?:maj|ma|Δ)13)"),
        make(
            "major-minor",
            r"(?P<kind>min\(maj7\)|mi\(ma7\)|m\(ma7\)|-Δ7)",
        ),
        make("minor-sixth", r"(?P<kind>(?:min|mi|m|-|−)6)"),
        make("minor-seventh", r"(?P<kind>(?:min|mi|m|-|−)7)"),
        make("minor-ninth", r"(?P<kind>(?:min|mi|m|-|−)9)"),
        make("minor-11th", r"(?P<kind>(?:min|mi|m|-|−)11)"),
        make("minor-13th", r"(?P<kind>(?:min|mi|m|-|−)13)"),
        make("power", r"(?P<kind>5|power)"),
        make("major", r"(?P<kind>maj|ma|Δ)?"),
        make("minor", r"(?P<kind>min|mi|m|-|−)"),
        make("Italian", r"(?P<kind>It6)"),
        make("French", r"(?P<kind>Fr6)"),
        make("German", r"(?P<kind>Gr6)"),
        make("Tristan", r"(?P<kind>Tristan)"),
    ]
});

static DEGREE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(DEGREE_PATTERN).expect("valid degree regex"));

fn default_chord_symbol(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "augmented" => "+",
        "augmented-seventh" => "+7",
        "diminished" => "°",
        "diminished-seventh" => "°7",
        "half-diminished" => "ø",
        "dominant" => "7",
        "dominant-ninth" => "9",
        "dominant-11th" => "11",
        "dominant-13th" => "13",
        "major" => "Δ",
        "major-sixth" => "Δ6",
        "major-seventh" => "Δ7",
        "major-ninth" => "Δ9",
        "major-11th" => "Δ11",
        "major-13th" => "Δ13",
        "major-minor" => "-Δ7",
        "minor" => "-",
        "minor-sixth" => "-6",
        "minor-seventh" => "-7",
        "minor-ninth" => "-9",
        "minor-11th" => "-11",
        "minor-13th" => "-13",
        "suspended-fourth" => "sus4",
        "suspended-second" => "sus2",
        "power" => "5",
        "Italian" => "It6",
        "French" => "Fr6",
        "German" => "Gr6",
        "Tristan" => "Tristan",
        _ => return None,
    })
}

/// Identify the kind of chord and its extensions.
pub fn translate_chord_suffix(chord_suffix: Option<&str>) -> ChordSuffix {
    let major = || ChordSuffix {
        kind: "major".to_string(),
        use_symbols: "no".to_string(),
        parentheses_degrees: "no".to_string(),
        text: String::new(),
        degrees: Vec::new(),
    };

    let chord_suffix = match chord_suffix.map(str::trim) {
        Some(s) if !s.is_empty() => s,
        _ => return major(),
    };

    // Special-cased suffixes (helper.CHORD_SUFFIX).
    if chord_suffix == "69" || chord_suffix == "6/9" {
        return ChordSuffix {
            kind: "major-sixth".to_string(),
            use_symbols: "yes".to_string(),
            parentheses_degrees: "no".to_string(),
            text: String::new(),
            degrees: vec![Degree {
                degree_value: 9,
                degree_alter: 0,
                degree_type: "add".to_string(),
            }],
        };
    }

    for (kind_name, pattern) in CHORD_PATTERNS.iter() {
        let Some(caps) = pattern.captures(chord_suffix) else {
            continue;
        };
        let mut kind = (*kind_name).to_string();
        let mut text = caps
            .name("kind")
            .map(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let mut parentheses_degrees = if caps.name("parentheses_open").is_some()
            && caps.name("parentheses_closed").is_some()
        {
            "yes".to_string()
        } else {
            "no".to_string()
        };

        let mut degrees = Vec::new();
        if let Some(degrees_text) = caps.name("degrees").map(|m| m.as_str()) {
            for dm in DEGREE_RE.captures_iter(degrees_text) {
                let degree_alter = match dm.name("alter").map(|m| m.as_str()) {
                    Some("-") | Some("b") => -1,
                    Some("+") | Some("#") => 1,
                    _ => 0,
                };
                let degree_type = dm.name("type").map(|m| m.as_str());
                match degree_type {
                    Some("alt") => continue,
                    Some("sus") => {
                        kind = "suspended-fourth".to_string();
                        text.push_str("sus");
                        parentheses_degrees = "no".to_string();
                        continue;
                    }
                    Some("maj7") => {
                        if kind.starts_with("minor") {
                            kind = "major-minor".to_string();
                            text.push_str("(maj7)");
                            parentheses_degrees = "no".to_string();
                        }
                        if kind.starts_with("diminished") {
                            text.push_str("(addmaj7)");
                            parentheses_degrees = "no".to_string();
                        }
                        continue;
                    }
                    _ => {}
                }
                let mapped_type = match degree_type {
                    Some("omit") => "subtract",
                    _ => "add",
                };
                if let Some(value) = dm.name("value").map(|m| m.as_str()) {
                    if let Ok(degree_value) = value.parse::<i64>() {
                        degrees.push(Degree {
                            degree_value,
                            degree_alter,
                            degree_type: mapped_type.to_string(),
                        });
                    }
                }
            }
        }

        let use_symbols = if Some(text.as_str()) == default_chord_symbol(&kind) {
            text = String::new();
            "yes".to_string()
        } else {
            "no".to_string()
        };

        return ChordSuffix {
            kind,
            use_symbols,
            parentheses_degrees,
            text,
            degrees,
        };
    }

    tracing::warn!("could not translate suffix {chord_suffix}");
    ChordSuffix {
        kind: "other".to_string(),
        use_symbols: "no".to_string(),
        parentheses_degrees: "no".to_string(),
        text: chord_suffix.to_string(),
        degrees: Vec::new(),
    }
}

// -------------------------------------------------------------------------
// Key / pitch math
// -------------------------------------------------------------------------

/// `(mode, key_fifths)` for a Finale key integer and transposition adjustment.
pub fn calculate_mode_and_key_fifths(key: Option<i64>, transp_key_adjust: i64) -> (String, i64) {
    let mode = if key.map_or(true, |k| k < 256) {
        "major"
    } else {
        "minor"
    };
    let mut key_fifths = match key {
        None => 0,
        Some(k) if k > 384 => k - 512,
        Some(k) if k > 128 => k - 256,
        Some(k) => k,
    };
    key_fifths += transp_key_adjust;
    if key_fifths > 7 {
        key_fifths -= 12;
    }
    if key_fifths < -7 {
        key_fifths += 12;
    }
    (mode.to_string(), key_fifths)
}

fn calculate_alter(step: &str, key_fifths: i64) -> i64 {
    if key_fifths == 0 {
        0
    } else if key_fifths > 0 {
        if SHARPS_AND_FLATS[..key_fifths as usize].contains(&step) {
            1
        } else {
            0
        }
    } else if SHARPS_AND_FLATS[(7 + key_fifths) as usize..].contains(&step) {
        -1
    } else {
        0
    }
}

fn base_semitone(step: &str) -> i64 {
    match step {
        "C" => 0,
        "D" => 2,
        "E" => 4,
        "F" => 5,
        "G" => 7,
        "A" => 9,
        "B" => 11,
        _ => 0,
    }
}

/// Return an enharmonic equivalent note with a different letter name.
pub fn calculate_enharmonic(step: &str, alter: i64) -> (String, i64) {
    let p = (base_semitone(step) + alter).rem_euclid(12);
    let notes = ["C", "D", "E", "F", "G", "A", "B"];

    let mut best_candidate: Option<&str> = None;
    let mut best_acc: i64 = 0;
    for n in notes {
        if n == step {
            continue;
        }
        let mut diff = p - base_semitone(n);
        diff = (diff + 6).rem_euclid(12) - 6;
        if best_candidate.is_none() || diff.abs() < best_acc.abs() {
            best_candidate = Some(n);
            best_acc = diff;
        }
    }
    (best_candidate.unwrap_or(step).to_string(), best_acc)
}

/// `(step, alter, octave)` for a Finale harmonic level / alteration.
pub fn calculate_step_alter_and_octave(
    harm_lev: i64,
    harm_alt: i64,
    key: Option<i64>,
    transp_key_adjust: i64,
    transp_interval: i64,
    enharmonic: bool,
) -> (String, i64, String) {
    let (mode, fifths) = calculate_mode_and_key_fifths(key, transp_key_adjust);
    let notes = ["C", "D", "E", "F", "G", "A", "B"];
    let mut harm_lev = harm_lev;
    if mode == "minor" {
        harm_lev -= 2;
    }
    let index = (harm_lev + 4 * fifths).rem_euclid(7) as usize;
    let mut step = notes[index].to_string();

    let (_, fifths_no_key_adjust) = calculate_mode_and_key_fifths(key, 0);
    let mut octave =
        4 + (harm_lev + (4 * fifths_no_key_adjust).rem_euclid(7) + transp_interval).div_euclid(7);
    if !(0..=9).contains(&octave) {
        tracing::warn!("Octave out of range: {octave}");
        octave = octave.clamp(0, 9);
    }
    let mut alter = harm_alt + calculate_alter(&step, fifths);
    if enharmonic {
        let (s, a) = calculate_enharmonic(&step, alter);
        step = s;
        alter = a;
    }
    (step, alter, octave.to_string())
}

// -------------------------------------------------------------------------
// Text / duration translation
// -------------------------------------------------------------------------

static TEMPO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)^(.*?\s+)?([({]\s*)?(m\s+)?([xeqh])([d|.])?\s*=\s*(c[a.]{0,2}\s+)?(\d+)(\s*[)}])?(\s+.*)?")
        .expect("valid tempo regex")
});

/// Parsed tempo marking: `(words, beat_unit, has_dot, per_minute, parentheses)`.
pub struct TempoMarks {
    pub words: Option<String>,
    pub beat_unit: Option<String>,
    pub has_dot: bool,
    pub per_minute: Option<String>,
    pub parentheses: Option<String>,
}

pub fn translate_tempo_marks(text: &str) -> TempoMarks {
    let text_without_tags = remove_styling_tags(text);

    if let Some(caps) = TEMPO_RE.captures(&text_without_tags) {
        let prefix = caps.get(1).map(|m| m.as_str().trim().to_string());
        let has_bracket_open = caps.get(2).is_some();
        let has_mm = caps.get(3).is_some();
        let note = caps.get(4).map(|m| m.as_str()).unwrap_or("");
        let has_dot = caps.get(5).is_some();
        let has_ca = caps.get(6).is_some();
        let mut per_minute = caps.get(7).map(|m| m.as_str().to_string());
        let has_bracket_closed = caps.get(8).is_some();
        let postfix = caps.get(9).map(|m| m.as_str().trim().to_string());

        let mut words = prefix;
        if let Some(post) = postfix {
            words = Some(match words {
                Some(w) => format!("{w} {post}"),
                None => post,
            });
        }
        if has_mm {
            words = Some(format!("{} M. M.", words.unwrap_or_default()));
        }
        let beat_unit = note
            .chars()
            .next()
            .and_then(engraver_note_type)
            .map(str::to_string);
        if has_ca {
            per_minute = per_minute.map(|p| format!("c. {p}"));
        }
        let parentheses = if has_bracket_open && has_bracket_closed {
            "yes"
        } else {
            "no"
        };
        TempoMarks {
            words,
            beat_unit,
            has_dot,
            per_minute,
            parentheses: Some(parentheses.to_string()),
        }
    } else {
        if text_without_tags.contains('=') {
            tracing::warn!("Could not parse tempo markings : {text}");
        }
        TempoMarks {
            words: Some(text_without_tags),
            beat_unit: None,
            has_dot: false,
            per_minute: None,
            parentheses: None,
        }
    }
}

/// `(note_type, num_dots)` from a Finale duration integer.
pub fn calculate_type_and_dots(dura: i64) -> (Option<String>, i64) {
    let mut note_type: Option<String> = None;
    let mut num_dots = 0;
    for (flag, type_name) in FLAG_TO_TYPE {
        if dura & flag != 0 {
            if note_type.is_none() {
                note_type = Some(type_name.to_string());
            } else {
                num_dots += 1;
            }
        } else if note_type.is_some() {
            break;
        }
    }
    (note_type, num_dots)
}

/// `(instrument_name, instrument_sound)` for a Finale instrument UUID.
pub fn translate_instrument(inst_uuid: &str) -> (Option<String>, Option<String>) {
    if let Some(entry) = INST_UUID_MAP.get(inst_uuid) {
        (Some(entry.name.clone()), Some(entry.sound_id.clone()))
    } else {
        tracing::warn!("instrument not found {inst_uuid}");
        (None, None)
    }
}

/// `(sign, octave_change)` for an engraver clef char (string of an integer).
pub fn translate_clef_sign(clef_char: Option<&str>) -> (String, i64) {
    if let Some(code) = clef_char.and_then(|c| c.parse::<i64>().ok()) {
        if let Some((sign, octave)) = engraver_clef(code) {
            return (sign.to_string(), octave);
        }
    }
    tracing::warn!("Unknown clef char: {clef_char:?}");
    ("G".to_string(), 0)
}

pub fn translate_bar_style(bar_line_type: &str, bac_rep_bar: bool, bar_ending: bool) -> String {
    if bac_rep_bar || bar_ending {
        "light-heavy".to_string()
    } else {
        map_bar_line_type(bar_line_type)
            .unwrap_or("regular")
            .to_string()
    }
}

/// A tuplet definition accumulated across the entries it spans.
#[derive(Debug, Clone)]
pub struct TupletAttr {
    pub symbolic_num: i64,
    pub symbolic_dur: i64,
    pub ref_num: i64,
    pub ref_dur: i64,
    pub count: f64,
    pub number: String,
}

/// Port of `count_tuplet` — advance each tuplet's `count` by this entry's dura.
pub fn count_tuplet(tuplet_attributes: &mut [TupletAttr], dura: i64) {
    let mut refactor = 1.0_f64;
    for attributes in tuplet_attributes.iter_mut().rev() {
        attributes.count += refactor * dura as f64 / attributes.symbolic_dur as f64;
        refactor *= attributes.ref_num as f64 / attributes.symbolic_num as f64;
    }
}

static STYLING_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\^(?:font|fontid|Font|fontMus|fontTxt|fontNum|size|nfx|baseline)\([^)]*\)")
        .expect("valid styling regex")
});

pub fn remove_styling_tags(text: &str) -> String {
    STYLING_TAG_RE.replace_all(text, "").trim().to_string()
}

pub fn replace_music_symbols(text: &str) -> String {
    text.replace("^flat()", "\u{266D}")
        .replace("^sharp()", "\u{266F}")
        .replace("^natural()", "\u{266E}")
}

pub fn translate_dynamics(text: &str) -> Option<String> {
    let text = remove_styling_tags(text);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => engraver_dynamic(c as u32).map(str::to_string),
        _ => None,
    }
}

/// `(tag_name, type)` for an engraver articulation char (string of an integer).
pub fn translate_articulation(char_main: &str) -> (String, Option<String>) {
    if let Ok(code) = char_main.parse::<i64>() {
        if let Some((tag, ty)) = engraver_articulation(code) {
            return (tag.to_string(), ty.map(str::to_string));
        }
    }
    ("other-articulation".to_string(), None)
}

pub fn translate_chord_step(
    key: Option<i64>,
    transp_key_adjust: i64,
    root_scale_num: Option<&str>,
    root_alter: Option<&str>,
) -> (String, i64) {
    let harm_lev = root_scale_num.and_then(|s| s.parse().ok()).unwrap_or(0);
    let harm_alt = root_alter.and_then(|s| s.parse().ok()).unwrap_or(0);
    let (step, alter, _) =
        calculate_step_alter_and_octave(harm_lev, harm_alt, key, transp_key_adjust, 0, false);
    (step, alter)
}

/// Diatonic interval (concert→instrument) → MusicXML `(diatonic, chromatic, octave_change)`.
pub fn calculate_transpose(interval: i64) -> (i64, i64, i64) {
    let (is_up, interval) = if interval < 0 {
        (true, -interval)
    } else {
        (false, interval)
    };
    let octave_change = interval / 7;
    let diatonic = interval % 7;
    let chromatic = if diatonic > 2 {
        diatonic * 2 - 1
    } else {
        diatonic * 2
    };
    if is_up {
        (diatonic, chromatic, octave_change)
    } else {
        (-diatonic, -chromatic, -octave_change)
    }
}

/// A single lyric syllable: `(text, syllabic, extend)`.
pub fn find_nth_syllabic(lyrics: &str, n: i64) -> (String, String, bool) {
    let lyrics = remove_styling_tags(lyrics);
    let lyrics = lyrics.replace("_ ", "_").replace('_', "_ ");

    let mut syllabics: Vec<(String, &'static str, bool)> = Vec::new();
    for word in lyrics.split_whitespace() {
        let parts: Vec<&str> = word.split('-').collect();
        let len = parts.len();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            let extend = part.ends_with('_');
            let part = part.trim_end_matches('_').to_string();
            let syllabic = if len == 1 {
                "single"
            } else if i == 0 {
                "begin"
            } else if i == len - 1 {
                "end"
            } else {
                "middle"
            };
            syllabics.push((part, syllabic, extend));
        }
    }

    if n >= 1 && (n as usize) <= syllabics.len() {
        let (text, syllabic, extend) = syllabics[(n - 1) as usize].clone();
        (text, syllabic.to_string(), extend)
    } else {
        tracing::warn!("No {n}th syllabic found for {lyrics}");
        ("???".to_string(), "single".to_string(), false)
    }
}
