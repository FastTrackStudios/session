;; Re-parse `--- chordpro ---` block content as ChordPro. This is a
;; placeholder: keyflow's tree-sitter grammar already understands inline
;; `[chord]lyric` markers, so a separate ChordPro grammar is only useful
;; if a downstream editor wants the strict 6.07 rules. Replace with
;; `(#set! "language" "chordpro")` once tree-sitter-chordpro exists.

;; Inject section comment text as plain string for now.
((section_comment) @injection.content
 (#set! injection.language "string"))
