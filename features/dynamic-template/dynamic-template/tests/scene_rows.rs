//! Every scene, as data, against its committed row list.
//!
//! The `.rows` half of a scene fixture (spec #48's amendment): which
//! rows survived the folds, how deep each sits and how wide it opens,
//! compared **byte for byte**. This is the half that carries the
//! decision, it is the same on every machine, and it is what the scene
//! engine is actually responsible for — the `.png` beside it is the
//! renderer's half and is gated structurally by
//! `apps/session-daw/tests/golden_scenes.rs`.
//!
//! Resolved here from the golden session's own tree rather than through
//! the bench, so the loop is milliseconds and a wrong rule is a diff
//! rather than a render.

use dynamic_template::golden_session::{fixtures_dir, maximal, rpp::flatten, vocal_fx};
use dynamic_template::scenes::{self, Size, Surface};

/// The panel height every committed row list was written at.
const PANEL: f64 = 1440.0;

/// One scene's row list, in the bench's own format: depth, name, width.
fn rows_of(slug: &str) -> String {
    let scene = scenes::scene(slug).unwrap_or_else(|| panic!("no scene {slug}"));
    let shape = if slug.starts_with("lead-vocal") {
        vocal_fx()
    } else {
        maximal()
    };
    let flats = flatten(&shape);
    let facts = scenes::from_flat(&flats);
    let mut text = String::new();
    for row in scenes::resolve(scene, &facts, Surface::Mixer, None, None, None) {
        let name = match &row.target {
            // A performer header carries no guid, only a name — the
            // same name the window's synthetic header track is given
            // (`session_daw::plan::apply_scene`), so this text agrees
            // with what the bench actually renders
            // (`apps/session-daw/tests/golden_scenes.rs`).
            scenes::Target::PerformerHeader(performer) => performer.clone(),
            scenes::Target::Track(guid) => flats
                .iter()
                .find(|f| &f.guid == guid)
                .map_or(String::new(), |f| f.name.clone()),
        };
        let width = scenes::TABLES.width(row.size, PANEL).max(1.0).round();
        text.push_str(&format!("{}\t{name}\t{width}\n", row.depth));
    }
    // The widths are whole pixels in the committed file.
    text.replace(".0\n", "\n")
}

fn committed(slug: &str) -> String {
    let path = fixtures_dir().join("scenes").join(format!("{slug}.rows"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The acceptance criterion: all nine scenes, as data, reproduce the row
/// list their hand-written functions produced.
///
/// r[verify flow.scenes.render]
/// r[verify flow.drums.tracking.full]
/// r[verify flow.drums.mixing.scenes]
/// r[verify flow.vocals.mixing.main]
/// r[verify flow.vocals.mixing.fx]
#[test]
fn every_scene_resolves_to_its_committed_row_list() {
    let mut failures = Vec::new();
    for scene in scenes::scenes() {
        let fresh = rows_of(&scene.slug);
        let want = committed(&scene.slug);
        if fresh != want {
            let (n, a, b) = fresh
                .lines()
                .zip(want.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map_or((0, "", ""), |(n, (a, b))| (n + 1, a, b));
            failures.push(format!(
                "{}: {} rows, committed {}; first difference at row {n}:\n    \
                 resolved:  {a}\n    committed: {b}",
                scene.slug,
                fresh.lines().count(),
                want.lines().count()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A scene that resolves to nothing visible fails its fixture.
///
/// The negative control for the test above: a committed row list of
/// nothing could be written once and never looked at again, so an empty
/// resolution has to be an error on its own terms rather than a fixture
/// that happens to match.
#[test]
fn a_scene_resolving_to_no_rows_fails_its_fixture() {
    for scene in scenes::scenes() {
        assert!(
            rows_of(&scene.slug).lines().count() > 0,
            "{} resolves to nothing",
            scene.slug
        );
    }
    // And the control: a scene whose default hides everything does
    // resolve to nothing, and this test would catch it.
    let mut blind = scenes::scene("drum-mixing").expect("a scene").clone();
    blind.default = scenes::Effect::hidden();
    blind.rules.clear();
    let facts = scenes::from_flat(&flatten(&maximal()));
    let rows = scenes::resolve(&blind, &facts, Surface::Mixer, None, None, None);
    assert!(
        rows.is_empty(),
        "a scene that hides everything shows nothing"
    );
}

/// The two surfaces are two tables: the same scene gives a strip a width
/// and a row a height, and neither number is the other.
#[test]
fn a_scene_sizes_both_surfaces_from_its_own_table() {
    let facts = scenes::from_flat(&flatten(&maximal()));
    let scene = scenes::scene("drum-mixing").expect("a scene");
    let mixer = scenes::resolve(scene, &facts, Surface::Mixer, None, None, None);
    let arrange = scenes::resolve(scene, &facts, Surface::Arrange, None, None, None);
    assert_eq!(mixer.len(), arrange.len(), "the same rows survive");
    let kick = mixer
        .iter()
        .zip(&arrange)
        .find(|(m, _)| m.size == Size::Working)
        .expect("something is worked on");
    assert!((scenes::TABLES.width(kick.0.size, PANEL) - 133.0).abs() < f64::EPSILON);
    assert!((scenes::TABLES.height(kick.1.size) - 96.0).abs() < f64::EPSILON);
}

#[test]
#[ignore]
fn tmp_write_fixture() {
    let path = fixtures_dir().join("scenes").join("drum-tracking.rows");
    std::fs::write(&path, rows_of("drum-tracking")).unwrap();
}
