//! The golden session as the window first shows it — opened on
//! daw-standalone and read back the way the app reads a song
//! (`StudioSession::read_current`), its rows planned by the opening scene
//! (`drum-mixing`), and the strips the arrangement hands the mixer docked
//! under it (`ArrangementWidget::with_mixer`).
//!
//! Not the scene table on its own (`scenes::table`'s tests do that): the
//! same open, read-back, taxonomy, plan and fold the app goes through,
//! because a rule that resolves right in isolation once never reached the
//! mixer — the docked mixer was handed the ARRANGEMENT's rows.
//!
//! One test, one process: the daw facade is bound to the first engine
//! opened.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use session_daw::studio::StudioSession;
use session_daw::widget::{ArrangementWidget, MixerLinks, View};

type Rows = Vec<(daw_proto::Track, u32)>;

fn template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../features/dynamic-template/fixtures/golden/template.rpp")
}

/// Each row as `depth name`, the folder's name, for the rows from `from`
/// (a top-level folder) to the next top-level row.
fn family(rows: &Rows, from: &str) -> Vec<String> {
    let Some(start) = rows.iter().position(|(t, d)| *d == 0 && t.name == from) else {
        return Vec::new();
    };
    rows.iter()
        .skip(start)
        .enumerate()
        .take_while(|(i, (_, depth))| *i == 0 || *depth > 0)
        .map(|(_, (track, depth))| format!("{depth} {}", track.name))
        .collect()
}

#[test]
fn the_default_view_folds_the_vocals_and_keeps_process_out_of_view() {
    let path = template();
    session_daw::open::open_silent(&path).expect("the golden session opens");
    let session = StudioSession::read_current(Some(&path), None).expect("and reads back");

    // What the arrangement shows.
    let arranged: Rows = session.rows.as_slice().to_vec();
    // What it hands the mixer docked under it.
    let shared = Rc::new(RefCell::new((0_u64, Rows::new())));
    let _widget = ArrangementWidget::for_session(
        &session.project,
        &session.rows,
        &session.previews,
        false,
        Rc::new(RefCell::new(View::OPENING)),
        false,
    )
    .with_planner(session.planner.clone(), Rc::default())
    .with_mixer(MixerLinks {
        toggle: Rc::default(),
        rows: Rc::clone(&shared),
        echo: Rc::default(),
        keys: Rc::default(),
    });
    let docked: Rows = shared.borrow().1.clone();
    // And the mixer on its own, outside Mix mode.
    let alone: Rows = session
        .planner
        .plan_strips(&session.planner.raw)
        .as_slice()
        .to_vec();

    // The vocals in the arrangement: each lead's line and double one row,
    // each BGV and choir part one row, the returns not there at all.
    assert_eq!(
        family(&arranged, "Vocals"),
        [
            "0 Vocals", "1 Ron", "2 Main", "2 DBL", "1 Belen", "2 Main", "2 DBL", "1 Aline",
            "2 Main", "2 DBL", "1 VOX EN", "1 VOX ES", "1 VOX PT",
        ]
    );
    // The BGVs and the choir are folders of their own beside Vocals,
    // each part one row.
    assert_eq!(
        family(&arranged, "BGVs"),
        [
            "0 BGVs",
            "1 Octave Up",
            "1 Octave Down",
            "1 Higher Harmony",
            "1 Lower Harmony",
            "1 Hey",
        ]
    );
    assert_eq!(
        family(&arranged, "Choir"),
        ["0 Choir", "1 Soprano", "1 Alto", "1 Tenor", "1 Bass"]
    );
    // Folded, not dropped: a part's row carries its sources' items.
    let ron_main = arranged
        .iter()
        .skip_while(|(t, d)| !(*d == 1 && t.name == "Ron"))
        .find(|(t, d)| *d == 2 && t.name == "Main")
        .map(|(t, _)| t.guid.clone())
        .expect("Ron's Main");
    assert!(
        session
            .project
            .0
            .folds
            .get(&ron_main)
            .is_some_and(|fold| !fold.items.is_empty()),
        "Ron's Main row carries his language takes"
    );

    // The kit's parallel chain is out of the arrangement too.
    assert!(
        family(&arranged, "Process").is_empty(),
        "the arrangement shows {:?}",
        family(&arranged, "Process")
    );

    for (mixer, strips) in [("docked", &docked), ("alone", &alone)] {
        // The mixer: no Process strip, and nothing of it.
        assert!(
            family(strips, "Process").is_empty(),
            "the {mixer} mixer shows {:?}",
            family(strips, "Process")
        );
        // Negative control: the drums beside it are still there, every
        // mic of the kick a strip — the kick's fold is the arrangement's.
        let kit = family(strips, "Drum Kit");
        assert!(
            kit.contains(&"3 In".to_owned()),
            "the {mixer} mixer: {kit:?}"
        );
        // Every vocal source and return is a strip.
        let vocals = family(strips, "Vocals");
        for want in ["1 Vox FX", "3 EN", "3 ES", "3 PT"] {
            assert!(
                vocals.iter().any(|row| row == want),
                "the {mixer} mixer has no {want}: {vocals:?}"
            );
        }
        // The BGVs' every layer is a strip, in their own folder.
        let bgvs = family(strips, "BGVs");
        for want in ["2 EN", "2 All 8"] {
            assert!(
                bgvs.iter().any(|row| row == want),
                "the {mixer} mixer has no {want}: {bgvs:?}"
            );
        }
        assert_eq!(
            vocals.len(),
            family(&alone, "Vocals").len(),
            "the {mixer} mixer"
        );
    }
}
