//! Parity snapshots vs the reference Lua engine.
//!
//! The expected values below were produced by the original engine
//! (`reference-data/css-orchestrator/css_engine.lua`) via
//! `luajit test/test_engine.lua` on `theme-from-jurassic-park.mxl`
//! (2026-07-01). If an intentional engine change breaks these, regenerate
//! with the Lua engine or update deliberately.
//!
//! Known intentional deviations from the Lua engine:
//! - **CC64 re-bow pedal spans are merged per channel** when runs overlap in
//!   time (divisi voices sharing a channel). The Lua engine emitted them
//!   interleaved, so a state machine — including CSS itself — read the pedal
//!   as released mid-run. CC event counts below are therefore slightly lower
//!   than the Lua engine's on parts with overlapping re-bow runs
//!   (Trombone 1: 274→272, Violon 1: 1202→1190).

use keyflow_orchestra::{Config, PartOutput, ProfileKind, process_part};

fn run(part_name: &str, profile: ProfileKind) -> PartOutput {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/mxl/theme-from-jurassic-park.mxl");
    let score = keyflow_orchestra::score::load(&path).expect("parse");
    let part = score
        .parts
        .iter()
        .find(|p| p.name == part_name)
        .unwrap_or_else(|| panic!("part {part_name} missing"));
    let mut cfg = Config::default();
    cfg.profile = profile;
    process_part(part, &cfg)
}

/// (start_qn, end_qn, chan, pitch, vel, lead_ms)
type NoteRow = (f64, f64, u8, i32, u8, i64);

fn assert_first_notes(out: &PartOutput, expected: &[NoteRow]) {
    for (i, &(s, e, ch, p, v, lead)) in expected.iter().enumerate() {
        let n = &out.notes[i];
        assert!(
            (n.start_qn - s).abs() < 5e-4 && (n.end_qn - e).abs() < 5e-4,
            "note {i}: times {:.3}->{:.3}, want {s:.3}->{e:.3}",
            n.start_qn,
            n.end_qn
        );
        assert_eq!(
            (n.chan, n.pitch, n.vel),
            (ch, p, v),
            "note {i} chan/pitch/vel"
        );
        assert_eq!(n.lead_ms as i64, lead, "note {i} lead_ms");
    }
}

#[test]
fn flute1_woodwinds_matches_lua() {
    let out = run("Flute 1", ProfileKind::Woodwinds);
    assert_eq!(out.notes.len(), 215);
    assert_eq!(out.ccs.len(), 443);
    assert_eq!(out.channels, vec![1]);
    assert!((out.start_qn - 15.0).abs() < 1e-6 && (out.end_qn - 158.5).abs() < 1e-6);
    assert_first_notes(
        &out,
        &[
            (15.000, 18.031, 1, 77, 64, 0),
            (17.851, 20.456, 1, 82, 8, 160),
            (20.472, 20.781, 1, 82, 102, 30),
            (20.722, 20.984, 1, 81, 102, 30),
            (21.000, 22.031, 1, 82, 64, 0),
            (21.851, 23.531, 1, 84, 37, 160),
        ],
    );
}

#[test]
fn trombone1_brass_matches_lua() {
    let out = run("Trombone 1", ProfileKind::Brass);
    assert_eq!(out.notes.len(), 69);
    assert_eq!(out.ccs.len(), 272); // Lua: 274 — see CC64 merge note above
    assert_eq!(out.channels, vec![1]);
    assert!((out.start_qn - 69.0).abs() < 1e-6 && (out.end_qn - 158.5).abs() < 1e-6);
    assert_first_notes(
        &out,
        &[
            (69.000, 70.031, 1, 53, 101, 0),
            (69.920, 71.031, 1, 58, 77, 40),
            (70.920, 72.031, 1, 53, 101, 40),
            (71.920, 72.984, 1, 58, 77, 40),
            (73.000, 74.031, 1, 53, 101, 0),
        ],
    );
    // Brass: no CC2 vibrato at all.
    assert!(out.ccs.iter().all(|c| c.cc != 2), "brass must not emit CC2");
}

#[test]
fn violin1_strings_divisi_matches_lua() {
    let out = run("Violon 1", ProfileKind::Strings);
    assert_eq!(out.notes.len(), 143);
    assert_eq!(out.ccs.len(), 1190); // Lua: 1202 — see CC64 merge note above
    assert_eq!(out.channels, vec![1, 2]);
    assert!((out.start_qn - 52.5).abs() < 1e-6 && (out.end_qn - 170.0).abs() < 1e-6);
    assert_first_notes(
        &out,
        &[
            (52.500, 52.781, 1, 70, 64, 0),
            (52.670, 52.984, 1, 69, 119, 40),
            (53.000, 54.404, 1, 70, 64, 0),
        ],
    );
}

#[test]
fn harp_gliss_and_velocity_dynamics_match_lua() {
    let out = run("Harpe", ProfileKind::Harp);
    assert_eq!(out.notes.len(), 89);
    // Harp: no CC1/CC2/CC58 — dynamics ride velocity; only CC11 resets remain.
    assert_eq!(out.ccs.len(), 16);
    assert_eq!(
        out.channels,
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16]
    );
    assert!((out.start_qn - 27.0).abs() < 1e-6 && (out.end_qn - 167.0).abs() < 1e-6);
    assert_first_notes(
        &out,
        &[
            (27.000, 27.500, 7, 46, 89, 0),
            (27.500, 28.000, 7, 53, 87, 0),
            (28.000, 28.500, 7, 62, 85, 0),
            (28.500, 29.000, 7, 60, 82, 0),
            (28.500, 29.000, 1, 77, 82, 0),
        ],
    );
}
