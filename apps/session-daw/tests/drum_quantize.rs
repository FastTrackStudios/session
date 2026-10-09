//! A drum quantize, end to end on the golden session: the kit's hits read
//! off its folded row, every mic cut and moved onto the sixteenths
//! through the engine, the session read back — and the hits found in it
//! again are on the grid, where the drummer's were up to 18 ms off it
//! (`golden_session::perform::kit_played`).
//!
//! One test, one process: the daw facade is bound to the first engine
//! opened.

use std::path::Path;

use daw_ui::studio::project::Project;
use session_daw::arrange_edit::{Editor, Effect};
use session_daw::folder_item::{
    GroupBy,
    hits::hits,
    load::{DETECT_BLOCK_SIZE, load_at},
};
use session_daw::studio::StudioSession;

/// The session as the engine has it now, with the kit's folder shut.
fn read(path: &Path) -> (Project, String) {
    let session = StudioSession::read_current(Some(path), None).expect("the session reads back");
    let mut project = (*session.project.0).clone();
    let kit = project
        .tracks
        .iter()
        .find(|t| t.name == "Drums")
        .expect("the kit")
        .guid
        .clone();
    let under = |t: &daw_proto::Track| {
        let mut parent = t.parent_guid.clone();
        while let Some(p) = parent {
            if p == kit {
                return true;
            }
            parent = project
                .tracks
                .iter()
                .find(|x| x.guid == p)
                .and_then(|x| x.parent_guid.clone());
        }
        false
    };
    let shown: Vec<daw_proto::Track> = project
        .tracks
        .iter()
        .filter(|t| !under(t))
        .cloned()
        .collect();
    daw_ui::studio::folded::refold(&mut project, &shown, true);
    (project, kit)
}

/// The kit's hits between `from` and `to`, read off its mics' peaks.
fn kit_hits(project: &Project, kit: &str, from: f64, to: f64) -> Vec<f64> {
    let runtime = session_daw::open::runtime().expect("a runtime");
    runtime.block_on(async {
        let daw = daw::rpc::Daw::try_get().expect("the facade");
        let current = daw.current_project().await.expect("a project");
        let (folders, _) = load_at(
            &current,
            project,
            &[(kit.to_owned(), GroupBy::Role)],
            vello::peniko::Color::WHITE,
            DETECT_BLOCK_SIZE,
        )
        .await;
        folders
            .iter()
            .find(|f| f.guid == kit)
            .map(|f| hits(f, 0, from, to))
            .unwrap_or_default()
    })
}

/// How far each hit is from the nearest sixteenth, in seconds.
fn off_grid(hits: &[f64], step: f64) -> Vec<f64> {
    hits.iter()
        .map(|h| (h - (h / step).round() * step).abs())
        .collect()
}

#[test]
fn quantized_the_kit_is_on_the_grid() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../features/dynamic-template/fixtures/golden/template.rpp");
    session_daw::open::open_silent(&path).expect("the golden session opens");
    let (mut project, kit) = read(&path);
    let fold = project.folds.get(&kit).expect("the shut kit's fold");
    // The longest take inside the song (the performance is the song's
    // length; past it the mics are silent).
    let song_end = 64.0 * 2.0;
    let row = fold
        .items
        .iter()
        .filter(|i| i.position.as_seconds() + i.length.as_seconds() <= song_end)
        .max_by(|a, b| a.length.as_seconds().total_cmp(&b.length.as_seconds()))
        .expect("a take on the row")
        .clone();
    let (from, to) = (
        row.position.as_seconds(),
        row.position.as_seconds() + row.length.as_seconds(),
    );
    let step = 60.0 / 120.0 / 4.0;

    let found = kit_hits(&project, &kit, from, to);
    assert!(found.len() > 8, "the kit plays: {} hits", found.len());
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let before = mean(&off_grid(&found, step));
    assert!(before > 0.004, "the drummer is off the grid: {before}");

    let mut effects = Vec::new();
    Editor::quantize(
        &mut project,
        &[(row.guid.clone(), found)],
        step,
        1.0,
        &mut effects,
    );
    let sent: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(edit) => Some(edit),
            _ => None,
        })
        .collect();
    assert!(
        sent.len() > 20,
        "every mic cut and moved: {} edits ({:?})",
        sent.len(),
        effects
            .iter()
            .filter(|e| !matches!(e, Effect::Send(_)))
            .collect::<Vec<_>>()
    );
    for edit in sent {
        assert!(session_daw::engine::apply_now(edit));
    }

    // Judged against the truth, not a second detection (whose own
    // resolution, a peak block, is 14.5 ms on these 11 kHz files): every
    // kick the drummer played, where the kick In mic plays it now. A
    // slice plays its source from its offset, so a moment `t` of the
    // performance (the source is the song, from zero) plays at
    // `position + t - offset` in the slice holding it.
    let (project, _) = read(&path);
    let kick_in = project
        .tracks
        .iter()
        .find(|t| {
            t.name == "In"
                && project
                    .tracks
                    .iter()
                    .any(|p| Some(&p.guid) == t.parent_guid.as_ref() && p.name == "Sum")
        })
        .expect("the kick's In");
    let slices = project.lane(&kick_in.guid);
    let plays = |t: f64| {
        slices.iter().find_map(|i| {
            let (pos, off) = (i.position.as_seconds(), i.start_offset.as_seconds());
            let len = i.length.as_seconds() - 0.006;
            (t >= off && t < off + len).then_some(pos + t - off)
        })
    };
    use dynamic_template::golden_session::{perform, shape};
    let (mut was, mut now) = (Vec::new(), Vec::new());
    let song = shape::maximal();
    for (on, piece, _) in perform::groove(&song) {
        if piece != perform::Piece::Kick || on < from + 0.1 || on > to - 0.1 {
            continue;
        }
        let played = perform::kit_played(on, piece, song.bpm);
        let Some(at) = plays(played) else { continue };
        was.push((played - on).abs());
        now.push((at - on).abs());
    }
    assert!(now.len() > 8, "kicks in the take: {}", now.len());
    let (was, now) = (mean(&was), mean(&now));
    assert!(
        now < 0.5 * was,
        "quantized, the kicks are on the grid: {:.1} ms off on average, from {:.1} ms",
        now * 1e3,
        was * 1e3
    );
}
