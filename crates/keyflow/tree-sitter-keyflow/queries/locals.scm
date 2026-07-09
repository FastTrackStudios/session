;; Locals: melody-variable definitions and `$name` references.
;;
;; A `mainRiff = m{ … }` line defines a melody variable; later `$mainRiff`
;; tokens reference it. Tree-sitter local-scope queries let editors
;; jump-to-definition and rename across these.

(directive
  name: (directive_name) @local.scope
  (#eq? @local.scope "x_keyflow_melody_def"))

;; Plain identifier captures so editors can resolve `$mainRiff`.
(identifier) @local.reference
