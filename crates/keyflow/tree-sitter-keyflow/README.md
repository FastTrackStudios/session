# tree-sitter-keyflow

[Tree-sitter](https://tree-sitter.github.io/tree-sitter/) grammar for
**Keyflow** chart notation (`.kf` documents) including embedded
[ChordPro 6.07](https://www.chordpro.org/) lyric blocks.

This grammar is the **structural / highlighting** path. The
[`keyflow_text::ide`](../keyflow-text) engine remains the source of truth
for diagnostics, completion, and hover; the LSP server in
[`keyflow-lsp`](../keyflow-lsp) provides those over the network. Editors
that prefer tree-sitter for fast incremental highlighting (Zed, GitHub
web view, Neovim's nvim-treesitter) can use this grammar in parallel with
the LSP server, with both surfaces agreeing on token kinds.

## What it covers

- `--- chordpro ---` / `--- keyflow ---` block separators
- ChordPro `{directive: value}` with conditional `{title-en: …}` selectors
- ChordPro `[chord]Lyric` and `[*annotation]` markers
- Keyflow rhythm/chord lines with `|` bars, `/` slash runs, `.` dot
  repeats, push (`'`) / accent (`>`) prefixes, explicit duration suffixes
  (`_8t`, `.4`)
- Section headers: `VS 1:`, `CH 4 "Down":`, `Bridge`, `IN`, `PreCH`, …
- Metadata header: `120bpm 4/4 #C`
- Config directives: `/push = triplet`, `/swing = 0.667`
- `;` line comments

See [`grammar.js`](./grammar.js) for the full rule set and
[`queries/highlights.scm`](./queries/highlights.scm) for the highlight
captures.

## Building the parser

The C parser source (`src/parser.c`) is generated from `grammar.js` by
the tree-sitter CLI. We don't check it in (it's machine-generated and
large), so contributors run:

```bash
cd crates/tree-sitter-keyflow
npm install        # installs tree-sitter-cli
npm run gen        # tree-sitter generate -> src/parser.c
npm run test       # tree-sitter test
```

After `src/parser.c` exists, the Rust crate (`Cargo.toml` next to this
file) builds normally and exposes `tree_sitter_keyflow::LANGUAGE`. Until
then, `cargo build -p tree-sitter-keyflow` succeeds with a warning and
the Rust binding compiles as a stub — useful for the rest of the
workspace's CI.

## Editor integration

### Neovim (nvim-treesitter)

```lua
require'nvim-treesitter.parsers'.get_parser_configs().keyflow = {
  install_info = {
    url = "/path/to/keyflow/crates/tree-sitter-keyflow",
    files = { "src/parser.c" },
    branch = "main",
  },
  filetype = "keyflow",
}
```

### Zed

Zed picks up tree-sitter grammars from a Zed extension. A `keyflow.json`
extension manifest pointing at this crate is in
[`../keyflow-lsp/editors/zed-keyflow/`](../keyflow-lsp/editors/zed-keyflow).

### Helix

Helix needs `runtime/grammars/sources/keyflow/` linked to this crate and
`languages.toml` pointing the `keyflow` language at `keyflow-lsp` for
diagnostics + this grammar for highlighting. The LSP README has the
languages.toml block.

## Architecture

```
.kf source ─┬─► tree-sitter-keyflow ──► editor highlighting
            │     (this crate)
            │
            └─► keyflow-lsp ──► editor diagnostics / completion / hover
                  └► keyflow-text::ide ◄── (single source of truth)
                         └► keyflow_chordpro::parse for `--- chordpro ---`
```

Both paths consume the **same** input bytes and agree on token kinds —
the LSP semantic-tokens output legend matches the highlight captures
emitted by this grammar (chord = `function`, section = `keyword.control`,
etc.).
