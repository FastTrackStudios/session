//! ChordPro 6.07 parser.
//!
//! See the [crate docs](crate) for an overview. The parser performs:
//!
//! 1. **Pre-pass**: line continuation (`\` at EOL) and `\uXXXX` escape
//!    expansion, while keeping a byte map back to the original source so
//!    `Span`s point at the user's text.
//! 2. **Line walk**: dispatch on first non-whitespace character to
//!    directive / lyric / hash-comment / blank.
//! 3. **Directive parse**: split on `:` / whitespace, normalize aliases,
//!    detect `-selector` conditional suffix, classify into a typed
//!    [`DirectiveKind`].
//! 4. **Lyric parse**: walk the line collecting `[chord]` / `[*annotation]`
//!    markers and the text between them.
//!
//! Unrecognized directives become [`DirectiveKind::Custom`] so user `x_*`
//! and forward-compat directives still round-trip.

use crate::ast::*;

/// Knobs for the parser. All defaults match ChordPro 6.07 semantics.
#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Expand `\uXXXX` Unicode escapes (default `true`).
    pub expand_unicode_escapes: bool,
    /// Honor trailing `\` line continuations (default `true`).
    pub allow_line_continuations: bool,
    /// Treat lines starting with `#` as comments (default `true`).
    pub allow_hash_comments: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            expand_unicode_escapes: true,
            allow_line_continuations: true,
            allow_hash_comments: true,
        }
    }
}

/// Parse a ChordPro document with default options.
pub fn parse(input: &str) -> Result<Document, ParseError> {
    parse_with_options(input, &ParseOptions::default())
}

/// Parse a ChordPro document, configurable.
pub fn parse_with_options(input: &str, opts: &ParseOptions) -> Result<Document, ParseError> {
    // The pre-pass produces a logical (continued + escape-expanded) string,
    // plus a byte map: each byte in the logical buffer maps to a byte index
    // in `input` for accurate spans. When mapping isn't 1:1 (escapes), the
    // map points at the start of the original sequence.
    let pre = preprocess(input, opts);

    let mut doc = Document::new();
    let bytes = pre.text.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let line_end = find_byte(bytes, i, b'\n').unwrap_or(bytes.len());
        let line = &pre.text[i..line_end];

        let original_start = pre.map.get(i).copied().unwrap_or(input.len());
        let original_end = if line_end == 0 || line_end > pre.map.len() {
            input.len()
        } else if line_end == bytes.len() {
            input.len()
        } else {
            pre.map[line_end]
        };
        let line_span = Span::new(original_start, original_end - original_start);

        if let Some(parsed) = parse_one_line(line, line_span, opts) {
            doc.lines.push(parsed);
        }

        i = if line_end == bytes.len() {
            bytes.len()
        } else {
            line_end + 1
        };
    }

    Ok(doc)
}

fn parse_one_line(line: &str, line_span: Span, opts: &ParseOptions) -> Option<Line> {
    let trimmed = line.trim_start();
    let trim_offset = line.len() - trimmed.len();

    if trimmed.is_empty() {
        return Some(Line::Empty { span: line_span });
    }

    if opts.allow_hash_comments && trimmed.starts_with('#') {
        return Some(Line::HashComment {
            text: trimmed[1..].trim_start().to_string(),
            span: line_span,
        });
    }

    if let Some(d) = try_parse_directive(trimmed, line_span, trim_offset) {
        return Some(Line::Directive(d));
    }

    if let Some(d) = try_parse_plain_heading(trimmed, line_span, trim_offset) {
        return Some(Line::Directive(d));
    }

    let chunks = parse_lyric_chunks(line, line_span);
    Some(Line::Lyric {
        chunks,
        span: line_span,
    })
}

// ---------------- Directive parsing ----------------

fn try_parse_directive(line: &str, line_span: Span, trim_offset: usize) -> Option<Directive> {
    if !line.starts_with('{') {
        return None;
    }
    let close = line.find('}')?;
    let body = &line[1..close];

    // Split on the first ':' OR whitespace boundary between name and value.
    // Keep the original separators in mind to preserve the directive shape:
    //   `{title: My Song}`     → name="title", value="My Song"
    //   `{soc}`                → name="soc", value=""
    //   `{soc Verse 1}`        → name="soc", value="Verse 1"
    //   `{title-en: Hello}`    → name="title", condition="en", value="Hello"
    let (name_raw, value_raw) = split_directive_body(body);
    let value = unquote(value_raw.trim());

    let (name_lower, condition) = split_conditional(name_raw);

    let span = Span::new(line_span.start + trim_offset, close + 1);
    let kind = classify_directive(&name_lower, &value);

    Some(Directive {
        kind,
        condition,
        span,
    })
}

fn try_parse_plain_heading(line: &str, line_span: Span, trim_offset: usize) -> Option<Directive> {
    let (name_raw, value_raw) = line.split_once(':')?;
    if name_raw.contains('[') || name_raw.contains(']') {
        return None;
    }

    let name = name_raw.trim();
    if name.is_empty() {
        return None;
    }

    let span = Span::new(line_span.start + trim_offset, line.len());

    if let Some(kind) = classify_plain_section_heading(name) {
        return Some(Directive {
            kind,
            condition: None,
            span,
        });
    }

    let normalized = normalize_plain_metadata_name(name)?;
    let mut value = value_raw.trim().to_string();
    if normalized == "key" {
        value = value
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(&value)
            .to_string();
    }

    let kind = match normalized.as_str() {
        "title" => DirectiveKind::Title(value),
        "subtitle" => DirectiveKind::Subtitle(value),
        item => DirectiveKind::Meta(MetaItem {
            item: item.to_string(),
            value,
        }),
    };

    Some(Directive {
        kind,
        condition: None,
        span,
    })
}

fn normalize_plain_metadata_name(name: &str) -> Option<String> {
    let normalized = name.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    match normalized.as_str() {
        "title" | "artist" | "key" | "original_key" | "book" | "subtitle" | "composer"
        | "copyright" | "year" | "tempo" | "time" | "capo" => Some(normalized),
        _ => None,
    }
}

fn classify_plain_section_heading(name: &str) -> Option<DirectiveKind> {
    let first = name.split_whitespace().next()?.to_ascii_lowercase();
    let env = match first.as_str() {
        "verse" => Environment::Verse,
        "chorus" => Environment::Chorus,
        "bridge" => Environment::Bridge,
        "intro" | "interlude" | "instrumental" | "inst" | "outro" | "solo" | "vamp" => {
            Environment::Section
        }
        _ => return None,
    };

    Some(DirectiveKind::StartOfEnvironment {
        env,
        label: Some(format!("{name} sync=lines")),
    })
}

fn split_directive_body(body: &str) -> (String, &str) {
    // Find the earliest of `:` or unquoted whitespace and split there.
    for (i, c) in body.char_indices() {
        if c == ':' {
            return (body[..i].trim().to_lowercase(), &body[i + 1..]);
        }
        if c.is_whitespace() {
            return (body[..i].trim().to_lowercase(), &body[i..]);
        }
    }
    (body.trim().to_lowercase(), "")
}

fn split_conditional(name: String) -> (String, Option<String>) {
    if let Some((base, sel)) = name.split_once('-') {
        // ChordPro reserves a few directive names that *contain* hyphens by
        // design (none today, but `start_of_*` uses underscores). The
        // conditional rule is: the suffix after `-` is the selector.
        return (base.to_string(), Some(sel.to_string()));
    }
    (name, None)
}

/// Strip a single layer of `"…"` or `'…'` quotes if present.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn classify_directive(name: &str, value: &str) -> DirectiveKind {
    use DirectiveKind::*;

    // Aliases.
    let canonical = canonicalize_directive(name);
    let n = canonical.as_str();

    // Environments. After canonicalization, all aliased forms (`soc`, `eoc`,
    // …) become `start_of_*` / `end_of_*`, so checking the canonical name is
    // sufficient.
    if let Some(env) = n.strip_prefix("start_of_").and_then(Environment::from_name) {
        return StartOfEnvironment {
            env,
            label: if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            },
        };
    }
    if let Some(env) = n.strip_prefix("end_of_").and_then(Environment::from_name) {
        return EndOfEnvironment { env };
    }

    // Recall (special: `chorus` w/ no value, or `chorus: Label`).
    if n == "chorus" {
        return ChorusRecall {
            label: if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            },
        };
    }

    // Pagination.
    match n {
        "new_page" | "np" => return NewPage,
        "new_physical_page" | "npp" => return NewPhysicalPage,
        "column_break" | "cb" => return ColumnBreak,
        _ => {}
    }
    if n == "new_song" || n == "ns" {
        return NewSong { toc: None };
    }
    if n == "columns" || n == "col" {
        return Columns(value.parse().unwrap_or(1));
    }
    if n == "pagetype" {
        return PageType(value.to_string());
    }

    // Text/metadata.
    match n {
        "title" | "t" => return Title(value.to_string()),
        "subtitle" | "st" => return Subtitle(value.to_string()),
        "comment" | "c" => return Comment(value.to_string()),
        "comment_box" => return CommentBox(value.to_string()),
        "comment_italic" | "ci" => return CommentItalic(value.to_string()),
        "highlight" => return Highlight(value.to_string()),
        _ => {}
    }
    if n == "meta" {
        return parse_meta_value(value);
    }
    // The cheat sheet's `meta album name` is also reachable via `{album: name}`.
    if matches!(
        n,
        "album"
            | "artist"
            | "capo"
            | "composer"
            | "copyright"
            | "duration"
            | "key"
            | "lyricist"
            | "sortartist"
            | "sorttitle"
            | "tag"
            | "tempo"
            | "time"
            | "year"
    ) {
        return Meta(MetaItem {
            item: n.to_string(),
            value: value.to_string(),
        });
    }

    // Chord definitions.
    if n == "define" || n == "chord" {
        return Define {
            def: parse_chord_definition(value),
            is_define: n == "define",
        };
    }
    if n == "diagrams" || n == "grid" || n == "g" || n == "no_grid" || n == "ng" {
        let v = match n {
            "grid" | "g" => "on".to_string(),
            "no_grid" | "ng" => "off".to_string(),
            _ => value.to_string(),
        };
        return Diagrams(v);
    }

    // Transpose.
    if n == "transpose" {
        return Transpose(value.parse().unwrap_or(0));
    }

    // Images.
    if n == "image" {
        return Image(parse_kv_pairs(value));
    }

    // Title flush.
    if n == "titles" {
        return TitlesFlush(value.to_string());
    }

    // Generic styling — anything ending in `font`/`size`/`colour`/`color`.
    if n.ends_with("font") || n.ends_with("size") || n.ends_with("colour") || n.ends_with("color") {
        return Style {
            name: n.to_string(),
            value: value.to_string(),
        };
    }

    // Custom (`x_*`) and unknown.
    Custom {
        name: n.to_string(),
        value: if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        },
    }
}

fn canonicalize_directive(name: &str) -> String {
    match name {
        "t" => "title",
        "st" => "subtitle",
        "c" => "comment",
        "cb" => "column_break",
        "ci" => "comment_italic",
        "cf" => "chordfont",
        "cs" => "chordsize",
        "tf" => "textfont",
        "ts" => "textsize",
        "ns" => "new_song",
        "np" => "new_page",
        "npp" => "new_physical_page",
        "col" => "columns",
        "soc" => "start_of_chorus",
        "eoc" => "end_of_chorus",
        "sov" => "start_of_verse",
        "eov" => "end_of_verse",
        "sob" => "start_of_bridge",
        "eob" => "end_of_bridge",
        "sot" => "start_of_tab",
        "eot" => "end_of_tab",
        "sog" => "start_of_grid",
        "eog" => "end_of_grid",
        "g" => "grid",
        "ng" => "no_grid",
        other => other,
    }
    .to_string()
}

/// Parse `{meta album name}` body or `{meta album: name}`.
fn parse_meta_value(value: &str) -> DirectiveKind {
    let v = value.trim();
    let (item, val) = if let Some((a, b)) = v.split_once(':') {
        (a.trim().to_string(), b.trim().to_string())
    } else if let Some((a, b)) = v.split_once(char::is_whitespace) {
        (a.trim().to_string(), b.trim().to_string())
    } else {
        (v.to_string(), String::new())
    };
    DirectiveKind::Meta(MetaItem { item, value: val })
}

fn parse_chord_definition(value: &str) -> ChordDefinition {
    // Body is `name [base-fret N] [frets …] [fingers …] [keys …]
    //          [display …] [diagram …] [format …]`. Tokenize on whitespace
    // honoring quoted strings.
    let tokens = tokenize_args(value);
    let mut def = ChordDefinition::default();
    let mut i = 0;
    if let Some(first) = tokens.first() {
        if !is_section_keyword(first) {
            def.name = first.clone();
            i = 1;
        }
    }
    while i < tokens.len() {
        let key = tokens[i].as_str();
        i += 1;
        // Collect the remaining-of-this-section args until the next section
        // keyword or end.
        let mut acc: Vec<String> = Vec::new();
        while i < tokens.len() && !is_section_keyword(&tokens[i]) {
            acc.push(tokens[i].clone());
            i += 1;
        }
        match key {
            "base-fret" => def.base_fret = acc.first().and_then(|s| s.parse().ok()),
            "frets" => def.frets = Some(acc),
            "fingers" => def.fingers = Some(acc),
            "keys" => def.keys = Some(acc),
            "display" => def.display = Some(acc.join(" ")),
            "diagram" => def.diagram = Some(acc.join(" ")),
            "format" => def.format = Some(acc.join(" ")),
            other => def.extra.push((other.to_string(), acc.join(" "))),
        }
    }
    def
}

fn is_section_keyword(s: &str) -> bool {
    matches!(
        s,
        "base-fret" | "frets" | "fingers" | "keys" | "display" | "diagram" | "format"
    )
}

fn parse_kv_pairs(value: &str) -> Vec<(String, String)> {
    // `image` directives use `key=value` pairs separated by whitespace, with
    // quoted values supported.
    let mut out = Vec::new();
    let toks = tokenize_args(value);
    for tok in toks {
        if let Some((k, v)) = tok.split_once('=') {
            out.push((k.to_string(), unquote(v)));
        } else {
            // bare `border` flag etc.
            out.push((tok, String::new()));
        }
    }
    out
}

fn tokenize_args(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;
    for ch in value.chars() {
        match (in_quote, ch) {
            (Some(q), c) if c == q => {
                in_quote = None;
                cur.push(c);
            }
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                in_quote = Some(ch);
                cur.push(ch);
            }
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ---------------- Lyric parsing ----------------

fn parse_lyric_chunks(line: &str, line_span: Span) -> Vec<ChordChunk> {
    let mut out: Vec<ChordChunk> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut current = ChordChunk::default();
    current.span = Span::new(line_span.start, 0);

    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Flush any pending text into a chunk.
            if !current.text.is_empty() || current.chord.is_some() || current.annotation.is_some() {
                let chunk_len = i - (current.span.start - line_span.start);
                current.span.len = chunk_len;
                out.push(std::mem::take(&mut current));
                current = ChordChunk::default();
                current.span = Span::new(line_span.start + i, 0);
            }
            // Find matching ']'.
            let close_rel = find_byte(bytes, i + 1, b']');
            let Some(close) = close_rel else {
                // Unclosed bracket — treat the rest as text.
                current.text.push_str(&line[i..]);
                break;
            };
            let inner = &line[i + 1..close];
            if let Some(rest) = inner.strip_prefix('*') {
                current.annotation = Some(Annotation {
                    text: rest.to_string(),
                    span: Span::new(line_span.start + i, close - i + 1),
                });
            } else {
                current.chord = Some(inner.to_string());
            }
            i = close + 1;
            continue;
        }
        // Plain text byte. Push the next codepoint.
        let next = next_char_boundary(line, i);
        current.text.push_str(&line[i..next]);
        i = next;
    }

    if !current.text.is_empty() || current.chord.is_some() || current.annotation.is_some() {
        let chunk_len = i - (current.span.start - line_span.start);
        current.span.len = chunk_len;
        out.push(current);
    }
    out
}

// ---------------- Pre-processing (line continuation + escapes) ----------------

struct Pre {
    text: String,
    /// `map[i]` = byte offset in original `input` corresponding to byte `i`
    /// in `text`. Length `text.len() + 1` (the trailing entry maps to the
    /// end of input).
    map: Vec<usize>,
}

fn preprocess(input: &str, opts: &ParseOptions) -> Pre {
    let mut text = String::with_capacity(input.len());
    let mut map = Vec::with_capacity(input.len() + 1);
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Line continuation: backslash immediately before EOL.
        if opts.allow_line_continuations
            && bytes[i] == b'\\'
            && (bytes.get(i + 1) == Some(&b'\n')
                || (bytes.get(i + 1) == Some(&b'\r') && bytes.get(i + 2) == Some(&b'\n')))
        {
            // Skip the backslash + newline; also skip leading whitespace on the next line.
            i += 1;
            // skip \r if present
            if bytes.get(i) == Some(&b'\r') {
                i += 1;
            }
            // skip \n
            if bytes.get(i) == Some(&b'\n') {
                i += 1;
            }
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            continue;
        }

        // \uXXXX unicode escape.
        if opts.expand_unicode_escapes
            && bytes[i] == b'\\'
            && bytes.get(i + 1) == Some(&b'u')
            && i + 6 <= bytes.len()
            && bytes[i + 2..i + 6].iter().all(|b| b.is_ascii_hexdigit())
        {
            let hex = &input[i + 2..i + 6];
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                if let Some(c) = char::from_u32(cp) {
                    let original_offset = i;
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    for _ in s.bytes() {
                        map.push(original_offset);
                    }
                    text.push_str(s);
                    i += 6;
                    continue;
                }
            }
        }

        let ch_end = next_char_boundary(input, i);
        let original_offset = i;
        for _ in 0..(ch_end - i) {
            map.push(original_offset);
        }
        text.push_str(&input[i..ch_end]);
        i = ch_end;
    }
    map.push(input.len());
    Pre { text, map }
}

// ---------------- Tiny utilities ----------------

fn find_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|&b| b == needle)
        .map(|i| i + start)
}

fn next_char_boundary(s: &str, start: usize) -> usize {
    let mut end = start + 1;
    while end < s.len() && !s.is_char_boundary(end) {
        end += 1;
    }
    end.min(s.len())
}
