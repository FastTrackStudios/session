/**
 * tree-sitter-keyflow
 *
 * Grammar for Keyflow chart notation (`.kf` documents) including embedded
 * ChordPro 6.07 lyric blocks. The grammar is intentionally permissive —
 * tree-sitter is used for syntax highlighting and structural navigation,
 * not strict validation. The `keyflow_text::ide` engine remains the source
 * of truth for diagnostics; the LSP semantic-tokens output and this
 * grammar should agree on the high-level node types.
 *
 * Build:
 *   npm install   # installs tree-sitter-cli
 *   npm run gen   # tree-sitter generate -> src/parser.c
 *
 * @see https://www.chordpro.org/chordpro/  ChordPro 6.07 reference
 */

module.exports = grammar({
  name: "keyflow",

  extras: ($) => [/[ \t]+/, $.comment],

  word: ($) => $.identifier,

  rules: {
    // ---------------------------------------------------------------- root
    document: ($) => repeat(choice($.block_separator, $._line)),

    // -------------------------------------------------------- block frames
    // `--- chordpro ---`, `--- keyflow ---`, etc. (>=3 dashes each side).
    block_separator: ($) =>
      seq(
        /---+/,
        field("name", $.block_name),
        /---+/,
        $._eol,
      ),
    block_name: ($) => /[a-zA-Z_][a-zA-Z0-9_-]*/,

    // ----------------------------------------------------------- one line
    _line: ($) =>
      choice(
        $.metadata_line,
        $.section_header,
        $.directive,
        $.chord_line,
        $.lyric_line,
        $.config_directive,
        $.empty_line,
      ),

    empty_line: ($) => $._eol,

    // -------------------------------------------------------- ChordPro `{...}`
    // `{title: My Song}`, `{soc}`, `{define: …}`, etc. Conditional
    // selector `{title-en: …}` is captured via the optional condition.
    directive: ($) =>
      seq(
        "{",
        field("name", $.directive_name),
        optional(seq("-", field("condition", $.directive_condition))),
        optional(
          seq(choice(":", /[ \t]+/), field("value", $.directive_value)),
        ),
        "}",
      ),
    directive_name: ($) => /[a-zA-Z_][a-zA-Z0-9_]*/,
    directive_condition: ($) => /[a-zA-Z0-9_-]+/,
    directive_value: ($) => /[^}]*/,

    // ------------------------------------------------------ ChordPro lyrics
    lyric_line: ($) =>
      prec(
        1,
        seq(
          choice($.chord_marker, $.annotation_marker, $.lyric_text),
          repeat(
            choice($.chord_marker, $.annotation_marker, $.lyric_text),
          ),
          $._eol,
        ),
      ),
    chord_marker: ($) =>
      seq("[", field("chord", $.chord_symbol), "]"),
    annotation_marker: ($) =>
      seq("[*", field("annotation", $.annotation_text), "]"),
    chord_symbol: ($) =>
      // Roughly: optional accidental + (note-letter | digit 1-7 | roman) +
      // any chord-quality / extension chars. Tree-sitter sees this as one
      // opaque token; the LSP / engine refines.
      /[#b]?(?:[A-G][a-zA-Z0-9#b\/+\-\(\)]*|[1-7][a-zA-Z0-9#b\/+\-\(\)]*|[ivIV]+[a-zA-Z0-9#b\/+\-\(\)]*)/,
    annotation_text: ($) => /[^\]]+/,
    lyric_text: ($) => /[^\[\{\n;#][^\[\{\n;]*/,

    // ----------------------------------------- Keyflow chord/rhythm lines
    // E.g. `| 1 4 5 1 |`, `Cm7 / Eb // | F#dim`, `'Bb_8t`. The chord-line
    // rule is matched on lines whose first non-whitespace char looks like
    // a chord head (uppercase letter, accidental, digit, or `|`).
    chord_line: ($) =>
      prec(
        2,
        seq(
          repeat1(choice($.bar, $.chord_token, $.slash_run, $.dot_repeat)),
          $._eol,
        ),
      ),
    bar: ($) => "|",
    slash_run: ($) => /\/+\.?/,
    dot_repeat: ($) => ".",
    chord_token: ($) =>
      // Keyflow chord with optional push-pull prefix and explicit duration
      // suffix (`'_8t`, `_4`, …). Permissive — engine validates.
      /[>']?[#b]?(?:[A-G][a-zA-Z0-9#b\/+\-\(\)]*|[1-7][a-zA-Z0-9#b\/+\-\(\)]*|[ivIV]+[a-zA-Z0-9#b\/+\-\(\)]*)(?:_[0-9]+t?\.?)?/,

    // ------------------------------------------------------ section header
    // `VS 1:`, `CH 4 "Down":`, `Bridge:`, `IN`.
    section_header: ($) =>
      prec(
        3,
        seq(
          field("kind", $.section_kind),
          optional(field("count", $.measure_count)),
          optional(field("comment", $.section_comment)),
          optional(":"),
          $._eol,
        ),
      ),
    section_kind: ($) =>
      choice(
        /VS|CH|IN|PreCH|Bridge|Solo|INST|Interlude|Outro|End|HITS|Pre[A-Z][a-zA-Z]*|Post[A-Z][a-zA-Z]*/,
      ),
    measure_count: ($) => /[0-9]+/,
    section_comment: ($) => /"[^"]*"/,

    // -------------------------------------------------------- metadata header
    // `120bpm 4/4 #C` style metadata line at top of chart.
    metadata_line: ($) =>
      prec(
        4,
        seq(
          choice($.tempo_token, $.time_signature_token, $.key_signature),
          repeat(
            choice($.tempo_token, $.time_signature_token, $.key_signature),
          ),
          $._eol,
        ),
      ),
    tempo_token: ($) => /[0-9]+bpm/,
    time_signature_token: ($) => /[0-9]+\/[0-9]+/,
    key_signature: ($) => /[#b][A-G][b#]?m?(?:in)?/,

    // -------------------------------------------------- `/push = triplet`
    config_directive: ($) =>
      seq(
        "/",
        field("name", $.config_name),
        optional(seq("=", field("value", $.config_value))),
        $._eol,
      ),
    config_name: ($) => /[a-zA-Z_][a-zA-Z0-9_]*/,
    config_value: ($) => /[^\n;]+/,

    // ---------------------------------------------------------- comments
    comment: ($) => token(seq(";", /[^\n]*/)),

    // ---------------------------------------------------------- terminals
    identifier: ($) => /[a-zA-Z_][a-zA-Z0-9_]*/,
    _eol: ($) => /\r?\n/,
  },
});
