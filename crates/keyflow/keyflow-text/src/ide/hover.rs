//! Hover info for the token under the cursor.
//!
//! Chord / scale-degree hovers consult the parsed `Chart`: the chord instance
//! under the cursor is resolved against the key **in effect at its position**
//! (`Chart::key_at_position`), so a `5` after a mid-song `#G` key change
//! resolves in G, not the chart's initial key. `$`/`/` tokens and unparsed
//! tokens fall back to the rule-based slice handling.

use crate::chart::{Chart, ChordInstance};
use crate::parsing::TextSpan;

#[derive(Debug, Clone)]
pub struct HoverInfo {
    /// Multi-line markdown body. Renderers should treat the leading line as
    /// a one-liner summary.
    pub markdown: String,
    /// Range of the token the hover applies to.
    pub range: TextSpan,
}

/// Compute hover info at `byte_offset`. Returns `None` if no recognized
/// token sits under the cursor.
#[must_use]
pub fn hover(text: &str, byte_offset: usize, chart: &Chart) -> Option<HoverInfo> {
    let offset = byte_offset.min(text.len());
    let bytes = text.as_bytes();

    // Find the whitespace-bounded token under the cursor.
    let mut start = offset;
    while start > 0 {
        let b = bytes[start - 1];
        if b.is_ascii_whitespace() || b == b'|' {
            break;
        }
        start -= 1;
    }
    let mut end = offset;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_whitespace() || b == b'|' {
            break;
        }
        end += 1;
    }
    if start == end {
        return None;
    }

    let token = &text[start..end];
    let range = TextSpan::new(start, end - start);

    if let Some(name) = token.strip_prefix('$') {
        let body = if let Some(melody) = chart.melody_variables.get(name) {
            format!(
                "**`${}`** — recall stored melody\n\n```\n{}\n```",
                name, melody
            )
        } else {
            format!(
                "**`${}`** — melody-variable recall\n\n_(unknown variable; \
                 define it earlier with `{} = m{{ … }}`)_",
                name, name
            )
        };
        return Some(HoverInfo {
            markdown: body,
            range,
        });
    }

    if let Some(name) = token.strip_prefix('/') {
        return Some(HoverInfo {
            markdown: format!("**`/{}`** — chart command / config directive", name),
            range,
        });
    }

    // Chord / scale-degree resolved against the key IN EFFECT at this position.
    // Key changes can happen in any measure, so look up the chord instance
    // covering the cursor and resolve against `key_at_position` — not the
    // chart-level key. A key-relative symbol (Nashville / Roman) shows its
    // absolute letter-name chord; a note-name chord shows its scale degree.
    if let Some(ci) = chord_at_offset(chart, offset) {
        let span = ci.source_span.unwrap_or(range);
        if let Some(key) = chart
            .key_at_position(&ci.position)
            .or(chart.current_key.as_ref())
        {
            let key_str = format!("{key}");
            let degree = ci
                .parsed
                .root
                .resolve(Some(key))
                .and_then(|n| key.degree_of_note(&n));
            let deg_str = degree
                .map(|d| format!(" · scale degree {d}"))
                .unwrap_or_default();
            let markdown = if let Some(resolved) = ci.resolved_symbol(key) {
                // Key-relative: `5` → `G`, `V7` → `G7`.
                format!(
                    "**`{}`** → **`{}`** in **{}**{}",
                    ci.original_token.trim(),
                    resolved,
                    key_str,
                    deg_str
                )
            } else {
                // Already a note-name chord — show its function in the key.
                format!(
                    "**`{}`** — chord in **{}**{}",
                    ci.full_symbol, key_str, deg_str
                )
            };
            return Some(HoverInfo {
                markdown,
                range: span,
            });
        }
    }

    // Scale-degree number (1..=7) or roman numeral — fallback for tokens not
    // matched to a parsed chord instance above (uses the chart-level key).
    if let Ok(deg) = token.parse::<u8>() {
        if (1..=7).contains(&deg) {
            let key_str = chart
                .current_key
                .as_ref()
                .map(|k| format!("{}", k))
                .unwrap_or_else(|| "C major".into());
            return Some(HoverInfo {
                markdown: format!("**`{}`** — scale degree {} of **{}**", deg, deg, key_str),
                range,
            });
        }
    }

    // Looks like a chord token (starts with a note letter or accidental).
    let first = token.chars().next()?;
    if first.is_ascii_alphabetic() && first.is_ascii_uppercase() {
        return Some(HoverInfo {
            markdown: format!("**`{}`** — chord", token),
            range,
        });
    }

    None
}

/// Find the parsed chord instance whose source span covers `offset`.
///
/// `ChordInstance::source_span` is document-absolute (assigned in
/// `ChartParser::assign_chord_source_spans`), so a direct containment check
/// maps a cursor offset to its chord.
fn chord_at_offset(chart: &Chart, offset: usize) -> Option<&ChordInstance> {
    chart
        .sections
        .iter()
        .flat_map(|section| section.measures())
        .flat_map(|measure| measure.chords.iter())
        .find(|ci| {
            ci.source_span
                .is_some_and(|span| offset >= span.start && offset < span.end())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_on_dollar_token() {
        let chart = Chart::new();
        let h = hover("| 1 $mainRiff |", 7, &chart).expect("hover for $name");
        assert!(h.markdown.contains("$main"));
    }

    #[test]
    fn hover_on_scale_degree() {
        let chart = Chart::new();
        let h = hover("| 4 |", 2, &chart).expect("hover for digit");
        assert!(h.markdown.contains("scale degree"));
    }

    #[test]
    fn hover_returns_none_on_whitespace() {
        let chart = Chart::new();
        assert!(hover("   ", 1, &chart).is_none());
    }

    #[test]
    fn hover_resolves_degree_against_section_key() {
        // `#G` mid-stream changes the key: the first `5` resolves in C → G;
        // the second `5` (after the change) resolves in G → D. Only
        // per-position resolution gives both.
        let text = "120bpm 4/4 #C\n\nvs\n1 5 #G 1 5\n";
        let chart = crate::ide::analyze(text).chart;

        let first5 = text.find('5').expect("first 5");
        let h1 = hover(text, first5, &chart).expect("hover on first 5");
        assert!(
            h1.markdown.contains('G'),
            "first 5 (key C) should resolve to G: {}",
            h1.markdown
        );

        let last5 = text.rfind('5').expect("second 5");
        let h2 = hover(text, last5, &chart).expect("hover on second 5");
        assert!(
            h2.markdown.contains('D'),
            "second 5 (key G after #G) should resolve to D: {}",
            h2.markdown
        );
    }
}
