//! Chord parsing for charts
//!
//! Handles parsing of chord lines, individual chord tokens, and related
//! functionality including duration calculation, slash chords, and push/pull notation.

use super::ChartParser;
use super::helpers::{PushPullModifier, RepeatCount};
use crate::chart::cues::TextCue;
use crate::chart::dynamics::DynamicMarking;
use crate::chart::melody::{Melody, MelodyNote};
use crate::chart::types::{
    ChordInstance, KeyChange, Measure, RestInstance, RhythmElement, SpaceInstance,
};
use crate::chord::{ChordQuality, ChordRhythm, LilySyntax, NotationSystem};
use crate::key::Key;
use crate::parsing::{Lexer, TextSpan};
use crate::primitives::{Accidental, RootNotation};
use crate::sections::SectionType;
use crate::time::{
    AbsolutePosition, MusicalDuration, MusicalPosition, MusicalPositionExt, TimeSignature,
    TimeSignatureExt,
};
use keyflow_proto::chart::{
    Dynamic, DynamicLevel, FiguredBass, FiguredBassRow, Hairpin, HairpinKind, Placement,
    RepeatMark, StaffText, SuspensionFigure, Volta,
};

// region:    --- Token Helpers

impl<'a> ChartParser<'a> {
    fn parse_quoted_text_token(token: &str) -> Option<(String, Placement)> {
        let (token, placement) = if let Some(rest) = token.strip_prefix("^\"") {
            (format!("\"{rest}"), Placement::Above)
        } else if let Some(rest) = token.strip_prefix("_\"") {
            (format!("\"{rest}"), Placement::Below)
        } else {
            (token.to_string(), Placement::Below)
        };
        let inner = token.strip_prefix('"')?.strip_suffix('"')?;
        Some((unescape_quoted(inner), placement))
    }

    pub(super) fn parse_alias_declaration(line: &str) -> Option<(String, String)> {
        let (name, value) = if let Some(rest) = line.strip_prefix("/alias ") {
            let mut parts = rest.splitn(2, char::is_whitespace);
            (parts.next()?.trim(), parts.next()?.trim())
        } else if let Some(rest) = line.strip_prefix("let ") {
            let (name, value) = rest.split_once('=')?;
            (name.trim(), value.trim())
        } else {
            return None;
        };
        if name.is_empty() || value.is_empty() {
            return None;
        }
        let value = value
            .strip_prefix('{')
            .and_then(|inner| inner.strip_suffix('}'))
            .map(str::trim)
            .unwrap_or(value);
        Some((name.to_string(), value.to_string()))
    }

    pub(super) fn expand_aliases_in_line(&self, line: &str) -> String {
        let spans = tokenize_with_spans(line);
        if spans.is_empty() {
            return line.to_string();
        }

        let mut out = String::new();
        let mut cursor = 0usize;
        let mut changed = false;
        for span in spans {
            out.push_str(&line[cursor..span.start]);
            let token = &line[span.as_range()];
            if let Some(expanded) = self.expand_alias_token(token) {
                out.push_str(&expanded);
                changed = true;
            } else {
                out.push_str(token);
            }
            cursor = span.start + span.len;
        }
        out.push_str(&line[cursor..]);

        if changed { out } else { line.to_string() }
    }

    fn expand_alias_token(&self, token: &str) -> Option<String> {
        if let Some(name) = token.strip_prefix("^<").and_then(|t| t.strip_suffix('>')) {
            return self.aliases.get(name).map(|value| {
                apply_alias_placement(value, Some(Placement::Above))
                    .unwrap_or_else(|| value.to_string())
            });
        }
        if let Some(name) = token.strip_prefix("_<").and_then(|t| t.strip_suffix('>')) {
            return self.aliases.get(name).map(|value| {
                apply_alias_placement(value, Some(Placement::Below))
                    .unwrap_or_else(|| value.to_string())
            });
        }
        if let Some(name) = token.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
            return self.aliases.get(name).map(|value| {
                apply_alias_placement(value, None).unwrap_or_else(|| value.to_string())
            });
        }
        if let Some((chord, name)) = split_chord_attached_alias(token) {
            return self
                .aliases
                .get(name)
                .and_then(|value| alias_quoted_payload(value))
                .map(|payload| format!("{chord}\"{payload}\""));
        }
        None
    }

    fn parse_standalone_staff_text_line(&self, line: &str) -> Option<Vec<StaffText>> {
        if line.starts_with("<<") {
            return None;
        }
        let expanded = self.expand_aliases_in_line(line);
        let spans = tokenize_with_spans(&expanded);
        if spans.is_empty() {
            return None;
        }

        let mut items = Vec::new();
        for span in spans {
            let token = &expanded[span.as_range()];
            let (text, placement) = Self::parse_quoted_text_token(token)?;
            items.push(StaffText {
                text,
                beat: 1,
                placement,
                source_default_x: None,
                boxed: false,
                bold: false,
                italic: false,
            });
        }

        Some(items)
    }

    fn parse_classical_dynamic_line(line: &str) -> Option<Dynamic> {
        let value = line
            .strip_prefix("/dyn ")
            .or_else(|| line.strip_prefix("/dynamic "))
            .or_else(|| line.strip_prefix("dyn "))
            .or_else(|| line.strip_prefix("dynamic "))?
            .trim();
        let mut parts = value.split_whitespace();
        let token = parts.next()?;
        let (level_token, beat) = token
            .split_once('@')
            .map(|(level, beat)| (level, beat.parse::<u8>().ok().unwrap_or(1)))
            .unwrap_or((token, 1));
        let level = Self::parse_dynamic_level(level_token)?;
        let placement = parts
            .next()
            .and_then(Self::parse_placement)
            .unwrap_or(Placement::Below);
        Some(Dynamic {
            level,
            beat,
            placement,
        })
    }

    fn parse_hairpin_line(line: &str) -> Option<Hairpin> {
        let value = line
            .strip_prefix("/hairpin ")
            .or_else(|| line.strip_prefix("hairpin "))?
            .trim();
        let mut parts = value.split_whitespace();
        let kind = match parts.next()? {
            "<" | "crescendo" | "cresc" => HairpinKind::Crescendo,
            ">" | "decrescendo" | "decresc" | "dim" | "diminuendo" => HairpinKind::Decrescendo,
            _ => return None,
        };
        let span = parts.next().unwrap_or("1..1");
        let (start, end) = span
            .split_once("..")
            .or_else(|| span.split_once('-'))
            .unwrap_or(("1", "1"));
        let start_beat = start
            .trim_start_matches('@')
            .parse::<u8>()
            .ok()
            .unwrap_or(1);
        let end_beat = end
            .trim_start_matches('@')
            .parse::<u8>()
            .ok()
            .unwrap_or(start_beat);
        let placement = parts
            .next()
            .and_then(Self::parse_placement)
            .unwrap_or(Placement::Below);
        Some(Hairpin {
            kind,
            start_beat,
            end_measure_offset: 0,
            end_beat,
            placement,
        })
    }

    fn parse_dynamic_level(value: &str) -> Option<DynamicLevel> {
        match value {
            "ppp" => Some(DynamicLevel::Ppp),
            "pp" => Some(DynamicLevel::Pp),
            "p" => Some(DynamicLevel::P),
            "mp" => Some(DynamicLevel::Mp),
            "mf" => Some(DynamicLevel::Mf),
            "f" => Some(DynamicLevel::F),
            "ff" => Some(DynamicLevel::Ff),
            "fff" => Some(DynamicLevel::Fff),
            "sf" => Some(DynamicLevel::Sf),
            "sfz" => Some(DynamicLevel::Sfz),
            "fp" => Some(DynamicLevel::Fp),
            _ => None,
        }
    }

    fn parse_placement(value: &str) -> Option<Placement> {
        match value.to_ascii_lowercase().as_str() {
            "above" | "^" => Some(Placement::Above),
            "below" | "_" => Some(Placement::Below),
            _ => None,
        }
    }

    /// A floating slash-bass token like `/D`, `/Bb`, `/F#` — a leading slash
    /// followed by a bare note name (optional accidental) and nothing else.
    /// These inherit the previous chord's root (`Bb` → `Bb/D`) but display
    /// verbatim. Returns the bass-note text (without the slash).
    ///
    /// Deliberately rejects rhythm slashes (`//`, `/.`), durations (`/4`),
    /// commands (`/fermata`), and slash-family notation (`/maj7`).
    fn parse_floating_slash_bass(token: &str) -> Option<&str> {
        let bass = token.strip_prefix('/')?;
        if bass.is_empty() || bass.contains('/') {
            return None;
        }
        let mut chars = bass.chars();
        let first = chars.next()?;
        if !matches!(first, 'A'..='G' | 'a'..='g') {
            return None;
        }
        // Remaining chars (if any) must be a single accidental.
        match chars.as_str() {
            "" | "#" | "b" | "n" | "\u{266d}" | "\u{266f}" | "\u{266e}" => Some(bass),
            _ => None,
        }
    }

    /// A floating suspension figure token. Only the hyphenated form (`4-3`,
    /// `2-3`) is recognized when standalone: a lone digit like `2`, `3`, or
    /// `4` is a valid scale-degree chord in keyflow, so accepting it here would
    /// swallow real chords. Single-digit figures must be written attached to a
    /// note-name root (`Eb2`, `F3`), which [`extract_suspension_suffix`]
    /// handles unambiguously. Returns the figure verbatim.
    fn parse_bare_suspension_figure(token: &str) -> Option<String> {
        let (a, b) = token.split_once('-')?;
        (!a.is_empty()
            && !b.is_empty()
            && a.chars().all(|c| c.is_ascii_digit())
            && b.chars().all(|c| c.is_ascii_digit()))
        .then(|| token.to_string())
    }

    /// Split a trailing suspension figure off an attached chord token, e.g.
    /// `F4-3` → (`F`, `4-3`), `Eb2` → (`Eb`, `2`), `F3` → (`F`, `3`),
    /// `E3-4-3` → (`E`, `3-4-3`).
    ///
    /// Only fires when the chord part is a bare note root (letter + optional
    /// accidental) and the suffix is a figure: a hyphenated digit run (`d-d`,
    /// `d-d-d`, …, always a figure) or a lone `2`/`3`/`4` (the suspension
    /// degrees — so `Csus4`, `C6`, `Cm7`, `Gm7b5` stay intact).
    fn extract_suspension_suffix(token: &str) -> (String, Option<String>) {
        // Slash chords and quoted suffixes are handled elsewhere.
        if token.contains('/') || token.contains('"') {
            return (token.to_string(), None);
        }
        // Length of the leading bare note root (letter + optional accidental).
        let root_len = Self::bare_note_root_len(token);
        if root_len == 0 {
            return (token.to_string(), None);
        }
        let (root, suffix) = token.split_at(root_len);
        let is_figure = if suffix.contains('-') {
            // Hyphenated digit run: every dash-separated part is digits.
            suffix
                .split('-')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        } else {
            // Lone suspension degree.
            matches!(suffix, "2" | "3" | "4")
        };
        if is_figure {
            (root.to_string(), Some(suffix.to_string()))
        } else {
            (token.to_string(), None)
        }
    }

    /// Byte length of a leading bare note root: a letter `A`–`G` (either case)
    /// plus an optional single accidental. Returns 0 if the token doesn't
    /// start with a note name.
    fn bare_note_root_len(s: &str) -> usize {
        let mut chars = s.char_indices();
        match chars.next() {
            Some((_, 'A'..='G' | 'a'..='g')) => {}
            _ => return 0,
        }
        match chars.next() {
            Some((i, c @ ('#' | 'b' | 'n' | '\u{266d}' | '\u{266f}' | '\u{266e}'))) => {
                i + c.len_utf8() // root = note letter + this accidental
            }
            Some((i, _)) => i, // root is just the note letter
            None => s.len(),   // single-char token, all root
        }
    }

    /// Root portion of the most recent chord (current measure first, then the
    /// last completed measure). A floating slash-bass inherits this — existing
    /// bass is dropped so `F/Eb` then `/D` yields `F/D`.
    fn previous_chord_root(current: &Measure, measures: &[Measure]) -> Option<String> {
        let prev = current
            .chords
            .last()
            .or_else(|| measures.iter().rev().find_map(|m| m.chords.last()))?;
        let base = prev
            .full_symbol
            .split('/')
            .next()
            .unwrap_or(&prev.full_symbol);
        (!base.is_empty()).then(|| base.to_string())
    }

    fn push_suspension_at_current_beat(
        measure: &mut Measure,
        figure: &str,
        placement: Placement,
        current_measure_beats: f64,
        standalone: bool,
    ) {
        let beat = current_measure_beats.floor().max(0.0) as u8 + 1;
        measure.suspensions.push(SuspensionFigure {
            figure: figure.to_string(),
            beat,
            placement,
            standalone,
        });
    }

    fn extract_figured_bass_suffix(token: &str) -> (String, Option<Vec<FiguredBassRow>>) {
        if let Some((chord_token, quoted)) = split_chord_attached_quote(token) {
            let text = unescape_quoted(quoted);
            if let Some(rows) = Self::parse_figured_bass_text(&text) {
                return (chord_token.to_string(), Some(rows));
            }
        }

        (token.to_string(), None)
    }

    /// Recognise a `^`-marked **inversion** figure on a chord (`V^6`, `V^65`,
    /// `V^43`, …) and return how to realise it as a real inverted chord:
    /// `(chord_token, display, append_seventh, bass_thirds)`.
    ///
    /// - `chord_token` is what to actually parse (the root, plus `7` for the
    ///   seventh-chord figures), with any trailing `_dur` kept.
    /// - `display` is the original `root^figure` to show in the chart.
    /// - `bass_thirds` is how many thirds above the root the bass tone sits
    ///   (1 = 3rd, 2 = 5th, 3 = 7th), used to set the slash bass.
    ///
    /// Suspension figures (`4-3`) and anything unrecognised return `None` and
    /// fall through to [`extract_caret_figure`] (a plain figured-bass figure).
    fn extract_caret_inversion(token: &str) -> Option<(String, String, bool, u8)> {
        let caret = token.find('^')?;
        let after = &token[caret + 1..];
        let fig_len = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        let figure = &after[..fig_len];
        let (append_seventh, bass_thirds) = match figure {
            "6" => (false, 1),
            "64" => (false, 2),
            "65" => (true, 1),
            "43" => (true, 2),
            "42" | "2" => (true, 3),
            _ => return None,
        };
        let root_part = &token[..caret];
        let trailing = &after[fig_len..]; // e.g. a `_4` duration
        // The figure states the chord exactly, so it must ignore chord memory:
        // append `7` for the seventh figures (explicit family), and prefix `!`
        // on the triad figures so a remembered seventh can't sneak in
        // (`V^65 V^6` → the `V^6` stays a triad).
        let chord_token = if append_seventh {
            format!("{root_part}7{trailing}")
        } else {
            format!("!{root_part}{trailing}")
        };
        let display = format!("{root_part}^{figure}");
        Some((chord_token, display, append_seventh, bass_thirds))
    }

    /// Pull an inline `^`-marked figured-bass / inversion figure off a chord
    /// token: `V^65`, `V^6`, `V^43`, `V^4-3`. The `^` keeps these distinct from
    /// a plain chord (`V6` is still an added-6th chord) and from the quoted
    /// `"4-3"` form. Returns the chord token (with the figure removed, any
    /// trailing duration preserved) and the parsed figured-bass rows. A `^` that
    /// introduces above-staff text (`^"…"`, its own token) or a non-figure is
    /// left untouched.
    fn extract_caret_figure(token: &str) -> (String, Option<Vec<FiguredBassRow>>) {
        let Some(caret) = token.find('^') else {
            return (token.to_string(), None);
        };
        let after = &token[caret + 1..];
        if after.starts_with('"') {
            return (token.to_string(), None); // ^"text" is above-staff text
        }
        // The figure runs over figured-bass characters; anything past it (e.g. a
        // `_4` duration) stays on the chord token.
        let fig_len = after
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '#' | 'b' | 'n')))
            .unwrap_or(after.len());
        let figure = &after[..fig_len];
        let Some(rows) = Self::parse_figured_bass_text(figure) else {
            return (token.to_string(), None);
        };
        let chord_token = format!("{}{}", &token[..caret], &after[fig_len..]);
        (chord_token, Some(rows))
    }

    fn parse_figured_bass_rows(input: &str) -> Vec<FiguredBassRow> {
        let row_texts = if input.contains('/') || input.contains(',') {
            input
                .split(['/', ','])
                .map(str::trim)
                .filter(|row| !row.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        } else {
            split_figured_bass_words(input)
        };

        row_texts
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .filter(|row| !row.is_empty())
            .map(|row| {
                let (accidental, text) = Self::split_figured_bass_accidental(row);
                FiguredBassRow { accidental, text }
            })
            .collect()
    }

    fn parse_figured_bass_text(input: &str) -> Option<Vec<FiguredBassRow>> {
        let rows = Self::parse_figured_bass_rows(input);
        (!rows.is_empty()
            && rows
                .iter()
                .all(|row| looks_like_figured_bass_text(&row.text)))
        .then_some(rows)
    }

    fn split_figured_bass_accidental(row: &str) -> (String, String) {
        for accidental in ["bb", "##", "#", "b", "n", "\u{266e}"] {
            if let Some(text) = row.strip_prefix(accidental) {
                return (accidental.to_string(), text.to_string());
            }
        }
        (String::new(), row.to_string())
    }

    fn push_figured_bass_at_current_beat(
        measure: &mut Measure,
        rows: &[FiguredBassRow],
        placement: Placement,
        current_measure_beats: f64,
    ) {
        let beat = current_measure_beats.floor().max(0.0) as u8 + 1;
        measure.figured_bass.push(FiguredBass {
            rows: rows.to_vec(),
            beat,
            placement,
            source_default_x: None,
            source_relative_x: None,
        });
    }

    fn parse_chord_length_value(
        token: &str,
        time_sig: TimeSignature,
    ) -> Option<(ChordRhythm, MusicalDuration)> {
        if let Some((beats, dotted)) = parse_lily_duration_beats(token, time_sig) {
            return Some((
                ChordRhythm::Slashes {
                    count: beats.round().max(1.0) as u8,
                    dotted,
                    tied: false,
                },
                MusicalDuration::from_beats(beats, time_sig),
            ));
        }

        let dotted = token.ends_with('.');
        let slashes = token.trim_end_matches('.');
        if !slashes.is_empty() && slashes.chars().all(|c| c == '/') {
            let count = slashes.len() as u8;
            let beats = if dotted {
                f64::from(count) * dotted_slash_beats(time_sig)
            } else {
                f64::from(count)
            };
            return Some((
                ChordRhythm::Slashes {
                    count,
                    dotted,
                    tied: false,
                },
                MusicalDuration::from_beats(beats, time_sig),
            ));
        }

        None
    }

    fn token_has_explicit_chord_length(token: &str) -> bool {
        token.contains('_') || token.contains("//") || token.ends_with("/.") || token.ends_with('/')
    }

    fn parse_volta_token(token: &str) -> Option<Volta> {
        let token = token.split_once('_').map_or(token, |(base, _)| base);
        let inner = token.strip_prefix('[')?.strip_suffix(']')?;
        let numbers = inner
            .split(',')
            .filter_map(|part| part.trim().parse::<u8>().ok())
            .collect::<Vec<_>>();
        if numbers.is_empty() {
            return None;
        }

        Some(Volta {
            label: format!(
                "{}.",
                numbers
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            numbers,
            length_measures: 1,
        })
    }

    fn finalize_measure_for_separator(
        measures: &mut Vec<Measure>,
        current_measure: &Measure,
        measure_was_created_by_separator: bool,
    ) {
        if !current_measure.chords.is_empty()
            || !current_measure.rhythm_elements.is_empty()
            || !current_measure.figured_bass.is_empty()
            || !current_measure.staff_text.is_empty()
            || !current_measure.text_cues.is_empty()
            || !current_measure.melodies.is_empty()
            || current_measure.volta_start.is_some()
            || !matches!(current_measure.start_repeat, RepeatMark::None)
            || !matches!(current_measure.end_repeat, RepeatMark::None)
            || measure_was_created_by_separator
        {
            measures.push(current_measure.clone());
        }
    }

    fn fresh_measure(time_sig: TimeSignature) -> Measure {
        let mut measure = Measure::new();
        measure.time_signature = (time_sig.numerator as u8, time_sig.denominator as u8);
        measure
    }

    pub(super) fn join_multiline_parallel_containers(lines: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut parallel_depth = 0usize;
        let mut let_block_depth = 0usize;

        for line in lines {
            let trimmed = line.trim();
            if parallel_depth == 0 && let_block_depth == 0 {
                current.clear();
            } else if !current.is_empty() {
                if let_block_depth > 0 {
                    current.push('\n');
                } else {
                    current.push(' ');
                }
            }
            current.push_str(trimmed);
            parallel_depth = parallel_depth_for_line(parallel_depth, trimmed);
            let_block_depth = let_block_depth_for_line(let_block_depth, trimmed);

            if parallel_depth == 0 && let_block_depth == 0 {
                out.push(std::mem::take(&mut current));
            }
        }

        if !current.trim().is_empty() {
            out.push(current);
        }

        out
    }

    /// Normalize chord case - capitalize first letter for note names
    /// This allows "cmaj7", "dm7", "g7", "bbmaj7" to be parsed as "Cmaj7", "Dm7", "G7", "Bbmaj7"
    pub(super) fn normalize_chord_case(token: &str) -> String {
        if token.is_empty() {
            return token.to_string();
        }

        // A token starting with I/V/i/v is a Roman numeral — those letters are
        // never note names, so preserve case (lowercase = minor) regardless of
        // what follows. This keeps `v7`, `i:m7`, `viidim`, `IVmaj7` correct;
        // the old check only preserved case when the *second* char was also a
        // Roman letter, so single-numeral chords like `v7` lost their case.
        let first_char = token.chars().next().unwrap();
        if matches!(first_char, 'I' | 'V' | 'i' | 'v') {
            return token.to_string();
        }

        // If the first character is a lowercase letter (a-g), capitalize it
        if first_char.is_lowercase() && first_char.is_alphabetic() {
            let mut chars = token.chars();
            chars.next(); // skip first
            let mut result = first_char.to_uppercase().to_string();

            // Check if the second character is 'b' or '#' - if so, keep it as is
            // This handles "bbmaj7" -> "Bbmaj7" correctly
            result.push_str(chars.as_str());
            result
        } else {
            token.to_string()
        }
    }

    /// Extract leading push modifiers for push notation
    /// Examples: "'C" -> (modifier with count=1, "C")
    ///           "''Em" -> (modifier with count=2, "Em")
    ///           "'tC" -> (modifier with count=1, triplet=true, "C")
    ///           "':5C" -> (modifier with count=1, tuplet=5, "C")
    ///           "'_4C" -> (modifier with duration=Quarter, "C")
    ///           "'_8tC" -> (modifier with duration=Eighth, triplet=true, "C")
    ///           "'_4.C" -> (modifier with duration=Quarter, dotted=true, "C")
    ///           "C" -> (empty modifier, "C")
    pub(super) fn extract_leading_push_modifiers(token: &str) -> (PushPullModifier, &str) {
        let mut modifier = PushPullModifier::default();
        let mut pos = 0;
        let bytes = token.as_bytes();

        // Count leading apostrophes
        while pos < bytes.len() && bytes[pos] == b'\'' {
            modifier.count += 1;
            pos += 1;
        }

        if modifier.count == 0 {
            return (modifier, token);
        }

        // Check for duration-based push/pull ('_4, '_8, '_8t, '_4.)
        if pos < bytes.len() && bytes[pos] == b'_' {
            let underscore_pos = pos;
            pos += 1;

            // Try to match valid durations: 32, 16, 8, 4, 2, 1 (longest first)
            // This handles ambiguous cases like '_44' (quarter note push on chord "4")
            let remaining = &token[pos..];
            let duration_candidates = ["32", "16", "8", "4", "2", "1"];

            for candidate in &duration_candidates {
                if remaining.starts_with(candidate) {
                    if let Some(duration) = LilySyntax::from_number(candidate) {
                        modifier.duration = Some(duration);
                        modifier.count = 1; // Duration-based push/pull ignores count
                        pos += candidate.len();

                        // Check for dotted (.) or triplet (t) modifier after duration
                        if pos < bytes.len() {
                            if bytes[pos] == b'.' {
                                modifier.duration_dotted = true;
                                pos += 1;
                            } else if bytes[pos] == b't' {
                                modifier.duration_triplet = true;
                                pos += 1;
                            }
                        }
                        return (modifier, &token[pos..]);
                    }
                }
            }

            // No valid duration found after underscore - revert and treat as regular push
            pos = underscore_pos;
        }

        // Check for triplet 't' marker
        if pos < bytes.len() && bytes[pos] == b't' {
            modifier.is_triplet = true;
            pos += 1;
        }
        // Check for tuplet ':N' marker
        else if pos < bytes.len() && bytes[pos] == b':' {
            pos += 1;
            // Parse the number
            let num_start = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos > num_start {
                if let Ok(n) = token[num_start..pos].parse::<u8>() {
                    if n >= 3 {
                        modifier.tuplet = Some(n);
                    }
                }
            }
        }

        (modifier, &token[pos..])
    }

    /// Extract trailing pull modifiers for pull notation
    /// Examples: "C'" -> ("C", modifier with count=1)
    ///           "Em''" -> ("Em", modifier with count=2)
    ///           "C't" -> ("C", modifier with count=1, triplet=true)
    ///           "C':5" -> ("C", modifier with count=1, tuplet=5)
    ///           "C'_4" -> ("C", modifier with duration=Quarter)
    ///           "C'_8t" -> ("C", modifier with duration=Eighth, triplet=true)
    ///           "C'_4." -> ("C", modifier with duration=Quarter, dotted=true)
    ///           "D'//  -> ("D//", modifier with count=1) - stops at rhythm notation
    ///           "C" -> ("C", empty modifier)
    ///
    /// Find where rhythm notation starts in a token, distinguishing slash chords from rhythm slashes.
    ///
    /// Rhythm notation is:
    /// - "//" (multiple slashes for duration continuation)
    /// - "/" at end of token (standalone continuation)
    /// - "/" followed by '_' or '\'' (rhythm with duration/pull)
    ///
    /// Slash chord (NOT rhythm) is:
    /// - "/" followed by a letter (the bass note, e.g., "Gm7/D")
    /// - "/" followed by a digit (Nashville number bass, e.g., "1/3")
    ///
    /// Returns the position where rhythm notation starts, or token.len() if no rhythm notation.
    fn find_rhythm_slash_position(token: &str) -> usize {
        let bytes = token.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            if bytes[i] == b'/' {
                // Check what follows the slash
                if i + 1 >= bytes.len() {
                    // Slash at end - this is rhythm notation
                    return i;
                }

                let next_char = bytes[i + 1];
                if next_char == b'/' {
                    // Another slash follows - this is rhythm notation (e.g., "//")
                    return i;
                } else if next_char == b'_' || next_char == b'\'' {
                    // Duration or pull modifier follows - this is rhythm notation
                    return i;
                } else if next_char.is_ascii_alphabetic() || next_char.is_ascii_digit() {
                    // A letter or digit follows - this is a slash chord bass note
                    // Skip past the bass note to continue looking for rhythm
                    i += 1;
                    // Skip the bass note (letters, digits, accidentals)
                    while i < bytes.len()
                        && (bytes[i].is_ascii_alphanumeric()
                            || bytes[i] == b'#'
                            || bytes[i] == b'b')
                    {
                        i += 1;
                    }
                    continue;
                } else {
                    // Something unexpected follows - treat as rhythm notation
                    return i;
                }
            }
            i += 1;
        }

        // No rhythm notation found
        token.len()
    }

    pub(super) fn extract_trailing_pull_modifiers(token: &str) -> (String, PushPullModifier) {
        // First, find where the chord part ends and rhythm notation begins
        // Rhythm notation: / (but NOT '_4 style duration which is part of pull notation)
        // For pull notation, we need to distinguish between:
        // - "C'_4" = pull by quarter note (the _4 is part of the pull modifier)
        // - "C_4" = chord with quarter note duration (the _4 is rhythm notation)
        // So we only split on '/' for standalone slash rhythm
        //
        // IMPORTANT: We must NOT split at slash chords like "Gm7/D" where "/D" is the bass note.
        // Rhythm notation is: "//" (multiple slashes), "/" at end, or "/" followed by '_' or '\''
        // Slash chord is: "/" followed by a letter (the bass note)
        let rhythm_start = Self::find_rhythm_slash_position(token);

        // Only extract modifiers between chord and rhythm
        let chord_and_modifiers = &token[..rhythm_start];
        let rhythm_part = &token[rhythm_start..];

        let mut modifier = PushPullModifier::default();

        // Work backwards from end of chord_and_modifiers
        let bytes = chord_and_modifiers.as_bytes();
        let mut end_pos = bytes.len();

        // Check for duration-based pull notation (e.g., "'_4", "'_8t", "'_4." at the end)
        // We need to find "'_" followed by a valid duration (32, 16, 8, 4, 2, 1)
        // and optionally followed by 't' or '.'
        // This handles ambiguous cases like "4'_4" (chord "4" pulled by quarter note)
        if let Some(apostrophe_underscore_pos) = chord_and_modifiers.rfind("'_") {
            let after_underscore = &chord_and_modifiers[apostrophe_underscore_pos + 2..];
            let duration_candidates = ["32", "16", "8", "4", "2", "1"];

            for candidate in &duration_candidates {
                if let Some(after_duration) = after_underscore.strip_prefix(candidate) {
                    if let Some(duration) = LilySyntax::from_number(candidate) {
                        // Check for optional dotted (.) or triplet (t) modifier
                        let (has_dot, has_triplet, suffix_len) = if after_duration.starts_with('.')
                        {
                            (true, false, 1)
                        } else if after_duration.starts_with('t') {
                            (false, true, 1)
                        } else {
                            (false, false, 0)
                        };

                        // Verify nothing else follows (or it's end of string)
                        if after_duration.len() == suffix_len {
                            modifier.duration = Some(duration);
                            modifier.count = 1;
                            modifier.duration_dotted = has_dot;
                            modifier.duration_triplet = has_triplet;
                            end_pos = apostrophe_underscore_pos;

                            let chord_only = &chord_and_modifiers[..end_pos];
                            let result = format!("{}{}", chord_only, rhythm_part);
                            return (result, modifier);
                        }
                    }
                }
            }
        }

        // Check for trailing tuplet number first (e.g., ":5" at the end)
        // Pattern: ':' followed by digits at the very end
        if end_pos >= 2 {
            let num_end = end_pos;
            let mut num_start = end_pos;

            // Find digits at end
            while num_start > 0 && bytes[num_start - 1].is_ascii_digit() {
                num_start -= 1;
            }

            // Check if preceded by ':'
            if num_start > 0 && num_start < num_end && bytes[num_start - 1] == b':' {
                if let Ok(n) = chord_and_modifiers[num_start..num_end].parse::<u8>() {
                    if n >= 3 {
                        modifier.tuplet = Some(n);
                        end_pos = num_start - 1; // Position before ':'
                    }
                }
            }
        }

        // Check for trailing 't' marker
        if end_pos > 0 && bytes[end_pos - 1] == b't' {
            // Make sure it's not part of a chord name - check if preceded by apostrophe
            if end_pos > 1 && bytes[end_pos - 2] == b'\'' {
                modifier.is_triplet = true;
                end_pos -= 1;
            }
        }

        // Count trailing apostrophes
        while end_pos > 0 && bytes[end_pos - 1] == b'\'' {
            modifier.count += 1;
            end_pos -= 1;
        }

        if modifier.count > 0 {
            // Remove modifiers but keep rhythm
            let chord_only = &chord_and_modifiers[..end_pos];
            let result = format!("{}{}", chord_only, rhythm_part);
            (result, modifier)
        } else {
            (token.to_string(), modifier)
        }
    }

    /// Split a slash chord into chord and bass parts
    /// Examples: "1/3" -> ("1", Some("3"))
    ///           "Cmaj7/E" -> ("Cmaj7", Some("E"))
    ///           "Fm/Ab_4" -> ("Fm_4", Some("Ab")) - preserves duration suffix on chord
    ///           "g//" -> ("g//", None)  (rhythm slashes, not a slash chord)
    ///           "g" -> ("g", None)
    pub(super) fn split_slash_chord(token: &str) -> (String, Option<&str>) {
        // Find the first slash
        if let Some(slash_pos) = token.find('/') {
            // Check if this is followed by another slash (rhythm notation)
            if slash_pos + 1 < token.len() {
                let after_slash = &token[slash_pos + 1..];
                if after_slash.starts_with('/')
                    || after_slash.starts_with('_')
                    || after_slash.starts_with('\'')
                    || after_slash.is_empty()
                {
                    // This is rhythm notation, not a slash chord
                    return (token.to_string(), None);
                }

                // Check if what follows the slash looks like a note/degree
                let bass_candidate = after_slash.chars().next().unwrap();
                if bass_candidate.is_alphabetic() || bass_candidate.is_ascii_digit() {
                    // This looks like a slash chord
                    // Extract just the bass note (stop at any rhythm notation)
                    let bass_end = after_slash
                        .find(['/', '_', '\''])
                        .unwrap_or(after_slash.len());

                    let chord_root = &token[..slash_pos];
                    let bass_note = &after_slash[..bass_end];
                    let rhythm_suffix = &after_slash[bass_end..];

                    // Reconstruct chord part with rhythm suffix attached
                    // e.g., "Fm/Ab_4" -> chord_root="Fm", bass="Ab", rhythm="_4" -> "Fm_4"
                    let chord_part = format!("{}{}", chord_root, rhythm_suffix);
                    return (chord_part, Some(bass_note));
                }
            }
        }

        (token.to_string(), None)
    }

    /// Apply automatic durations to chords between measure separators
    /// If chords are between | separators, split the measure evenly
    /// Examples:
    ///   "| G C |" → "| G_2 C_2 |" (2 chords = half notes each)
    ///   "G C | D" → "G_2 C_2 | D_1" (2 chords before | = half notes, 1 after = whole)
    ///   "G C E D | A" → "G_4 C_4 E_4 D_4 | A_1" (4 chords before | = quarter notes, 1 after = whole)
    pub(super) fn apply_auto_durations_between_separators(
        line: &str,
        beats_per_measure: f64,
    ) -> String {
        use keyflow_proto::chart::commands::Command;

        // Check if line has any separators - if not, return as-is
        if !line.contains('|') {
            return line.to_string();
        }

        // Split by | to get segments
        let segments: Vec<&str> = line.split('|').collect();
        let mut result = String::new();

        for (i, segment) in segments.iter().enumerate() {
            let segment = segment.trim();

            // Handle empty segments (multiple | in a row or at start/end)
            if segment.is_empty() {
                // Add separator if not at the very end
                if i < segments.len() - 1 {
                    if !result.is_empty() {
                        result.push(' ');
                    }
                    result.push('|');
                }
                continue;
            }

            // Count chords in this segment (exclude commands, cues, dot repeats, etc.)
            // Count ALL chords, even those with explicit durations, to calculate segment duration
            let token_spans = tokenize_with_spans(segment);
            let tokens: Vec<&str> = token_spans
                .iter()
                .map(|span| &segment[span.as_range()])
                .collect();

            // Build a mask of which tokens are inside m{...} melody blocks
            // so we can skip them for chord counting and duration assignment
            let mut in_melody = false;
            let melody_mask: Vec<bool> = tokens
                .iter()
                .map(|t| {
                    if t.starts_with("m{") {
                        in_melody = true;
                        return true; // the m{ token itself is part of the melody
                    }
                    if in_melody {
                        if t.contains('}') {
                            in_melody = false;
                        }
                        return true;
                    }
                    false
                })
                .collect();

            // Mask annotation tokens that carry no rhythmic duration:
            // `dyn <level> [placement]` and `hairpin <dir> <span> [placement]`.
            // Like melody blocks, these must not be counted as chords or receive
            // auto-durations, or they steal beats from the real chords and
            // inflate the measure count (e.g. `dyn mp C#m //.` -> two measures).
            let annotation_mask: Vec<bool> = {
                let mut mask = vec![false; tokens.len()];
                let mut i = 0;
                while i < tokens.len() {
                    let consumed = match tokens[i] {
                        "dyn" | "dynamic" => {
                            // keyword + level + optional placement
                            let mut n = 1;
                            if i + n < tokens.len() {
                                n += 1;
                            }
                            if i + n < tokens.len()
                                && Self::parse_placement(tokens[i + n]).is_some()
                            {
                                n += 1;
                            }
                            n
                        }
                        "hairpin" => {
                            // keyword + direction + span + optional placement
                            let mut n = 1;
                            if i + n < tokens.len() {
                                n += 1;
                            }
                            if i + n < tokens.len() {
                                n += 1;
                            }
                            if i + n < tokens.len()
                                && Self::parse_placement(tokens[i + n]).is_some()
                            {
                                n += 1;
                            }
                            n
                        }
                        _ => 0,
                    };
                    if consumed > 0 {
                        for slot in mask.iter_mut().skip(i).take(consumed) {
                            *slot = true;
                        }
                        i += consumed;
                    } else {
                        i += 1;
                    }
                }
                mask
            };

            // Check if segment has standalone slashes (e.g., "/", "//", "///", "////")
            // If so, don't apply auto-duration - the slashes provide duration info
            let has_standalone_slashes = tokens
                .iter()
                .any(|t| t.chars().all(|c| c == '/') && !t.is_empty());
            let has_chord_length_directive = tokens
                .iter()
                .any(|t| *t == "/ChordLength" || *t == "/duration" || *t == "/Duration");

            let chord_count = tokens
                .iter()
                .enumerate()
                .filter(|(i, t)| {
                    // Skip tokens inside melody blocks or annotation sequences
                    if melody_mask[*i] || annotation_mask[*i] {
                        return false;
                    }
                    // Count as chord if it's not a command, cue, or other special token.
                    // Dot repeats ARE counted - they occupy time in the measure.
                    // `$name` melody-variable recall is NOT a chord — it shouldn't
                    // shrink the chord-duration share for the real chords in the bar.
                    !t.starts_with('/')
                        && !t.starts_with('@')
                        && !t.starts_with('"')
                        && !t.starts_with('$')
                        && **t != "%"
                        && Command::parse_stop_token(t).is_none()
                })
                .count();

            // Check if segment has melody blocks — don't apply auto-duration to melody measures
            // as it would make the chord look like explicit rhythm notation
            let has_melody_block = melody_mask.iter().any(|&m| m);

            // Count chords WITHOUT explicit durations (these need auto-duration)
            // If segment has standalone slashes or melody blocks, skip auto-duration entirely
            let chords_needing_duration =
                if has_standalone_slashes || has_melody_block || has_chord_length_directive {
                    0 // Don't apply auto-duration when slashes or melodies are present
                } else {
                    tokens
                        .iter()
                        .enumerate()
                        .filter(|(i, t)| {
                            // Skip tokens inside melody blocks or annotation sequences
                            if melody_mask[*i] || annotation_mask[*i] {
                                return false;
                            }
                            !t.starts_with('/')
                            && !t.starts_with('@')
                            && !t.starts_with('"')
                            && !t.starts_with('$')
                            && **t != "%"
                            && !t.contains('_')
                            && **t != "." // Dot repeats handle their own duration
                            && Command::parse_stop_token(t).is_none()
                        })
                        .count()
                };

            // Calculate duration per chord
            // If all chords have explicit durations, use whole note as default
            // Otherwise, split the measure evenly among all chords (including those with explicit durations)
            let duration_per_chord = if chord_count > 0 {
                beats_per_measure / chord_count as f64
            } else {
                beats_per_measure
            };

            // Apply auto-duration if there are chords that need it.
            // For multi-chord segments (including the last one), chords should share
            // the measure evenly. A single chord in the last segment takes a full measure.
            let should_apply_auto_duration = chords_needing_duration > 0;

            // Convert beats to LilySyntax duration
            // beats_per_measure = 4 in 4/4, so:
            // 4 beats = Whole (1)
            // 2 beats = Half (2)
            // 1 beat = Quarter (4)
            // 0.5 beats = Eighth (8)
            let lily_duration = if (duration_per_chord - beats_per_measure).abs() < 0.001 {
                "1" // Whole note
            } else if (duration_per_chord - beats_per_measure / 2.0).abs() < 0.001 {
                "2" // Half note
            } else if (duration_per_chord - beats_per_measure / 4.0).abs() < 0.001 {
                "4" // Quarter note
            } else if (duration_per_chord - beats_per_measure / 8.0).abs() < 0.001 {
                "8" // Eighth note
            } else if (duration_per_chord - beats_per_measure / 16.0).abs() < 0.001 {
                "16" // Sixteenth note
            } else {
                // Default to whole note if we can't match exactly
                "1"
            };

            // Rebuild segment with durations
            let mut segment_result = String::new();
            for (tok_idx, token) in tokens.iter().enumerate() {
                if !segment_result.is_empty() {
                    segment_result.push(' ');
                }

                // Keep melody-block and annotation tokens as-is (no duration mangling)
                if melody_mask[tok_idx] || annotation_mask[tok_idx] {
                    segment_result.push_str(token);
                    continue;
                }

                // Check if this is a chord (not a command, cue, dot repeat, stop token,
                // or `$name` melody-variable recall — those must round-trip unchanged).
                let is_dot_repeat = *token == ".";
                let is_measure_repeat = *token == "%";
                let is_stop_token = Command::parse_stop_token(token).is_some();
                if !token.starts_with('/')
                    && !token.starts_with('@')
                    && !token.starts_with('"')
                    && !token.starts_with('$')
                    && !is_dot_repeat
                    && !is_measure_repeat
                    && !is_stop_token
                {
                    // Check if token already has a duration
                    if token.contains('_') {
                        // Already has duration, keep as is
                        segment_result.push_str(token);
                    } else if should_apply_auto_duration {
                        // Add automatic duration
                        segment_result.push_str(token);
                        segment_result.push('_');
                        segment_result.push_str(lily_duration);
                    } else {
                        // No chords need duration, keep as is
                        segment_result.push_str(token);
                    }
                } else {
                    // Keep non-chord tokens (commands, cues, dot repeats) as is
                    segment_result.push_str(token);
                }
            }

            if !result.is_empty() && !segment_result.is_empty() {
                result.push(' ');
            }
            result.push_str(&segment_result);

            // Add separator if not last segment
            if i < segments.len() - 1 {
                result.push(' ');
                result.push('|');
            }
        }

        result
    }

    /// Extract repeat syntax from the end of a line
    /// Examples: "6 5 4 4 x4" -> ("6 5 4 4", 4)
    ///           "g c d" -> ("g c d", 1)
    pub(super) fn extract_repeat_syntax(line: &str) -> (&str, RepeatCount) {
        // Look for pattern like "x4" or "x^" at the end
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.is_empty() {
            return (line, RepeatCount::Fixed(1));
        }

        let last_token = tokens[tokens.len() - 1];

        // Check if last token matches x^ pattern (auto-repeat)
        if last_token == "x^" || last_token == "X^" {
            // Find where the last token starts in the original line
            if let Some(pos) = line.rfind(last_token) {
                let line_without_repeat = line[..pos].trim();
                return (line_without_repeat, RepeatCount::Auto);
            }
        }

        // Check if last token matches xN pattern (case insensitive)
        if (last_token.starts_with('x') || last_token.starts_with('X')) && last_token.len() > 1 {
            if let Ok(count) = last_token[1..].parse::<usize>() {
                if count > 0 {
                    // Find where the last token starts in the original line
                    if let Some(pos) = line.rfind(last_token) {
                        let line_without_repeat = line[..pos].trim();
                        return (line_without_repeat, RepeatCount::Fixed(count));
                    }
                }
            }
        }

        (line, RepeatCount::Fixed(1))
    }

    /// Strip auto-assigned duration suffix from a token.
    ///
    /// The auto-duration system adds suffixes like `_1`, `_2`, `_4`, `_8`, `_16`
    /// to tokens. This function removes those suffixes so we can tell if the
    /// user explicitly specified a duration or if it was auto-assigned.
    pub(super) fn strip_auto_duration_suffix(token: &str) -> String {
        // Auto-duration patterns: _1, _2, _4, _8, _16
        // These are whole, half, quarter, eighth, sixteenth notes
        for suffix in &["_16", "_8", "_4", "_2", "_1"] {
            if let Some(stripped) = token.strip_suffix(suffix) {
                return stripped.to_string();
            }
        }
        token.to_string()
    }

    /// Extract the root portion from the original token (before quality info and rhythm notation)
    /// Examples: "cmaj7" -> "C", "Dm7" -> "D", "g#m" -> "G#", "I" -> "I", "vi" -> "vi", "1" -> "1"
    /// Also handles rhythm notation: "e//" -> "E", "g_4" -> "G", "1///" -> "1"
    pub(super) fn extract_root_from_token(token: &str) -> String {
        if token.is_empty() {
            return token.to_string();
        }

        // First, strip any rhythm notation (slashes, underscores, etc.)
        // Find the first occurrence of /, _, or ' and take everything before it
        let chord_part = if let Some(slash_pos) = token.find('/') {
            &token[..slash_pos]
        } else if let Some(underscore_pos) = token.find('_') {
            &token[..underscore_pos]
        } else if let Some(apostrophe_pos) = token.find('\'') {
            &token[..apostrophe_pos]
        } else {
            token
        };

        if chord_part.is_empty() {
            return chord_part.to_string();
        }

        let mut chars = chord_part.chars();
        let first = chars.next().unwrap();

        // If it's a digit (scale degree 1-7), return just the digit
        if first.is_ascii_digit() {
            return first.to_string();
        }

        // If it's a Roman numeral (I-VII), extract the Roman numeral part (preserve case!)
        if first == 'I' || first == 'V' || first == 'i' || first == 'v' {
            let roman_part: String = chord_part
                .chars()
                .take_while(|c| *c == 'I' || *c == 'V' || *c == 'i' || *c == 'v')
                .collect();
            return roman_part;
        }

        // Otherwise, it's a note name - capitalize it and include accidental if present
        let first_upper = first.to_uppercase().to_string();
        let second = chars.next();
        if let Some(c) = second {
            if c == 'b' || c == '#' {
                return format!("{}{}", first_upper, c);
            }
        }

        first_upper
    }
}

// endregion: --- Token Helpers

// region:    --- Chord Line Parsing

impl<'a> ChartParser<'a> {
    /// Parse section content lines into measures
    pub(super) fn parse_section_measures(
        &mut self,
        lines: &[&str],
        section_type: &SectionType,
        section_measure_count: Option<usize>,
    ) -> Result<Vec<Measure>, String> {
        let mut measures: Vec<Measure> = Vec::new();
        let mut pending_cues: Vec<TextCue> = Vec::new();
        let mut pending_dynamics: Vec<DynamicMarking> = Vec::new();
        let mut pending_classical_dynamics: Vec<Dynamic> = Vec::new();
        let mut pending_hairpins: Vec<Hairpin> = Vec::new();
        let mut pending_staff_text: Vec<StaffText> = Vec::new();

        // Seed each section from the chart-wide `/Duration` default (if any).
        // A `/Duration` inside the section overrides these below.
        let mut section_chord_length: Option<(ChordRhythm, MusicalDuration)> =
            self.default_duration.as_ref().and_then(|v| {
                Self::parse_chord_length_value(
                    v,
                    self.time_signature.unwrap_or(TimeSignature::common_time()),
                )
            });
        let mut section_melody_duration: Option<String> = self.default_duration.clone();
        let mut section_melody_octave: Option<u8> = self.melody_octave_memory;
        let logical_lines = Self::join_multiline_parallel_containers(lines);

        for line in &logical_lines {
            let line = line.as_str();
            let trimmed = line.trim();

            if let Some((name, value)) = Self::parse_alias_declaration(trimmed) {
                self.aliases.insert(name, value);
                continue;
            }

            if let Some(value) = trimmed
                .strip_prefix("/duration ")
                .or_else(|| trimmed.strip_prefix("/Duration "))
            {
                let value = value.trim();
                section_chord_length = Self::parse_chord_length_value(
                    value,
                    self.time_signature.unwrap_or(TimeSignature::common_time()),
                );
                section_melody_duration = Some(value.to_string());
                continue;
            }

            if let Some(value) = trimmed
                .strip_prefix("/octave ")
                .or_else(|| trimmed.strip_prefix("/Octave "))
            {
                let value = value.trim();
                if let Some((octave, melody_block)) = value.split_once(char::is_whitespace) {
                    if melody_block.trim_start().starts_with("m{")
                        || melody_block.trim_start().starts_with("m {")
                    {
                        let inline_octave = octave.parse::<u8>().ok().or(section_melody_octave);
                        match Melody::parse_block_with_defaults(
                            melody_block.trim(),
                            section_melody_duration.as_deref(),
                            inline_octave,
                        ) {
                            Ok((name, melody)) => {
                                if let Some(var_name) = name {
                                    self.melody_variables.set(var_name, melody);
                                } else {
                                    if let Some(octave) = Self::last_melody_context_octave(&melody)
                                    {
                                        section_melody_octave = Some(octave);
                                    }
                                    let time_sig =
                                        self.time_signature.unwrap_or(TimeSignature::common_time());
                                    let mut measure = Self::fresh_measure(time_sig);
                                    measure.melodies.push(melody);
                                    let mut melody_measures =
                                        self.split_long_melody_branch_measures(vec![measure]);

                                    if !pending_cues.is_empty() && !melody_measures.is_empty() {
                                        melody_measures[0].text_cues.append(&mut pending_cues);
                                    }
                                    if !pending_dynamics.is_empty() && !melody_measures.is_empty() {
                                        melody_measures[0].dynamics.append(&mut pending_dynamics);
                                    }
                                    if !pending_staff_text.is_empty() && !melody_measures.is_empty()
                                    {
                                        melody_measures[0]
                                            .staff_text
                                            .append(&mut pending_staff_text);
                                    }
                                    if !pending_classical_dynamics.is_empty()
                                        && !melody_measures.is_empty()
                                    {
                                        melody_measures[0]
                                            .classical_dynamics
                                            .append(&mut pending_classical_dynamics);
                                    }
                                    if !pending_hairpins.is_empty() && !melody_measures.is_empty() {
                                        melody_measures[0].hairpins.append(&mut pending_hairpins);
                                    }

                                    measures.extend(melody_measures);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to parse melody block '{}': {}",
                                    melody_block.trim(),
                                    e
                                );
                            }
                        }
                        continue;
                    }
                }
                section_melody_octave = value.parse::<u8>().ok();
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("/ChordLength ") {
                section_chord_length = Self::parse_chord_length_value(
                    value.trim(),
                    self.time_signature.unwrap_or(TimeSignature::common_time()),
                );
                continue;
            }

            if let Some(staff_text) = self.parse_standalone_staff_text_line(trimmed) {
                pending_staff_text.extend(staff_text);
                continue;
            }

            if let Some(dynamic) = Self::parse_classical_dynamic_line(trimmed) {
                pending_classical_dynamics.push(dynamic);
                continue;
            }

            if let Some(hairpin) = Self::parse_hairpin_line(trimmed) {
                pending_hairpins.push(hairpin);
                continue;
            }

            // Check for melody variable definition (e.g., "mainRiff = m{ C_8 D_8 E_4 }")
            if trimmed.contains("= m{")
                || trimmed.contains("= m {")
                || trimmed.starts_with("m{")
                || trimmed.starts_with("m {")
            {
                match Melody::parse_block_with_defaults(
                    trimmed,
                    section_melody_duration.as_deref(),
                    section_melody_octave,
                ) {
                    Ok((name, melody)) => {
                        if let Some(var_name) = name {
                            // Store as a variable
                            self.melody_variables.set(var_name, melody);
                        } else {
                            if let Some(octave) = Self::last_melody_context_octave(&melody) {
                                section_melody_octave = Some(octave);
                            }
                            let time_sig =
                                self.time_signature.unwrap_or(TimeSignature::common_time());
                            let mut measure = Self::fresh_measure(time_sig);
                            measure.melodies.push(melody);
                            let mut melody_measures =
                                self.split_long_melody_branch_measures(vec![measure]);

                            if !pending_cues.is_empty() && !melody_measures.is_empty() {
                                melody_measures[0].text_cues.append(&mut pending_cues);
                            }
                            if !pending_dynamics.is_empty() && !melody_measures.is_empty() {
                                melody_measures[0].dynamics.append(&mut pending_dynamics);
                            }
                            if !pending_staff_text.is_empty() && !melody_measures.is_empty() {
                                melody_measures[0]
                                    .staff_text
                                    .append(&mut pending_staff_text);
                            }
                            if !pending_classical_dynamics.is_empty() && !melody_measures.is_empty()
                            {
                                melody_measures[0]
                                    .classical_dynamics
                                    .append(&mut pending_classical_dynamics);
                            }
                            if !pending_hairpins.is_empty() && !melody_measures.is_empty() {
                                melody_measures[0].hairpins.append(&mut pending_hairpins);
                            }

                            measures.extend(melody_measures);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse melody block '{}': {}", trimmed, e);
                    }
                }
                continue;
            }

            // Check if this is a text cue line
            if trimmed.starts_with('@') {
                // Parse text cue - always keep it pending for the next chord line
                match TextCue::parse(line) {
                    Ok(cue) => {
                        pending_cues.push(cue);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse text cue '{}': {}", line, e);
                    }
                }
            } else if trimmed.starts_with('<')
                && !trimmed.starts_with("<<")
                && trimmed.contains('>')
                && !tokenize_with_spans(trimmed)
                    .first()
                    .and_then(|span| {
                        trimmed[span.as_range()]
                            .strip_prefix('<')
                            .and_then(|token| token.strip_suffix('>'))
                    })
                    .map(|name| self.aliases.contains_key(name))
                    .unwrap_or(false)
                && !self
                    .expand_aliases_in_line(trimmed)
                    .trim_start()
                    .starts_with("<<")
            {
                // Check if this is a standalone dynamic marking line
                // Could be just "<Build>" or "<Build>:3" on its own line
                match DynamicMarking::parse(trimmed) {
                    Ok(dynamic) => {
                        pending_dynamics.push(dynamic);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse dynamic marking '{}': {}", line, e);
                    }
                }
            } else {
                // Parse chords from line
                // Pass section measure_count for x^ calculation
                let mut line_measures = self.parse_chord_line_with_default_chord_length(
                    line,
                    section_type,
                    section_measure_count,
                    section_chord_length.clone(),
                    section_melody_duration.clone(),
                    section_melody_octave,
                )?;

                // If we have pending cues, attach them to the first new measure
                if !pending_cues.is_empty() && !line_measures.is_empty() {
                    line_measures[0].text_cues.append(&mut pending_cues);
                }

                // If we have pending dynamics, attach them to the first new measure
                if !pending_dynamics.is_empty() && !line_measures.is_empty() {
                    line_measures[0].dynamics.append(&mut pending_dynamics);
                }
                if !pending_staff_text.is_empty() && !line_measures.is_empty() {
                    line_measures[0].staff_text.append(&mut pending_staff_text);
                }
                if !pending_classical_dynamics.is_empty() && !line_measures.is_empty() {
                    line_measures[0]
                        .classical_dynamics
                        .append(&mut pending_classical_dynamics);
                }
                if !pending_hairpins.is_empty() && !line_measures.is_empty() {
                    line_measures[0].hairpins.append(&mut pending_hairpins);
                }

                measures.extend(line_measures);
            }
        }

        self.melody_octave_memory = section_melody_octave;
        Ok(measures)
    }

    fn last_melody_context_octave(melody: &Melody) -> Option<u8> {
        melody
            .notes
            .iter()
            .rev()
            .find(|note| !note.is_rest() && !note.is_space())
            .and_then(Self::lowest_octave_in_note)
    }

    fn lowest_octave_in_note(note: &MelodyNote) -> Option<u8> {
        let mut lowest = note.octave;
        for (_, octave) in &note.extra_pitches {
            if let Some(octave) = octave {
                lowest = Some(lowest.map_or(*octave, |current| current.min(*octave)));
            }
        }
        lowest
    }

    /// Parse a line of chords into measures.
    ///
    /// Spans recorded for each chord token are byte-relative to `line` (with
    /// line/column = 0). When a parser is processing a sub-slice of a larger
    /// source line (parallel containers, repeats), use
    /// [`Self::parse_chord_line_with_offset`] to shift those spans into the
    /// original-line coordinate system.
    ///
    /// Test-only: production parses through
    /// `parse_chord_line_with_default_chord_length`. Kept as a convenience
    /// entry point for the parser unit tests.
    #[cfg(test)]
    pub(super) fn parse_chord_line(
        &mut self,
        line: &str,
        section_type: &SectionType,
        section_measure_count: Option<usize>,
    ) -> Result<Vec<Measure>, String> {
        self.parse_chord_line_with_offset(line, section_type, section_measure_count, 0)
    }

    pub(super) fn parse_chord_line_with_default_chord_length(
        &mut self,
        line: &str,
        section_type: &SectionType,
        section_measure_count: Option<usize>,
        default_chord_length: Option<(ChordRhythm, MusicalDuration)>,
        default_melody_duration: Option<String>,
        default_melody_octave: Option<u8>,
    ) -> Result<Vec<Measure>, String> {
        self.parse_chord_line_inner(
            line,
            section_type,
            section_measure_count,
            0,
            default_chord_length,
            default_melody_duration,
            default_melody_octave,
        )
    }

    /// Same as [`Self::parse_chord_line`] but shifts every emitted token span
    /// by `line_byte_offset` so spans line up with the surrounding source.
    ///
    /// Test-only: only reached via [`Self::parse_chord_line`].
    #[cfg(test)]
    #[allow(clippy::too_many_lines)]
    pub(super) fn parse_chord_line_with_offset(
        &mut self,
        line: &str,
        section_type: &SectionType,
        section_measure_count: Option<usize>,
        line_byte_offset: usize,
    ) -> Result<Vec<Measure>, String> {
        self.parse_chord_line_inner(
            line,
            section_type,
            section_measure_count,
            line_byte_offset,
            None,
            None,
            None,
        )
    }

    fn parse_chord_line_inner(
        &mut self,
        line: &str,
        section_type: &SectionType,
        section_measure_count: Option<usize>,
        line_byte_offset: usize,
        default_chord_length: Option<(ChordRhythm, MusicalDuration)>,
        mut default_melody_duration: Option<String>,
        default_melody_octave: Option<u8>,
    ) -> Result<Vec<Measure>, String> {
        use crate::chart::types::TempoChange;
        use keyflow_proto::chart::commands::Command;

        let mut time_sig = self.time_signature.unwrap_or(TimeSignature::common_time());
        let mut beats_per_measure = time_sig.numerator as f64;

        // Check for repeat syntax at the end of the line (e.g., "6 5 4 4 x4")
        let (line_to_parse, repeat_count) = Self::extract_repeat_syntax(line);

        // Byte offset of `line_to_parse` within the original `line` (always 0
        // unless `extract_repeat_syntax` trimmed a prefix). Combined with the
        // outer offset, this lets every emitted token span be expressed in the
        // original document's byte coordinates.
        let trim_offset = (line_to_parse.as_ptr() as usize).saturating_sub(line.as_ptr() as usize);
        let outer_offset = line_byte_offset + trim_offset;

        let line_to_parse = self.expand_aliases_in_line(line_to_parse);

        let trimmed_parallel = line_to_parse.trim();
        if (trimmed_parallel.starts_with("<<") && trimmed_parallel.ends_with(">>"))
            || (line_to_parse.contains("<<")
                && line_to_parse.contains(">>")
                && !line_to_parse.contains("|:")
                && !line_to_parse.contains(":|"))
        {
            return self.parse_parallel_chord_line(
                &line_to_parse,
                repeat_count,
                section_type,
                section_measure_count,
                outer_offset,
                default_chord_length,
                default_melody_duration,
                default_melody_octave,
            );
        }

        if default_chord_length.is_some() && line_to_parse.contains('|') {
            return self.parse_barred_line_with_default_chord_length(
                &line_to_parse,
                section_type,
                section_measure_count,
                outer_offset,
                default_chord_length,
                default_melody_duration,
                default_melody_octave,
            );
        }

        let line_to_parse = Self::normalize_parallel_container_syntax(&line_to_parse);
        let line_to_parse = line_to_parse
            .replace("|:", "| @repeat-start ")
            .replace(":|", " @repeat-end |")
            .replace("m {", "m{");

        // Preprocess: Expand `()` rhythm groups into explicit per-chord lily
        // durations (e.g. `(C G)` → `C_2 G_2`, `(D Em G)` → `D_2t Em_2t G_2t`).
        // Identity for lines with no group, so spans/tokens are unchanged there.
        let line_to_parse = expand_chord_groups(&line_to_parse, time_sig)?;

        // Preprocess: Calculate automatic durations for chords between measure separators
        // If chords are between | separators, split the measure evenly
        // e.g., "| G C |" → "G_2 C_2", "G C | D" → "G_2 C_2 D_1"
        let line_with_auto_durations =
            Self::apply_auto_durations_between_separators(&line_to_parse, beats_per_measure);

        let line_with_auto_token_spans = tokenize_with_spans(&line_with_auto_durations);
        let tokens_str: Vec<&str> = line_with_auto_token_spans
            .iter()
            .map(|span| &line_with_auto_durations[span.as_range()])
            .collect();

        // Byte spans of each whitespace-separated token in the *pre-auto-duration* line.
        // `apply_auto_durations_between_separators` only appends `_N` suffixes to existing
        // tokens (it does not add, remove, or reorder them), so token-index 1:1 mapping
        // back to `line_to_parse` is safe for the non-parallel path. Spans are
        // line-relative; line/column resolution to the original document is the caller's
        // responsibility.
        let original_token_spans: Vec<TextSpan> = tokenize_with_spans(&line_to_parse)
            .into_iter()
            .map(|s| TextSpan::new(s.start + outer_offset, s.len))
            .collect();
        let mut measures: Vec<Measure> = Vec::new();
        let mut current_measure = Self::fresh_measure(time_sig);
        let mut current_measure_beats: f64 = 0.0;
        let mut pending_cue: Option<TextCue> = None;
        let mut pending_dynamic: Option<DynamicMarking> = None;
        let mut pending_stop: Option<Command> = None;
        let mut just_processed_separator = false; // Track if we just processed a | separator
        let mut measure_was_created_by_separator = false; // Track if current measure was created by |
        let mut in_melody_block = false; // Track if we're inside a m{...} block
        let mut melody_search_offset = 0usize; // Cursor for finding successive m{ blocks in line_to_parse
        let mut line_melody_octave = default_melody_octave;
        let mut last_chord: Option<ChordInstance> = None; // Track last chord for dot repeat
        let mut measure_has_slash_rhythm = false; // Track if current measure has chords with slash rhythm
        let mut last_token_was_slash = false; // Track for slash accumulation (consecutive slashes accumulate)
        let mut pending_start_repeat = false;
        let mut chord_length_override: Option<(ChordRhythm, MusicalDuration)> =
            default_chord_length;
        let mut skip_next_token = false;
        // Number of upcoming tokens to skip — used by multi-token annotations
        // like `dyn <level> [above]` and `hairpin <dir> <span> [above]` that
        // are consumed in one place but span several tokens.
        let mut skip_tokens: usize = 0;
        // One-shot meter change (`!T2/4`): apply the new meter to exactly the
        // next measure, then revert to the prevailing meter. `oneshot_revert`
        // holds the (time_sig, beats_per_measure) to restore; the revert fires
        // once `measures.len()` passes `oneshot_measure_idx` — i.e. the moment
        // the one-shot measure has been pushed.
        let mut oneshot_revert: Option<(TimeSignature, f64)> = None;
        let mut oneshot_measure_idx: usize = 0;

        // Notation system for this line, used to disambiguate `b<digit>` roots
        // (note B vs flat degree). Line wins, then the chart parsed so far.
        let chord_system = self.resolve_notation_system(&tokens_str);
        // Contribute this line's decisive tokens to the chart-wide tally so
        // later ambiguous lines can fall back to it.
        self.record_line_system_votes(&tokens_str);

        for (token_idx, token_str) in tokens_str.iter().enumerate() {
            if skip_tokens > 0 {
                skip_tokens -= 1;
                continue;
            }
            if skip_next_token {
                skip_next_token = false;
                continue;
            }

            // Once a one-shot meter measure has been pushed, restore the
            // prevailing meter. The freshly started measure was stamped with the
            // one-shot meter when it was created; correct it (it has no content
            // yet) so following chords land in the reverted time signature.
            if let Some((revert_ts, revert_bpm)) = oneshot_revert {
                if measures.len() > oneshot_measure_idx {
                    time_sig = revert_ts;
                    beats_per_measure = revert_bpm;
                    current_measure.time_signature =
                        (time_sig.numerator as u8, time_sig.denominator as u8);
                    oneshot_revert = None;
                }
            }

            // Floating suspension figure (`4-3`, `2-3`, `3`, `2`): the prior
            // chord keeps sounding; record the figure at the current beat and
            // let following slash groups extend the held chord. Consumes no
            // beats of its own and never counts as rhythm.
            if let Some(figure) = Self::parse_bare_suspension_figure(token_str) {
                let slash_follows = tokens_str.get(token_idx + 1).is_some_and(|t| {
                    !t.is_empty() && t.trim_end_matches('.').chars().all(|c| c == '/')
                });

                if !current_measure.chords.is_empty() {
                    // Case A — the current bar is still open (the held chord
                    // carried explicit slashes, e.g. `Bb // 4-3 //`). The figure
                    // continues that chord in the same bar; a following slash
                    // group extends it (treated as a continuation).
                    let beat_anchor = current_measure_beats
                        .min((beats_per_measure - 1.0).max(0.0))
                        .max(0.0);
                    Self::push_suspension_at_current_beat(
                        &mut current_measure,
                        &figure,
                        Placement::Above,
                        beat_anchor,
                        true,
                    );
                    last_token_was_slash = true;
                    continue;
                }

                if let Some(prev) = measures.last().and_then(|m| m.chords.last()).cloned() {
                    // Case B — the prior bare chord already filled its bar
                    // (`F 4-3 ///`). The figure opens a NEW bar that restates the
                    // same chord carrying the figure. The following slash group
                    // sets how many beats the figure holds (`4-3 ///` = 3 beats),
                    // and the rest of the bar is filled by what comes after (so
                    // `... /D` lands on beat 4). With no slash, the figure fills
                    // the whole bar on its own.
                    let mut bar_chord = prev;
                    bar_chord.commands.clear();
                    bar_chord.push_pull = None;
                    // The figure itself is the visible symbol for this bar (its
                    // own `4-3`); hide the restated chord's name so the held
                    // harmony stays for playback without printing `F` over it.
                    bar_chord.display_override = Some(String::new());
                    let figure_beats = if slash_follows {
                        let max = beats_per_measure as usize;
                        tokens_str
                            .get(token_idx + 1)
                            .map(|t| t.chars().filter(|c| *c == '/').count())
                            .unwrap_or(1)
                            .clamp(1, max) as u8
                    } else {
                        beats_per_measure as u8
                    };
                    bar_chord.rhythm = ChordRhythm::slashes(figure_beats);
                    bar_chord.duration =
                        MusicalDuration::from_beats(f64::from(figure_beats), time_sig);
                    current_measure
                        .rhythm_elements
                        .push(RhythmElement::Chord(bar_chord.clone()));
                    current_measure.chords.push(bar_chord);
                    Self::push_suspension_at_current_beat(
                        &mut current_measure,
                        &figure,
                        Placement::Above,
                        0.0,
                        true,
                    );
                    current_measure_beats = f64::from(figure_beats);
                    // The slash group's beats are now owned by the figure chord;
                    // consume that token so it isn't re-applied.
                    if slash_follows {
                        skip_next_token = true;
                    }
                    measure_has_slash_rhythm = true;
                    last_token_was_slash = false;
                    if (current_measure_beats - beats_per_measure).abs() < 0.001 {
                        measures.push(current_measure.clone());
                        current_measure = Self::fresh_measure(time_sig);
                        current_measure_beats = 0.0;
                        measure_has_slash_rhythm = false;
                    }
                    continue;
                }

                // Degenerate: figure with no preceding chord. Record it and move
                // on rather than dropping it.
                Self::push_suspension_at_current_beat(
                    &mut current_measure,
                    &figure,
                    Placement::Above,
                    0.0,
                    true,
                );
                continue;
            }

            // Floating slash-bass (`/D`): inherit the previous chord's root as
            // `<root>/D` for harmony, but display the token verbatim. Rewrite
            // to the synthetic chord token here and route it through normal
            // chord parsing below; the original `/D` becomes the display text.
            let synth_slash_bass: Option<String> = Self::parse_floating_slash_bass(token_str)
                .zip(Self::previous_chord_root(&current_measure, &measures))
                .map(|(bass, root)| format!("{root}/{bass}"));
            let (effective_token, display_override): (&str, Option<String>) =
                match &synth_slash_bass {
                    Some(synth) => (synth.as_str(), Some((*token_str).to_string())),
                    None => (token_str, None),
                };

            // Inline classical dynamic / hairpin: zero-duration annotations that
            // must NOT consume measure beats. Without this they fall through to
            // chord parsing and inflate the measure count (e.g. `dyn mp C#m //.`
            // would parse as two measures). Mirrors the standalone-line forms
            // `parse_classical_dynamic_line` / `parse_hairpin_line`.
            if *token_str == "dyn" || *token_str == "dynamic" {
                if let Some(level_tok) = tokens_str.get(token_idx + 1) {
                    let default_beat = current_measure_beats.floor().max(0.0) as u8 + 1;
                    let (level_token, beat) = level_tok
                        .split_once('@')
                        .map(|(level, beat)| (level, beat.parse::<u8>().ok().unwrap_or(1)))
                        .unwrap_or((*level_tok, default_beat));
                    if let Some(level) = Self::parse_dynamic_level(level_token) {
                        skip_tokens = 1;
                        let placement = tokens_str
                            .get(token_idx + 2)
                            .and_then(|t| Self::parse_placement(t));
                        if placement.is_some() {
                            skip_tokens = 2;
                        }
                        current_measure.classical_dynamics.push(Dynamic {
                            level,
                            beat,
                            placement: placement.unwrap_or(Placement::Below),
                        });
                    }
                }
                continue;
            }

            if *token_str == "hairpin" {
                if let (Some(kind_tok), Some(span_tok)) =
                    (tokens_str.get(token_idx + 1), tokens_str.get(token_idx + 2))
                {
                    let kind = match *kind_tok {
                        "<" | "crescendo" | "cresc" => Some(HairpinKind::Crescendo),
                        ">" | "decrescendo" | "decresc" | "dim" | "diminuendo" => {
                            Some(HairpinKind::Decrescendo)
                        }
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        let (start, end) = span_tok
                            .split_once("..")
                            .or_else(|| span_tok.split_once('-'))
                            .unwrap_or(("1", "1"));
                        let start_beat = start
                            .trim_start_matches('@')
                            .parse::<u8>()
                            .ok()
                            .unwrap_or(1);
                        let end_beat = end
                            .trim_start_matches('@')
                            .parse::<u8>()
                            .ok()
                            .unwrap_or(start_beat);
                        skip_tokens = 2;
                        let placement = tokens_str
                            .get(token_idx + 3)
                            .and_then(|t| Self::parse_placement(t));
                        if placement.is_some() {
                            skip_tokens = 3;
                        }
                        current_measure.hairpins.push(Hairpin {
                            kind,
                            start_beat,
                            end_measure_offset: 0,
                            end_beat,
                            placement: placement.unwrap_or(Placement::Below),
                        });
                    }
                }
                continue;
            }

            if *token_str == "/octave" {
                if let Some(next) = tokens_str.get(token_idx + 1) {
                    line_melody_octave = next.parse::<u8>().ok().or(line_melody_octave);
                    skip_next_token = true;
                }
                continue;
            }

            if *token_str == "/ChordLength"
                || *token_str == "/duration"
                || *token_str == "/Duration"
            {
                if let Some(next) = tokens_str.get(token_idx + 1) {
                    chord_length_override = Self::parse_chord_length_value(next, time_sig);
                    if *token_str == "/duration" || *token_str == "/Duration" {
                        default_melody_duration = Some(next.to_string());
                    }
                    skip_next_token = true;
                }
                continue;
            }

            if *token_str == "%" {
                if !current_measure.chords.is_empty()
                    || !current_measure.rhythm_elements.is_empty()
                    || !current_measure.figured_bass.is_empty()
                    || !current_measure.staff_text.is_empty()
                    || !current_measure.text_cues.is_empty()
                {
                    measures.push(current_measure.clone());
                    current_measure = Self::fresh_measure(time_sig);
                    current_measure_beats = 0.0;
                    measure_has_slash_rhythm = false;
                }

                if let Some(previous_measure) = measures.last().cloned() {
                    measures.push(previous_measure);
                }

                just_processed_separator = false;
                measure_was_created_by_separator = false;
                continue;
            }

            if let Some((text, placement)) = Self::parse_quoted_text_token(token_str) {
                let beat = current_measure_beats.floor().max(0.0) as u8 + 1;
                let target_measure = if current_measure.chords.is_empty()
                    && current_measure.rhythm_elements.is_empty()
                    && current_measure.figured_bass.is_empty()
                    && current_measure.staff_text.is_empty()
                {
                    measures.last_mut().unwrap_or(&mut current_measure)
                } else {
                    &mut current_measure
                };
                target_measure.staff_text.push(StaffText {
                    text,
                    beat,
                    placement,
                    source_default_x: None,
                    boxed: false,
                    bold: false,
                    italic: false,
                });
                continue;
            }

            let token_for_barline = token_str
                .split_once('_')
                .map_or(*token_str, |(base, _)| base);
            if token_for_barline.starts_with("|{") && token_for_barline.ends_with('}') {
                Self::finalize_measure_for_separator(
                    &mut measures,
                    &current_measure,
                    measure_was_created_by_separator,
                );
                current_measure = Self::fresh_measure(time_sig);
                current_measure_beats = 0.0;
                just_processed_separator = true;
                measure_was_created_by_separator = true;
                measure_has_slash_rhythm = false;
                continue;
            }

            // Check for command (e.g., "/fermata", "/accent")
            // Commands are applied to the PREVIOUS chord
            if token_str.starts_with('/') && display_override.is_none() {
                if *token_str == "/octave" {
                    skip_next_token = true;
                    continue;
                }

                if let Some(cmd) = Command::parse_slash(token_str) {
                    // Apply command to the last chord in rhythm_elements (source of truth)
                    // and also to chords for backward compatibility during parsing
                    let applied = if let Some(RhythmElement::Chord(c)) =
                        current_measure.rhythm_elements.last_mut()
                    {
                        c.commands.push(cmd.clone());
                        true
                    } else {
                        false
                    };

                    // Also apply to chords vec for backward compatibility
                    if let Some(last_chord) = current_measure.chords.last_mut() {
                        last_chord.commands.push(cmd.clone());
                    }

                    // If current measure has no chord, try previous measure
                    if !applied && !measures.is_empty() {
                        if let Some(last_measure) = measures.last_mut() {
                            if let Some(RhythmElement::Chord(c)) =
                                last_measure.rhythm_elements.last_mut()
                            {
                                c.commands.push(cmd.clone());
                            }
                            if let Some(last_chord) = last_measure.chords.last_mut() {
                                last_chord.commands.push(cmd);
                            }
                        }
                    }
                    continue;
                }

                // Check for standalone slash duration notation (e.g., "//", "///", "////")
                // This allows syntax like "Ab9' //" where the slashes are separated by a space
                //
                // NEW BEHAVIOR: Single "/" between chords represents a continuation beat,
                // not a duration modifier. This allows "Cm/Eb / 'Eb //" to work correctly:
                // - Cm/Eb takes 1 beat
                // - / adds 1 beat of continuation (beat 2)
                // - 'Eb is pushed, creating triplet on beat 2
                // - // adds 2 more beats (beats 3-4)
                //
                // DOTTED SLASHES: /. = 1.5 beats, //. = 2.5 beats, etc.
                // The dot adds 50% to the slash duration.

                // First check for dotted slash notation (e.g., "/.", "//.", "///.")
                let is_dotted_slash = token_str.ends_with('.')
                    && token_str.len() >= 2
                    && token_str[..token_str.len() - 1].chars().all(|c| c == '/');

                if is_dotted_slash {
                    let slash_count = token_str.len() - 1; // Exclude the dot
                    if slash_count > 0 {
                        // Dotted slashes: each slash is 1 beat, dot adds 50%
                        // /. = 1.5 beats (dotted quarter)
                        // //. = 2.5 beats (half + dotted quarter... approximated as 2.5)
                        // Actually: interpret as (slashes * 1 beat) * 1.5 for the dotted effect
                        // So /. = 1 * 1.5 = 1.5 beats
                        // //. = 2 * 1.5 = 3.0 beats? Or is it 2 + 0.5 = 2.5?
                        // Let's use: /. = dotted quarter (1.5), //. = dotted half (3.0)
                        let dotted_duration = slash_count as f64 * dotted_slash_beats(time_sig);

                        // Convert to Lily rhythm for proper representation
                        let _lily_duration = match slash_count {
                            1 => LilySyntax::Quarter,   // /. = dotted quarter
                            2 => LilySyntax::Half,      // //. = dotted half
                            3 | 4 => LilySyntax::Whole, // ///. or ////. = dotted whole
                            _ => LilySyntax::Quarter,
                        };

                        // Use dotted slash notation
                        let dotted_slash_rhythm = ChordRhythm::Slashes {
                            count: slash_count as u8,
                            dotted: true,
                            tied: false,
                        };

                        if last_token_was_slash && !current_measure.chords.is_empty() {
                            let space_duration =
                                MusicalDuration::from_beats(dotted_duration, time_sig);
                            let space = SpaceInstance::new(
                                dotted_slash_rhythm.clone(),
                                space_duration,
                                AbsolutePosition::new(
                                    MusicalPosition::try_new(
                                        measures.len() as i32,
                                        current_measure_beats as i32,
                                        0,
                                    )
                                    .unwrap_or_else(|_| MusicalPosition::start()),
                                    self.sections.len(),
                                ),
                                token_str.to_string(),
                            );
                            current_measure
                                .rhythm_elements
                                .push(RhythmElement::Space(space));
                            current_measure_beats += dotted_duration;
                            measure_has_slash_rhythm = true;

                            if !just_processed_separator
                                && (current_measure_beats - beats_per_measure).abs() < 0.001
                            {
                                measures.push(current_measure.clone());
                                current_measure = Self::fresh_measure(time_sig);
                                current_measure_beats = 0.0;
                                measure_has_slash_rhythm = false;
                            }

                            last_token_was_slash = true;
                            continue;
                        }

                        let applied = if let Some(last_chord) = current_measure.chords.last_mut() {
                            last_chord.rhythm = dotted_slash_rhythm.clone();
                            last_chord.duration =
                                MusicalDuration::from_beats(dotted_duration, time_sig);

                            // Also update the chord in rhythm_elements
                            for elem in current_measure.rhythm_elements.iter_mut().rev() {
                                if let RhythmElement::Chord(c) = elem {
                                    c.rhythm = dotted_slash_rhythm.clone();
                                    c.duration =
                                        MusicalDuration::from_beats(dotted_duration, time_sig);
                                    break;
                                }
                            }
                            true
                        } else if !measures.is_empty() {
                            // Current measure is empty - apply to previous measure's last chord
                            // This happens when the chord auto-completed a measure (e.g., "C /.")
                            if let Some(last_measure) = measures.last_mut() {
                                if let Some(last_chord) = last_measure.chords.last_mut() {
                                    last_chord.rhythm = dotted_slash_rhythm.clone();
                                    last_chord.duration =
                                        MusicalDuration::from_beats(dotted_duration, time_sig);

                                    // Also update in rhythm_elements
                                    for elem in last_measure.rhythm_elements.iter_mut().rev() {
                                        if let RhythmElement::Chord(c) = elem {
                                            c.rhythm = dotted_slash_rhythm.clone();
                                            c.duration = MusicalDuration::from_beats(
                                                dotted_duration,
                                                time_sig,
                                            );
                                            break;
                                        }
                                    }
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if applied {
                            if !current_measure.chords.is_empty()
                                || !current_measure.rhythm_elements.is_empty()
                            {
                                current_measure_beats = current_measure
                                    .chords
                                    .iter()
                                    .map(|c| c.duration.to_beats(time_sig))
                                    .sum();
                                measure_has_slash_rhythm = true;
                            } else if !measures.is_empty() {
                                let prev_measure_beats: f64 = measures
                                    .last()
                                    .unwrap()
                                    .chords
                                    .iter()
                                    .map(|c| c.duration.to_beats(time_sig))
                                    .sum();

                                if prev_measure_beats < beats_per_measure - 0.001 {
                                    current_measure = measures.pop().unwrap();
                                    current_measure_beats = prev_measure_beats;
                                    measure_has_slash_rhythm = true;
                                }
                            }
                            last_token_was_slash = true;
                            continue;
                        }
                    }
                }

                if token_str.chars().all(|c| c == '/') {
                    let slash_count = token_str.len() as u8;
                    if slash_count > 0 {
                        // Check if last chord is tied - if so, skip this block
                        // and let the tie handling code (below) process this slash
                        let last_chord_is_tied = current_measure
                            .chords
                            .last()
                            .is_some_and(|c| c.rhythm.is_tied())
                            || (!measures.is_empty()
                                && measures
                                    .last()
                                    .and_then(|m| m.chords.last())
                                    .is_some_and(|c| c.rhythm.is_tied()));

                        if last_chord_is_tied {
                            // Fall through to tie handling code below
                        } else {
                            // Apply slash rhythm to the last chord
                            // Within same measure: slashes SET the duration (explicit slashes override auto-fill)
                            // Across bar line (after |): slashes ADD to existing duration for cross-barline chords
                            // "Cm/Eb /" means Cm/Eb for 1 beat
                            // "Eb ///" means Eb for 3 beats
                            // "Abmaj9 //// | //" means Abmaj9 for 6 beats (spans 1.5 bars)
                            let applied = if let Some(last_chord) =
                                current_measure.chords.last_mut()
                            {
                                // Only accumulate if previous token was also a slash (consecutive slashes).
                                // This prevents accumulating on top of auto-filled slash rhythm.
                                // "C // G //" - when we see first "/" after C, it should SET to 1 (not accumulate).
                                // When we see second "/" after first, it should accumulate to 2.
                                let new_count = if last_token_was_slash {
                                    if let ChordRhythm::Slashes {
                                        count: existing, ..
                                    } = &last_chord.rhythm
                                    {
                                        existing + slash_count
                                    } else {
                                        slash_count
                                    }
                                } else {
                                    // First slash after a chord - SET (override auto-fill)
                                    slash_count
                                };
                                last_chord.rhythm = ChordRhythm::slashes(new_count);
                                last_chord.duration =
                                    MusicalDuration::from_beats(f64::from(new_count), time_sig);

                                // Also update the chord in rhythm_elements
                                for elem in current_measure.rhythm_elements.iter_mut().rev() {
                                    if let RhythmElement::Chord(c) = elem {
                                        c.rhythm = ChordRhythm::slashes(new_count);
                                        c.duration = MusicalDuration::from_beats(
                                            f64::from(new_count),
                                            time_sig,
                                        );
                                        break;
                                    }
                                }
                                true
                            } else if !measures.is_empty() {
                                // Current measure is empty - slashes apply to previous measure's chord
                                // Check if the previous chord has EXPLICIT rhythm (slashes) vs DEFAULT (auto-fill)
                                // - If previous chord has Slashes: measure is explicitly full, add SPACES
                                // - If previous chord has Default: it was auto-filled, so SET the duration
                                let prev_chord_has_explicit_rhythm = measures
                                    .last()
                                    .and_then(|m| m.chords.last())
                                    .is_some_and(|c| {
                                        matches!(
                                            c.rhythm,
                                            ChordRhythm::Slashes { .. } | ChordRhythm::Explicit(_)
                                        )
                                    });
                                // Beats the previous bar already holds in chords *before* its
                                // last one — captured here so the tie-across-barline branch can
                                // tell whether extending the fill chord would overflow the bar
                                // (computing it later conflicts with the `&mut last_chord` borrow).
                                let prev_other_beats: f64 = measures
                                    .last()
                                    .map(|m| {
                                        let idx = m.chords.len().saturating_sub(1);
                                        m.chords[..idx]
                                            .iter()
                                            .map(|c| c.duration.to_beats(time_sig))
                                            .sum()
                                    })
                                    .unwrap_or(0.0);
                                let next_measure_index = measures.len();
                                let sections_len = self.sections.len();

                                if let Some(last_measure) = measures.last_mut() {
                                    if let Some(last_chord) = last_measure.chords.last_mut() {
                                        // If after explicit | separator OR previous chord has explicit rhythm,
                                        // add SPACES to current measure instead of modifying previous chord.
                                        // "Abmaj9 //// | //" = Abmaj9 (4 slashes) + 2 spaces (continuation)
                                        // "C //// //" = C (4 slashes) + 2 spaces (explicit slashes = full)
                                        // "Cm/Eb / Eb ///" = SET to 1 beat (Default rhythm = auto-filled)
                                        if measure_was_created_by_separator
                                            || prev_chord_has_explicit_rhythm
                                        {
                                            // After explicit separator: add SPACES to current measure
                                            // as continuation of the previous chord. These render as
                                            // rhythm slashes without a chord symbol.

                                            // Create a space for each beat of continuation
                                            for i in 0..slash_count {
                                                let space_duration =
                                                    MusicalDuration::from_beats(1.0, time_sig);
                                                let space_rhythm = ChordRhythm::slashes(1);
                                                let space = SpaceInstance::new(
                                                    space_rhythm,
                                                    space_duration,
                                                    AbsolutePosition::new(
                                                        MusicalPosition::try_new(
                                                            measures.len() as i32,
                                                            i as i32,
                                                            0,
                                                        )
                                                        .unwrap_or_else(|_| {
                                                            MusicalPosition::start()
                                                        }),
                                                        self.sections.len(),
                                                    ),
                                                    format!("/{}", "/".repeat(i as usize)),
                                                );
                                                current_measure
                                                    .rhythm_elements
                                                    .push(RhythmElement::Space(space));
                                            }
                                            current_measure_beats += f64::from(slash_count);
                                            true
                                        } else {
                                            // Auto-completed measure: only accumulate if previous was slash
                                            let new_count = if last_token_was_slash {
                                                if let ChordRhythm::Slashes {
                                                    count: existing,
                                                    ..
                                                } = &last_chord.rhythm
                                                {
                                                    existing + slash_count
                                                } else {
                                                    slash_count
                                                }
                                            } else {
                                                slash_count
                                            };
                                            // Beats the previous bar holds in chords *other* than
                                            // this one (the fill chord we're about to extend),
                                            // captured before the `&mut last_chord` borrow above.
                                            let other_beats = prev_other_beats;
                                            if other_beats + f64::from(new_count)
                                                > beats_per_measure + 0.001
                                            {
                                                // Setting the fill chord to `new_count` would
                                                // overflow its already-full bar (e.g. the trailing
                                                // `//` in `Eb/D / Eb // Eb/F //`). Tie the chord
                                                // across the barline: it fills the rest of this bar,
                                                // and the leftover beats become continuation slashes
                                                // at the start of the next measure.
                                                let in_bar =
                                                    (beats_per_measure - other_beats).max(1.0);
                                                let overflow =
                                                    (f64::from(new_count) - in_bar).max(0.0);
                                                let tied = ChordRhythm::Slashes {
                                                    count: in_bar as u8,
                                                    dotted: false,
                                                    tied: true,
                                                };
                                                let in_bar_dur =
                                                    MusicalDuration::from_beats(in_bar, time_sig);
                                                last_chord.rhythm = tied.clone();
                                                last_chord.duration = in_bar_dur;
                                                for elem in
                                                    last_measure.rhythm_elements.iter_mut().rev()
                                                {
                                                    if let RhythmElement::Chord(c) = elem {
                                                        c.rhythm = tied.clone();
                                                        c.duration = in_bar_dur;
                                                        break;
                                                    }
                                                }
                                                let overflow_beats = overflow.round() as u32;
                                                for i in 0..overflow_beats {
                                                    let space = SpaceInstance::new(
                                                        ChordRhythm::slashes(1),
                                                        MusicalDuration::from_beats(1.0, time_sig),
                                                        AbsolutePosition::new(
                                                            MusicalPosition::try_new(
                                                                next_measure_index as i32,
                                                                i as i32,
                                                                0,
                                                            )
                                                            .unwrap_or_else(|_| {
                                                                MusicalPosition::start()
                                                            }),
                                                            sections_len,
                                                        ),
                                                        format!("/{}", "/".repeat(i as usize)),
                                                    );
                                                    current_measure
                                                        .rhythm_elements
                                                        .push(RhythmElement::Space(space));
                                                }
                                                current_measure_beats += f64::from(overflow_beats);
                                            } else {
                                                last_chord.rhythm = ChordRhythm::slashes(new_count);
                                                last_chord.duration = MusicalDuration::from_beats(
                                                    f64::from(new_count),
                                                    time_sig,
                                                );

                                                // Also update the chord in rhythm_elements
                                                for elem in
                                                    last_measure.rhythm_elements.iter_mut().rev()
                                                {
                                                    if let RhythmElement::Chord(c) = elem {
                                                        c.rhythm = ChordRhythm::slashes(new_count);
                                                        c.duration = MusicalDuration::from_beats(
                                                            f64::from(new_count),
                                                            time_sig,
                                                        );
                                                        break;
                                                    }
                                                }
                                            }
                                            true
                                        }
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if applied {
                                // Recalculate total beats for current measure since the duration changed
                                if !current_measure.chords.is_empty()
                                    || !current_measure.rhythm_elements.is_empty()
                                {
                                    // Resync beats with the (possibly extended) chord
                                    // durations. Without this, a slash group that
                                    // accumulates onto the last chord — e.g. the second
                                    // `//` in `Bb // 4-3 //`, or `Bb // //` — would leave
                                    // current_measure_beats stale and let the next chord
                                    // wrongly share the measure. Mirrors the dotted-slash
                                    // path above.
                                    current_measure_beats = current_measure
                                        .chords
                                        .iter()
                                        .map(|c| c.duration.to_beats(time_sig))
                                        .sum();
                                    // Mark that this measure has slash rhythm, so subsequent
                                    // chords can fill remaining beats
                                    measure_has_slash_rhythm = true;
                                } else if !measures.is_empty() {
                                    // The slash was applied to the previous measure (SET mode).
                                    // Check if it now has room and we should "un-pop" it.
                                    let prev_measure_beats: f64 = measures
                                        .last()
                                        .unwrap()
                                        .chords
                                        .iter()
                                        .map(|c| c.duration.to_beats(time_sig))
                                        .sum();

                                    if prev_measure_beats < beats_per_measure - 0.001 {
                                        // Previous measure now has room - pop it back to current_measure
                                        // so subsequent chords can be added to it
                                        current_measure = measures.pop().unwrap();
                                        current_measure_beats = prev_measure_beats;
                                        // Mark that this measure has slash rhythm since we un-popped it
                                        measure_has_slash_rhythm = true;
                                    }
                                    // Note: if we didn't un-pop (measure is full), don't set
                                    // measure_has_slash_rhythm for the empty current measure
                                }
                                last_token_was_slash = true;
                                continue;
                            }
                        } // end else !last_chord_is_tied
                    }
                }
            }

            // Reset slash tracking for non-slash tokens
            last_token_was_slash = false;

            // Check for tie symbol (~)
            // The tie connects the previous chord's duration to the following duration.
            // Example: "C // ~ /" = C with 2 beats + 1 beat = 3 beats total
            if *token_str == "~" {
                // Find the last chord and mark it as having a pending tie
                // The next slash/duration token will add to this chord's duration
                let last_chord_found = if let Some(last_chord) = current_measure.chords.last_mut() {
                    // Set tied flag using the new method
                    last_chord.rhythm = last_chord.rhythm.clone().with_tie();
                    // Also update in rhythm_elements
                    for elem in current_measure.rhythm_elements.iter_mut().rev() {
                        if let RhythmElement::Chord(c) = elem {
                            c.rhythm = c.rhythm.clone().with_tie();
                            break;
                        }
                    }
                    true
                } else if !measures.is_empty() {
                    if let Some(last_measure) = measures.last_mut() {
                        if let Some(last_chord) = last_measure.chords.last_mut() {
                            last_chord.rhythm = last_chord.rhythm.clone().with_tie();
                            // Also update in rhythm_elements
                            for elem in last_measure.rhythm_elements.iter_mut().rev() {
                                if let RhythmElement::Chord(c) = elem {
                                    c.rhythm = c.rhythm.clone().with_tie();
                                    break;
                                }
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if last_chord_found {
                    // Mark that the next duration token should add to the tied chord
                    // We'll handle this by checking for slashes that follow ties
                }
                continue;
            }

            // Check for slashes following a tie: these should ADD to the previous chord's duration
            // Check if the previous token was a tie (we handle this by checking rhythm flags)
            // Since we can't easily look back at previous tokens, we check the chord's tied flag
            if token_str.chars().all(|c| c == '/') && !token_str.is_empty() {
                let slash_count = token_str.len() as u8;

                // Check if the last chord is tied - if so, add to its duration
                let _last_chord_is_tied = {
                    if let Some(last_chord) = current_measure.chords.last() {
                        last_chord.rhythm.is_tied()
                    } else if !measures.is_empty() {
                        if let Some(last_measure) = measures.last() {
                            if let Some(last_chord) = last_measure.chords.last() {
                                last_chord.rhythm.is_tied()
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                // Check if we have a tied chord that we should extend
                // Look for a chord with tied=true
                let extended_tied = {
                    let mut found = false;
                    if let Some(last_chord) = current_measure.chords.last_mut() {
                        if last_chord.rhythm.is_tied() {
                            // Add slash duration to the tied chord
                            let current_beats = last_chord.duration.to_beats(time_sig);
                            let new_beats = current_beats + f64::from(slash_count);
                            last_chord.duration = MusicalDuration::from_beats(new_beats, time_sig);
                            // Clear the tie flag after extending
                            last_chord.rhythm.clear_tie();
                            // Update in rhythm_elements too
                            for elem in current_measure.rhythm_elements.iter_mut().rev() {
                                if let RhythmElement::Chord(c) = elem {
                                    c.duration = last_chord.duration;
                                    c.rhythm.clear_tie();
                                    break;
                                }
                            }
                            found = true;
                        }
                    } else if !measures.is_empty() {
                        if let Some(last_measure) = measures.last_mut() {
                            if let Some(last_chord) = last_measure.chords.last_mut() {
                                if last_chord.rhythm.is_tied() {
                                    let current_beats = last_chord.duration.to_beats(time_sig);
                                    let new_beats = current_beats + f64::from(slash_count);
                                    last_chord.duration =
                                        MusicalDuration::from_beats(new_beats, time_sig);
                                    last_chord.rhythm.clear_tie();
                                    for elem in last_measure.rhythm_elements.iter_mut().rev() {
                                        if let RhythmElement::Chord(c) = elem {
                                            c.duration = last_chord.duration;
                                            c.rhythm.clear_tie();
                                            break;
                                        }
                                    }
                                    found = true;
                                }
                            }
                        }
                    }
                    found
                };

                if extended_tied {
                    continue;
                }
                // If not a tied chord, fall through to normal slash handling below
            }

            // Check for melody variable reference (e.g., "$mainRiff").
            //
            // If the previous chord auto-completed the measure (whole-note,
            // half-note that filled remaining beats, etc.), `current_measure`
            // is empty here; attach the recalled melody to the most recently
            // pushed measure instead. Mirrors the inline `m{...}` branch.
            if let Some(var_name) = token_str.strip_prefix('$') {
                if let Some(melody) = self.melody_variables.get(var_name).cloned() {
                    if current_measure.chords.is_empty() && !measures.is_empty() {
                        measures.last_mut().unwrap().melodies.push(melody);
                    } else {
                        current_measure.melodies.push(melody);
                    }
                } else {
                    tracing::warn!("Unknown melody variable '{}'", var_name);
                }
                continue;
            }

            // Check for inline melody block (e.g., "m{ C_8 D_8 E_4 }")
            if token_str.starts_with("m{") {
                in_melody_block = true;
                // Find the full melody block in the original line (advancing cursor for successive blocks)
                if let Some(m_pos) = line_to_parse[melody_search_offset..].find("m{") {
                    let abs_pos = melody_search_offset + m_pos;
                    let melody_start = &line_to_parse[abs_pos..];
                    // Find the closing brace
                    if let Some(close_pos) = melody_start.find('}') {
                        let melody_str = &melody_start[..close_pos + 1];
                        // Advance cursor past this melody block for next search
                        melody_search_offset = abs_pos + close_pos + 1;
                        match Melody::parse_block_with_defaults(
                            melody_str,
                            default_melody_duration.as_deref(),
                            line_melody_octave,
                        ) {
                            Ok((name, melody)) => {
                                if let Some(var_name) = name {
                                    // Store as a variable
                                    self.melody_variables.set(var_name, melody.clone());
                                }
                                // Attach melody to current measure.
                                // If the current measure is empty (chord filled the previous
                                // measure exactly and it was auto-pushed), attach to the
                                // last pushed measure instead.
                                if current_measure.chords.is_empty() && !measures.is_empty() {
                                    measures.last_mut().unwrap().melodies.push(melody);
                                } else {
                                    current_measure.melodies.push(melody);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to parse inline melody '{}': {}",
                                    melody_str,
                                    e
                                );
                            }
                        }
                    }
                }
                // Check if this token contains the closing brace (e.g., "m{C_4}")
                if token_str.contains('}') {
                    in_melody_block = false;
                }
                continue;
            }

            // Skip tokens that are inside a melody block
            if in_melody_block {
                // Check if this token ends the melody block
                if token_str.contains('}') {
                    in_melody_block = false;
                }
                continue;
            }

            if *token_str == "@repeat-start" {
                Self::finalize_measure_for_separator(&mut measures, &current_measure, false);
                current_measure = Self::fresh_measure(time_sig);
                current_measure.start_repeat = RepeatMark::Forward;
                pending_start_repeat = true;
                current_measure_beats = 0.0;
                just_processed_separator = true;
                measure_was_created_by_separator = true;
                measure_has_slash_rhythm = false;
                continue;
            }

            if *token_str == "@repeat-end" {
                if current_measure.chords.is_empty()
                    && current_measure.rhythm_elements.is_empty()
                    && current_measure.figured_bass.is_empty()
                    && current_measure.volta_start.is_none()
                {
                    if let Some(measure) = measures.last_mut() {
                        measure.end_repeat = RepeatMark::Backward;
                    }
                } else {
                    current_measure.end_repeat = RepeatMark::Backward;
                    Self::finalize_measure_for_separator(
                        &mut measures,
                        &current_measure,
                        measure_was_created_by_separator,
                    );
                }
                current_measure = Self::fresh_measure(time_sig);
                current_measure_beats = 0.0;
                just_processed_separator = true;
                measure_was_created_by_separator = false;
                measure_has_slash_rhythm = false;
                continue;
            }

            // Check for inline text cue (e.g., "@keys", "synth", "here" followed by closing quote logic)
            if token_str.starts_with('@') {
                // Start of an inline cue - collect tokens until we find the closing quote
                // For simplicity, assume the cue is in format: @group "text in quotes"
                // We need to look ahead and collect the full cue string
                // For now, we'll handle this by reconstructing the cue from remaining tokens

                // Find the cue in the original line starting from this position
                if let Some(at_pos) = line_to_parse.find(token_str) {
                    let cue_start = &line_to_parse[at_pos..];
                    // Try to parse the cue
                    if let Some(quote_start) = cue_start.find('"') {
                        if let Some(quote_end) = cue_start[quote_start + 1..].find('"') {
                            let cue_str = &cue_start[..quote_start + 1 + quote_end + 1];
                            if let Ok(cue) = TextCue::parse(cue_str) {
                                pending_cue = Some(cue);
                            }
                        }
                    }
                }
                continue;
            }

            // Skip tokens that are part of the cue text (between quotes)
            if pending_cue.is_some() && (token_str.starts_with('"') || token_str.ends_with('"')) {
                continue;
            }

            // Check for inline dynamic marking (e.g., "<Build>", "<Down>", "<Hit>:3")
            if token_str.starts_with('<') && token_str.contains('>') {
                if let Ok(dynamic) = DynamicMarking::parse(token_str) {
                    pending_dynamic = Some(dynamic);
                }
                continue;
            }

            // Check for stop sign tokens (e.g., "!STOP", "!STOPGROOVE")
            // Position depends on context:
            //   !STOP C  → Stop (before) on C — "stop, then hit C"
            //   C !STOP  → StopAfter on C — "hit C, then stop"
            if let Some(stop_cmd) = Command::parse_stop_token(token_str) {
                // If there's already a chord in this measure, attach as "after" to it
                if let Some(last) = current_measure.chords.last_mut() {
                    last.commands.push(stop_cmd.clone().to_stop_after());
                    // Also update rhythm_elements for consistency
                    if let Some(crate::chart::types::RhythmElement::Chord(c)) =
                        current_measure.rhythm_elements.last_mut()
                    {
                        c.commands.push(stop_cmd.to_stop_after());
                    }
                } else if !just_processed_separator && !measure_was_created_by_separator {
                    // Current measure is empty but NOT because of a `|` separator —
                    // it was auto-pushed when the previous chord filled the measure.
                    // (e.g., "C !STOP" where C's auto-duration fills the measure)
                    if let Some(prev_measure) = measures.last_mut() {
                        if let Some(last) = prev_measure.chords.last_mut() {
                            last.commands.push(stop_cmd.clone().to_stop_after());
                        }
                        if let Some(crate::chart::types::RhythmElement::Chord(c)) =
                            prev_measure.rhythm_elements.last_mut()
                        {
                            c.commands.push(stop_cmd.to_stop_after());
                        }
                    } else {
                        pending_stop = Some(stop_cmd);
                    }
                } else {
                    // After a `|` separator — pend for the next chord (renders before)
                    pending_stop = Some(stop_cmd);
                }
                continue;
            }

            // Check for tempo change (e.g., "->140bpm", "->120")
            if token_str.starts_with("->") {
                if let Some(new_tempo) = TempoChange::parse_syntax(token_str) {
                    // Record the tempo change
                    let position = AbsolutePosition::new(
                        MusicalPosition::try_new(
                            measures.len() as i32,
                            current_measure_beats as i32,
                            0,
                        )
                        .unwrap_or_else(|_| MusicalPosition::start()),
                        self.sections.len(),
                    );
                    let tempo_change =
                        TempoChange::new(position, self.tempo, new_tempo, self.sections.len());
                    self.tempo_changes.push(tempo_change);
                    self.tempo = Some(new_tempo);
                }
                continue;
            }

            if let Some(volta) = Self::parse_volta_token(token_str) {
                current_measure.volta_start = Some(volta);
                continue;
            }

            // Check for measure separator (|)
            // This forces a measure boundary regardless of beat count
            if *token_str == "|" {
                // Finalize current measure if it has chords, or if it was created by a previous separator
                // (This allows multiple | in a row to create empty measures)
                // But don't push auto-created empty measures (created when a measure fills up)
                Self::finalize_measure_for_separator(
                    &mut measures,
                    &current_measure,
                    measure_was_created_by_separator,
                );
                // Always start a new measure after |
                current_measure = Self::fresh_measure(time_sig);
                current_measure_beats = 0.0;
                just_processed_separator = true; // Mark that we just processed a separator
                measure_was_created_by_separator = true; // Mark that this measure was created by |
                measure_has_slash_rhythm = false; // Reset for new measure
                continue;
            }

            // Check for inline time signature change (e.g., "T6/8", "T3/4").
            // The leading `T` (Time) is required so meter changes never collide
            // with number/Nashville slash chords like `4/6`. Bare fractions are
            // only honored on the header line (see parse_metadata_line).
            //
            // A `!` prefix (`!T2/4`) makes it one-shot: the meter applies to the
            // single next measure, then reverts to the prevailing meter — the
            // same "one-time" sigil used for durations.
            let oneshot = token_str.starts_with("!T");
            let ts_token: &str = token_str.strip_prefix('!').unwrap_or(token_str);
            if let Some(ts_body) = ts_token.strip_prefix('T') {
                if ts_body.contains('/') {
                    if let Some((num, den)) = Self::parse_time_signature(ts_body) {
                        // Finalize the current measure before the time sig change
                        if !current_measure.chords.is_empty() {
                            measures.push(current_measure.clone());
                            current_measure = Self::fresh_measure(time_sig);
                            current_measure_beats = 0.0;
                        }

                        if oneshot {
                            // Remember what to restore once this single measure
                            // is pushed. Leave self.time_signature untouched so
                            // the prevailing meter still governs later lines.
                            oneshot_revert = Some((time_sig, beats_per_measure));
                            oneshot_measure_idx = measures.len();
                        } else {
                            // Persistent change: subsequent measures keep this.
                            self.time_signature = Some(TimeSignature::new(num as u32, den as u32));
                        }

                        time_sig = TimeSignature::new(num as u32, den as u32);
                        current_measure.time_signature = (num, den);
                        beats_per_measure = num as f64;
                        continue;
                    }
                }
            }

            // Check for key change - should be ONLY a key signature (short token like "#G", "bBb")
            // Not a chord like "bbmaj7" or "g7"
            let looks_like_key_sig =
                token_str.len() <= 3 && (token_str.starts_with('#') || token_str.starts_with('b'));

            if looks_like_key_sig {
                if let Ok(new_key) = Key::parse(token_str) {
                    // Section index = number of sections already pushed onto the chart
                    // (the in-progress section hasn't been pushed yet).
                    let section_index = self.chart.sections.len();

                    // Position is line-relative: measures finished in this line plus
                    // beats into the current measure. Cross-line carryover within a
                    // single section is not yet tracked here — see parse_section_measures.
                    let beats_into_section =
                        (measures.len() as f64) * beats_per_measure + current_measure_beats;
                    let position = AbsolutePosition::new(
                        MusicalDuration::from_beats(beats_into_section, time_sig),
                        section_index,
                    );

                    let key_change = KeyChange::new(
                        position,
                        self.current_key.clone(),
                        new_key.clone(),
                        section_index,
                    );
                    self.key_changes.push(key_change);
                    self.current_key = Some(new_key);
                    continue;
                }
            }

            // Check for standalone rest token (r4, r8, r8t, r4t, r2, etc.)
            // These don't have a chord symbol but should be stored as rhythm elements
            if token_str.starts_with('r') && token_str.len() >= 2 {
                // Try to parse as a rest
                let mut lexer = Lexer::new(token_str.to_string());
                let tokens = lexer.tokenize();
                if let Ok((rhythm, _)) = ChordRhythm::parse(&tokens) {
                    if rhythm.is_rest() {
                        let duration = rhythm.to_duration(time_sig);
                        let chord_beats = duration.to_beats(time_sig);

                        // Check if we need to start a new measure
                        if !just_processed_separator
                            && current_measure_beats + chord_beats > beats_per_measure + 0.001
                        {
                            if !current_measure.chords.is_empty()
                                || !current_measure.rhythm_elements.is_empty()
                            {
                                measures.push(current_measure.clone());
                            }
                            current_measure = Measure::new();
                            current_measure.time_signature =
                                (time_sig.numerator as u8, time_sig.denominator as u8);
                            current_measure_beats = 0.0;
                        }
                        just_processed_separator = false;
                        measure_was_created_by_separator = false;

                        // Create a RestInstance and add it to rhythm_elements
                        let rest_instance = RestInstance::new(
                            rhythm,
                            duration,
                            AbsolutePosition::new(
                                MusicalPosition::try_new(
                                    measures.len() as i32,
                                    current_measure_beats as i32,
                                    0,
                                )
                                .unwrap_or_else(|_| MusicalPosition::start()),
                                self.sections.len(),
                            ),
                            token_str.to_string(),
                        );

                        current_measure
                            .rhythm_elements
                            .push(RhythmElement::Rest(rest_instance));
                        current_measure_beats += chord_beats;

                        // Check if measure is complete
                        if (current_measure_beats - beats_per_measure).abs() < 0.001 {
                            measures.push(current_measure.clone());
                            current_measure = Measure::new();
                            current_measure.time_signature =
                                (time_sig.numerator as u8, time_sig.denominator as u8);
                            current_measure_beats = 0.0;
                        }
                        continue;
                    }
                }
            }

            // Check for standalone space token (s1, s2, s4, s8, etc.)
            // These represent "invisible" duration - the measure will be filled with automatic slashes
            if token_str.starts_with('s') && token_str.len() >= 2 {
                // Try to parse as a space
                let mut lexer = Lexer::new(token_str.to_string());
                let tokens = lexer.tokenize();
                if let Ok((rhythm, _)) = ChordRhythm::parse(&tokens) {
                    if rhythm.is_space() {
                        let duration = rhythm.to_duration(time_sig);
                        let chord_beats = duration.to_beats(time_sig);

                        // Check if we need to start a new measure
                        if !just_processed_separator
                            && current_measure_beats + chord_beats > beats_per_measure + 0.001
                        {
                            if !current_measure.chords.is_empty()
                                || !current_measure.rhythm_elements.is_empty()
                            {
                                measures.push(current_measure.clone());
                            }
                            current_measure = Measure::new();
                            current_measure.time_signature =
                                (time_sig.numerator as u8, time_sig.denominator as u8);
                            current_measure_beats = 0.0;
                        }
                        just_processed_separator = false;
                        measure_was_created_by_separator = false;

                        // Create a SpaceInstance (not a chord - just a placeholder for auto-fill)
                        let space_instance = SpaceInstance::new(
                            rhythm,
                            duration,
                            AbsolutePosition::new(
                                MusicalPosition::try_new(
                                    measures.len() as i32,
                                    current_measure_beats as i32,
                                    0,
                                )
                                .unwrap_or_else(|_| MusicalPosition::start()),
                                self.sections.len(),
                            ),
                            token_str.to_string(),
                        );

                        // Add to rhythm_elements only (not chords - space is not a chord)
                        current_measure
                            .rhythm_elements
                            .push(RhythmElement::Space(space_instance));
                        current_measure_beats += chord_beats;

                        // Check if measure is complete
                        if (current_measure_beats - beats_per_measure).abs() < 0.001 {
                            measures.push(current_measure.clone());
                            current_measure = Measure::new();
                            current_measure.time_signature =
                                (time_sig.numerator as u8, time_sig.denominator as u8);
                            current_measure_beats = 0.0;
                        }
                        continue;
                    }
                }
            }

            // Check for dot repeat token (. repeats the last chord)
            // Note: apply_auto_durations may add _N suffix, so check for "." or "._N" pattern
            if *token_str == "." || token_str.starts_with("._") {
                if let Some(ref prev_chord) = last_chord {
                    // Clone the last chord with a fresh position
                    let mut repeat_chord = prev_chord.clone();
                    repeat_chord.original_token = ".".to_string();
                    repeat_chord.position = AbsolutePosition::at_beginning(); // Will be recalculated
                    // Clear push/pull - the dot repeat doesn't inherit the timing modifier
                    repeat_chord.push_pull = None;
                    // Inherit the source chord's rhythm and duration
                    // "F/C ." = two measures (F/C for 4 beats, then F/C repeated for 4 beats)
                    // The rhythm and duration are already set from the clone

                    let chord_beats = repeat_chord.duration.to_beats(time_sig);

                    // Handle measure boundaries (same logic as regular chord)
                    if !just_processed_separator
                        && current_measure_beats + chord_beats > beats_per_measure + 0.001
                    {
                        if !current_measure.chords.is_empty()
                            || !current_measure.rhythm_elements.is_empty()
                        {
                            measures.push(current_measure.clone());
                        }
                        current_measure = Measure::new();
                        current_measure.time_signature =
                            (time_sig.numerator as u8, time_sig.denominator as u8);
                        current_measure_beats = 0.0;
                    }
                    just_processed_separator = false;
                    measure_was_created_by_separator = false;

                    current_measure
                        .rhythm_elements
                        .push(RhythmElement::Chord(repeat_chord.clone()));
                    current_measure.chords.push(repeat_chord);
                    current_measure_beats += chord_beats;

                    // Auto-advance measure if full
                    if !just_processed_separator
                        && (current_measure_beats - beats_per_measure).abs() < 0.001
                    {
                        measures.push(current_measure.clone());
                        current_measure = Measure::new();
                        current_measure.time_signature =
                            (time_sig.numerator as u8, time_sig.denominator as u8);
                        current_measure_beats = 0.0;
                    }
                }
                continue;
            }

            // Parse chord. Pass through the token's byte span in the original line
            // (line-relative) so diagnostics can point at real source positions.
            let token_span = original_token_spans.get(token_idx).copied();
            // In a degree-based context a bare `b<digit>` is a flat scale degree
            // (`b3` = ♭3) and a merged `17` is degree-1 + a 7th — not the note B
            // with a figured-bass/suspension figure. Skip the suffix extractors
            // so they don't strip the digit, and split `17` into `1:7`.
            // `^`-marked figured bass (`V^65`, `V^4-3`) is unambiguous in any
            // notation system, so strip it before the degree-specific handling.
            // An *inversion* figure (`V^65`) rewrites to a real inverted chord
            // (parse `V7`, then set the bass below); the original `V^65` is kept
            // for display. Other figures (`^4-3`) fall through to figured bass.
            let inversion_token;
            let (effective_token, inversion_apply) =
                match Self::extract_caret_inversion(effective_token) {
                    Some((ct, display, _append_seventh, bass_thirds)) => {
                        inversion_token = ct;
                        (inversion_token.as_str(), Some((display, bass_thirds)))
                    }
                    None => (effective_token, None),
                };
            let (caret_token, caret_rows) = Self::extract_caret_figure(effective_token);
            let effective_token = caret_token.as_str();
            let (chord_token, figured_bass_rows, suspension_figure) = if chord_system
                == NotationSystem::Degree
                && (Self::is_leading_flat_degree(effective_token)
                    || Self::is_merged_degree(effective_token))
            {
                (Self::split_merged_degree(effective_token), caret_rows, None)
            } else {
                let (fb_token, fb_rows) = Self::extract_figured_bass_suffix(effective_token);
                let (ct, susp) = Self::extract_suspension_suffix(&fb_token);
                (ct, caret_rows.or(fb_rows), susp)
            };
            match self.parse_chord_token(
                &chord_token,
                section_type,
                time_sig,
                token_span,
                chord_system,
            ) {
                Ok(mut chord) => {
                    if let Some(text) = &display_override {
                        chord.display_override = Some(text.clone());
                    }
                    // Realise a `^` inversion: set the slash bass to the chord
                    // tone (3rd/5th/7th above the root) so the chord resolves as
                    // a real inversion, while the chart keeps showing `V^65`.
                    if let Some((display, bass_thirds)) = &inversion_apply {
                        if let Some(root_deg) = chord.parsed.root.scale_degree() {
                            // Diatonic position of the bass tone (root + N thirds).
                            let bass_deg = ((u16::from(root_deg) - 1 + u16::from(*bass_thirds) * 2)
                                % 7) as u8
                                + 1;
                            // Spell it exactly: with a key in hand the bass is the
                            // chord's *actual* 3rd/5th/7th, so a chromatic chord
                            // tone (e.g. the G♯ of `III`) gets the right accidental.
                            let accidental = self.current_key.as_ref().and_then(|key| {
                                let notes = chord.parsed.notes(Some(key))?;
                                let actual = notes.get(*bass_thirds as usize)?;
                                let diatonic = key.get_scale_degree(bass_deg)?;
                                match (i16::from(actual.semitone) - i16::from(diatonic.semitone))
                                    .rem_euclid(12)
                                {
                                    0 => None,
                                    1 => Some(Accidental::Sharp),
                                    11 => Some(Accidental::Flat),
                                    _ => None,
                                }
                            });
                            let bass = RootNotation::from_scale_degree(bass_deg, accidental);
                            chord.parsed.set_bass(bass);
                            chord.full_symbol = display.clone();
                            chord.display_override = Some(display.clone());
                        }
                    }
                    let token_has_explicit_length =
                        Self::token_has_explicit_chord_length(&chord_token);
                    // A rhythm-slash token immediately after this chord (`E/B /`)
                    // sets its duration explicitly, so the `/Duration` default
                    // must NOT pre-fill it — otherwise the slash would only add a
                    // continuation on top of the default instead of overriding it.
                    let slash_token_follows = tokens_str.get(token_idx + 1).is_some_and(|t| {
                        !t.is_empty() && t.trim_end_matches('.').chars().all(|c| c == '/')
                    });
                    if let Some((rhythm, duration)) = &chord_length_override {
                        if chord.rhythm == ChordRhythm::Default
                            && !token_has_explicit_length
                            && !slash_token_follows
                        {
                            chord.rhythm = rhythm.clone();
                            chord.duration = *duration;
                        }
                    }
                    if token_has_explicit_length
                        && !chord_token.starts_with('!')
                        && chord.rhythm != ChordRhythm::Default
                    {
                        chord_length_override = Some((chord.rhythm.clone(), chord.duration));
                    }
                    let mut chord_beats = chord.duration.to_beats(time_sig);

                    // If we just processed a separator, we're already in a new measure
                    // Don't create another one automatically
                    if !just_processed_separator {
                        // Check if adding this chord would exceed the measure
                        if current_measure_beats + chord_beats > beats_per_measure + 0.001 {
                            // Look ahead to see if slashes are coming for this chord
                            // If so, fill remaining beats instead of creating a new measure
                            let slashes_coming = tokens_str[token_idx + 1..]
                                .iter()
                                .take_while(|t| {
                                    // Stop at measure separator or new chord
                                    **t != "|"
                                        && !t.chars().next().is_none_or(|c| c.is_alphabetic())
                                })
                                .any(|t| t.chars().all(|c| c == '/') && !t.is_empty());

                            // If the measure has slash rhythm and the chord has default rhythm,
                            // OR if slashes are coming for this chord,
                            // adjust to fill remaining beats instead of starting a new measure.
                            // e.g., "Cm/Eb / Eb ///" - Cm/Eb takes 1 beat (from /), Eb will take
                            // remaining 3 beats when its /// is processed
                            let remaining_beats = beats_per_measure - current_measure_beats;
                            if (measure_has_slash_rhythm || slashes_coming)
                                && chord.rhythm == ChordRhythm::Default
                                && remaining_beats > 0.001
                            {
                                // Adjust chord to fill remaining beats (duration only)
                                // IMPORTANT: Keep rhythm as Default so actual slashes can override.
                                // If we set rhythm to Slashes here, subsequent slashes would think
                                // it's explicitly set and create spaces instead of setting duration.
                                chord.duration =
                                    MusicalDuration::from_beats(remaining_beats, time_sig);
                                chord_beats = remaining_beats;
                            } else {
                                // small epsilon for float comparison
                                // Current measure is full, start a new one
                                if !current_measure.chords.is_empty()
                                    || !current_measure.rhythm_elements.is_empty()
                                {
                                    measures.push(current_measure.clone());
                                }
                                current_measure = Measure::new();
                                current_measure.time_signature =
                                    (time_sig.numerator as u8, time_sig.denominator as u8);
                                current_measure_beats = 0.0;
                                measure_has_slash_rhythm = false; // Reset for new measure
                                let _measure_was_created_by_separator = false; // Auto-created, not by separator
                            }
                        }
                    }
                    just_processed_separator = false; // Reset flag after processing chord
                    measure_was_created_by_separator = false; // Reset flag - measure now has content

                    // Attach pending stop command to this chord
                    if let Some(stop_cmd) = pending_stop.take() {
                        chord.commands.push(stop_cmd);
                    }

                    if pending_start_repeat && current_measure.chords.is_empty() {
                        current_measure.start_repeat = RepeatMark::Forward;
                        pending_start_repeat = false;
                    }

                    // Store as last chord for dot repeat
                    last_chord = Some(chord.clone());

                    // Add chord to both chords (for backward compat) and rhythm_elements
                    current_measure
                        .rhythm_elements
                        .push(RhythmElement::Chord(chord.clone()));
                    current_measure.chords.push(chord);

                    if let Some(rows) = figured_bass_rows {
                        Self::push_figured_bass_at_current_beat(
                            &mut current_measure,
                            &rows,
                            Placement::Above,
                            current_measure_beats,
                        );
                    }

                    // Attached suspension figure (`Eb2`, `F4-3`) anchors at the
                    // chord's own beat and hugs it as a superscript.
                    if let Some(figure) = &suspension_figure {
                        Self::push_suspension_at_current_beat(
                            &mut current_measure,
                            figure,
                            Placement::Above,
                            current_measure_beats,
                            false,
                        );
                    }

                    current_measure_beats += chord_beats;

                    // Attach pending cue to this measure if present
                    if let Some(cue) = pending_cue.take() {
                        current_measure.text_cues.push(cue);
                    }

                    // Attach pending dynamic to this measure if present
                    if let Some(dynamic) = pending_dynamic.take() {
                        current_measure.dynamics.push(dynamic);
                    }

                    // If we've completed exactly one measure, start a new one
                    // (but only if we didn't just process a separator)
                    // When we just processed a separator, we're already in a fresh measure,
                    // so we don't want to auto-create another one
                    if !just_processed_separator
                        && (current_measure_beats - beats_per_measure).abs() < 0.001
                    {
                        // small epsilon for float comparison
                        measures.push(current_measure.clone());
                        current_measure = Measure::new();
                        current_measure.time_signature =
                            (time_sig.numerator as u8, time_sig.denominator as u8);
                        current_measure_beats = 0.0;
                        measure_has_slash_rhythm = false; // Reset for new measure
                        // Auto-created measure, not by separator
                    }
                }
                Err(_e) => {
                    // Skip unparseable tokens silently
                    // (might be a formatting token or invalid chord)
                }
            }
        }

        // Add last measure if it has content
        // (If we just processed a separator, the empty measure was already pushed)
        if !current_measure.chords.is_empty()
            || !current_measure.rhythm_elements.is_empty()
            || !current_measure.figured_bass.is_empty()
            || !current_measure.staff_text.is_empty()
            || !current_measure.text_cues.is_empty()
            || !current_measure.melodies.is_empty()
        {
            measures.push(current_measure);
        }

        self.expand_phrase_repeats(
            measures,
            repeat_count,
            time_sig,
            beats_per_measure,
            section_type,
            section_measure_count,
        )
    }

    /// Resolve a phrase's repeat count (explicit `xN` or auto `x^`) and expand
    /// the measure list accordingly. Split out of `parse_chord_line_inner`:
    /// it runs once after the token loop and touches none of the loop state,
    /// only the completed `measures` plus the prevailing meter and section
    /// context.
    fn expand_phrase_repeats(
        &self,
        mut measures: Vec<Measure>,
        repeat_count: RepeatCount,
        time_sig: TimeSignature,
        beats_per_measure: f64,
        section_type: &SectionType,
        section_measure_count: Option<usize>,
    ) -> Result<Vec<Measure>, String> {
        let final_repeat_count = match repeat_count {
            RepeatCount::Fixed(count) => count,
            RepeatCount::Auto => {
                // Calculate auto-repeat based on section length vs phrase length
                // We need to calculate phrase length in BEATS, not measures,
                // because chords with explicit durations (like 6_2) might not fill complete measures

                if measures.is_empty() {
                    // Empty phrase, can't calculate
                    return Err("Cannot use x^ with empty phrase".to_string());
                }

                // Calculate total beats in the phrase
                let phrase_beats: f64 = measures
                    .iter()
                    .flat_map(|m| &m.chords)
                    .map(|chord| chord.duration.to_beats(time_sig))
                    .sum();

                // Get section length in measures. Inference fallbacks (in order):
                //   1. Explicit count from the section header (e.g. `VS 16`).
                //   2. Most recent prior section of the same type (chart.sections).
                //   3. The chart's section_measure_memory (set by earlier sections).
                //   4. Bail out with the same error as before.
                let section_measures = if let Some(count) = section_measure_count {
                    count
                } else if let Some(prior_count) = self
                    .chart
                    .sections
                    .iter()
                    .rev()
                    .find(|s| s.section.section_type == *section_type)
                    .map(|s| s.measures().len())
                    .filter(|&n| n > 0)
                {
                    tracing::debug!(
                        "x^: inferred section length {} measures from prior {:?}",
                        prior_count,
                        section_type
                    );
                    prior_count
                } else if let Some(&remembered) = self
                    .chart
                    .section_measure_memory
                    .get(section_type)
                    .filter(|&&n| n > 0)
                {
                    tracing::debug!(
                        "x^: inferred section length {} measures from section_measure_memory",
                        remembered
                    );
                    remembered
                } else {
                    return Err(format!(
                        "Cannot use x^ without explicit section measure count \
                         (no prior {:?} section to infer from). Add a count: e.g. 'VS 16'",
                        section_type
                    ));
                };

                // Convert section length to beats
                let section_beats = section_measures as f64 * beats_per_measure;

                // Calculate how many times to repeat: section_beats / phrase_beats
                let repeat_count_f = section_beats / phrase_beats;

                // Check if it's a whole number (within floating point precision)
                let repeat_count_rounded = repeat_count_f.round();
                if (repeat_count_f - repeat_count_rounded).abs() > 0.001 {
                    return Err(format!(
                        "Cannot use x^: section length ({} measures = {} beats) is not evenly divisible by phrase length ({} beats). Would need {} repeats.",
                        section_measures, section_beats, phrase_beats, repeat_count_f
                    ));
                }

                let repeat_count_int = repeat_count_rounded as usize;
                if repeat_count_int == 0 {
                    return Err(format!(
                        "Cannot use x^: phrase length ({} beats) is longer than section length ({} measures = {} beats)",
                        phrase_beats, section_measures, section_beats
                    ));
                }

                repeat_count_int
            }
        };

        // If repeat count specified, store it on the last measure of the pattern
        // and duplicate the measures
        if final_repeat_count > 1 && !measures.is_empty() {
            let pattern_length = measures.len();

            // Store repeat count on the last measure of the pattern for display purposes
            // Do this BEFORE cloning so it's preserved in the original
            measures[pattern_length - 1].repeat_count = final_repeat_count;

            // Duplicate all measures for the actual playback/structure
            let original_measures = measures.clone();
            for _ in 1..final_repeat_count {
                let mut repeated = original_measures.clone();
                // Clear repeat_count on duplicates so only first occurrence shows it
                for measure in &mut repeated {
                    measure.repeat_count = 1;
                }
                measures.extend(repeated);
            }
        }

        Ok(measures)
    }

    fn parse_barred_line_with_default_chord_length(
        &mut self,
        line: &str,
        section_type: &SectionType,
        section_measure_count: Option<usize>,
        line_byte_offset: usize,
        default_chord_length: Option<(ChordRhythm, MusicalDuration)>,
        default_melody_duration: Option<String>,
        default_melody_octave: Option<u8>,
    ) -> Result<Vec<Measure>, String> {
        let mut measures = Vec::new();
        for (part_offset, part) in Self::split_top_level_measures_spanned(line) {
            let trim_left = part.len() - part.trim_start().len();
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parsed = self.parse_chord_line_inner(
                trimmed,
                section_type,
                Some(1),
                line_byte_offset + part_offset + trim_left,
                default_chord_length.clone(),
                default_melody_duration.clone(),
                default_melody_octave,
            )?;
            measures.append(&mut parsed);
        }

        if measures.is_empty() {
            self.parse_chord_line_inner(
                line,
                section_type,
                section_measure_count,
                line_byte_offset,
                default_chord_length,
                default_melody_duration,
                default_melody_octave,
            )
        } else {
            Ok(measures)
        }
    }

    fn normalize_parallel_container_syntax(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        let mut parallel_depth = 0usize;

        while let Some(ch) = chars.next() {
            if ch == '<' && chars.peek() == Some(&'<') {
                chars.next();
                parallel_depth += 1;
                continue;
            }

            if ch == '>' && chars.peek() == Some(&'>') {
                chars.next();
                parallel_depth = parallel_depth.saturating_sub(1);
                continue;
            }

            if parallel_depth > 0 && ch == ';' {
                out.push(' ');
            } else {
                out.push(ch);
            }
        }

        out
    }

    fn parse_parallel_chord_line(
        &mut self,
        line: &str,
        repeat_count: RepeatCount,
        section_type: &SectionType,
        _section_measure_count: Option<usize>,
        line_byte_offset: usize,
        default_chord_length: Option<(ChordRhythm, MusicalDuration)>,
        default_melody_duration: Option<String>,
        default_melody_octave: Option<u8>,
    ) -> Result<Vec<Measure>, String> {
        let trimmed_line = line.trim();
        if trimmed_line.starts_with("<<") && trimmed_line.ends_with(">>") {
            let trim_left = line.len() - line.trim_start().len();
            let mut measures = self.parse_parallel_sequence(
                trimmed_line,
                section_type,
                line_byte_offset + trim_left,
                default_chord_length,
                default_melody_duration.clone(),
                default_melody_octave,
            )?;
            if let RepeatCount::Fixed(count) = repeat_count {
                if count > 1 {
                    if let Some(last) = measures.last_mut() {
                        last.repeat_count = count;
                    }
                }
            }
            return Ok(measures);
        }

        let measure_parts = Self::split_top_level_measures_spanned(line);
        let mut measures = Vec::new();

        for (part_offset, part) in measure_parts {
            let trimmed_full = part.as_str();
            let trim_left = trimmed_full.len() - trimmed_full.trim_start().len();
            let trimmed = trimmed_full.trim();
            if trimmed.is_empty() {
                continue;
            }
            let measure_offset = line_byte_offset + part_offset + trim_left;

            if trimmed.starts_with("<<") && trimmed.ends_with(">>") {
                measures.push(self.parse_parallel_measure(
                    trimmed,
                    section_type,
                    measure_offset,
                    default_chord_length.clone(),
                    default_melody_duration.clone(),
                    default_melody_octave,
                )?);
            } else {
                let parsed = self.parse_chord_line_inner(
                    trimmed,
                    section_type,
                    Some(1),
                    measure_offset,
                    default_chord_length.clone(),
                    default_melody_duration.clone(),
                    default_melody_octave,
                )?;
                if let Some(measure) = parsed.into_iter().next() {
                    measures.push(measure);
                }
            }
        }

        if let RepeatCount::Fixed(count) = repeat_count {
            if count > 1 {
                if let Some(last) = measures.last_mut() {
                    last.repeat_count = count;
                }
            }
        }

        Ok(measures)
    }

    fn parse_parallel_sequence(
        &mut self,
        container_text: &str,
        section_type: &SectionType,
        container_byte_offset: usize,
        default_chord_length: Option<(ChordRhythm, MusicalDuration)>,
        default_melody_duration: Option<String>,
        default_melody_octave: Option<u8>,
    ) -> Result<Vec<Measure>, String> {
        let inner_with_caps = container_text
            .strip_prefix("<<")
            .and_then(|s| s.strip_suffix(">>"))
            .ok_or_else(|| format!("Invalid parallel container: {}", container_text))?;
        let inner = inner_with_caps.trim();
        let inner_lead = inner_with_caps.len() - inner_with_caps.trim_start().len();
        let inner_offset = container_byte_offset + 2 + inner_lead;

        let branches = Self::split_top_level_parallel_branches_spanned(inner);
        let mut merged: Vec<Measure> = Vec::new();

        for (branch_offset, branch) in branches {
            let trim_left = branch.len() - branch.trim_start().len();
            let trimmed = branch.trim();
            if trimmed.is_empty() {
                continue;
            }
            let parsed = self.parse_chord_line_inner(
                trimmed,
                section_type,
                None,
                inner_offset + branch_offset + trim_left,
                default_chord_length.clone(),
                default_melody_duration.clone(),
                default_melody_octave,
            )?;
            let parsed = self.split_long_melody_branch_measures(parsed);

            if merged.len() < parsed.len() {
                let time_sig = self.time_signature.unwrap_or(TimeSignature::common_time());
                merged.resize_with(parsed.len(), || Self::fresh_measure(time_sig));
            }
            for (idx, branch_measure) in parsed.into_iter().enumerate() {
                Self::merge_parallel_branch_measure(&mut merged[idx], branch_measure);
            }
        }

        Ok(merged)
    }

    fn parse_parallel_measure(
        &mut self,
        measure_text: &str,
        section_type: &SectionType,
        measure_byte_offset: usize,
        default_chord_length: Option<(ChordRhythm, MusicalDuration)>,
        default_melody_duration: Option<String>,
        default_melody_octave: Option<u8>,
    ) -> Result<Measure, String> {
        let trimmed_outer = measure_text.trim();
        // Offset of `trimmed_outer` within `measure_text`.
        let outer_lead = measure_text.len() - measure_text.trim_start().len();

        let inner_with_caps = trimmed_outer
            .strip_prefix("<<")
            .and_then(|s| s.strip_suffix(">>"))
            .ok_or_else(|| format!("Invalid parallel container: {}", measure_text))?;
        let inner = inner_with_caps.trim();
        // Offset of `inner` within `measure_text`: outer trim + "<<" + inner trim.
        let inner_lead = inner_with_caps.len() - inner_with_caps.trim_start().len();
        let inner_offset = measure_byte_offset + outer_lead + 2 /* "<<" */ + inner_lead;

        let branches = Self::split_top_level_parallel_branches_spanned(inner);
        let time_sig = self.time_signature.unwrap_or(TimeSignature::common_time());
        let mut merged = Self::fresh_measure(time_sig);

        for (branch_offset, branch) in branches {
            let trim_left = branch.len() - branch.trim_start().len();
            let trimmed = branch.trim();
            if trimmed.is_empty() {
                continue;
            }

            let parsed = self.parse_chord_line_inner(
                trimmed,
                section_type,
                Some(1),
                inner_offset + branch_offset + trim_left,
                default_chord_length.clone(),
                default_melody_duration.clone(),
                default_melody_octave,
            )?;
            let Some(branch_measure) = parsed.into_iter().next() else {
                continue;
            };

            Self::merge_parallel_branch_measure(&mut merged, branch_measure);
        }

        Ok(merged)
    }

    fn merge_parallel_branch_measure(merged: &mut Measure, branch_measure: Measure) {
        merged.chords.extend(branch_measure.chords);
        merged
            .rhythm_elements
            .extend(branch_measure.rhythm_elements);
        merged.rhythm_slashes.extend(branch_measure.rhythm_slashes);
        merged.text_cues.extend(branch_measure.text_cues);
        merged.staff_text.extend(branch_measure.staff_text);
        merged.figured_bass.extend(branch_measure.figured_bass);
        merged.dynamics.extend(branch_measure.dynamics);
        merged.melodies.extend(branch_measure.melodies);
        merged.repeat_count = merged.repeat_count.max(branch_measure.repeat_count);
        merged.time_signature = branch_measure.time_signature;
        if merged.source_span.is_none() {
            merged.source_span = branch_measure.source_span;
        }
    }

    fn split_long_melody_branch_measures(&self, measures: Vec<Measure>) -> Vec<Measure> {
        if measures.len() != 1 {
            return measures;
        }

        let mut iter = measures.into_iter();
        let measure = iter.next().expect("one measure");
        if !measure.chords.is_empty()
            || !measure.rhythm_elements.is_empty()
            || !measure.figured_bass.is_empty()
            || !measure.staff_text.is_empty()
            || !measure.text_cues.is_empty()
            || measure.melodies.len() != 1
        {
            return vec![measure];
        }

        let time_sig = TimeSignature::new(
            measure.time_signature.0.into(),
            measure.time_signature.1.into(),
        );
        let beats_per_measure =
            f64::from(time_sig.numerator) * 4.0 / f64::from(time_sig.denominator);
        if beats_per_measure <= 0.0 {
            return vec![measure];
        }

        let melody = measure.melodies.into_iter().next().expect("one melody");
        let mut split = Vec::new();
        let mut current = Self::fresh_measure(time_sig);
        let mut current_notes = Vec::new();
        let mut current_beats = 0.0;

        for note in melody.notes {
            let note_beats = note.duration_beats();
            if !current_notes.is_empty() && current_beats + note_beats > beats_per_measure + 0.001 {
                current.melodies.push(Melody::with_notes(current_notes));
                split.push(current);
                current = Self::fresh_measure(time_sig);
                current_notes = Vec::new();
                current_beats = 0.0;
            }
            current_beats += note_beats;
            current_notes.push(note);

            if (current_beats - beats_per_measure).abs() < 0.001 {
                current.melodies.push(Melody::with_notes(current_notes));
                split.push(current);
                current = Self::fresh_measure(time_sig);
                current_notes = Vec::new();
                current_beats = 0.0;
            }
        }

        if !current_notes.is_empty() {
            current.melodies.push(Melody::with_notes(current_notes));
            split.push(current);
        }

        split
    }

    /// Like `split_top_level_measures`, but also returns the byte offset of
    /// each emitted part within `input`. Splits on `|` only at parallel-depth
    /// 0 and brace-depth 0, identical to the un-spanned version.
    /// Whitespace is preserved in the returned slice (callers re-trim and
    /// account for the leading whitespace when computing absolute offsets).
    fn split_top_level_measures_spanned(input: &str) -> Vec<(usize, String)> {
        let mut parts: Vec<(usize, String)> = Vec::new();
        let mut current = String::new();
        let mut current_start: usize = 0;
        let mut last_byte_consumed: usize = 0;
        let mut parallel_depth = 0usize;
        let mut brace_depth = 0usize;
        let bytes = input.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            let b = bytes[i];
            // Two-byte tokens `<<` / `>>` first.
            if b == b'<' && bytes.get(i + 1) == Some(&b'<') {
                parallel_depth += 1;
                if current.is_empty() {
                    current_start = i;
                }
                current.push('<');
                current.push('<');
                i += 2;
                last_byte_consumed = i;
                continue;
            }
            if b == b'>' && bytes.get(i + 1) == Some(&b'>') {
                parallel_depth = parallel_depth.saturating_sub(1);
                if current.is_empty() {
                    current_start = i;
                }
                current.push('>');
                current.push('>');
                i += 2;
                last_byte_consumed = i;
                continue;
            }

            match b {
                b'{' => brace_depth += 1,
                b'}' => brace_depth = brace_depth.saturating_sub(1),
                b'|' if parallel_depth == 0 && brace_depth == 0 => {
                    if !current.trim().is_empty() {
                        parts.push((current_start, std::mem::take(&mut current)));
                    } else {
                        current.clear();
                    }
                    i += 1;
                    last_byte_consumed = i;
                    continue;
                }
                _ => {}
            }

            // Push character (handle multi-byte by slicing original input).
            let ch_end = next_utf8_boundary(input, i);
            if current.is_empty() {
                current_start = i;
            }
            current.push_str(&input[i..ch_end]);
            i = ch_end;
            last_byte_consumed = i;
        }
        let _ = last_byte_consumed;

        if !current.trim().is_empty() {
            parts.push((current_start, current));
        }
        parts
    }

    /// Like `split_top_level_parallel_branches`, but with byte offsets.
    /// Splits on `;` at brace-depth 0.
    pub(super) fn split_top_level_parallel_branches_spanned(input: &str) -> Vec<(usize, String)> {
        let mut parts: Vec<(usize, String)> = Vec::new();
        let mut current = String::new();
        let mut current_start: usize = 0;
        let mut brace_depth = 0usize;
        let bytes = input.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            let b = bytes[i];
            match b {
                b'{' => brace_depth += 1,
                b'}' => brace_depth = brace_depth.saturating_sub(1),
                b';' if brace_depth == 0 => {
                    if !current.trim().is_empty() {
                        parts.push((current_start, std::mem::take(&mut current)));
                    } else {
                        current.clear();
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }

            let ch_end = next_utf8_boundary(input, i);
            if current.is_empty() {
                current_start = i;
            }
            current.push_str(&input[i..ch_end]);
            i = ch_end;
        }

        if !current.trim().is_empty() {
            parts.push((current_start, current));
        }
        parts
    }
}

/// Find the byte index of the next UTF-8 char boundary at or after `start`.
/// `start` must already be a valid char boundary in `s`.
fn next_utf8_boundary(s: &str, start: usize) -> usize {
    let bytes = s.as_bytes();
    let mut end = start + 1;
    while end < bytes.len() && !s.is_char_boundary(end) {
        end += 1;
    }
    end
}

// endregion: --- Chord Line Parsing

// region:    --- Chord Token Parsing

impl<'a> ChartParser<'a> {
    /// Classify a chord token's root notation system, for resolving the
    /// ambiguous `b<digit>` root. `Some(true)` = degree-based (Nashville
    /// number or Roman numeral), `Some(false)` = letter name, `None` = the
    /// token doesn't decide (ambiguous `b`+digit, rests, annotations).
    fn classify_token_system(token: &str) -> Option<bool> {
        let t = token.trim_start_matches(['>', '.', '\'', '(']);
        let mut chars = t.chars();
        let c0 = chars.next()?;
        let c1 = t.chars().nth(1);
        let is_degree_digit = |c: char| ('1'..='7').contains(&c);
        let is_roman = |c: char| matches!(c, 'I' | 'V' | 'i' | 'v');
        match c0 {
            c if is_degree_digit(c) => Some(true),            // 1..7
            'I' | 'V' | 'i' | 'v' => Some(true),              // Roman numeral
            'A'..='G' => Some(false),                         // note letter (upper)
            'a' | 'c' | 'd' | 'e' | 'f' | 'g' => Some(false), // note letter (lower)
            '#' => match c1 {
                Some(c) if is_degree_digit(c) || is_roman(c) => Some(true),
                _ => None,
            },
            'b' => match c1 {
                Some(c) if is_roman(c) => Some(true), // bIII (flat Roman)
                _ => None,                            // b+digit is the ambiguous case
            },
            _ => None,
        }
    }

    /// Decide a scope's notation system by majority vote of its decisive
    /// tokens. `None` when there's a tie or no decisive tokens (defer).
    fn classify_scope<'t>(tokens: impl Iterator<Item = &'t str>) -> Option<bool> {
        let (mut letter, mut degree) = (0u32, 0u32);
        for t in tokens {
            match Self::classify_token_system(t) {
                Some(true) => degree += 1,
                Some(false) => letter += 1,
                None => {}
            }
        }
        match degree.cmp(&letter) {
            std::cmp::Ordering::Greater => Some(true),
            std::cmp::Ordering::Less => Some(false),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// Whether the token explicitly forces a major triad, overriding the key's
    /// diatonic quality: `2M`, `2maj`, `2Major`, `2:maj`, `2△`, `2^`. Lowercase
    /// `m`/`dim`/`aug`/`sus` already parse to a non-major quality, so they're
    /// handled by the quality check — this only catches the major markers.
    fn token_specifies_major(chord_part: &str, root: &str) -> bool {
        let rest = chord_part.strip_prefix(root).unwrap_or(chord_part);
        let rest = rest.strip_prefix(':').unwrap_or(rest);
        rest.starts_with('M')
            || rest.starts_with("maj")
            || rest.starts_with("Maj")
            || rest.starts_with('△')
            || rest.starts_with('^')
    }

    /// A bare leading flat degree: `b` followed by a digit 1-7 (`b3`, `b7`).
    fn is_leading_flat_degree(token: &str) -> bool {
        let b = token.as_bytes();
        b.first() == Some(&b'b') && matches!(b.get(1), Some(d) if (b'1'..=b'7').contains(d))
    }

    /// A degree with its numeric quality run together: `17` (= `1:7`), `46`,
    /// `b17` — an optional accidental, a degree digit 1-7, then more digits.
    /// The `:` separator (`1:7`) is the readable form; this is the terse one.
    fn is_merged_degree(token: &str) -> bool {
        let b = token.as_bytes();
        let i = usize::from(matches!(b.first(), Some(b'#' | b'b')));
        matches!(b.get(i), Some(d) if (b'1'..=b'7').contains(d))
            && b.get(i + 1).is_some_and(u8::is_ascii_digit)
    }

    /// Insert the implicit `:` into a merged degree (`17` -> `1:7`), so the
    /// digit after the degree is parsed as the quality, not eaten as a figure.
    /// Tokens that aren't a merged degree are returned unchanged.
    fn split_merged_degree(token: &str) -> String {
        if Self::is_merged_degree(token) {
            let b = token.as_bytes();
            let i = usize::from(matches!(b.first(), Some(b'#' | b'b')));
            let (head, tail) = token.split_at(i + 1);
            format!("{head}:{tail}")
        } else {
            token.to_string()
        }
    }

    /// Resolve the notation system for an ambiguous `b<digit>` chord: the
    /// current line wins, then the chart's accumulated system (from earlier
    /// lines), else `Auto` (which reads `b7` as the note B).
    fn resolve_notation_system(&self, line_tokens: &[&str]) -> NotationSystem {
        let to_system = |is_degree: bool| {
            if is_degree {
                NotationSystem::Degree
            } else {
                NotationSystem::Letter
            }
        };
        // Line scope wins.
        if let Some(d) = Self::classify_scope(line_tokens.iter().copied()) {
            return to_system(d);
        }
        // Chart scope: tally from earlier chord lines.
        match self.chart_degree_votes.cmp(&self.chart_letter_votes) {
            std::cmp::Ordering::Greater => NotationSystem::Degree,
            std::cmp::Ordering::Less => NotationSystem::Letter,
            std::cmp::Ordering::Equal => NotationSystem::Auto,
        }
    }

    /// Fold a chord line's decisive tokens into the chart-wide system tally, so
    /// later lines can fall back to it when their own scope is ambiguous.
    fn record_line_system_votes(&mut self, line_tokens: &[&str]) {
        for t in line_tokens {
            match Self::classify_token_system(t) {
                Some(true) => self.chart_degree_votes += 1,
                Some(false) => self.chart_letter_votes += 1,
                None => {}
            }
        }
    }

    /// Parse a single chord token
    ///
    /// # Arguments
    /// * `token` - The chord token string (e.g., "Cmaj7", "Am7/G")
    /// * `section_type` - Current section type for chord memory
    /// * `time_sig` - Current time signature for duration calculation
    /// * `source_span` - Optional source text span for linking back to input
    /// * `system` - Notation-system hint for resolving an ambiguous `b<digit>`
    pub(super) fn parse_chord_token(
        &mut self,
        token: &str,
        section_type: &SectionType,
        time_sig: TimeSignature,
        source_span: Option<TextSpan>,
        system: NotationSystem,
    ) -> Result<ChordInstance, String> {
        use crate::chord::Chord;
        use keyflow_proto::chart::commands::Command;

        // Check for accent prefix (>) BEFORE push - indicates accent on the pushed beat
        // Supports: >'C (accent on the anticipation beat 4.66)
        let (accent_before_push, token_after_leading_accent) = if token.starts_with('>') {
            (true, token.strip_prefix('>').unwrap_or(token))
        } else {
            (false, token)
        };

        // Check for staccato prefix (.) AFTER accent, BEFORE push
        // Supports: .C (staccato chord), >.'C (accented staccato push), .'C (staccato push)
        let (is_staccato, token_after_staccato) = if token_after_leading_accent.starts_with('.') {
            (
                true,
                token_after_leading_accent
                    .strip_prefix('.')
                    .unwrap_or(token_after_leading_accent),
            )
        } else {
            (false, token_after_leading_accent)
        };

        // Check for one-time override (prefix !)
        let (is_override, token_clean) = if token_after_staccato.starts_with('!') {
            (
                true,
                token_after_staccato
                    .strip_prefix('!')
                    .unwrap_or(token_after_staccato),
            )
        } else {
            (false, token_after_staccato)
        };

        // Check for push notation (leading apostrophes with optional triplet/tuplet: 'C, ''Em, 'tC, ':5C)
        let (push_modifier, token_after_push) = Self::extract_leading_push_modifiers(token_clean);

        // Check for accent AFTER push prefix - indicates accent on the downbeat
        // Supports: '>C (accent on beat 1, the downbeat)
        let (accent_after_push, token_after_post_accent) = if token_after_push.starts_with('>') {
            (
                true,
                token_after_push
                    .strip_prefix('>')
                    .unwrap_or(token_after_push),
            )
        } else {
            (false, token_after_push)
        };

        // Check for accent shorthand (->) - legacy syntax, also supported (treated as regular accent)
        let (has_accent_inline, token_no_accent) = if token_after_post_accent.contains("->") {
            (true, token_after_post_accent.replace("->", ""))
        } else {
            (false, token_after_post_accent.to_string())
        };
        let token_after_post_accent = token_no_accent.as_str();

        // Check for pull notation (trailing apostrophes with optional triplet/tuplet: C', Em'', C't, C':5)
        let (token_after_pull, pull_modifier) =
            Self::extract_trailing_pull_modifiers(token_after_post_accent);

        // Check for slash chord (e.g., "1/3", "Cmaj7/E", "g/b")
        // But NOT rhythm slashes (e.g., "g//", "C///")
        let (chord_part, bass_part) = Self::split_slash_chord(&token_after_pull);

        // Normalize case for chord parsing - capitalize first letter if it's a
        // note name. This allows "cmaj7" to be parsed as "Cmaj7". A leading
        // ASCII `b` is a flat, though: keep it lowercase when it precedes a
        // Roman numeral (`bIII` — always unambiguous) or a digit in a
        // degree-based context (`b7` = ♭7), instead of turning it into note B.
        let second = chord_part.as_bytes().get(1).copied();
        let next_is_roman = matches!(second, Some(b'I' | b'V' | b'i' | b'v'));
        let next_is_digit = second.is_some_and(|c| c.is_ascii_digit());
        let keep_lowercase_flat = chord_part.starts_with('b')
            && (next_is_roman || (next_is_digit && system == NotationSystem::Degree));
        let normalized_token = if keep_lowercase_flat {
            chord_part.to_string()
        } else {
            Self::normalize_chord_case(&chord_part)
        };

        // Parse the chord using the Chord parser
        let mut lexer = Lexer::new(normalized_token.clone());
        let tokens = lexer.tokenize();

        let mut chord = Chord::parse_with_system(&tokens, system)
            .map_err(|e| format!("Failed to parse chord '{}': {:?}", chord_part, e))?;

        // Add bass note if this is a slash chord
        if let Some(bass_str) = bass_part {
            let bass_normalized = Self::normalize_chord_case(bass_str);
            let bass_notation = RootNotation::from_string(&bass_normalized)
                .ok_or_else(|| format!("Invalid bass note: {}", bass_str))?;
            chord.bass = Some(bass_notation);
        }

        // Extract the root from the ORIGINAL token (before normalization, but after apostrophes)
        // This preserves the original casing/format (lowercase note, scale degree, Roman numeral)
        // For scale degrees with quality (e.g., "2maj"), we need to extract just the number part
        // But we pass the full chord_part to process_chord so it can detect explicit quality
        let root_from_token = Self::extract_root_from_token(&chord_part);
        let chord_part_without_duration = chord_part
            .split_once('_')
            .map_or(chord_part.as_str(), |(base, _)| base);

        // For slash chords with just a root (no explicit quality), don't recall from memory.
        // Writing "F/C" means "F major over C bass", not "whatever F I used before with C bass".
        let is_slash_chord_with_just_root =
            chord.bass.is_some() && chord_part_without_duration.len() <= root_from_token.len() + 1; // +1 for possible accidental

        // Use ChordMemory to process this chord and get the appropriate full symbol
        // Pass chord_part (which includes quality like "2maj") so it can detect explicit quality
        let current_key = self.current_key.clone();
        let mut full_symbol = if is_slash_chord_with_just_root {
            // Slash chord with just root - use the normalized symbol, don't recall from memory
            chord.normalized.clone()
        } else {
            self.chord_memory.process_chord(
                &root_from_token,
                &chord_part, // Use chord_part (after apostrophes stripped) - includes "2maj" not just "2"
                &chord.normalized,
                section_type,
                is_override,
                current_key.as_ref(),
            )
        };

        // Append bass note to full_symbol for slash chords (e.g., "F" + "/C" -> "F/C")
        if let Some(bass) = &chord.bass {
            full_symbol = format!("{}/{}", full_symbol, bass);
        }

        // Give a bare number-system chord its key-implied (diatonic) quality:
        // `2` in C major is ii (minor), `7` is vii° (diminished). The terse
        // display (full_symbol) is unchanged — only the chord's actual quality
        // and intervals are set, so a seventh/extension stacks correctly
        // (`2:7` → ii m7). Skipped under a `!` literal override, when an
        // explicit quality is written (`2m`, `2M`, `2maj`, `2dim`…), for Roman
        // numerals (case already carries quality), and chromatic degrees.
        let infer_diatonic = !is_override
            && chord.quality == ChordQuality::Major
            && !Self::token_specifies_major(&chord_part, &root_from_token);
        if infer_diatonic {
            if let Some((key, degree)) =
                current_key.as_ref().zip(chord.root.diatonic_scale_degree())
            {
                if let Some(dq) = key.diatonic_quality(degree) {
                    if dq != ChordQuality::Major {
                        chord.set_triad_quality(dq);
                    }
                }
            }
        }

        // Get rhythm and duration (push/pull will be applied later)
        let rhythm = chord.duration.clone().unwrap_or(ChordRhythm::Default);
        let duration = rhythm.to_duration(time_sig);

        // Store push/pull info for later adjustment
        // Use settings.push_mode as the default base, but explicit 't' or ':N' modifiers override it
        let default_push_mode = self.settings.push_mode;
        let push_pull_info = if push_modifier.is_present() {
            push_modifier
                .to_amount(default_push_mode)
                .map(|amt| (true, amt))
        } else if pull_modifier.is_present() {
            pull_modifier
                .to_amount(default_push_mode)
                .map(|amt| (false, amt))
        } else {
            None
        };

        // Create a RootNotation from the original token to preserve casing
        // (e.g., "vi" stays "vi", not "Vi")
        // Fall back to the parsed chord's root if we can't parse the original
        let root_notation =
            RootNotation::from_string(&root_from_token).unwrap_or_else(|| chord.root.clone());

        // Store original token without auto-assigned duration suffix
        // Auto-duration adds _1, _2, _4, _8, _16 to tokens, but we want to preserve
        // whether the USER explicitly specified a duration
        let original_token = Self::strip_auto_duration_suffix(token);

        // Create chord instance
        let mut instance = ChordInstance::new(
            root_notation,
            full_symbol,
            chord,
            rhythm,
            original_token,
            duration,
            AbsolutePosition::at_beginning(), // Will be calculated in post-processing
        )
        .with_push_pull(push_pull_info);

        // Add source span for click-to-highlight and editing
        if let Some(span) = source_span {
            instance = instance.with_source_span(span);
        }

        // Add accent command if present
        // The order of > and ' determines which beat gets the accent:
        // - >'C = accent before push = AccentOnPush (accent on the anticipation beat)
        // - '>C = accent after push = Accent (accent on the downbeat)
        let has_push = push_modifier.is_present();
        if accent_before_push && has_push {
            // Accent comes before push marker (>'C) - accent goes on the pushed/anticipation beat
            instance = instance.add_command(Command::AccentOnPush);
        } else if accent_before_push || accent_after_push || has_accent_inline {
            // Regular accent - on the downbeat (or no push involved)
            instance = instance.add_command(Command::Accent);
        }

        // Add staccato command if present (.C, >.'C, .'C)
        if is_staccato {
            instance = instance.add_command(Command::Staccato);
        }

        Ok(instance)
    }
}

// endregion: --- Chord Token Parsing

fn dotted_slash_beats(time_sig: TimeSignature) -> f64 {
    1.5 * (time_sig.denominator as f64 / 4.0)
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::ChartParser as Chart;
    use super::*;
    use crate::chart::parse_chart;

    /// Helper: parse a chord line in isolation and return the parsed measures.
    fn parse_line(line: &str) -> Vec<crate::chart::types::Measure> {
        let mut chart = crate::chart::Chart::new();
        let mut parser = ChartParser::new(&mut chart);
        parser
            .parse_chord_line(line, &SectionType::Verse, Some(1))
            .expect("parse_chord_line failed")
    }

    fn collect_chord_spans(
        measures: &[crate::chart::types::Measure],
    ) -> Vec<(String, Option<crate::parsing::TextSpan>)> {
        let mut out = Vec::new();
        for m in measures {
            for c in &m.chords {
                out.push((c.original_token.clone(), c.source_span));
            }
        }
        out
    }

    /// Collect rendered chord symbols from a parsed line.
    fn chord_symbols(line: &str) -> Vec<String> {
        parse_line(line)
            .iter()
            .flat_map(|m| m.chords.iter().map(|c| c.full_symbol.clone()))
            .collect()
    }

    /// Parse a full chart (with a key header) and collect each chord's quality.
    fn chord_qualities(src: &str) -> Vec<String> {
        parse_chart(src).unwrap().sections[0]
            .measures()
            .iter()
            .flat_map(|m| m.chords.iter().map(|c| format!("{:?}", c.parsed.quality)))
            .collect()
    }

    /// Like [`parse_line`] but with an explicit section measure count, for
    /// content that spans more than one bar.
    fn parse_line_n(line: &str, count: usize) -> Vec<crate::chart::types::Measure> {
        let mut chart = crate::chart::Chart::new();
        let mut parser = ChartParser::new(&mut chart);
        parser
            .parse_chord_line(line, &SectionType::Verse, Some(count))
            .expect("parse_chord_line failed")
    }

    // --- `()` rhythm grouping ---------------------------------------------

    #[test]
    fn expand_chord_groups_rewrites_to_lily_durations() {
        let c44 = TimeSignature::common_time();
        // Default target is one measure, split equally.
        assert_eq!(expand_chord_groups("(C G)", c44).unwrap(), "C_2 G_2");
        // Three over a measure → half-note triplet.
        assert_eq!(
            expand_chord_groups("(D Em G)", c44).unwrap(),
            "D_2t Em_2t G_2t"
        );
        // Slash run sets the target: two beats, one each.
        assert_eq!(expand_chord_groups("(D Em)//", c44).unwrap(), "D_4 Em_4");
        assert_eq!(expand_chord_groups("(D Em) //", c44).unwrap(), "D_4 Em_4");
        // Attached lily target: a quarter split three ways → eighth triplet.
        assert_eq!(
            expand_chord_groups("(D Em G)_4", c44).unwrap(),
            "D_8t Em_8t G_8t"
        );
        // A group is just one element among bare chords.
        assert_eq!(
            expand_chord_groups("G C (Em D) G", c44).unwrap(),
            "G C Em_2 D_2 G"
        );
        // Chord descriptors inside a group are preserved.
        assert_eq!(expand_chord_groups("(Dm7 G7)", c44).unwrap(), "Dm7_2 G7_2");
    }

    #[test]
    fn expand_chord_groups_respects_meter() {
        // In 6/8 a measure is six eighth-beats; two chords → dotted quarters.
        let c68 = TimeSignature::new(6, 8);
        assert_eq!(expand_chord_groups("(A B)", c68).unwrap(), "A_4. B_4.");
    }

    #[test]
    fn expand_chord_groups_leaves_non_groups_untouched() {
        let c44 = TimeSignature::common_time();
        // No parens → identity (so spans/tokens are unchanged for normal lines).
        assert_eq!(expand_chord_groups("C F G Am", c44).unwrap(), "C F G Am");
        // Parens inside quotes and m{…} melody blocks are left alone.
        assert_eq!(
            expand_chord_groups(r#"C "a (b) c" G"#, c44).unwrap(),
            r#"C "a (b) c" G"#
        );
        // A `( )` inside a melody block is left alone — chord-group expansion
        // never reaches into `m{…}`.
        assert_eq!(
            expand_chord_groups("C m{ (D E) } G", c44).unwrap(),
            "C m{ (D E) } G"
        );
    }

    #[test]
    fn expand_chord_groups_reports_errors() {
        let c44 = TimeSignature::common_time();
        assert!(expand_chord_groups("(C G", c44).is_err());
        assert!(expand_chord_groups("()", c44).is_err());
        // Five over a measure has no power-of-two/triplet notation → error.
        assert!(expand_chord_groups("(C D E F G)", c44).is_err());
    }

    #[test]
    fn beats_to_lily_suffix_picks_canonical_token() {
        let c44 = TimeSignature::common_time();
        assert_eq!(beats_to_lily_suffix(2.0, c44).as_deref(), Some("_2")); // half
        assert_eq!(beats_to_lily_suffix(1.0, c44).as_deref(), Some("_4")); // quarter
        assert_eq!(beats_to_lily_suffix(1.5, c44).as_deref(), Some("_4.")); // dotted quarter
        assert_eq!(beats_to_lily_suffix(4.0 / 3.0, c44).as_deref(), Some("_2t"));
        assert_eq!(beats_to_lily_suffix(1.0 / 3.0, c44).as_deref(), Some("_8t"));
        assert_eq!(beats_to_lily_suffix(0.8, c44), None); // quintuplet — unsupported
    }

    #[test]
    fn group_splits_a_bar_equally() {
        let ts = TimeSignature::common_time();
        let measures = parse_line("(C G)");
        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 2);
        assert_eq!(measures[0].chords[0].full_symbol, "C");
        assert_eq!(measures[0].chords[0].duration.to_beats(ts), 2.0);
        assert_eq!(measures[0].chords[1].full_symbol, "G");
        assert_eq!(measures[0].chords[1].duration.to_beats(ts), 2.0);
    }

    #[test]
    fn group_of_three_is_a_whole_measure_triplet() {
        let ts = TimeSignature::common_time();
        let measures = parse_line("(D Em G)");
        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 3);
        for chord in &measures[0].chords {
            // Duration is stored at milli-beat resolution, so 4/3 lands on 1.333.
            assert!((chord.duration.to_beats(ts) - 4.0 / 3.0).abs() < 1e-3);
            // The notation carries a triplet so the engraver brackets it.
            match &chord.rhythm {
                ChordRhythm::Explicit(nd) => assert!(
                    nd.tuplet.is_some(),
                    "expected a tuplet on a triplet group member"
                ),
                other => panic!("expected Explicit rhythm, got {other:?}"),
            }
        }
    }

    #[test]
    fn group_is_one_element_among_bare_chords() {
        let ts = TimeSignature::common_time();
        let measures = parse_line_n("G C (Em D) G", 4);
        assert_eq!(measures.len(), 4);
        // The three bare chords each own a whole bar.
        assert_eq!(measures[0].chords[0].full_symbol, "G");
        assert_eq!(measures[0].chords[0].duration.to_beats(ts), 4.0);
        assert_eq!(measures[1].chords[0].full_symbol, "C");
        // The grouped bar holds both chords, half each.
        assert_eq!(measures[2].chords.len(), 2);
        assert_eq!(measures[2].chords[0].full_symbol, "Em");
        assert_eq!(measures[2].chords[0].duration.to_beats(ts), 2.0);
        assert_eq!(measures[2].chords[1].full_symbol, "D");
        assert_eq!(measures[2].chords[1].duration.to_beats(ts), 2.0);
        assert_eq!(measures[3].chords[0].full_symbol, "G");
    }

    #[test]
    fn group_slash_target_sets_total_beats() {
        let ts = TimeSignature::common_time();
        // `(D Em)//` → two beats split one each.
        let measures = parse_line("(D Em)//");
        assert_eq!(measures[0].chords.len(), 2);
        assert_eq!(measures[0].chords[0].duration.to_beats(ts), 1.0);
        assert_eq!(measures[0].chords[1].duration.to_beats(ts), 1.0);
    }

    // --- `|` bar lines ----------------------------------------------------

    #[test]
    fn chords_inside_one_bar_split_it_evenly() {
        let ts = TimeSignature::common_time();
        // `| G C Em D |` → one bar, four chords, one beat each in 4/4.
        let measures = parse_line("| G C Em D |");
        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 4);
        for (chord, name) in measures[0].chords.iter().zip(["G", "C", "Em", "D"]) {
            assert_eq!(chord.full_symbol, name);
            assert_eq!(chord.duration.to_beats(ts), 1.0);
        }
    }

    #[test]
    fn bar_line_per_chord_is_one_chord_per_bar() {
        let ts = TimeSignature::common_time();
        // A leading `|` before each chord opens a fresh bar; each chord fills it.
        // Works with or without spaces around the bars.
        for line in ["|G |C |Em |D", "|G|C|Em|D"] {
            let measures = parse_line_n(line, 4);
            assert_eq!(measures.len(), 4, "line: {line}");
            for (m, name) in measures.iter().zip(["G", "C", "Em", "D"]) {
                assert_eq!(m.chords.len(), 1, "line: {line}");
                assert_eq!(m.chords[0].full_symbol, name);
                assert_eq!(m.chords[0].duration.to_beats(ts), 4.0);
            }
        }
    }

    #[test]
    fn bars_split_independently() {
        let ts = TimeSignature::common_time();
        // `| G C | Em D | F |` → two half-bar pairs, then a whole-bar F.
        let measures = parse_line_n("| G C | Em D | F |", 3);
        assert_eq!(measures.len(), 3);
        assert_eq!(measures[0].chords[0].duration.to_beats(ts), 2.0);
        assert_eq!(measures[0].chords[1].duration.to_beats(ts), 2.0);
        assert_eq!(measures[1].chords[0].full_symbol, "Em");
        assert_eq!(measures[1].chords[1].duration.to_beats(ts), 2.0);
        assert_eq!(measures[2].chords[0].full_symbol, "F");
        assert_eq!(measures[2].chords[0].duration.to_beats(ts), 4.0);
    }

    #[test]
    fn numbers_take_diatonic_quality_in_major_key() {
        let q = chord_qualities("P\n4/4 120bpm #C\n\nVS\n1 2 3 4 5 6 7\n");
        assert_eq!(
            q,
            [
                "Major",
                "Minor",
                "Minor",
                "Major",
                "Major",
                "Minor",
                "Diminished"
            ]
        );
    }

    #[test]
    fn numbers_take_diatonic_quality_in_minor_key() {
        let q = chord_qualities("P\n4/4 120bpm #Am\n\nVS\n1 2 4 5\n");
        assert_eq!(q, ["Minor", "Diminished", "Minor", "Minor"]);
    }

    #[test]
    fn diatonic_quality_is_overridable() {
        // !2 = literal major; 2M / 2maj = explicit major; 2m = explicit minor;
        // 2:7 keeps the diatonic minor and stacks the seventh (Dm7).
        let q = chord_qualities("P\n4/4 120bpm #C\n\nVS\n!2 2M 2maj 2m 2:7\n");
        assert_eq!(q, ["Major", "Major", "Major", "Minor", "Minor"]);
    }

    #[test]
    fn optional_colon_separates_root_and_quality() {
        assert_eq!(chord_symbols("C:7"), ["C7"]);
        assert_eq!(chord_symbols("1 4:maj9"), ["1", "4maj9"]);
        assert_eq!(chord_symbols("I:7"), ["I7"]);
    }

    #[test]
    fn colon_and_quality_work_on_roman_numerals() {
        assert_eq!(chord_symbols("I:7 IV:maj7"), ["I7", "IVmaj7"]);
        // Lowercase Roman keeps its case (minor), with or without the colon.
        assert_eq!(chord_symbols("v:7"), ["vm7"]);
        assert_eq!(chord_symbols("v7"), ["vm7"]);
        assert_eq!(chord_symbols("i:m7"), ["im7"]);
    }

    #[test]
    fn merged_degree_and_quality_parses_without_colon() {
        // `17` is degree-1 + a 7th (the terse, less-readable form of `1:7`).
        assert_eq!(chord_symbols("1 17"), ["1", "17"]);
        assert_eq!(chord_symbols("1 46"), ["1", "46"]);
    }

    #[test]
    fn flat_roman_roots_parse_in_a_chart_line() {
        // `normalize_chord_case` must not uppercase the leading flat into note B.
        assert_eq!(chord_symbols("I bIII IV"), ["I", "bIII", "IV"]);
        assert_eq!(chord_symbols("i bvi V"), ["im", "bvim", "V"]);
    }

    #[test]
    fn contextual_b7_letter_line_is_note_b() {
        assert_eq!(chord_symbols("C F b7 G"), ["C", "F", "B7", "G"]);
    }

    #[test]
    fn contextual_b7_number_line_is_flat_degree() {
        assert_eq!(chord_symbols("1 4 b7 5"), ["1", "4", "b7", "5"]);
    }

    #[test]
    fn contextual_b7_roman_line_is_flat_degree() {
        assert_eq!(chord_symbols("I IV b7 V"), ["I", "IV", "b7", "V"]);
    }

    #[test]
    fn contextual_b7_no_context_defaults_to_note_b() {
        assert_eq!(chord_symbols("b7"), ["B7"]);
    }

    #[test]
    fn flat_degrees_two_three_four_parse_in_number_context() {
        // These get stripped as sus/figured-bass figures on note B unless the
        // degree context bypasses the suffix extractors.
        assert_eq!(chord_symbols("1 b2 b3 b4 5"), ["1", "b2", "b3", "b4", "5"]);
    }

    #[test]
    fn b9_is_always_note_b_even_in_number_context() {
        // Scale degrees only reach 7, so `b9` can only be the note B + a 9th.
        assert_eq!(chord_symbols("1 4 b9 5"), ["1", "4", "B9", "5"]);
    }

    #[test]
    fn contextual_b7_falls_back_to_chart_when_line_is_ambiguous() {
        // A line that is only `b7` follows the chart's number system -> ♭7.
        let chart = parse_chart("P\n4/4 120bpm #C\n\nVS\n1 4 5 1\nb7\n").expect("parse chart");
        let syms: Vec<String> = chart
            .sections
            .iter()
            .flat_map(|s| s.measures().iter())
            .flat_map(|m| m.chords.iter().map(|c| c.full_symbol.clone()))
            .collect();
        assert!(
            syms.contains(&"b7".to_string()),
            "expected ♭7, got {syms:?}"
        );
    }

    #[test]
    fn caret_inversion_resolves_to_a_real_inverted_chord() {
        let c_major = Key::parse("C").unwrap();
        let chord = |s: &str| {
            // One chord per line; resolve against C major to check the bass.
            let m = parse_line(s);
            m[0].chords[0].clone()
        };

        // `V^65` shows as V^65 but resolves to a real first-inversion dominant
        // 7th: a seventh chord with the 3rd (B in C) in the bass.
        let v65 = chord("V^65");
        assert_eq!(v65.full_symbol, "V^65");
        assert!(v65.parsed.family.is_some(), "^65 implies a seventh chord");
        let bass = v65.parsed.bass.as_ref().expect("inversion sets a bass");
        assert_eq!(bass.resolve(Some(&c_major)).unwrap().name, "B");

        // `V^6` is a first-inversion *triad* (no seventh), 3rd in the bass.
        let v6 = chord("V^6");
        assert_eq!(v6.full_symbol, "V^6");
        assert!(v6.parsed.family.is_none(), "^6 is a triad");
        assert_eq!(
            v6.parsed
                .bass
                .as_ref()
                .unwrap()
                .resolve(Some(&c_major))
                .unwrap()
                .name,
            "B"
        );

        // `V^43` → 7th, 5th (D) in the bass; `V^42` → 7th, 7th (F) in the bass.
        assert_eq!(
            chord("V^43")
                .parsed
                .bass
                .as_ref()
                .unwrap()
                .resolve(Some(&c_major))
                .unwrap()
                .name,
            "D"
        );
        assert_eq!(
            chord("V^42")
                .parsed
                .bass
                .as_ref()
                .unwrap()
                .resolve(Some(&c_major))
                .unwrap()
                .name,
            "F"
        );

        // A trailing duration is preserved on the chord.
        let with_dur = parse_line("V^65_2 V");
        assert_eq!(with_dur[0].chords[0].full_symbol, "V^65");
        assert_eq!(
            with_dur[0].chords[0]
                .duration
                .to_beats(TimeSignature::common_time()),
            2.0
        );

        // A suspension figure (`^4-3`) is not an inversion — it stays a figure.
        assert_eq!(parse_line("V^4-3")[0].figured_bass[0].rows[0].text, "4-3");

        // Without the caret, `V6` is still an ordinary added-6th chord.
        let plain = parse_line("V6");
        assert!(plain[0].figured_bass.is_empty());
        assert!(plain[0].chords[0].parsed.bass.as_ref().is_none());
        assert_eq!(plain[0].chords[0].full_symbol, "V6");
    }

    #[test]
    fn caret_inversion_spells_chromatic_bass_and_ignores_chord_memory() {
        let c_major = Key::parse("C").unwrap();
        // A key is needed so the bass is spelled from the chord's real tones.
        let chart = parse_chart("T\n4/4 #C\n\nvs 4\nI III^6 V^65 V^6\n").unwrap();
        let m = chart.sections[0].measures();

        // `III` is a major chord (E–G♯–B); its 3rd is G♯, so `III^6` puts a
        // sharpened scale degree in the bass, not the diatonic G.
        let iii6 = m[1].chords[0].parsed.bass.as_ref().unwrap();
        assert_eq!(iii6.resolve(Some(&c_major)).unwrap().name, "G#");

        // `V^6` right after `V^65` stays a triad — the remembered seventh from
        // `V^65` does not carry into it.
        assert!(
            m[2].chords[0].parsed.family.is_some(),
            "V^65 is a seventh chord"
        );
        assert!(
            m[3].chords[0].parsed.family.is_none(),
            "V^6 must stay a triad despite the preceding V^65"
        );
        assert_eq!(
            m[3].chords[0]
                .parsed
                .bass
                .as_ref()
                .unwrap()
                .resolve(Some(&c_major))
                .unwrap()
                .name,
            "B"
        );
    }

    #[test]
    fn chord_token_can_attach_figured_bass_rows() {
        let measures = parse_line("| A\"#4-3/2-1\" /// |");

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords[0].full_symbol, "A");
        assert_eq!(measures[0].figured_bass.len(), 1);

        let figured_bass = &measures[0].figured_bass[0];
        assert_eq!(figured_bass.beat, 1);
        assert_eq!(figured_bass.placement, Placement::Above);
        assert_eq!(figured_bass.rows.len(), 2);
        assert_eq!(figured_bass.rows[0].accidental, "#");
        assert_eq!(figured_bass.rows[0].text, "4-3");
        assert_eq!(figured_bass.rows[1].accidental, "");
        assert_eq!(figured_bass.rows[1].text, "2-1");
    }

    #[test]
    fn chord_attached_quote_can_be_figured_bass() {
        let measures = parse_line("| A\"#4-3 2-1\" /// |");

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords[0].full_symbol, "A");
        assert_eq!(measures[0].figured_bass.len(), 1);

        let figured_bass = &measures[0].figured_bass[0];
        assert_eq!(figured_bass.rows[0].accidental, "#");
        assert_eq!(figured_bass.rows[0].text, "4-3");
        assert_eq!(figured_bass.rows[1].accidental, "");
        assert_eq!(figured_bass.rows[1].text, "2-1");
    }

    #[test]
    fn floating_slash_bass_inherits_root_and_displays_verbatim() {
        // `/D` after Bb is harmonically Bb/D but prints "/D".
        let measures = parse_line("Bb /D Eb F");
        assert_eq!(measures.len(), 4);
        let slash = &measures[1].chords[0];
        assert_eq!(slash.full_symbol, "Bb/D");
        assert_eq!(slash.display_override.as_deref(), Some("/D"));
        assert_eq!(
            slash.parsed.bass.as_ref().map(|b| format!("{b}")),
            Some("D".to_string())
        );
    }

    #[test]
    fn floating_slash_bass_drops_prior_bass() {
        // `F/Eb` then `/D` inherits the root F, not the bass: F/D.
        let measures = parse_line("F/Eb /D");
        assert_eq!(measures[1].chords[0].full_symbol, "F/D");
        assert_eq!(
            measures[1].chords[0].display_override.as_deref(),
            Some("/D")
        );
    }

    #[test]
    fn attached_suspension_figure_on_note_root() {
        // `Eb2` and `F4-3` are a chord plus a suspension figure; the digit is
        // not folded into the chord quality.
        let measures = parse_line("Eb2 F4-3");
        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].chords[0].full_symbol, "Eb");
        assert_eq!(measures[0].suspensions.len(), 1);
        assert_eq!(measures[0].suspensions[0].figure, "2");
        assert_eq!(measures[1].chords[0].full_symbol, "F");
        assert_eq!(measures[1].suspensions[0].figure, "4-3");
    }

    #[test]
    fn attached_multi_part_and_chord_quality_figures() {
        // Multi-part hyphenated figure keeps every part (`E3-4-3`).
        let m = parse_line("E3-4-3");
        assert_eq!(m[0].chords[0].full_symbol, "E");
        assert_eq!(m[0].suspensions[0].figure, "3-4-3");

        // Real chord qualities are never mistaken for figures.
        for sym in ["C6", "Csus4", "Cm7", "Gm7b5", "C9"] {
            let m = parse_line(sym);
            assert_eq!(m[0].chords[0].full_symbol, sym, "{sym} should stay a chord");
            assert!(m[0].suspensions.is_empty(), "{sym} should have no figure");
        }
    }

    #[test]
    fn bare_scale_degree_is_not_a_suspension_figure() {
        // A lone `4` is a scale-degree chord, never a floating figure.
        let measures = parse_line("4");
        assert_eq!(measures[0].chords[0].full_symbol, "4");
        assert!(measures[0].suspensions.is_empty());
    }

    #[test]
    fn floating_hyphenated_figure_holds_chord_for_full_measure() {
        // `Bb // 4-3 //` is one measure: Bb sustains all four beats with the
        // 4-3 figure mid-bar, not two separate measures.
        let measures = parse_line("Bb // 4-3 // /D");
        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].chords.len(), 1);
        assert_eq!(measures[0].chords[0].full_symbol, "Bb");
        assert_eq!(
            measures[0].chords[0]
                .duration
                .to_beats(TimeSignature::common_time()),
            4.0
        );
        assert_eq!(measures[0].suspensions.len(), 1);
        assert_eq!(measures[0].suspensions[0].figure, "4-3");
        assert_eq!(measures[1].chords[0].full_symbol, "Bb/D");
    }

    #[test]
    fn bare_chord_then_spaced_figure_opens_a_new_bar() {
        // `F 4-3 ///` opens a NEW bar restating F with the 4-3 figure. The `///`
        // gives that figure 3 beats, so the following chord lands on beat 4 of
        // the SAME bar (not a third bar): bar of F, then bar of [F4-3(3), Eb(1)].
        let measures = parse_line("F 4-3 /// Eb");
        assert_eq!(measures.len(), 2);
        let ts = TimeSignature::common_time();
        assert_eq!(measures[0].chords[0].full_symbol, "F");
        assert!(measures[0].suspensions.is_empty());
        assert_eq!(measures[0].chords[0].duration.to_beats(ts), 4.0);
        // Bar 2: F restated (3 beats, with 4-3) + Eb filling beat 4.
        assert_eq!(measures[1].chords[0].full_symbol, "F");
        assert_eq!(measures[1].chords[0].duration.to_beats(ts), 3.0);
        assert_eq!(measures[1].suspensions[0].figure, "4-3");
        assert_eq!(measures[1].chords[1].full_symbol, "Eb");
        assert_eq!(measures[1].chords[1].duration.to_beats(ts), 1.0);
    }

    #[test]
    fn rhythm_slash_overrides_duration_default() {
        // Under `/Duration 2` (half notes) a trailing rhythm slash sets that
        // chord's length explicitly, ignoring the default. From "Life Giving
        // Water": `E B/D# A/C# E/B / /G# / A B E B4` must tile into four 4/4
        // bars, with `E/B` (one beat, via `/`) and the floating `/G#` → `E/G#`
        // (one beat) both landing in measure 2.
        let measures = parse_line("/Duration 2 E B/D# A/C# E/B / /G# / A B E B4");
        assert_eq!(measures.len(), 4);
        let ts = TimeSignature::common_time();

        // m0: E, B/D# — the default half-note duration applies (2 beats each).
        assert_eq!(measures[0].chords[0].full_symbol, "E");
        assert_eq!(measures[0].chords[0].duration.to_beats(ts), 2.0);
        assert_eq!(measures[0].chords[1].full_symbol, "B/D#");
        assert_eq!(measures[0].chords[1].duration.to_beats(ts), 2.0);

        // m1: A/C# (2) + E/B (1, slash overrides default) + E/G# (1, floating
        // slash-bass + slash). E/G# is in measure 2, not spilled forward.
        assert_eq!(measures[1].chords.len(), 3);
        assert_eq!(measures[1].chords[0].full_symbol, "A/C#");
        assert_eq!(measures[1].chords[0].duration.to_beats(ts), 2.0);
        assert_eq!(measures[1].chords[1].full_symbol, "E/B");
        assert_eq!(measures[1].chords[1].duration.to_beats(ts), 1.0);
        assert_eq!(measures[1].chords[2].full_symbol, "E/G#");
        assert_eq!(
            measures[1].chords[2].display_override.as_deref(),
            Some("/G#")
        );
        assert_eq!(measures[1].chords[2].duration.to_beats(ts), 1.0);

        // m3 ends on E + B4 (B carrying an attached suspension figure "4").
        assert_eq!(measures[3].chords[1].full_symbol, "B");
        assert_eq!(measures[3].suspensions[0].figure, "4");
    }

    #[test]
    fn section_header_trailing_key_change_is_recognized_and_applied() {
        // `BR 8 #G` (key change on the header) must still register as a Bridge
        // section — without stripping the `#G`, the header is unrecognized and
        // its content merges into the previous section. The key change also
        // applies entering the bridge.
        let input = "Song\n120 BPM #E 4/4\n\nVS 1\nE\n\nBR 1 #G\nG\n";
        let chart = parse_chart(input).expect("parse");
        assert_eq!(chart.sections.len(), 2, "bridge must be its own section");
        assert_eq!(chart.sections[1].section.section_type, SectionType::Bridge);
        assert!(
            chart
                .key_changes
                .iter()
                .any(|kc| format!("{}", kc.to_key).contains('G')),
            "expected a key change to G entering the bridge, got {:?}",
            chart.key_changes
        );
    }

    #[test]
    fn life_giving_water_parses_into_its_twelve_sections() {
        // Full-chart regression: exercises section-scoped `/Duration`, the
        // rhythm-slash override of the duration default, floating slash-bass,
        // attached + standalone suspension figures, a trailing key change on a
        // section header (`BR 8 #G`), a quoted section comment (`CH 6 "New?"`),
        // and inline time changes (`!T2/4`). parse_chart only succeeds if every
        // section's parsed measure count matches its header.
        let chart = parse_chart(LIFE_GIVING_WATER).expect("Life Giving Water should parse");
        let kinds: Vec<_> = chart
            .sections
            .iter()
            .map(|s| s.section.section_type.clone())
            .collect();
        assert_eq!(chart.sections.len(), 12, "section kinds: {kinds:?}");
        assert_eq!(chart.sections[0].section.section_type, SectionType::Intro);
        assert_eq!(chart.sections[4].section.section_type, SectionType::Bridge);
        assert_eq!(
            chart.sections[6].section.section_type,
            SectionType::Instrumental
        );
        // `BR 8 #G` applied a key change to G entering the bridge.
        assert!(
            chart
                .key_changes
                .iter()
                .any(|kc| format!("{}", kc.to_key).contains('G')),
            "expected key change to G, got {:?}",
            chart.key_changes
        );
    }

    const LIFE_GIVING_WATER: &str = r##"Life Giving Water
64 BPM #E 4/4

/Duration 2

Intro 4
/Duration 4
C#m /// /B / A // E/G# // D //// B4-3 ////

VS 8
E B/D# A/C# E/B / /G# / A B E B4
E B/D# A/C# E/B / /G# / A B E B/A

VS 8
E B/D# A/C# E / /G# / A B E B/A
E //// A E A B E B/A

CH 8
E B/D# A/C# E/B / /G# / A C#m B4-3 ////
E E/G# C#m E/B //// A B E3-4-3 ////

BR 8 #G
G D/F# C/E G/D C D G D/C
G D/F# C/E G/B C D/C G D/C

CH 8
G D/F# C/E G C //// Em D
G G/B Em G/D C D G D/C

INST 3
G D/F# C/E G/D / /B / C D

CH "New?" 5
G D/F# Em G/D C G/B G !T2/4 Am7 #A Esus ////

CH 8
A E/G# D/F# A/E / A/C# / D //// F#m E
A E/G# D/F# A/E / A/C# / D // E // F#m E/G#

CH 8
A E/G# D/F# A/E / A/C# / D //// F#m E
A A/C# F#m A/E / A/C# / D E F#m ////

Tag 5
D E F#m ////
!T2/4 D Esus // E // E5 ////

Out 4
A E/G# D/F# A/E D // Esus // F#m ////
"##;

    #[test]
    fn percent_repeats_previous_measure() {
        let measures = parse_line("C#\"Major Triad\" %");

        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].chords.len(), 1);
        assert_eq!(measures[1].chords.len(), 1);
        assert_eq!(measures[0].chords[0].full_symbol, "C#");
        assert_eq!(measures[1].chords[0].full_symbol, "C#");
    }

    #[test]
    fn parallel_container_can_merge_chords_with_long_melody_branch() {
        let input = r#"
Parallel Melody Test
120bpm 6/8 #E

opening 2
<< /ChordLength 8. F#m7 G#m7 Amaj7 B | C#m ;
   m { <F# 'C#>4. <G# 'D#>4. <A 'E>4. <B 'F#>4. } >>
"#;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();

        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].chords.len(), 4);
        assert_eq!(measures[1].chords.len(), 1);
        assert_eq!(measures[0].melodies.len(), 1);
        assert_eq!(measures[0].melodies[0].notes.len(), 2);
        assert_eq!(measures[0].melodies[0].notes[0].extra_pitches[0].0, "C#");
        assert_eq!(measures[1].melodies.len(), 1);
        assert_eq!(measures[1].melodies[0].notes.len(), 2);
    }

    #[test]
    fn aliases_expand_for_chord_attached_and_standalone_text() {
        let input = r##"
Alias Test
120bpm 4/4 #C
/alias fb ^"4-3 2-1"
/alias #fb ^"#4-3 2-1"

Verse 2
A<#fb>
C#m <fb> /. _<fb> /.
"##;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();

        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].chords[0].full_symbol, "A");
        assert_eq!(measures[0].figured_bass.len(), 1);
        assert_eq!(measures[0].figured_bass[0].rows[0].accidental, "#");
        assert_eq!(measures[0].figured_bass[0].rows[0].text, "4-3");

        assert_eq!(measures[1].figured_bass.len(), 0);
        assert_eq!(measures[1].staff_text.len(), 2);
        assert_eq!(measures[1].staff_text[0].text, "4-3 2-1");
        assert_eq!(measures[1].staff_text[0].placement, Placement::Above);
        assert_eq!(measures[1].staff_text[1].text, "4-3 2-1");
        assert_eq!(measures[1].staff_text[1].placement, Placement::Below);
    }

    #[test]
    fn standalone_quoted_numeric_text_stays_staff_text() {
        let measures = parse_line("| A / _\"#4-3 2-1\" // |");

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].figured_bass.len(), 0);
        assert_eq!(measures[0].staff_text.len(), 1);
        assert_eq!(measures[0].staff_text[0].beat, 2);
        assert_eq!(measures[0].staff_text[0].placement, Placement::Below);
        assert_eq!(measures[0].staff_text[0].text, "#4-3 2-1");
    }

    #[test]
    fn quoted_staff_text_attaches_to_current_measure() {
        let measures = parse_line("| \"Ac. Gtr. groove\" A /// |");

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].staff_text.len(), 1);
        assert_eq!(measures[0].staff_text[0].text, "Ac. Gtr. groove");
        assert_eq!(measures[0].staff_text[0].beat, 1);
        assert_eq!(measures[0].staff_text[0].placement, Placement::Below);
    }

    #[test]
    fn quoted_staff_text_can_choose_placement() {
        let measures = parse_line("| _\"below staff\" ^\"above staff\" A /// |");

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].staff_text.len(), 2);
        assert_eq!(measures[0].staff_text[0].text, "below staff");
        assert_eq!(measures[0].staff_text[0].placement, Placement::Below);
        assert_eq!(measures[0].staff_text[1].text, "above staff");
        assert_eq!(measures[0].staff_text[1].placement, Placement::Above);
    }

    #[test]
    fn parses_repeat_barlines_and_alternate_endings() {
        let measures = parse_line("|: A | [1] D :| | [2] E |");
        let measures = measures
            .iter()
            .filter(|measure| !measure.chords.is_empty())
            .collect::<Vec<_>>();

        assert_eq!(measures.len(), 3);
        assert!(matches!(measures[0].start_repeat, RepeatMark::Forward));
        assert!(matches!(measures[1].end_repeat, RepeatMark::Backward));

        let first_ending = measures[1].volta_start.as_ref().expect("first ending");
        assert_eq!(first_ending.numbers, vec![1]);
        assert_eq!(first_ending.label, "1.");

        let second_ending = measures[2].volta_start.as_ref().expect("second ending");
        assert_eq!(second_ending.numbers, vec![2]);
        assert_eq!(second_ending.label, "2.");
    }

    #[test]
    fn numbered_measure_check_behaves_like_barline() {
        let measures = parse_line("|{1} A |{2} D |");

        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].chords[0].full_symbol, "A");
        assert_eq!(measures[1].chords[0].full_symbol, "D");
    }

    #[test]
    fn chord_length_directive_applies_to_following_chords() {
        let measures = parse_line("| /ChordLength /. C#m B/C# |");
        let time_sig = TimeSignature::new(4, 4);

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 2);
        for chord in &measures[0].chords {
            assert!(matches!(
                chord.rhythm,
                ChordRhythm::Slashes {
                    count: 1,
                    dotted: true,
                    tied: false
                }
            ));
            assert!((chord.duration.to_beats(time_sig) - 1.5).abs() < 0.001);
        }
    }

    #[test]
    fn section_chord_length_directive_applies_to_content_lines() {
        let input = r#"
Chord Length Section
120bpm 4/4 #C

Intro 1
/ChordLength 4.
| C#m B/C# |
"#;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();
        let time_sig = TimeSignature::new(4, 4);

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 2);
        for chord in &measures[0].chords {
            assert!((chord.duration.to_beats(time_sig) - 1.5).abs() < 0.001);
        }
    }

    #[test]
    fn duration_directive_applies_to_chords_and_melody() {
        let input = r#"
Duration Section
120bpm 6/8 #C

Intro 1
/Duration 8.
<< C#m B/C# A/C# G#m7/C# ;
   m { C# D# E F# } >>
"#;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();
        let time_sig = TimeSignature::new(6, 8);

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 4);
        for chord in &measures[0].chords {
            assert!((chord.duration.to_beats(time_sig) - 1.5).abs() < 0.001);
        }
        assert_eq!(measures[0].melodies.len(), 1);
        assert_eq!(measures[0].melodies[0].notes.len(), 4);
        for note in &measures[0].melodies[0].notes {
            assert_eq!(note.duration, 8);
            assert!(note.dotted);
        }
    }

    #[test]
    fn global_duration_applies_to_sections_until_overridden() {
        // A top-level `/Duration` (before any section) sets a chart-wide default
        // that each section inherits; a section's own `/Duration` overrides it.
        let input = r#"
Global Duration
120bpm 4/4 #C

/Duration 2

VS 2
C G Am F

CH 2
/Duration 4
C E G Am Bm Dm F G
"#;
        let chart = parse_chart(input).expect("Should parse");

        // Verse inherits the global half-note default: 2 chords per 4/4 measure.
        let verse = chart.sections[0].measures();
        assert_eq!(verse.len(), 2);
        assert_eq!(verse[0].chords.len(), 2);

        // Chorus overrides to quarter notes: 4 chords per measure.
        let chorus = chart.sections[1].measures();
        assert_eq!(chorus.len(), 2);
        assert_eq!(chorus[0].chords.len(), 4);
    }

    #[test]
    fn inline_t_prefix_time_signature_change() {
        // `T2/4` changes meter mid-line; the leading `T` distinguishes it from a
        // number/slash chord like `4/6`.
        let input = r#"
Meter Change
120bpm 4/4 #C

VS 3
C T2/4 Am T4/4 G
"#;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();
        assert_eq!(measures.len(), 3);
        assert_eq!(measures[0].time_signature, (4, 4));
        assert_eq!(measures[1].time_signature, (2, 4));
        assert_eq!(measures[2].time_signature, (4, 4));
    }

    #[test]
    fn oneshot_t_prefix_reverts_after_one_measure() {
        // `!T2/4` applies 2/4 to exactly the next measure, then reverts to 4/4.
        // Contrast with persistent `T2/4`, which would keep 2/4 for the rest.
        let input = r#"
One Shot
120bpm 4/4 #C

VS 4
C !T2/4 Am C G
"#;
        let chart = parse_chart(input).expect("Should parse");
        let m = chart.sections[0].measures();
        assert_eq!(m.len(), 4);
        assert_eq!(m[0].time_signature, (4, 4));
        assert_eq!(m[1].time_signature, (2, 4)); // the one-shot measure
        assert_eq!(m[2].time_signature, (4, 4)); // reverted
        assert_eq!(m[3].time_signature, (4, 4));

        // Persistent form keeps 2/4 from the change onward.
        let persistent =
            parse_chart("Persist\n120bpm 4/4 #C\n\nVS 4\nC T2/4 Am C G\n").expect("Should parse");
        let p = persistent.sections[0].measures();
        assert_eq!(p[1].time_signature, (2, 4));
        assert_eq!(p[2].time_signature, (2, 4));
        assert_eq!(p[3].time_signature, (2, 4));
    }

    #[test]
    fn explicit_chord_duration_sticks_to_following_chords() {
        let input = r#"
Sticky Duration Section
120bpm 6/8 #C

Intro 1
F#m7_8. G#m7 Amaj7 B
"#;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();
        let time_sig = TimeSignature::new(6, 8);

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 4);
        for chord in &measures[0].chords {
            assert!((chord.duration.to_beats(time_sig) - 1.5).abs() < 0.001);
        }
    }

    #[test]
    fn let_aliases_expand_for_chord_attached_and_standalone_text() {
        let input = r##"
Let Alias Section
120bpm 4/4 #C

let fb = ^"4-3 2-1"
let #fb = ^"#4-3 2-1"

Intro 2
A<#fb>
<fb> /.
"##;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();

        assert_eq!(measures[0].chords[0].full_symbol, "A");
        assert_eq!(measures[0].figured_bass.len(), 1);
        assert_eq!(measures[0].figured_bass[0].rows.len(), 2);
        assert_eq!(measures[0].figured_bass[0].rows[0].accidental, "#");

        assert_eq!(measures[1].figured_bass.len(), 0);
        assert_eq!(measures[1].staff_text.len(), 1);
        assert_eq!(measures[1].staff_text[0].text, "4-3 2-1");
        assert_eq!(measures[1].staff_text[0].placement, Placement::Above);
        assert_eq!(measures[1].staff_text[0].beat, 1);
    }

    #[test]
    fn let_block_alias_can_invoke_parallel_container() {
        let input = r#"
Let Block Section
120bpm 6/8 #E

let openingHits = {
  <<
    /ChordLength 8. F#m7 G#m7 Amaj7 B ;
    m { <F# 'C#>8. <G# 'D#>8. <A 'E>8. <B 'F#>8. }
  >>
}

Opening 1
<openingHits>
"#;
        let chart = parse_chart(input).expect("Should parse");
        let measures = chart.sections[0].measures();

        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 4);
        assert_eq!(measures[0].chords[0].full_symbol, "F#m7");
        assert_eq!(measures[0].chords[2].full_symbol, "Amaj7");
        assert_eq!(measures[0].melodies.len(), 1);
        assert_eq!(measures[0].melodies[0].notes.len(), 4);
    }

    #[test]
    fn sectioned_lane_aliases_can_merge_as_whole_chart_parallel() {
        let input = r#"
Lane Chart
120bpm 6/8 #E

let chords = {
  intro 2
  C#m
  F#m7_8. G#m7 Amaj7 B
}

let melody = {
  intro 2
  /octave 3 m { C#2. }
  /octave 2 m { <F# 'C#>8. <G# 'D#> <A 'E> <B 'F#> }
}

<< <chords> ; <melody> >>
"#;

        let chart = parse_chart(input).expect("Should parse sectioned lanes");
        assert_eq!(chart.sections.len(), 1);
        let measures = chart.sections[0].measures();
        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].chords[0].full_symbol, "C#m");
        assert_eq!(measures[0].melodies.len(), 1);
        assert_eq!(measures[1].chords.len(), 4);
        assert_eq!(measures[1].chords[2].full_symbol, "Amaj7");
        assert_eq!(measures[1].melodies[0].notes.len(), 4);
    }

    #[test]
    fn inline_dynamic_does_not_inflate_measure_count() {
        // `dyn <level>` inside a bar is a zero-duration annotation; it must not
        // be counted as a chord (which would steal beats and split the bar).
        // Regression for the MusicXML round-trip exporter, which emits dynamics
        // inline like `| dyn mp C#m //. |`.
        let input = "T\n8th=168bpm 6/8 #E\n\nvs 1\n| dyn mp C#m //. |\n";
        let chart = parse_chart(input).expect("inline dyn should parse");
        let measures = chart.sections[0].measures();
        assert_eq!(measures.len(), 1, "dyn must not add a measure");
        assert_eq!(measures[0].chords[0].full_symbol, "C#m");
        assert_eq!(measures[0].classical_dynamics.len(), 1);
        assert_eq!(measures[0].classical_dynamics[0].level, DynamicLevel::Mp);
    }

    #[test]
    fn inline_hairpin_does_not_inflate_measure_count() {
        let input = "T\n8th=168bpm 6/8 #E\n\nvs 1\n| hairpin < 2..4 C#m //. |\n";
        let chart = parse_chart(input).expect("inline hairpin should parse");
        let measures = chart.sections[0].measures();
        assert_eq!(measures.len(), 1, "hairpin must not add a measure");
        assert_eq!(measures[0].hairpins.len(), 1);
    }

    #[test]
    fn melody_octave_memory_carries_across_sections() {
        let input = r#"
Octave Carry
120bpm 6/8 #E

intro 1
/octave 4
m { C#8. D# E F# }

vs 1
m { <,,F# 'C#>8. <G# 'D#> <A 'E> <B 'F#> }
"#;

        let chart = parse_chart(input).expect("Should parse");
        let intro = chart.sections[0].measures();
        assert_eq!(intro[0].melodies[0].notes[3].pitch, "F#");
        assert_eq!(intro[0].melodies[0].notes[3].octave, Some(4));

        let verse = chart.sections[1].measures();
        let first_hit = &verse[0].melodies[0].notes[0];
        assert_eq!(first_hit.pitch, "F#");
        assert_eq!(first_hit.octave, Some(2));
        assert_eq!(first_hit.extra_pitches[0], ("C#".to_string(), Some(4)));
    }

    #[test]
    fn reserved_melody_lane_accepts_bare_melody_lines() {
        let input = r#"
Melody Lane
120bpm 6/8 #E

intro 2

let chords = {
  intro
  C#m
  F#m7_8. G#m7 Amaj7 B
}

let melody = {
  intro
  /octave 4
  C#2.
  <,,F# 'C#>8. <G# 'D#> <A 'E> <B 'F#>
}

<< <chords> ; <melody> >>
"#;

        let chart = parse_chart(input).expect("Should parse reserved melody lane");
        let measures = chart.sections[0].measures();
        assert_eq!(measures[0].chords[0].full_symbol, "C#m");
        assert_eq!(measures[0].melodies[0].notes[0].pitch, "C#");
        assert_eq!(measures[0].melodies[0].notes[0].octave, Some(4));
        assert_eq!(measures[1].chords.len(), 4);
        assert_eq!(measures[1].melodies[0].notes[0].pitch, "F#");
        assert_eq!(measures[1].melodies[0].notes[0].octave, Some(2));
    }

    #[test]
    fn parses_classical_dynamics_and_hairpins() {
        let input = r#"
Dynamics
120bpm 6/8 #E

intro 1
hairpin < 2..4
dyn fff@4
C#m
"#;

        let chart = parse_chart(input).expect("Should parse dynamics and hairpins");
        let measure = &chart.sections[0].measures()[0];
        assert_eq!(measure.classical_dynamics.len(), 1);
        assert_eq!(measure.classical_dynamics[0].level, DynamicLevel::Fff);
        assert_eq!(measure.classical_dynamics[0].beat, 4);
        assert_eq!(measure.hairpins.len(), 1);
        assert_eq!(measure.hairpins[0].kind, HairpinKind::Crescendo);
        assert_eq!(measure.hairpins[0].start_beat, 2);
        assert_eq!(measure.hairpins[0].end_beat, 4);
    }

    #[test]
    fn melody_variable_dollar_recall_attaches_stored_riff() {
        // Pre-load a melody variable directly, then call parse_chord_line so we
        // exercise the recall path independently of section / inline-content
        // dispatch quirks.
        use crate::chart::Melody;
        let mut chart = crate::chart::Chart::new();
        let melody = Melody::parse("C_8 D_8 E_4").expect("parse melody");
        chart.melody_variables.set("mainRiff".to_string(), melody);

        let mut parser = ChartParser::new(&mut chart);
        let measures = parser
            .parse_chord_line("| 1 $mainRiff |", &SectionType::Verse, Some(1))
            .expect("parse_chord_line failed");

        let total_melodies: usize = measures.iter().map(|m| m.melodies.len()).sum();
        assert!(
            total_melodies > 0,
            "$mainRiff should attach a melody (got 0 across {} measures)",
            measures.len()
        );
    }

    #[test]
    fn section_template_recall_replays_prior_chorus() {
        // CH 4 defines a 4-measure progression. A later bare `CH` recalls it.
        let input = "\
CH 4: 1 4 5 1
VS 4: 6 5 4 1
CH 4:
";
        let chart = parse_chart(input).expect("parse_chart should succeed");
        let chs: Vec<_> = chart
            .sections
            .iter()
            .filter(|s| matches!(s.section.section_type, SectionType::Chorus))
            .collect();
        assert_eq!(chs.len(), 2, "expected two CH sections");
        let recalled = chs[1];
        assert!(
            !recalled.measures().is_empty(),
            "second CH should be filled by template recall, got {} measures",
            recalled.measures().len()
        );
    }

    #[test]
    fn parallel_container_token_spans_resolve_to_original_line() {
        // Two branches in a single parallel measure; verify each chord's span
        // points at the right slice of the original input.
        let line = "<< C7 ; F7 >>";
        let measures = parse_line(line);
        let spans = collect_chord_spans(&measures);
        // Find C7 and F7 spans.
        let c_span = spans
            .iter()
            .find(|(t, _)| t == "C7")
            .and_then(|(_, s)| *s)
            .expect("C7 span");
        let f_span = spans
            .iter()
            .find(|(t, _)| t == "F7")
            .and_then(|(_, s)| *s)
            .expect("F7 span");
        assert_eq!(&line[c_span.as_range()], "C7");
        assert_eq!(&line[f_span.as_range()], "F7");
    }

    #[test]
    fn parallel_container_token_spans_across_multiple_measures() {
        // Plain measure followed by a parallel-container measure.
        let line = "Am | << Dm7 ; G7 >> | Cmaj7";
        let measures = parse_line(line);
        let spans = collect_chord_spans(&measures);
        for (token, span) in &spans {
            let span = span.expect("every chord token should have a span");
            assert_eq!(
                &line[span.as_range()],
                token,
                "span for {} resolved to {}",
                token,
                &line[span.as_range()]
            );
        }
    }

    #[test]
    fn test_duration_push_parsing_ambiguous_cases() {
        // Test '_44 - scale degree 4 pushed by quarter note
        let (modifier, remaining) = Chart::extract_leading_push_modifiers("'_44");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Quarter);
        assert_eq!(remaining, "4");

        // Test '_84 - scale degree 4 pushed by eighth note
        let (modifier, remaining) = Chart::extract_leading_push_modifiers("'_84");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Eighth);
        assert_eq!(remaining, "4");

        // Test '_164 - scale degree 4 pushed by sixteenth note
        let (modifier, remaining) = Chart::extract_leading_push_modifiers("'_164");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Sixteenth);
        assert_eq!(remaining, "4");

        // Test '_324 - scale degree 4 pushed by thirty-second note
        let (modifier, remaining) = Chart::extract_leading_push_modifiers("'_324");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::ThirtySecond);
        assert_eq!(remaining, "4");

        // Test '_4C - C pushed by quarter note (non-ambiguous)
        let (modifier, remaining) = Chart::extract_leading_push_modifiers("'_4C");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Quarter);
        assert_eq!(remaining, "C");

        // Test '_8tC - C pushed by triplet eighth
        let (modifier, remaining) = Chart::extract_leading_push_modifiers("'_8tC");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Eighth);
        assert!(modifier.duration_triplet);
        assert_eq!(remaining, "C");

        // Test '_4.C - C pushed by dotted quarter
        let (modifier, remaining) = Chart::extract_leading_push_modifiers("'_4.C");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Quarter);
        assert!(modifier.duration_dotted);
        assert_eq!(remaining, "C");
    }

    #[test]
    fn test_duration_pull_parsing_ambiguous_cases() {
        // Test 4'_4 - scale degree 4 pulled by quarter note
        let (chord, modifier) = Chart::extract_trailing_pull_modifiers("4'_4");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Quarter);
        assert_eq!(chord, "4");

        // Test C'_8 - C pulled by eighth note
        let (chord, modifier) = Chart::extract_trailing_pull_modifiers("C'_8");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Eighth);
        assert_eq!(chord, "C");

        // Test C'_8t - C pulled by triplet eighth
        let (chord, modifier) = Chart::extract_trailing_pull_modifiers("C'_8t");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Eighth);
        assert!(modifier.duration_triplet);
        assert_eq!(chord, "C");

        // Test C'_4. - C pulled by dotted quarter
        let (chord, modifier) = Chart::extract_trailing_pull_modifiers("C'_4.");
        assert!(modifier.duration.is_some());
        assert_eq!(modifier.duration.unwrap(), LilySyntax::Quarter);
        assert!(modifier.duration_dotted);
        assert_eq!(chord, "C");
    }

    #[test]
    fn test_accent_prefix_parsing() {
        use crate::sections::SectionType;
        use keyflow_proto::chart::commands::Command;

        // Test that >C parses as C with an accent command
        let input = r#"
Accent Test
120bpm 4/4 #C

VS
>C
"#;
        let chart = parse_chart(input).expect("Should parse");
        let measures: Vec<_> = chart
            .sections
            .iter()
            .filter(|s| {
                !s.section.section_type.is_compact() && s.section.section_type != SectionType::End
            })
            .flat_map(|s| s.measures())
            .collect();

        assert!(!measures.is_empty(), "Should have at least one measure");
        let chords: Vec<_> = measures[0]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();
        assert!(
            !chords.is_empty(),
            "Should have at least one non-space chord"
        );

        let chord = &chords[0];
        assert_eq!(chord.full_symbol, "C", "Chord symbol should be C");
        assert!(
            chord.commands.iter().any(|c| matches!(c, Command::Accent)),
            "Chord should have accent command"
        );
    }

    #[test]
    fn test_accent_on_push_and_downbeat_parsing() {
        use crate::sections::SectionType;
        use keyflow_proto::chart::commands::Command;

        // Test that >'C parses as AccentOnPush (accent on anticipation)
        // and '>C parses as Accent (accent on downbeat)
        // Based on real-world chart: Thriller - Dirty Loops
        let input = r#"
Thriller - Dirty Loops, Cory Wong
Transcribed By: Cody Wright
120bpm 4/4 #Ab
/push = triplet

COUNT 2

IN
r8t >Ab9_8t r8t r8t r8t >F9_8t r2 | s1

VS
>'F/C . Cm . 'F/C . Cm . 'F/C . Cm . 'F/C . Cm Cm9

CH
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9  ////
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A | r8t >Ab9_8t r8t r8t >'F9_8t r8t r4 Fm/Ab_4 | s1

BR
'_4F7 | . |  Abmaj9 //// | // r8t Abmaj9_8t r8t Bb_8t r8t Cm7_8t | Cm7 | Ebmaj7/Bb | Am7b5 | Abmaj7 | G7sus4 | 'G7

VS
'F/C . Cm . 'F/C . Cm . 'F/C . Cm . 'F/C . Cm Cm9
"#;
        let chart = parse_chart(input).expect("Should parse");

        // Find VS section (first one after COUNT and IN)
        let verse_sections: Vec<_> = chart
            .sections
            .iter()
            .filter(|s| s.section.section_type == SectionType::Verse)
            .collect();
        assert!(!verse_sections.is_empty(), "Should have VS sections");

        // First VS measure should have >'F/C which is AccentOnPush
        let vs_measures = verse_sections[0].measures();
        let vs_chords: Vec<_> = vs_measures[0]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();
        assert!(!vs_chords.is_empty(), "VS should have non-space chords");

        let vs_chord = vs_chords[0];
        assert_eq!(vs_chord.full_symbol, "F/C", "VS chord should be F/C");
        assert!(
            vs_chord
                .commands
                .iter()
                .any(|c| matches!(c, Command::AccentOnPush)),
            ">'F/C should have AccentOnPush command (accent on anticipation)"
        );
        assert!(
            vs_chord.push_pull.is_some(),
            "VS chord should have push_pull"
        );

        // Find CH section to test both accent types
        let chorus_sections: Vec<_> = chart
            .sections
            .iter()
            .filter(|s| s.section.section_type == SectionType::Chorus)
            .collect();
        assert!(!chorus_sections.is_empty(), "Should have CH sections");

        // CH measure 7 (second-to-last) has >'F9 which is AccentOnPush
        // Input: r8t >Ab9_8t r8t r8t >'F9_8t r8t r4 Fm/Ab_4
        let ch_measures = chorus_sections[0].measures();
        assert!(ch_measures.len() >= 8, "CH should have at least 8 measures");

        let ch7_chords: Vec<_> = ch_measures[7]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s" && c.full_symbol != "r")
            .collect();

        // Should have Ab9, F9, and Fm/Ab
        assert!(
            ch7_chords.len() >= 2,
            "CH measure 7 should have at least 2 chords"
        );

        // Ab9 has regular accent (>Ab9)
        let ab9_chord = ch7_chords.iter().find(|c| c.full_symbol == "Ab9");
        assert!(ab9_chord.is_some(), "Should have Ab9 chord");
        let ab9 = ab9_chord.unwrap();
        assert!(
            ab9.commands.iter().any(|c| matches!(c, Command::Accent)),
            ">Ab9 should have regular Accent (no push)"
        );

        // F9 has AccentOnPush (>'F9 with push)
        let f9_chord = ch7_chords.iter().find(|c| c.full_symbol == "F9");
        assert!(f9_chord.is_some(), "Should have F9 chord");
        let f9 = f9_chord.unwrap();
        assert!(
            f9.commands
                .iter()
                .any(|c| matches!(c, Command::AccentOnPush)),
            ">'F9 should have AccentOnPush (accent before push marker)"
        );
        assert!(f9.push_pull.is_some(), "F9 should have push_pull");
    }

    #[test]
    fn test_accent_not_in_chord_memory() {
        use crate::sections::SectionType;
        use keyflow_proto::chart::commands::Command;

        // With the new chord memory behavior:
        // - Basic chords (C, Cm) don't participate in memory at all
        // - Writing "C" gives "C" (basic major), NOT a recalled Cmaj7
        // This test verifies that accented chords don't bleed accents into basic chords
        let input = r#"
Accent Memory Test
120bpm 4/4 #C

VS
>Cmaj7 | C D E F
"#;
        let chart = parse_chart(input).expect("Should parse");

        // Get all verse sections
        let verse_sections: Vec<_> = chart
            .sections
            .iter()
            .filter(|s| s.section.section_type == SectionType::Verse)
            .collect();

        assert!(
            !verse_sections.is_empty(),
            "Should have at least 1 verse section"
        );

        // VS measure 0: >Cmaj7 should have accent
        let vs_measures = verse_sections[0].measures();
        assert!(vs_measures.len() >= 2, "VS should have at least 2 measures");

        let m0_chords: Vec<_> = vs_measures[0]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();
        assert!(!m0_chords.is_empty(), "M0 should have non-space chords");

        // First chord: >Cmaj7 should have accent
        let first_chord = m0_chords[0];
        assert_eq!(first_chord.full_symbol, "Cmaj7");
        assert!(
            first_chord
                .commands
                .iter()
                .any(|c| matches!(c, Command::Accent)),
            "First chord should have accent"
        );

        // VS measure 1: C (basic major) should recall Cmaj7 from major family memory
        // Basic chords CAN recall but don't store - split memory by chord family
        let m1_chords: Vec<_> = vs_measures[1]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();
        assert!(!m1_chords.is_empty(), "M1 should have non-space chords");

        let second_chord = m1_chords[0];
        // Basic chords DO recall from their family's memory
        assert_eq!(
            second_chord.full_symbol, "Cmaj7",
            "Basic C should recall Cmaj7 from major family memory"
        );
        // But the accent is NOT recalled (it's a modifier on the chord instance, not stored in memory)
        assert!(
            !second_chord
                .commands
                .iter()
                .any(|c| matches!(c, Command::Accent)),
            "Recalled chord should NOT have accent from memory. Commands: {:?}",
            second_chord.commands
        );
    }

    #[test]
    fn test_accent_preserved_in_section_template() {
        use crate::sections::SectionType;
        use keyflow_proto::chart::commands::Command;

        // Accents CAN be committed to section memory (templates).
        // If we define VS with >C, then recall VS, the accent should be preserved.
        let input = r#"
Accent Template Test
120bpm 4/4 #C

VS
>C D E F

VS
"#;
        let chart = parse_chart(input).expect("Should parse");
        let verse_sections: Vec<_> = chart
            .sections
            .iter()
            .filter(|s| s.section.section_type == SectionType::Verse)
            .collect();

        assert_eq!(verse_sections.len(), 2, "Should have 2 verse sections");

        // VS1 (template definition): >C should have accent
        let vs1_chords: Vec<_> = verse_sections[0].measures()[0]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();
        assert!(!vs1_chords.is_empty());
        assert!(
            vs1_chords[0]
                .commands
                .iter()
                .any(|c| matches!(c, Command::Accent)),
            "VS1 first chord should have accent"
        );

        // VS2 (recalled from template): should also have accent on first chord
        let vs2_chords: Vec<_> = verse_sections[1].measures()[0]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();
        assert!(!vs2_chords.is_empty());
        assert!(
            vs2_chords[0]
                .commands
                .iter()
                .any(|c| matches!(c, Command::Accent)),
            "VS2 recalled chord should have accent - templates preserve commands"
        );
    }

    #[test]
    fn test_push_duration_then_accent() {
        use keyflow_proto::chart::commands::Command;

        // '_4>F7 = push with quarter duration, then accent (accent on downbeat)
        // >'_4F7 = accent before push (accent on pushed beat)
        let input = r#"
Test
120bpm 4/4

BR
'_4>F7 | >'_4G7 |
"#;
        let chart = parse_chart(input).expect("Should parse");

        // Find BR section
        let br_sections: Vec<_> = chart
            .sections
            .iter()
            .filter(|s| s.section.section_type == crate::sections::SectionType::Bridge)
            .collect();
        assert!(!br_sections.is_empty(), "Should have BR section");

        let measures = br_sections[0].measures();

        // Measure 0: '_4>F7 - accent AFTER push = regular Accent (downbeat)
        let m0_chords: Vec<_> = measures[0]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s" && c.full_symbol != "r")
            .collect();
        assert!(!m0_chords.is_empty(), "Measure 0 should have chords");
        let f7 = &m0_chords[0];
        assert_eq!(f7.full_symbol, "F7", "Should be F7");
        assert!(f7.push_pull.is_some(), "F7 should have push");
        assert!(
            f7.commands.iter().any(|c| matches!(c, Command::Accent)),
            "'_4>F7 should have Accent (accent after push = downbeat). Got: {:?}",
            f7.commands
        );
        assert!(
            !f7.commands
                .iter()
                .any(|c| matches!(c, Command::AccentOnPush)),
            "'_4>F7 should NOT have AccentOnPush. Got: {:?}",
            f7.commands
        );

        // Measure 1: >'_4G7 - accent BEFORE push = AccentOnPush (pushed beat)
        let m1_chords: Vec<_> = measures[1]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s" && c.full_symbol != "r")
            .collect();
        assert!(!m1_chords.is_empty(), "Measure 1 should have chords");
        let g7 = &m1_chords[0];
        assert_eq!(g7.full_symbol, "G7", "Should be G7");
        assert!(g7.push_pull.is_some(), "G7 should have push");
        assert!(
            g7.commands
                .iter()
                .any(|c| matches!(c, Command::AccentOnPush)),
            ">'_4G7 should have AccentOnPush (accent before push = pushed beat). Got: {:?}",
            g7.commands
        );
        assert!(
            !g7.commands.iter().any(|c| matches!(c, Command::Accent)),
            ">'_4G7 should NOT have regular Accent. Got: {:?}",
            g7.commands
        );
    }
}

// endregion: --- Tests

/// Walk `line` and return a `TextSpan` for each whitespace-separated token,
/// in order. Spans are byte offsets relative to `line` (line/column = 0).
fn tokenize_with_spans(line: &str) -> Vec<TextSpan> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        let mut in_quote = false;
        let mut escaped = false;
        while i < bytes.len() {
            let ch = bytes[i] as char;
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }
            if ch == '\\' && in_quote {
                escaped = true;
                i += 1;
                continue;
            }
            if ch == '"' {
                in_quote = !in_quote;
                i += 1;
                continue;
            }
            if !in_quote && ch.is_whitespace() {
                break;
            }
            i += 1;
        }
        spans.push(TextSpan::new(start, i - start));
    }
    spans
}

fn unescape_quoted(s: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    if escaped {
        out.push('\\');
    }
    out
}

fn split_chord_attached_quote(token: &str) -> Option<(&str, &str)> {
    let quote_start = token.find('"')?;
    if quote_start == 0 || !token.ends_with('"') {
        return None;
    }
    Some((
        &token[..quote_start],
        &token[quote_start + 1..token.len() - 1],
    ))
}

fn split_chord_attached_alias(token: &str) -> Option<(&str, &str)> {
    let alias_start = token.find('<')?;
    if alias_start == 0 || !token.ends_with('>') {
        return None;
    }
    Some((
        &token[..alias_start],
        &token[alias_start + 1..token.len() - 1],
    ))
}

fn alias_quoted_payload(value: &str) -> Option<&str> {
    let value = value.trim();
    let value = value
        .strip_prefix("^\"")
        .or_else(|| value.strip_prefix("_\""))
        .or_else(|| value.strip_prefix('"'))?;
    value.strip_suffix('"')
}

fn apply_alias_placement(value: &str, placement: Option<Placement>) -> Option<String> {
    let value = value.trim();
    let payload = alias_quoted_payload(value)?;
    let prefix = match placement {
        Some(Placement::Below) => "_",
        Some(Placement::Above) => "^",
        None if value.starts_with("^\"") => "^",
        None if value.starts_with("_\"") => "_",
        None => "",
    };
    Some(format!("{prefix}\"{payload}\""))
}

fn parallel_depth_for_line(mut depth: usize, line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut brace_depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                brace_depth += 1;
                i += 1;
            }
            b'}' => {
                brace_depth = brace_depth.saturating_sub(1);
                i += 1;
            }
            b'<' if brace_depth == 0 && bytes.get(i + 1) == Some(&b'<') => {
                depth += 1;
                i += 2;
            }
            b'>' if brace_depth == 0 && bytes.get(i + 1) == Some(&b'>') => {
                depth = depth.saturating_sub(1);
                i += 2;
            }
            _ => i += 1,
        }
    }
    depth
}

fn let_block_depth_for_line(mut depth: usize, line: &str) -> usize {
    if depth == 0 {
        let Some(rest) = line.trim_start().strip_prefix("let ") else {
            return 0;
        };
        let Some((_, value)) = rest.split_once('=') else {
            return 0;
        };
        if !value.trim_start().starts_with('{') {
            return 0;
        }
    }

    let mut in_quote = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_quote {
            escaped = c == '\\' && !escaped;
            if c == '"' && !escaped {
                in_quote = false;
            }
            if c != '\\' {
                escaped = false;
            }
            continue;
        }

        match c {
            '"' => in_quote = true,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth
}

fn split_figured_bass_words(input: &str) -> Vec<String> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    if parts.len() > 1 {
        parts.into_iter().map(str::to_string).collect()
    } else if let Some(only) = parts.first() {
        split_compacted_figured_bass(only)
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    }
}

fn split_compacted_figured_bass(row: &str) -> Vec<&str> {
    let stripped = row
        .strip_prefix("bb")
        .or_else(|| row.strip_prefix("##"))
        .or_else(|| row.strip_prefix('#'))
        .or_else(|| row.strip_prefix('b'))
        .or_else(|| row.strip_prefix('n'))
        .unwrap_or(row);
    let accidental_len = row.len() - stripped.len();
    let chars = stripped.chars().collect::<Vec<_>>();
    if chars.len() == 6
        && chars[0].is_ascii_digit()
        && chars[1] == '-'
        && chars[2].is_ascii_digit()
        && chars[3].is_ascii_digit()
        && chars[4] == '-'
        && chars[5].is_ascii_digit()
    {
        vec![&row[..accidental_len + 3], &row[accidental_len + 3..]]
    } else {
        vec![row]
    }
}

fn looks_like_figured_bass_text(text: &str) -> bool {
    text.chars()
        .all(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '#' | 'b' | 'n'))
        && text.chars().any(|ch| ch.is_ascii_digit())
}

fn parse_lily_duration_beats(token: &str, time_sig: TimeSignature) -> Option<(f64, bool)> {
    let dotted = token.ends_with('.');
    let base = token.trim_end_matches('.');
    let note_denominator = match base {
        "1" => 1.0,
        "2" => 2.0,
        "4" => 4.0,
        "8" => 8.0,
        "16" => 16.0,
        "32" => 32.0,
        _ => return None,
    };
    let beats = f64::from(time_sig.denominator) / note_denominator;
    Some((if dotted { beats * 1.5 } else { beats }, dotted))
}

/// Inverse of [`parse_lily_duration_beats`], extended with triplets: find the
/// lily duration token whose value equals `beats` in the given meter, and
/// return its suffix including the leading underscore (e.g. `"_2"`, `"_4."`,
/// `"_8t"`). Searches plain, dotted, triplet, then dotted-triplet forms of each
/// note value, longest note first, so the canonical token wins. Returns `None`
/// when no notatable note value matches (e.g. a quintuplet division). Used by
/// [`expand_chord_groups`] to rewrite `()` rhythm groups.
fn beats_to_lily_suffix(beats: f64, time_sig: TimeSignature) -> Option<String> {
    const EPS: f64 = 1e-6;
    let denom = f64::from(time_sig.denominator);
    for value in [1u32, 2, 4, 8, 16, 32] {
        let base = denom / f64::from(value);
        // (beats for this form, suffix after the number)
        let candidates = [
            (base, ""),
            (base * 1.5, "."),
            (base * 2.0 / 3.0, "t"),
            (base * 1.5 * 2.0 / 3.0, "t."),
        ];
        for (cand, suffix) in candidates {
            if (cand - beats).abs() < EPS {
                return Some(format!("_{value}{suffix}"));
            }
        }
    }
    None
}

/// Resolve a `()` chord group's *target duration* in beats, starting at byte
/// index `after_paren` (the position just past the closing `)`). The target is,
/// in priority order:
///
/// 1. an attached lily suffix — `)_4`, `)_8t`, `)_2.`
/// 2. a slash run — `)//`, `) //`, `)/.` (each slash = one beat, `.` dots it)
/// 3. otherwise one full measure (`beats_per_measure`).
///
/// Returns the target beats and the byte index where parsing should resume
/// (past any consumed suffix/slashes). A slash run immediately followed by an
/// alphanumeric/`#` is a floating slash-bass (`/E`, `/3`), not a target, and is
/// left for the main parser.
fn group_target_beats(
    line: &str,
    after_paren: usize,
    beats_per_measure: f64,
    time_sig: TimeSignature,
) -> Result<(f64, usize), String> {
    let bytes = line.as_bytes();

    // 1. Attached lily suffix: `_4`, `_8t`, `_2.`
    if bytes.get(after_paren) == Some(&b'_') {
        let num_start = after_paren + 1;
        let mut k = num_start;
        while k < bytes.len() && bytes[k].is_ascii_digit() {
            k += 1;
        }
        let value: f64 = line[num_start..k]
            .parse()
            .map_err(|_| "Expected a duration after '_' on a chord group".to_string())?;
        let mut triplet = false;
        let mut dotted = false;
        if bytes.get(k) == Some(&b't') {
            triplet = true;
            k += 1;
        }
        if bytes.get(k) == Some(&b'.') {
            dotted = true;
            k += 1;
        }
        let mut beats = f64::from(time_sig.denominator) / value;
        if triplet {
            beats *= 2.0 / 3.0;
        }
        if dotted {
            beats *= 1.5;
        }
        return Ok((beats, k));
    }

    // 2. Slash run (allow whitespace between `)` and the slashes).
    let mut k = after_paren;
    while matches!(bytes.get(k), Some(&b' ') | Some(&b'\t')) {
        k += 1;
    }
    if bytes.get(k) == Some(&b'/') {
        let mut slashes = 0u32;
        let mut dotted = false;
        while k < bytes.len() {
            match bytes[k] {
                b'/' => {
                    slashes += 1;
                    k += 1;
                }
                b'.' => {
                    dotted = true;
                    k += 1;
                }
                _ => break,
            }
        }
        let next_is_token_char = bytes
            .get(k)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'#');
        if slashes > 0 && !next_is_token_char {
            let mut beats = f64::from(slashes);
            if dotted {
                beats *= 1.5;
            }
            return Ok((beats, k));
        }
    }

    // 3. Default: one full measure. Consume nothing.
    Ok((beats_per_measure, after_paren))
}

/// Expand `()` rhythm groups into explicit per-chord lily durations.
///
/// A `(a b c …)` group has a *target duration* (see [`group_target_beats`])
/// divided equally among its N inner chords. Each chord is rewritten with the
/// lily token for `target / N`, so the existing chord-line parser renders the
/// division — including triplets — with no further special-casing:
/// ```text
///   (C G)       -> C_2 G_2            two half notes
///   (D Em G)    -> D_2t Em_2t G_2t    half-note triplet over the bar
///   (D Em)//    -> D_4 Em_4           group = 2 beats, one each
///   (D Em G)_4  -> D_8t Em_8t G_8t    eighth-note triplet over a quarter
/// ```
/// Quoted text and `m{…}` melody blocks are copied verbatim, so their
/// parentheses (e.g. melody octaves) are left untouched. Note: byte offsets of
/// expanded lines no longer map 1:1 to the source, so source spans on a line
/// that contains a group are approximate.
fn expand_chord_groups(line: &str, time_sig: TimeSignature) -> Result<String, String> {
    if !line.contains('(') {
        return Ok(line.to_string());
    }
    let beats_per_measure = f64::from(time_sig.numerator);
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = line[i..].chars().next().unwrap();

        // Copy quoted strings verbatim.
        if ch == '"' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // include the closing quote
            }
            out.push_str(&line[start..i]);
            continue;
        }

        // Copy `m{…}` melody blocks verbatim (track brace depth).
        if ch == 'm' && bytes.get(i + 1) == Some(&b'{') {
            let start = i;
            i += 2;
            let mut depth = 1u32;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            out.push_str(&line[start..i]);
            continue;
        }

        if ch == '(' {
            let inner_start = i + 1;
            let mut j = inner_start;
            let mut depth = 1u32;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                return Err("Unclosed '(' in chord rhythm group".to_string());
            }
            let elems: Vec<&str> = line[inner_start..j].split_whitespace().collect();
            if elems.is_empty() {
                return Err("Empty '()' chord rhythm group".to_string());
            }
            let (target_beats, resume) =
                group_target_beats(line, j + 1, beats_per_measure, time_sig)?;
            let each = target_beats / elems.len() as f64;
            let suffix = beats_to_lily_suffix(each, time_sig).ok_or_else(|| {
                format!(
                    "Chord group of {} chords cannot be evenly notated over {} beat(s); \
                     supported divisions are powers of two and triplets",
                    elems.len(),
                    target_beats
                )
            })?;
            for (idx, e) in elems.iter().enumerate() {
                if idx > 0 {
                    out.push(' ');
                }
                out.push_str(e);
                out.push_str(&suffix);
            }
            i = resume;
            continue;
        }

        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

#[cfg(test)]
mod span_helper_tests {
    use super::tokenize_with_spans;

    #[test]
    fn tokenize_with_spans_matches_split_whitespace() {
        let line = "  G_2  C_2 |  Am ";
        let spans = tokenize_with_spans(line);
        let toks: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(spans.len(), toks.len());
        for (span, tok) in spans.iter().zip(toks.iter()) {
            assert_eq!(&line[span.as_range()], *tok);
            assert_eq!(span.len, tok.len());
        }
    }

    #[test]
    fn tokenize_with_spans_handles_empty_and_whitespace_only() {
        assert!(tokenize_with_spans("").is_empty());
        assert!(tokenize_with_spans("   \t  ").is_empty());
    }

    #[test]
    fn tokenize_with_spans_keeps_quoted_text_together() {
        let line = r#"| "Ac. Gtr. groove" A |"#;
        let spans = tokenize_with_spans(line);
        let toks: Vec<&str> = spans.iter().map(|span| &line[span.as_range()]).collect();
        assert_eq!(toks, vec!["|", r#""Ac. Gtr. groove""#, "A", "|"]);
    }
}
