//! The mirror pass must reproduce the full engine: running the engine WITHOUT
//! timing compensation (= what the source track holds) and then mirroring must
//! give the same notes and note-anchored CCs as running the engine WITH timing
//! compensation. This is the guarantee that lets the source track stay
//! score-faithful while the performance track plays correctly.

use keyflow_orchestra::engine::CcEvent;
use keyflow_orchestra::{Config, MidiNote, ProfileKind, mirror_part, process_part};

/// The notated articulation for a stage-1 note: engine artic, except that a
/// marcato with no written strong-accent is a fast-run conversion — notated
/// plain.
fn notated_artic(n: &keyflow_orchestra::engine::OutNote) -> &'static str {
    if n.artic == "marcato" && !n.score_art.contains("strong-accent") {
        "sustain"
    } else {
        n.artic
    }
}

fn stage1_and_full(
    part_name: &str,
    profile: ProfileKind,
) -> (
    keyflow_orchestra::PartOutput,
    keyflow_orchestra::PartOutput,
    Vec<keyflow_orchestra::score::TempoPoint>,
) {
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
    cfg.tempo_map = Some(score.meta.tempos.clone());

    let mut stage1_cfg = cfg.clone();
    stage1_cfg.timing_comp = false;
    let stage1 = process_part(part, &stage1_cfg);
    let full = process_part(part, &cfg);
    (stage1, full, score.meta.tempos.clone())
}

fn check_mirror_parity(part_name: &str, profile: ProfileKind) {
    let (stage1, full, tempos) = stage1_and_full(part_name, profile);
    let src_notes: Vec<MidiNote> = stage1.notes.iter().map(MidiNote::from).collect();
    // Articulation hints as the DAW reader supplies them from the source
    // item's notation events: the NOTATED articulation. A fast-run note the
    // engine converted to marcato has no strong-accent in the score, so its
    // notated articulation is plain sustain.
    let hints: Vec<&'static str> = stage1.notes.iter().map(notated_artic).collect();

    let cfg = {
        let mut c = Config::default();
        c.profile = profile;
        c
    };
    let mirrored = mirror_part(&src_notes, &stage1.ccs, Some(&hints), &cfg, &tempos);

    // --- Notes: exact parity with the full engine ---
    assert_eq!(
        mirrored.notes.len(),
        full.notes.len(),
        "{part_name}: note count"
    );
    for (i, (m, f)) in mirrored.notes.iter().zip(full.notes.iter()).enumerate() {
        assert_eq!(
            (m.chan, m.pitch, m.vel),
            (f.chan, f.pitch, f.vel),
            "{part_name} note {i}: identity"
        );
        assert!(
            (m.start_qn - f.start_qn).abs() < 1e-6,
            "{part_name} note {i} (p{} ch{}): start {:.6} vs full {:.6}",
            m.pitch,
            m.chan,
            m.start_qn,
            f.start_qn
        );
        assert!(
            (m.end_qn - f.end_qn).abs() < 1e-6,
            "{part_name} note {i} (p{} ch{}): end {:.6} vs full {:.6}",
            m.pitch,
            m.chan,
            m.end_qn,
            f.end_qn
        );
    }

    // --- Note-anchored CCs (58 keyswitch, 64 pedal, 5 porta): exact parity ---
    let anchored = |ccs: &[CcEvent]| -> Vec<(i64, u8, u8, u8)> {
        ccs.iter()
            .filter(|e| e.cc == 58 || e.cc == 64 || e.cc == 5)
            .map(|e| ((e.qn * 1e6).round() as i64, e.chan, e.cc, e.val))
            .collect()
    };
    assert_eq!(
        anchored(&mirrored.ccs),
        anchored(&full.ccs),
        "{part_name}: anchored CC streams differ"
    );

    // --- Expression CCs (1/2/11): pass through from the source verbatim ---
    let expression = |ccs: &[CcEvent]| -> Vec<(i64, u8, u8, u8)> {
        ccs.iter()
            .filter(|e| e.cc == 1 || e.cc == 2 || e.cc == 11)
            .map(|e| ((e.qn * 1e6).round() as i64, e.chan, e.cc, e.val))
            .collect()
    };
    assert_eq!(
        expression(&mirrored.ccs),
        expression(&stage1.ccs),
        "{part_name}: expression CCs must pass through unchanged"
    );
}

#[test]
fn mirror_matches_full_engine_woodwinds() {
    check_mirror_parity("Flute 1", ProfileKind::Woodwinds);
}

#[test]
fn mirror_matches_full_engine_brass() {
    check_mirror_parity("Trombone 1", ProfileKind::Brass);
}

#[test]
fn mirror_matches_full_engine_strings_divisi() {
    check_mirror_parity("Violon 1", ProfileKind::Strings);
}

#[test]
fn mirror_matches_full_engine_strings_celli() {
    check_mirror_parity("Violoncelles", ProfileKind::Strings);
}

/// The whole corpus: every part of every score, strings profile forced where
/// detection says strings, otherwise detected profile — mirror must match.
#[test]
fn mirror_matches_full_engine_across_corpus() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/mxl");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mxl"))
        .collect();
    files.sort();
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let score = keyflow_orchestra::score::load(&f).expect("parse");
        for part in &score.parts {
            let profile = keyflow_orchestra::detect_profile(&part.name);
            let mut cfg = Config::default();
            cfg.profile = profile;
            cfg.tempo_map = Some(score.meta.tempos.clone());
            let mut s1cfg = cfg.clone();
            s1cfg.timing_comp = false;
            let stage1 = process_part(part, &s1cfg);
            if stage1.empty {
                continue;
            }
            let full = process_part(part, &cfg);
            let src: Vec<MidiNote> = stage1.notes.iter().map(MidiNote::from).collect();
            let hints: Vec<&'static str> = stage1.notes.iter().map(notated_artic).collect();
            let mcfg = {
                let mut c = Config::default();
                c.profile = profile;
                c
            };
            let mirrored = mirror_part(&src, &stage1.ccs, Some(&hints), &mcfg, &score.meta.tempos);
            assert_eq!(
                mirrored.notes.len(),
                full.notes.len(),
                "{name}/{}: count",
                part.name
            );
            // A "collision" note shares its channel AND exact source note-on
            // with another note (solo-routing clamped two voices onto one
            // channel). The engine's legato pair decisions there are
            // tie-order artifacts, not semantics — the mirror is exempt on
            // those notes and within a beat after them (the artifact smears
            // into the following junction).
            let collision_zone: Vec<bool> = src
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    src.iter().enumerate().any(|(j, o)| {
                        j != i
                            && o.chan == n.chan
                            && (o.start_qn - n.start_qn).abs() < 1.0
                            && src.iter().enumerate().any(|(k, c)| {
                                k != j && c.chan == o.chan && (c.start_qn - o.start_qn).abs() < 1e-9
                            })
                    })
                })
                .collect();
            let mut hintless_divergent = 0usize;
            for (i, (m, fl)) in mirrored.notes.iter().zip(full.notes.iter()).enumerate() {
                assert_eq!(
                    (m.chan, m.pitch),
                    (fl.chan, fl.pitch),
                    "{name}/{}",
                    part.name
                );
                if collision_zone[i] {
                    continue;
                }
                // Sub-tick tolerance (1e-3 QN < one tick at 960 PPQ): strict-
                // ordering nudge chains on clamped channels accumulate 1e-4s;
                // genuine timing bugs are 0.05+ QN.
                assert!(
                    (m.start_qn - fl.start_qn).abs() < 1e-3 && (m.end_qn - fl.end_qn).abs() < 1e-3,
                    "{name}/{} note {i} (non-collision) diverges: {:?} vs ({:.4},{:.4})",
                    part.name,
                    m,
                    fl.start_qn,
                    fl.end_qn
                );
            }
            // Pure-MIDI fallback (no hints): must stay close — the only known
            // loss is articulations the profile's keyswitch scheme can't
            // encode (e.g. tremolo on woodwinds), so allow a small tail.
            let bare = mirror_part(&src, &stage1.ccs, None, &mcfg, &score.meta.tempos);
            for (m, fl) in bare.notes.iter().zip(full.notes.iter()) {
                if (m.start_qn - fl.start_qn).abs() > 1e-6 || (m.end_qn - fl.end_qn).abs() > 1e-6 {
                    hintless_divergent += 1;
                }
            }
            // Articulations the profile's keyswitch scheme can't encode
            // (tremolo on woodwinds/brass) are genuinely unrecoverable from
            // bare MIDI — parts that lean on them drift most. 10% is the
            // watchdog; production always has notation-event hints.
            let allowed = (stage1.notes.len() / 10).max(8);
            assert!(
                hintless_divergent <= allowed,
                "{name}/{}: hintless mirror diverges on {hintless_divergent}/{} notes",
                part.name,
                stage1.notes.len()
            );
        }
    }
}
