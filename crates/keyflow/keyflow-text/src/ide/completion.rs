//! Context-aware completion at a cursor position.
//!
//! `complete(text, byte_offset)` inspects the local context around the
//! caret and returns suggestions ranked by relevance. Today this is a
//! lightweight rule-based engine; once the engine moves to a CST-based
//! parser, completions can become semantic (e.g. only sections that exist
//! in the chart, only chord qualities valid for a given chord type).

use crate::chart::Chart;
use crate::parsing::TextSpan;

/// A single completion candidate.
#[derive(Debug, Clone)]
pub struct Completion {
    /// Display label.
    pub label: String,
    /// Categorization (drives the icon in editors).
    pub kind: CompletionKind,
    /// Optional secondary text shown in the popup (e.g. "minor 7th chord").
    pub detail: Option<String>,
    /// Text inserted on accept. If `None`, `label` is inserted verbatim.
    pub insert_text: Option<String>,
    /// Range that the inserted text replaces (the partial token under the
    /// cursor). When `None`, callers should treat it as inserting at the
    /// caret position.
    pub replace_range: Option<TextSpan>,
}

impl Completion {
    fn new(label: impl Into<String>, kind: CompletionKind) -> Self {
        Self {
            label: label.into(),
            kind,
            detail: None,
            insert_text: None,
            replace_range: None,
        }
    }

    fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }

    fn replace(mut self, range: TextSpan) -> Self {
        self.replace_range = Some(range);
        self
    }
}

/// Completion-item categorization. Values mirror LSP's `CompletionItemKind`
/// numeric ordering for convenience in the LSP layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    /// Chord (e.g. `Cmaj7`, `1`).
    Chord,
    /// Chord quality marker (`maj`, `m`, `dim`, `sus`).
    Quality,
    /// Section header (`VS`, `CH`, `IN`, `Bridge`).
    Section,
    /// Slash command (`/fermata`, `/accent`).
    Command,
    /// Melody-variable recall (`$mainRiff`).
    MelodyVar,
    /// Keyword / general suggestion.
    Keyword,
    /// Snippet template.
    Snippet,
}

/// Detected context for the cursor position.
///
/// Public so the LSP layer can switch on it for finer-grained behavior
/// (e.g. emit `triggerCharacters` based on the kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// Cursor sits at the start of a line (column 0 ignoring whitespace).
    LineStart,
    /// Cursor is inside a `$name` melody-variable token.
    MelodyVariable { partial: String, range: TextSpan },
    /// Cursor is inside a `/cmd` command token.
    SlashCommand { partial: String, range: TextSpan },
    /// Cursor is in a chord-line token position (after whitespace, after
    /// `|`, or extending an existing chord token).
    ChordPosition { partial: String, range: TextSpan },
}

/// Compute completions at `byte_offset`.
#[must_use]
pub fn complete(text: &str, byte_offset: usize, chart: &Chart) -> Vec<Completion> {
    let offset = byte_offset.min(text.len());
    let ctx = detect_context(text, offset);

    match ctx {
        CompletionContext::MelodyVariable { partial, range } => {
            complete_melody_variable(chart, &partial, range)
        }
        CompletionContext::SlashCommand { partial, range } => {
            complete_slash_command(&partial, range)
        }
        CompletionContext::LineStart => complete_line_start(),
        CompletionContext::ChordPosition { partial, range } => complete_chord(&partial, range),
    }
}

/// Inspect the byte slice around the caret to classify the cursor context.
pub fn detect_context(text: &str, offset: usize) -> CompletionContext {
    let bytes = text.as_bytes();

    // Walk left to find the start of the current "word" (whitespace-bounded).
    let mut start = offset;
    while start > 0 {
        let b = bytes[start - 1];
        if b.is_ascii_whitespace() || b == b'|' {
            break;
        }
        start -= 1;
    }
    let partial_str = &text[start..offset];
    let range = TextSpan::new(start, offset - start);

    // If the line up to here is empty / whitespace, treat as line-start.
    let line_start_idx = bytes[..offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let line_so_far = &text[line_start_idx..offset];
    if line_so_far.trim().is_empty() {
        return CompletionContext::LineStart;
    }

    if let Some(rest) = partial_str.strip_prefix('$') {
        return CompletionContext::MelodyVariable {
            partial: rest.to_string(),
            range,
        };
    }
    if let Some(rest) = partial_str.strip_prefix('/') {
        return CompletionContext::SlashCommand {
            partial: rest.to_string(),
            range,
        };
    }
    CompletionContext::ChordPosition {
        partial: partial_str.to_string(),
        range,
    }
}

fn complete_melody_variable(chart: &Chart, partial: &str, range: TextSpan) -> Vec<Completion> {
    chart
        .melody_variables
        .iter()
        .filter(|(name, _)| name.starts_with(partial))
        .map(|(name, _)| {
            Completion::new(format!("${}", name), CompletionKind::MelodyVar)
                .detail("Recall stored melody")
                .replace(range)
        })
        .collect()
}

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("fermata", "Hold the previous chord"),
    ("accent", "Accent on the previous chord"),
    ("staccato", "Detached / shortened previous chord"),
    ("marcato", "Strongly marked previous chord"),
    ("stop", "Stop / silence after the previous chord"),
    ("push", "Configure default push amount"),
    ("swing", "Configure swing ratio"),
];

fn complete_slash_command(partial: &str, range: TextSpan) -> Vec<Completion> {
    SLASH_COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(partial))
        .map(|(name, detail)| {
            Completion::new(format!("/{}", name), CompletionKind::Command)
                .detail(*detail)
                .replace(range)
        })
        .collect()
}

const SECTION_HEADERS: &[(&str, &str)] = &[
    ("IN", "Intro"),
    ("VS", "Verse"),
    ("PreCH", "Pre-Chorus"),
    ("CH", "Chorus"),
    ("Bridge", "Bridge"),
    ("Solo", "Solo"),
    ("INST", "Instrumental"),
    ("Interlude", "Interlude"),
    ("Outro", "Outro"),
    ("End", "Ending tag"),
    ("HITS", "Hits / stops"),
];

fn complete_line_start() -> Vec<Completion> {
    SECTION_HEADERS
        .iter()
        .map(|(label, detail)| {
            Completion::new(format!("{}:", label), CompletionKind::Section).detail(*detail)
        })
        .collect()
}

const CHORD_ROOTS: &[&str] = &["C", "D", "E", "F", "G", "A", "B"];
const CHORD_QUALITIES: &[(&str, &str)] = &[
    ("m", "minor"),
    ("maj7", "major 7th"),
    ("m7", "minor 7th"),
    ("7", "dominant 7th"),
    ("dim", "diminished"),
    ("dim7", "diminished 7th"),
    ("m7b5", "half-diminished"),
    ("aug", "augmented"),
    ("sus", "suspended 4th"),
    ("sus2", "suspended 2nd"),
    ("add9", "added 9th"),
    ("9", "dominant 9th"),
    ("maj9", "major 9th"),
    ("m9", "minor 9th"),
    ("11", "dominant 11th"),
    ("13", "dominant 13th"),
];

fn complete_chord(partial: &str, range: TextSpan) -> Vec<Completion> {
    // Empty / single-char partial → suggest roots and scale degrees.
    // Multi-char partial that begins with a root → suggest qualities glued
    // onto that root.
    let mut out = Vec::new();

    if partial.is_empty() {
        for r in CHORD_ROOTS {
            out.push(
                Completion::new(*r, CompletionKind::Chord)
                    .detail(format!("{} major", r))
                    .replace(range),
            );
        }
        for d in 1..=7 {
            out.push(
                Completion::new(d.to_string(), CompletionKind::Chord)
                    .detail("scale degree")
                    .replace(range),
            );
        }
        return out;
    }

    let first = partial.chars().next().unwrap();
    let is_root = CHORD_ROOTS.iter().any(|r| r.starts_with(first));
    if is_root {
        // Pull the root prefix off and suggest qualities glued onto it.
        let mut head_end = 1;
        let bytes = partial.as_bytes();
        if bytes.len() > 1 && (bytes[1] == b'#' || bytes[1] == b'b') {
            head_end = 2;
        }
        let head = &partial[..head_end.min(partial.len())];
        let tail = &partial[head_end.min(partial.len())..];
        for (qual, detail) in CHORD_QUALITIES {
            if qual.starts_with(tail) {
                out.push(
                    Completion::new(format!("{}{}", head, qual), CompletionKind::Quality)
                        .detail(format!("{} chord", detail))
                        .replace(range),
                );
            }
        }
    }

    if out.is_empty() {
        // Generic fallback: roots with the partial as prefix.
        for r in CHORD_ROOTS {
            if r.starts_with(partial) {
                out.push(Completion::new(*r, CompletionKind::Chord).replace(range));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_context_at_line_start() {
        assert_eq!(detect_context("", 0), CompletionContext::LineStart);
        assert_eq!(detect_context("\n", 1), CompletionContext::LineStart);
        assert_eq!(detect_context("   ", 3), CompletionContext::LineStart);
    }

    #[test]
    fn detect_context_for_dollar_token() {
        let text = "| 1 $main";
        let ctx = detect_context(text, text.len());
        match ctx {
            CompletionContext::MelodyVariable { partial, range } => {
                assert_eq!(partial, "main");
                assert_eq!(range.start, 4); // pointing at '$'
                assert_eq!(range.len, 5); // "$main"
            }
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn detect_context_for_slash_command() {
        let text = "Cmaj7 /fer";
        let ctx = detect_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::SlashCommand { .. }));
    }

    #[test]
    fn complete_returns_section_headers_at_line_start() {
        let chart = Chart::new();
        let items = complete("", 0, &chart);
        assert!(items.iter().any(|c| c.label.starts_with("VS")));
        assert!(items.iter().any(|c| c.label.starts_with("CH")));
    }

    #[test]
    fn complete_returns_qualities_after_root() {
        let chart = Chart::new();
        let items = complete("Cm", 2, &chart);
        // Should contain extensions starting with `m`: "Cm7", "Cmaj7", "Cm9", etc.
        assert!(items.iter().any(|c| c.label == "Cm7"));
        assert!(items.iter().any(|c| c.label == "Cmaj7" || c.label == "Cm"));
    }

    #[test]
    fn complete_returns_stored_melody_vars() {
        use crate::chart::Melody;
        let mut chart = Chart::new();
        chart.melody_variables.set(
            "mainRiff".to_string(),
            Melody::parse("C_8 D_8 E_4").unwrap(),
        );
        chart
            .melody_variables
            .set("ohMy".to_string(), Melody::parse("G_4").unwrap());
        let items = complete("$ma", 3, &chart);
        assert!(items.iter().any(|c| c.label == "$mainRiff"));
        assert!(items.iter().all(|c| c.label != "$ohMy"));
    }
}
