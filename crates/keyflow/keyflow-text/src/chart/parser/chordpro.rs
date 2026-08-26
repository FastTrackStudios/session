//! ChordPro format parser — delegates to [`keyflow_chordpro`].
//!
//! The legacy AST in `keyflow_proto::chord::chordpro` is now a *view* over
//! the comprehensive ChordPro 6.07 AST defined in the standalone
//! `keyflow-chordpro` crate. This file is the bridge: it parses with the
//! new engine and projects the result into the legacy types so the 70+
//! call sites that consume [`ChordProDocument`] keep working.
//!
//! New code should depend on `keyflow-chordpro` directly to access the
//! richer AST (typed [`DirectiveKind`], `[*annotation]` markers,
//! `{define}` chord definitions, conditional `-selector` directives,
//! line continuation, `\u` escapes, …).
//!
//! [`DirectiveKind`]: keyflow_chordpro::DirectiveKind

use crate::chord::{
    ChordProChunk, ChordProDirective, ChordProDocument, ChordProLine, ChordProSection,
};
use keyflow_chordpro::{Directive, DirectiveKind, Document as KcDocument, Environment, Line};

/// Parse ChordPro 6.07 input into the (legacy) `ChordProDocument` view.
///
/// Errors from the underlying parser are converted into `Err(String)` for
/// backwards compatibility. Use [`keyflow_chordpro::parse`] directly when
/// the richer error type or the new AST is needed.
pub fn parse_chordpro(input: &str) -> Result<ChordProDocument, String> {
    let kc = keyflow_chordpro::parse(input).map_err(|e| e.to_string())?;
    Ok(legacy_from(kc))
}

/// Direct re-export of the comprehensive parser's output, for callers that
/// want the full AST without going through the legacy projection.
pub use keyflow_chordpro::parse as parse_chordpro_v2;

/// Project the new AST onto the legacy `ChordProDocument` shape.
fn legacy_from(kc: KcDocument) -> ChordProDocument {
    let mut out = ChordProDocument::new();

    // Directives that come *before* the first environment land in the
    // top-level `directives` list. Once we enter an environment, lines
    // collect into the current `ChordProSection`.
    let mut current_section: Option<ChordProSection> = None;
    let mut implicit_section: Option<ChordProSection> = None;

    for line in kc.lines.iter() {
        match line {
            Line::Directive(d) => match &d.kind {
                DirectiveKind::StartOfEnvironment { env, label } => {
                    finish_section(&mut out, &mut current_section, &mut implicit_section);
                    let mut s = ChordProSection::new();
                    s.label = label.clone().or_else(|| Some(env_label(*env).to_string()));
                    current_section = Some(s);
                }
                DirectiveKind::EndOfEnvironment { .. } => {
                    if let Some(s) = current_section.take() {
                        out.sections.push(s);
                    }
                }
                _ => {
                    // Top-level directives (before any environment) feed the
                    // top-level `directives` vec; directives inside an
                    // environment surface as comments on the current
                    // section so existing renderers don't drop them.
                    if let Some(legacy) = legacy_directive(d) {
                        if current_section.is_some() {
                            // Emit as a comment line inside the current section
                            // to preserve the original behavior.
                            if let Some(s) = current_section.as_mut() {
                                s.lines.push(ChordProLine::Comment(format!(
                                    "{{{}: {}}}",
                                    legacy.name, legacy.value
                                )));
                            }
                        } else {
                            out.directives.push(legacy);
                        }
                    }
                }
            },
            Line::Lyric { chunks, .. } => {
                let legacy_chunks: Vec<ChordProChunk> = chunks
                    .iter()
                    .filter_map(|c| {
                        // Drop annotation-only empty chunks; they're
                        // metadata, not lyric content.
                        if c.text.is_empty() && c.chord.is_none() {
                            return None;
                        }
                        Some(ChordProChunk {
                            chord: c.chord.clone(),
                            text: c.text.clone(),
                        })
                    })
                    .collect();
                let line_obj = if legacy_chunks.is_empty() {
                    ChordProLine::Empty
                } else if legacy_chunks.iter().all(|c| c.text.is_empty()) {
                    ChordProLine::ChordOnly(legacy_chunks)
                } else {
                    ChordProLine::Lyric(legacy_chunks)
                };
                push_into_active_section(&mut current_section, &mut implicit_section, line_obj);
            }
            Line::HashComment { text, .. } => {
                push_into_active_section(
                    &mut current_section,
                    &mut implicit_section,
                    ChordProLine::Comment(text.clone()),
                );
            }
            Line::Empty { .. } => {
                push_into_active_section(
                    &mut current_section,
                    &mut implicit_section,
                    ChordProLine::Empty,
                );
            }
        }
    }

    finish_section(&mut out, &mut current_section, &mut implicit_section);
    out
}

fn push_into_active_section(
    current: &mut Option<ChordProSection>,
    implicit: &mut Option<ChordProSection>,
    line: ChordProLine,
) {
    if let Some(s) = current.as_mut() {
        s.lines.push(line);
        return;
    }
    if implicit.is_none() {
        *implicit = Some(ChordProSection::new());
    }
    if let Some(s) = implicit.as_mut() {
        s.lines.push(line);
    }
}

fn finish_section(
    out: &mut ChordProDocument,
    current: &mut Option<ChordProSection>,
    implicit: &mut Option<ChordProSection>,
) {
    if let Some(s) = implicit.take() {
        out.sections.push(s);
    }
    if let Some(s) = current.take() {
        out.sections.push(s);
    }
}

fn env_label(env: Environment) -> &'static str {
    match env {
        Environment::Verse => "Verse",
        Environment::Chorus => "Chorus",
        Environment::Bridge => "Bridge",
        Environment::Tab => "Tab",
        Environment::Grid => "Grid",
        Environment::Section => "Section",
    }
}

fn legacy_directive(d: &Directive) -> Option<ChordProDirective> {
    use DirectiveKind::*;
    let value: String = match &d.kind {
        Title(s) | Subtitle(s) | Comment(s) | CommentBox(s) | CommentItalic(s) | Highlight(s)
        | PageType(s) | TitlesFlush(s) | Diagrams(s) => s.clone(),
        Meta(m) => m.value.clone(),
        Style { value, .. } => value.clone(),
        Custom { value, .. } => value.clone().unwrap_or_default(),
        Columns(n) => n.to_string(),
        Transpose(n) => n.to_string(),
        ChorusRecall { label } => label.clone().unwrap_or_default(),
        // Boolean / structural directives become flag-style entries with empty
        // value; consumers that need richer data should switch to the new AST.
        NewPage | NewPhysicalPage | NewSong { .. } | ColumnBreak => String::new(),
        Define { def, .. } => def.name.clone(),
        Image(kvs) => kvs
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    k.clone()
                } else {
                    format!("{}={}", k, v)
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        // Environments are consumed at the section boundary.
        StartOfEnvironment { .. } | EndOfEnvironment { .. } => return None,
    };
    Some(ChordProDirective::new(d.name(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_directives_and_sections() {
        let input = "{title: Test}\n{soc}\n[C]Hello\n{eoc}\n";
        let doc = parse_chordpro(input).unwrap();
        assert_eq!(doc.directives.len(), 1);
        assert_eq!(doc.directives[0].name, "title");
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].label.as_deref(), Some("Chorus"));
    }

    #[test]
    fn delegates_to_v2_aliases_and_meta() {
        let doc = parse_chordpro("{t: Hi}\n{artist: Trad}\n").unwrap();
        assert!(doc
            .directives
            .iter()
            .any(|d| d.name == "title" && d.value == "Hi"));
        assert!(doc
            .directives
            .iter()
            .any(|d| d.name == "artist" && d.value == "Trad"));
    }

    #[test]
    fn preserves_lyric_chunks() {
        let doc = parse_chordpro("[C]Amazing [G]grace").unwrap();
        let line = &doc.sections[0].lines[0];
        match line {
            ChordProLine::Lyric(chunks) => {
                assert_eq!(chunks.len(), 2);
                assert_eq!(chunks[0].chord.as_deref(), Some("C"));
                assert_eq!(chunks[0].text, "Amazing ");
                assert_eq!(chunks[1].chord.as_deref(), Some("G"));
                assert_eq!(chunks[1].text, "grace");
            }
            other => panic!("expected lyric, got {:?}", other),
        }
    }
}
