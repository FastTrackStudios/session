# FTS Extensions

`fts-extensions` is the in-process REAPER extension host for FastTrackStudio.
It loads the launcher, dynamic template, session, sync, input, and keyflow
modules into one `reaper_fts_extensions` shared library.

## Workspace Layout

This repository is intentionally small. Most product logic lives in sibling
repositories under the same parent directory:

```text
FastTrackStudio/
  fts-extensions/      # this repo: REAPER extension host and integration tests
  fts-launcher/        # launcher actions and UI
  dynamic-template/    # template sorting, auto-color, visibility actions
  session/             # setlist/session actions and state
  sync/                # live sync/link actions
  input/               # REAPER input hooks, keybinds, workflows
  input_actions/       # input/action config schemas
  keyflow/             # chart/keyflow domain
  daw/                 # DAW facade, REAPER bridge, integration test harness
  reaper-lib/          # REAPER Dioxus/SWELL embedding support
  Plugins/fts-ui/      # shared FTS Dioxus component library
  blitz/               # patched Dioxus native/rendering stack
  vox/                 # service/RPC framework fork
  monarchy/            # hierarchy framework used by dynamic-template
```

The main extension collects module actions in
`crates/fts-extensions/src/lib.rs`, registers them with REAPER, installs the
FastTrackStudio menu, and registers Dioxus dock panels.

## Dependency Pinning

`Cargo.toml` declares FTS side repos as git dependencies pinned to explicit
commits. The current pins are:

| Repo | Commit |
| --- | --- |
| `fts-launcher` | `698a29121c2d20c24cba5a58d608c6f5da0f56a0` |
| `dynamic-template` | `d4467d3807b970476c09641d6c240321653f5148` |
| `session` | `189f942bd9fc07513ed913c488b36871f661849d` |
| `sync` | `7830a5a0b46afff799d6cfb18a0d9834f35a3d3f` |
| `input` | `7f06a6acf027956c94b6fa2cab5b50c425ca1067` |
| `input_actions` | `8503c47c097ce9f4e3d8fb369a82c5925cef9317` |
| `keyflow` | `c57b122ec309a8aade59b07b84041bc98a195a85` |
| `daw` | `273dbaa1ef5e4dfac9cd35988e13d2b4b954ab6e` |
| `reaper-lib` | `8fe87a0604a87f24009dd03056627d0f19f89bc5` |
| `fts-ui` | `d9f01261cb59708300cc5b31fa7fe6bd766a66aa` |
| `blitz` | `18cbfa7d63496441c37074c07470f71f3004d290` |
| `vox` | `2a2f793b868b22d82ae4be6e1abc581ca330f940` |
| `monarchy` | `b5efa6bce31cfd009f9f042403a034f89dabd2ab` |

The local-development `[patch]` sections in `Cargo.toml` redirect those git
packages to sibling checkouts. Keep them enabled for fast cross-repo iteration.
Comment them out only after the pinned commits exist on the remote and you want
to validate the remote-pinned dependency graph.

Important: several side repos still contain their own sibling `path`
dependencies. A fully remote-only Cargo graph will require those side repo
manifests to be made self-contained or to keep equivalent patches enabled at the
root.

## Common Commands

```sh
just check
just build
just test
just install
just integration-test
just snapshot-check
```

`just install` builds the extension and symlinks it into
`$REAPER_HOME/UserPlugins`, defaulting to `$HOME/.fts-dev/UserPlugins`.

`just install-config` symlinks input keybind/workflow config and launcher packs
from sibling repos into `$REAPER_HOME/fasttrackstudio`.

## Generated Files

The repo ignores local environment output such as `.devenv/`, `.direnv/`,
`.codex`, task databases, logs, `target/`, and generated frontend
`node_modules/` directories. Do not commit these.
