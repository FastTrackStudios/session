//! UACC emission: `Config::use_uacc` retargets articulation selection at a
//! latched-CC selector — CC32 carrying published UACC codes — wherever CC58
//! keyswitch bands would have been emitted, and suppresses the CSS-only mode
//! toggles (Legato On, Con Sordino On/Off) that have no UACC codes.

use keyflow_orchestra::score::{ArtSet, Part, RawNote};
use keyflow_orchestra::{process_part, Config};

fn raw(onset: f64, dur: f64, pitch: i32, tags: &[&str]) -> RawNote {
    let mut art = ArtSet::default();
    for t in tags {
        art.insert(*t);
    }
    RawNote {
        onset,
        dur,
        pitch,
        voice: 1,
        tie_start: false,
        tie_stop: false,
        art,
        slur_start: false,
        slur_stop: false,
        beat_qn: onset % 4.0,
        beats: 4,
        beat_type: 4,
        fifths: 0,
    }
}

fn two_artic_part() -> Part {
    Part {
        id: "P1".to_string(),
        name: "Violin I".to_string(),
        // A staccato short, then a plain sustain far enough away that the
        // articulation state must switch between them.
        notes: vec![raw(0.0, 0.5, 67, &["staccato"]), raw(4.0, 4.0, 69, &[])],
        dynamics: Vec::new(),
        tempos: Vec::new(),
        meters: Vec::new(),
        markings: Vec::new(),
        harmonies: Vec::new(),
    }
}

#[test]
fn uacc_emission_targets_cc32_with_standard_codes() {
    let part = two_artic_part();

    let mut cfg = Config::default();
    cfg.use_uacc();
    assert_eq!(cfg.cc_keyswitch, 32);
    let out = process_part(&part, &cfg);
    assert!(!out.empty);

    // Every articulation selection rides CC32; nothing rides CC58.
    let ks: Vec<u8> = out
        .ccs
        .iter()
        .filter(|e| e.cc == 32)
        .map(|e| e.val)
        .collect();
    assert!(out.ccs.iter().all(|e| e.cc != 58), "no CC58 under UACC");
    // Exactly the two selections with the published codes, in note order:
    // Short (staccato) = 40, then Long = 1. No toggle presses (the CSS
    // Legato On press has no UACC code and must be suppressed).
    assert_eq!(ks, vec![40, 1]);
}

#[test]
fn default_emission_still_uses_cc58_bands() {
    // The keyswitch stream without `use_uacc` is unchanged: CC58, CSS band
    // centres, Legato On toggle first.
    let part = two_artic_part();
    let cfg = Config::default();
    let out = process_part(&part, &cfg);

    let ks: Vec<u8> = out
        .ccs
        .iter()
        .filter(|e| e.cc == 58)
        .map(|e| e.val)
        .collect();
    assert!(out.ccs.iter().all(|e| e.cc != 32), "no CC32 by default");
    // Legato On (78), staccato band (23), then the sustain keyswitch for
    // the default Expressive mode (8).
    assert_eq!(ks, vec![78, 23, 8]);
}
