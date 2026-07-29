//! P1 acceptance: importer inventory round-trips against the raw XML.
//!
//! Every pitched/unpitched `<note>` element must become exactly one model
//! note (chord followers stack, graces and cues included), every `<rest>`
//! one model rest, and the measure grid must be uniform across parts.
//! Corpus files outside the repo are skipped when absent.

use std::path::{Path, PathBuf};

fn in_repo_fixture() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../../crates/keyflow/examples/png-project-charts");
    p.push("02 LORD OF THE FIGHT Master RS.musicxml");
    p
}

/// Raw-text ground truth, robust to attribute forms (`<rest/>`,
/// `<rest measure="yes"/>`).
fn raw_counts(xml: &str) -> (usize, usize) {
    let pitched = xml.matches("<pitch>").count() + xml.matches("<unpitched>").count();
    let rests = xml.matches("<rest").count();
    (pitched, rests)
}

fn xml_text(path: &Path) -> String {
    let data = std::fs::read(path).expect("fixture readable");
    if data.starts_with(b"PK") {
        // .mxl: count against the extracted score entry by re-running the
        // importer's own extraction path indirectly — simplest is to unzip
        // the largest XML entry here.
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).expect("zip");
        let mut best: Option<(usize, String)> = None;
        for i in 0..archive.len() {
            let entry = archive.by_index(i).expect("entry");
            let name = entry.name().to_string();
            let lower = name.to_lowercase();
            if !name.starts_with("META-INF")
                && (lower.ends_with(".xml") || lower.ends_with(".musicxml"))
                && best.as_ref().is_none_or(|(size, _)| entry.size() as usize > *size)
            {
                best = Some((entry.size() as usize, name));
            }
        }
        let (_, name) = best.expect("score entry in mxl");
        let mut file = archive.by_name(&name).expect("entry by name");
        let mut text = String::new();
        std::io::Read::read_to_string(&mut file, &mut text).expect("utf8 score xml");
        text
    } else {
        String::from_utf8_lossy(&data).into_owned()
    }
}

fn check_invariants(path: &Path) {
    let score = engraver_score::import_file(path).expect("import should succeed");
    let inventories = engraver_score::inventory(&score);
    assert!(!inventories.is_empty(), "score should have parts");

    let xml = xml_text(path);
    let (raw_pitched, raw_rests) = raw_counts(&xml);
    let model_notes: usize = inventories.iter().map(|i| i.notes).sum();
    let model_rests: usize = inventories.iter().map(|i| i.rests).sum();
    assert_eq!(
        model_notes,
        raw_pitched,
        "every <pitch>/<unpitched> must become one model note ({path:?})"
    );
    assert_eq!(
        model_rests, raw_rests,
        "every <rest> must become one model rest ({path:?})"
    );

    // Partwise invariant: all parts share the measure grid.
    let measures = inventories[0].measures;
    for inv in &inventories {
        assert_eq!(
            inv.measures, measures,
            "part {} measure count diverges ({path:?})",
            inv.name
        );
    }
}

#[test]
fn lord_of_the_fight_inventory_round_trips() {
    let path = in_repo_fixture();
    check_invariants(&path);

    let score = engraver_score::import_file(&path).unwrap();
    // The file's movement-title carries literal quote characters; the
    // importer preserves them verbatim (no editorializing).
    assert_eq!(
        score.movement_title.as_deref(),
        Some("\"WHO'S THERE, GOD'S THERE\"")
    );
    let inv = engraver_score::inventory(&score);
    assert_eq!(inv.len(), 1, "master rhythm chart is a single part");
    assert!(inv[0].notes > 0);
    // Lord of the Fight is in E major (4 sharps), bass clef.
    assert_eq!(inv[0].keys.first(), Some(&4));
    assert_eq!(inv[0].clefs.first().map(String::as_str), Some("F4"));
}

/// Out-of-repo corpus (Alan Parsons scores, film .mxl downloads): exercised
/// when present on this machine, silently skipped elsewhere.
#[test]
fn corpus_inventories_round_trip() {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{home}/.fts-scratch/aplp-sample/games.musicxml"),
        format!(
            "{home}/task-staging/alan-parsons/org/resources/latest-charts/Games People Play - Score.musicxml"
        ),
        "/run/media/Development/FastTrackStudio-legacy/keyflow/examples/mxl/the-epic-how-to-train-your-dragon-orchestral-suite.mxl"
            .to_string(),
        "/run/media/Development/FastTrackStudio-legacy/keyflow/examples/mxl/theme-from-jurassic-park.mxl"
            .to_string(),
    ];
    let mut checked = 0;
    for candidate in candidates {
        let path = PathBuf::from(&candidate);
        if !path.exists() {
            eprintln!("skip (absent): {candidate}");
            continue;
        }
        check_invariants(&path);
        checked += 1;
    }
    eprintln!("corpus files checked: {checked}");
}

#[test]
fn games_people_play_is_a_full_orchestra() {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = PathBuf::from(format!("{home}/.fts-scratch/aplp-sample/games.musicxml"));
    if !path.exists() {
        eprintln!("skip (absent): {path:?}");
        return;
    }
    let score = engraver_score::import_file(&path).expect("import");
    let inv = engraver_score::inventory(&score);
    assert_eq!(inv.len(), 23, "games.musicxml has 23 parts");
    let violin = inv
        .iter()
        .find(|i| i.name.contains("Violin"))
        .expect("violin part");
    assert!(violin.notes > 0, "violin should carry notes");
    // This Finale export splits keyboard instruments into single-staff
    // parts (zero <staves> elements in the file) — assert the clef spread
    // instead: treble and bass families must both be present.
    assert!(
        inv.iter().any(|i| i.clefs.iter().any(|c| c.starts_with('G'))),
        "some parts should be treble clef"
    );
    assert!(
        inv.iter().any(|i| i.clefs.iter().any(|c| c.starts_with('F'))),
        "some parts should be bass clef"
    );
    // Finale writes plain <rest/> with full-bar durations, never
    // measure="yes" — multirest detection in P2 must be duration-based.
    assert!(
        inv.iter().map(|i| i.rests).sum::<usize>() > 1000,
        "orchestral parts should be rest-heavy"
    );
}
