//! Corpus tests: every .mxl in examples/mxl must parse and analyze.

use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/mxl")
}

fn corpus_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .expect("examples/mxl missing")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mxl"))
        .collect();
    files.sort();
    files
}

#[test]
fn all_corpus_files_parse() {
    let files = corpus_files();
    assert!(
        files.len() >= 18,
        "expected ≥18 mxl files, got {}",
        files.len()
    );
    let mut failures = Vec::new();
    for f in &files {
        match keyflow_orchestra::score::load(f) {
            Ok(score) => {
                let total_notes: usize = score.parts.iter().map(|p| p.notes.len()).sum();
                assert!(
                    total_notes > 0,
                    "{}: parsed but zero notes",
                    f.file_name().unwrap().to_string_lossy()
                );
                assert!(!score.meta.tempos.is_empty());
            }
            Err(e) => failures.push(format!("{}: {e}", f.file_name().unwrap().to_string_lossy())),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus files failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn engine_runs_on_all_corpus_parts_and_invariants_hold() {
    use keyflow_orchestra::{detect_profile, process_part, Config};
    for f in corpus_files() {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let score = keyflow_orchestra::score::load(&f).expect("parse");
        for part in &score.parts {
            let mut cfg = Config::default();
            cfg.profile = detect_profile(&part.name);
            cfg.tempo_map = Some(score.meta.tempos.clone());
            let out = process_part(part, &cfg);
            if out.empty {
                continue;
            }
            let ctx = format!("{name} / {}", part.name);

            // Channels in range, 1-based.
            for n in &out.notes {
                assert!(
                    (1..=16).contains(&n.chan),
                    "{ctx}: channel {} out of range",
                    n.chan
                );
                assert!((1..=127).contains(&n.vel), "{ctx}: velocity {}", n.vel);
                assert!(
                    n.end_qn > n.start_qn,
                    "{ctx}: non-positive note length at {}",
                    n.start_qn
                );
            }

            // HARD GUARANTEE: same-pitch notes on a channel never overlap.
            let mut by_ch: std::collections::BTreeMap<
                u8,
                Vec<&keyflow_orchestra::engine::OutNote>,
            > = Default::default();
            for n in &out.notes {
                by_ch.entry(n.chan).or_default().push(n);
            }
            for (ch, mut list) in by_ch {
                list.sort_by(|a, b| a.start_qn.total_cmp(&b.start_qn));
                let mut last_end: std::collections::BTreeMap<i32, f64> = Default::default();
                for n in &list {
                    if let Some(&e) = last_end.get(&n.pitch) {
                        assert!(
                            n.start_qn >= e - 1e-9,
                            "{ctx}: same-pitch overlap on ch{ch} pitch {} at {} (prev ends {})",
                            n.pitch,
                            n.start_qn,
                            e
                        );
                    }
                    let cur = last_end.entry(n.pitch).or_insert(f64::NEG_INFINITY);
                    *cur = cur.max(n.end_qn);
                }

                // Keyswitches: first CC58 on this channel lands before the
                // channel's first note-on.
                let first_note = list.first().map(|n| n.start_qn).unwrap();
                if let Some(first_ks) = out
                    .ccs
                    .iter()
                    .filter(|c| c.cc == 58 && c.chan == ch)
                    .map(|c| c.qn)
                    .next()
                {
                    assert!(
                        first_ks <= first_note + 1e-9,
                        "{ctx}: first CC58 at {first_ks} after first note {first_note} on ch{ch}"
                    );
                }
            }

            // CC64 (re-bow/re-tongue pedal) must read as a clean state
            // machine per channel: alternating on/off at increasing times —
            // CSS itself interprets this stream, so an interleaved off would
            // drop the pedal mid-run.
            let mut pedal_state: std::collections::BTreeMap<u8, (bool, f64)> = Default::default();
            for e in out.ccs.iter().filter(|e| e.cc == 64) {
                let on = e.val >= 64;
                let (prev_on, prev_qn) = pedal_state
                    .get(&e.chan)
                    .copied()
                    .unwrap_or((false, f64::NEG_INFINITY));
                assert_ne!(
                    on, prev_on,
                    "{ctx}: CC64 on ch{} repeats state {} at {:.4}",
                    e.chan, e.val, e.qn
                );
                assert!(
                    e.qn > prev_qn,
                    "{ctx}: CC64 on ch{} not strictly ordered at {:.4}",
                    e.chan,
                    e.qn
                );
                pedal_state.insert(e.chan, (on, e.qn));
            }

            // CC stream is sorted by (qn, chan, cc).
            for w in out.ccs.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                assert!(
                    a.qn < b.qn + 1e-12 || (a.qn == b.qn && (a.chan, a.cc) <= (b.chan, b.cc)),
                    "{ctx}: CC stream unsorted"
                );
            }
        }
    }
}

#[test]
fn analysis_reports_instrumentation() {
    // Jurassic Park: a full orchestral score — should detect several families.
    let f = corpus_dir().join("theme-from-jurassic-park.mxl");
    let score = keyflow_orchestra::score::load(&f).expect("parse");
    let report = keyflow_orchestra::analysis::analyze(&score);
    assert!(
        report.parts.len() > 5,
        "expected many parts, got {}",
        report.parts.len()
    );
    use keyflow_orchestra::ProfileKind;
    let kinds: std::collections::HashSet<_> = report.parts.iter().map(|p| p.profile).collect();
    assert!(
        kinds.contains(&ProfileKind::Strings),
        "no strings detected: {:?}",
        report
            .parts
            .iter()
            .map(|p| (&p.name, p.profile))
            .collect::<Vec<_>>()
    );
    assert!(
        kinds.contains(&ProfileKind::Brass) || kinds.contains(&ProfileKind::BrassTrumpet),
        "no brass detected"
    );
}
