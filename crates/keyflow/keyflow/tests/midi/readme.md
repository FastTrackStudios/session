# MIDI Test Corpus and Goals

This directory stores **real songs used to pressure-test MIDI import quality**.
These files are not the main Rust test fixtures yet; they are a working corpus for finding failures and deciding what to lock down next.

## Why this exists

Synthetic test data is useful, but it misses musical edge cases that show up in real arrangements.
This corpus helps us validate that Keyflow can convert production-style MIDI into stable, readable chart output.

## Testing goals

1. Parse every corpus file without panics or dropped tracks.
2. Preserve core timing semantics:
   - PPQ interpretation
   - section boundaries
   - marker ordering
   - rests and duration math
3. Produce musically correct harmony output:
   - chord detection from note clusters
   - marker-based chord import when present
   - predictable spelling in the chosen key
4. Keep rhythm notation stable:
   - push/pull classification
   - triplet and subdivision handling
   - staccato/short-hit representation
5. Prevent regressions by promoting failures into deterministic fixture tests under `tests/`.

## Current files in this corpus

- `Bennie And The Jets - Elton John.mid`
- `Broadview - Slow Pulp.mid`
- `For Cryin' Out Loud - FINNEAS.mid`

## How to use this folder

1. Run/import each file through the current MIDI pipeline.
2. Note any parsing failure, bad chord spelling, wrong section labels, or rhythm notation issues.
3. For each issue:
   - create a minimal reproducible test in `cells/keyflow/keyflow/tests/`
   - commit the expected chart text (or focused assertions)
   - fix the importer
   - verify the new test fails before the fix and passes after

## Definition of done for MIDI import quality

We consider the importer healthy when:

1. Corpus files parse cleanly.
2. Existing exact-output tests remain stable (no accidental diffs).
3. New real-world failures are captured as regression tests before code changes are merged.

## Scope note

Large, exact-output assertions should live in `cells/keyflow/keyflow/tests/*.rs` fixtures.
This folder is the **discovery corpus** that feeds those regression tests.

## Snapshot harness

Each `.mid` in this folder also has a deterministic chart-text snapshot
written next to it as `<basename>.kf`. The snapshot is the output of
`generate_chart_text(&midi, &MidiChartConfig)` — i.e. the full
import-and-format pipeline. The tests live in
`crates/keyflow-midi/tests/snapshot_harness.rs` and are gated with
`#[ignore]` because the corpus may not be present in every clone.

```bash
# Verify all snapshots match (won't run without --ignored).
cargo test -p keyflow-midi -- --ignored snapshot_

# After a deliberate output change, regenerate:
KEYFLOW_UPDATE_SNAPSHOTS=1 cargo test -p keyflow-midi -- --ignored snapshot_
```

CI should run the verify mode whenever the corpus is checked in. A
panicking diff (first 30 differing lines + count) makes accidental
regressions obvious in PR output.

The complementary `assert_marker_chords_match` tests in
`midi_song_*.rs` verify *semantic* alignment with each MIDI's own chord
markers; the snapshot harness verifies *output stability* of the whole
chart-text pipeline.
