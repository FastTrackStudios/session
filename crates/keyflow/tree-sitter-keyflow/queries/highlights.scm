;; Tree-sitter highlight query for Keyflow.
;;
;; Capture names follow the standard tree-sitter highlight conventions
;; (https://tree-sitter.github.io/tree-sitter/syntax-highlighting). They
;; map cleanly to LSP `SemanticTokenType`s, so editors that prefer
;; tree-sitter over LSP semantic tokens get the same coloring.

;; ---- Block separators ------------------------------------------------------
(block_separator
  (block_name) @keyword.directive)

;; ---- ChordPro directives ---------------------------------------------------
(directive
  name: (directive_name) @keyword)
(directive
  condition: (directive_condition) @attribute)
(directive
  value: (directive_value) @string)

((directive
   name: (directive_name) @keyword.control.directive)
 (#match? @keyword.control.directive "^(start_of_|end_of_|chorus|new_song|new_page)"))

;; ---- ChordPro lyrics -------------------------------------------------------
(chord_marker
  chord: (chord_symbol) @function)
(annotation_marker
  annotation: (annotation_text) @comment.documentation)
(lyric_text) @string

;; ---- Keyflow rhythm / chord lines -----------------------------------------
(chord_line
  (chord_token) @function)
(chord_line
  (bar) @punctuation.delimiter)
(chord_line
  (slash_run) @operator)
(chord_line
  (dot_repeat) @operator)

;; ---- Section headers -------------------------------------------------------
(section_header
  kind: (section_kind) @keyword.control.section)
(section_header
  count: (measure_count) @number)
(section_header
  comment: (section_comment) @string.special)

;; ---- Metadata header -------------------------------------------------------
(tempo_token) @number
(time_signature_token) @number
(key_signature) @attribute

;; ---- Config / `/push = triplet` -------------------------------------------
(config_directive
  name: (config_name) @keyword.control)
(config_directive
  value: (config_value) @string)

;; ---- Comments --------------------------------------------------------------
(comment) @comment.line
