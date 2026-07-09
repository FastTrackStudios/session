# keyflow-lsp

Language Server Protocol server for [Keyflow](../..) chart notation.

Wraps the `keyflow_text::ide` engine in a `tower-lsp` server that speaks LSP
over stdio. Any editor that supports LSP can light up `.kf` / `.keyflow`
files: VS Code, Zed, Helix, Neovim, Sublime LSP, Emacs `eglot` / `lsp-mode`.

## Features

- **Diagnostics** — error squiggles with stable codes (`kf001-parse-failed`, …)
- **Completion** — chord roots, qualities, section headers (`VS:`, `CH:`, …),
  slash commands (`/fermata`, `/accent`, …), `$riff` melody-variable recall
- **Hover** — chord-token info, scale-degree resolution against the active key,
  melody-variable bodies
- **Semantic tokens** — high-fidelity highlighting that mirrors the
  parser's view of the document (better than regex-based grammars)

The same engine powers the in-process Dioxus editor in `keyflow-ui`, so a
fix landed here is automatically picked up there.

## Build

```bash
cargo build --release -p keyflow-lsp
# binary lands at target/release/keyflow-lsp
```

## Editor setup

### Helix

```toml
# ~/.config/helix/languages.toml
[[language]]
name = "keyflow"
scope = "source.keyflow"
file-types = ["kf", "keyflow"]
language-servers = ["keyflow-lsp"]

[language-server.keyflow-lsp]
command = "keyflow-lsp"
```

### Neovim (with `nvim-lspconfig` or native `vim.lsp.start`)

```lua
vim.lsp.start({
  name = "keyflow-lsp",
  cmd = { "keyflow-lsp" },
  filetypes = { "keyflow" },
  root_dir = vim.fn.getcwd(),
})
```

### VS Code

Use the bundled extension scaffold in `editors/vscode-keyflow/`:

```bash
cd crates/keyflow-lsp/editors/vscode-keyflow
npm install
npm run package          # produces a .vsix
code --install-extension keyflow-vscode-*.vsix
```

The extension launches `keyflow-lsp` from `$PATH`. Configure
`keyflow.serverPath` to override.

### Zed

A `languages/keyflow/config.toml` skeleton is in `editors/zed-keyflow/` for
when you're ready to wire it into a Zed extension.

## Architecture

```
                  ┌──────────────────────┐
   editor ◄─LSP──►│ keyflow-lsp (this)   │
                  │  tower-lsp glue      │
                  └──────────┬───────────┘
                             │
                  ┌──────────▼───────────┐
                  │  keyflow_text::ide   │
                  │  analyze / complete  │
                  │  hover / highlight   │
                  └──────────┬───────────┘
                             │
                  ┌──────────▼───────────┐
                  │  keyflow-text parser │
                  │  + keyflow-proto AST │
                  └──────────────────────┘
```

The engine is **pure** (no I/O, no async) and is what `keyflow-ui`'s
embedded editor calls directly. The LSP layer is a thin protocol adapter —
fewer than 300 lines.

## Roadmap

- [ ] Code Actions (Quick Fix from `Diagnostic.fixes`)
- [ ] Document symbols (sections as outline nodes)
- [ ] Find-references / rename for `$riff` melody variables
- [ ] Tree-sitter grammar (parallel; speeds up coloring in editors that prefer it)
- [ ] Incremental sync (`TextDocumentSyncKind::INCREMENTAL`) once charts get large
