//! MIDI corpus snapshot harness.
//!
//! Each corpus MIDI file gets a deterministic chart-text snapshot. A
//! snapshot is the output of `generate_chart_text` for a given MIDI +
//! `MidiChartConfig`, written to disk next to the MIDI as `<basename>.kf`.
//!
//! The tests in this module are gated with `#[ignore]` by default — the
//! corpus is large and may not be checked into every clone. Run on demand:
//!
//! ```bash
//! # Verify all corpus snapshots match.
//! cargo test -p keyflow-midi -- --ignored snapshot_
//!
//! # Regenerate snapshots after a deliberate output change.
//! KEYFLOW_UPDATE_SNAPSHOTS=1 cargo test -p keyflow-midi -- --ignored snapshot_
//! ```
//!
//! Compare with the chord-marker integrity tests in
//! `midi_song_*.rs` — those verify *semantic* alignment with the MIDI's
//! own chord markers; this harness verifies *output stability* of the
//! whole chart-text pipeline.

use keyflow_midi::import::{MidiChartConfig, MidiFile, generate_chart_text};
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../keyflow/tests/midi")
        .canonicalize()
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../keyflow/tests/midi")
                .to_path_buf()
        })
}

fn snapshot_path_for(midi_path: &Path) -> PathBuf {
    let stem = midi_path.file_stem().unwrap().to_string_lossy().to_string();
    midi_path.with_file_name(format!("{}.kf", stem))
}

fn assert_or_update_snapshot(midi_relative_path: &str, key_root: Option<&str>) {
    let midi_path = corpus_dir().join(midi_relative_path);
    if !midi_path.exists() {
        eprintln!(
            "snapshot_harness: corpus file '{}' missing, skipping",
            midi_path.display()
        );
        return;
    }
    let bytes =
        std::fs::read(&midi_path).unwrap_or_else(|e| panic!("read {}: {e}", midi_path.display()));
    let midi =
        MidiFile::parse(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", midi_path.display()));

    let config = MidiChartConfig {
        key_root: key_root.map(str::to_string),
        title: None,
        swing: None,
        ..Default::default()
    };
    let actual = generate_chart_text(&midi, &config);

    let snap = snapshot_path_for(&midi_path);
    let updating = std::env::var("KEYFLOW_UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if updating || !snap.exists() {
        std::fs::write(&snap, &actual).unwrap_or_else(|e| panic!("write {}: {e}", snap.display()));
        eprintln!("snapshot_harness: wrote {}", snap.display());
        return;
    }

    let expected =
        std::fs::read_to_string(&snap).unwrap_or_else(|e| panic!("read {}: {e}", snap.display()));
    if actual != expected {
        let diff = diff_lines(&expected, &actual);
        panic!(
            "snapshot mismatch for '{}'.\n\n  Update with KEYFLOW_UPDATE_SNAPSHOTS=1 if the change is intentional.\n\n{}",
            snap.display(),
            diff
        );
    }
}

/// Tiny line-diff for panic messages. Avoids pulling in a diff crate just
/// for the corpus tests.
fn diff_lines(expected: &str, actual: &str) -> String {
    let max = expected.lines().count().max(actual.lines().count());
    let mut out = String::new();
    let mut e_iter = expected.lines();
    let mut a_iter = actual.lines();
    let mut shown = 0usize;
    for i in 0..max {
        let e = e_iter.next();
        let a = a_iter.next();
        if e == a {
            continue;
        }
        if shown < 30 {
            out.push_str(&format!("  L{:>5}  -{}\n", i + 1, e.unwrap_or("")));
            out.push_str(&format!("         +{}\n", a.unwrap_or("")));
        }
        shown += 1;
    }
    if shown == 0 {
        out.push_str("  (whitespace-only or trailing-newline difference)\n");
    } else if shown > 30 {
        out.push_str(&format!("  ... {} more differing lines\n", shown - 30));
    }
    out
}

#[test]
#[ignore = "corpus snapshot test — opt in with `cargo test -- --ignored`"]
fn snapshot_bennie_and_the_jets() {
    assert_or_update_snapshot("Bennie And The Jets - Elton John.mid", Some("G"));
}

#[test]
#[ignore = "corpus snapshot test — opt in with `cargo test -- --ignored`"]
fn snapshot_broadview() {
    assert_or_update_snapshot("Broadview - Slow Pulp.mid", None);
}

#[test]
#[ignore = "corpus snapshot test — opt in with `cargo test -- --ignored`"]
fn snapshot_for_cryin_out_loud() {
    assert_or_update_snapshot("For Cryin' Out Loud - FINNEAS.mid", None);
}

#[test]
#[ignore = "corpus snapshot test — opt in with `cargo test -- --ignored`"]
fn snapshot_cryin_mateus_asato() {
    assert_or_update_snapshot("Cryin' - Mateus Asato.mid", None);
}
