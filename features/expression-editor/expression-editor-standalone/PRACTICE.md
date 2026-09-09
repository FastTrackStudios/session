# Crescendum drum practice

From the session repository:

```sh
just ee-practice                  # Set in Stone: fresh copy, editor + transport + mixer
just ee-practice unbreakable      # Unbreakable in a fresh workspace
just ee-practice-prepare          # Copy both songs, print paths; no window
just ee-practice-test             # Real-song load / split / undo / redo / save / reload test
```

Each invocation creates a retained `fts-drum-practice-*` directory in the system
temporary directory. Project files and every referenced media file are actual
copies, with project references rewritten to the local `Media` directory.
Record/render output paths are redirected there too. Shared recordings are copied
once per workspace; both songs together use roughly 10 GB. No hard links or
symlinks point back to the recordings. Preparation fails if a source is missing.

The source defaults to:

```text
/run/media/AudioHaven/Project/Crescendum-Rockstars-SESSION-BACKUP-2026-09-06/Crescendum
```

Override it with `EXPRESSION_EDITOR_PRACTICE_ALBUM`. The expected layout is
`<album>/<song>/<song>.organized.RPP` plus each song's `Media` directory.
Legacy Mac paths and cross-song media references are resolved during preparation.
Set `TMPDIR` to choose another scratch location.

Save writes a new `.fts-edit.rpp` beside the practice project, with numbered
filenames on subsequent saves. To resume an existing copy without copying again:

```sh
just workstation "/tmp/fts-drum-practice-XXXXX/set in stone.practice.fts-edit.rpp"
```

The same staged RPP can be opened in REAPER to practice with the expression editor
panel. The launch recipe itself opens the standalone workstation. `MANIFEST.txt`
records the original and copied paths. Workspaces remain after exit; remove them
when finished, or move them somewhere durable to keep edits beyond temporary-file
cleanup.

The opt-in real-song test verifies audible drums, a grouped split, whole-kit undo
and redo, saved audio placement and source references after reloading, and SHA-256
preservation of original projects, original recordings, and copied recordings.
Ordinary staging tests use tiny synthetic files and do not require the album:

```sh
cargo test -p expression-editor-standalone --lib practice::tests
```

## Navigating a song

The TCP and mixer show folder nesting. Click a folder's disclosure arrow to hide
or reveal its descendants in the TCP, arrangement, and mixer together. Nested
folders remember their own collapse choices while their parent is closed. These
are window view settings; collapsing does not mute or remove any tracks.

In the drum editor, click the timeline to focus it, then hold **Z** and drag right
or left to zoom around the time where you pressed. Hold **Shift** for finer
control. Hold **Z**, then **Alt-drag** across a passage to frame that time range.
Releasing Z restores the previous tool. Selecting the Zoom tool in the toolbar
supports the same gestures. **+ / −**, wheel navigation, **[ / ]** (four-bar
pages), and **\\** (frame four bars) remain available.

For DOM-driven navigation and timing reports of the whole workstation, use
`just ee-stress`. See [UI stress benchmark](../../../scripts/ui-stress/README.md).

The [performance testing guide](../../../scripts/ui-stress/GUIDE.md) covers baselines, phase isolation, diagnostics and agent handoffs.
