//! Per-line error recovery.
//!
//! When the whole-document parser fails on a single line, we segment the
//! input by blank lines (which are natural section boundaries) and try each
//! segment independently. Failing segments become diagnostics; healthy ones
//! still contribute to the resulting `Chart`.
//!
//! This is a *minimum-viable* recovery layer. A future rewrite onto a
//! parser-combinator (chumsky) or a lossless CST (rowan) will replace it
//! with proper synchronization-point recovery, but the call site in
//! `analyze` is stable.

use super::{Diagnostic, Severity};
use crate::chart::{Chart, parse_chart};
use crate::parsing::TextSpan;

/// Parse `text` line-by-line, accumulating diagnostics for failed segments.
///
/// Strategy: split the input on blank lines (which mark section boundaries
/// in keyflow), parse each chunk independently, merge the chunks that
/// parsed into a single `Chart`. Chunks that fail are reported as a single
/// diagnostic spanning the whole failing chunk.
pub(super) fn parse_with_recovery(text: &str, diagnostics: &mut Vec<Diagnostic>) -> Chart {
    let chunks = split_into_chunks(text);

    let mut merged = Chart::new();
    let mut any_chunk_parsed = false;

    for chunk in chunks {
        if chunk.text.trim().is_empty() {
            continue;
        }
        match parse_chart(chunk.text) {
            Ok(c) => {
                if !any_chunk_parsed {
                    merged = c;
                    any_chunk_parsed = true;
                } else {
                    merge_into(&mut merged, c);
                }
            }
            Err(e) => {
                // Trim trailing newlines from the reported range so the
                // squiggle hugs the actual content.
                let mut len = chunk.text.len();
                while len > 0
                    && matches!(chunk.text.as_bytes()[len - 1], b'\n' | b'\r' | b' ' | b'\t')
                {
                    len -= 1;
                }
                let range = TextSpan::new(chunk.start, len.max(1));
                diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "kf001-parse-failed",
                    e,
                    range,
                ));
            }
        }
    }

    merged
}

#[derive(Debug)]
struct Chunk<'a> {
    start: usize,
    text: &'a str,
}

/// Split text on **blank lines** while preserving original byte offsets.
///
/// A "blank line" is one whose content (excluding the trailing `\n`) is
/// either empty or whitespace-only.
fn split_into_chunks(text: &str) -> Vec<Chunk<'_>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut chunk_start = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        // Find the end of the current line.
        let line_end = match memchr(b'\n', &bytes[i..]) {
            Some(rel) => i + rel,
            None => bytes.len(),
        };
        let line = &text[i..line_end];
        let is_blank = line.trim().is_empty();
        let next_after_nl = if line_end < bytes.len() {
            line_end + 1
        } else {
            line_end
        };

        if is_blank && i > chunk_start {
            // Emit chunk [chunk_start, line_end).
            out.push(Chunk {
                start: chunk_start,
                text: &text[chunk_start..line_end],
            });
            chunk_start = next_after_nl;
        }
        i = next_after_nl;
    }
    if chunk_start < bytes.len() {
        out.push(Chunk {
            start: chunk_start,
            text: &text[chunk_start..],
        });
    }
    out
}

/// Tiny `memchr` shim — the standard library version requires a const
/// generic byte slice in older toolchains; keep this self-contained.
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

/// Merge `src` chart into `dst`. Conservative: appends sections, key
/// changes, and time-signature changes; leaves the head metadata of `dst`
/// alone unless it was empty.
fn merge_into(dst: &mut Chart, src: Chart) {
    dst.sections.extend(src.sections);
    dst.key_changes.extend(src.key_changes);
    dst.time_signature_changes
        .extend(src.time_signature_changes);
    dst.tempo_changes.extend(src.tempo_changes);

    if dst.current_key.is_none() {
        dst.current_key = src.current_key;
    }
    if dst.initial_key.is_none() {
        dst.initial_key = src.initial_key;
    }
    if dst.tempo.is_none() {
        dst.tempo = src.tempo;
    }
    if dst.time_signature.is_none() {
        dst.time_signature = src.time_signature;
    }
    if dst.initial_time_signature.is_none() {
        dst.initial_time_signature = src.initial_time_signature;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_into_chunks_handles_blank_separator() {
        let text = "VS 1: | 1 |\n\nCH 1: | 4 |\n";
        let chunks = split_into_chunks(text);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].text.starts_with("VS"));
        assert!(chunks[1].text.starts_with("CH"));
        // Second chunk's start offset must point at the 'C' of "CH".
        assert_eq!(&text[chunks[1].start..chunks[1].start + 2], "CH");
    }

    #[test]
    fn split_into_chunks_no_blank_lines() {
        let text = "VS 1: | 1 |\nCH 1: | 4 |";
        let chunks = split_into_chunks(text);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn recovery_returns_a_chart_for_blank_separated_input() {
        // Two valid chunks separated by a blank line — both should land in the
        // merged chart and no diagnostics should be emitted.
        let text = "VS 1: | 1 4 5 1 |\n\nCH 1: | 4 1 5 1 |\n";
        let mut diags = Vec::new();
        let chart = parse_with_recovery(text, &mut diags);
        assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
        assert!(
            !chart.sections.is_empty(),
            "expected sections from a valid input"
        );
    }
}
