//! Every scene renders from the golden session to its committed fixture.
//!
//! `plan::SCENES` is the visual track manager's scene table; each scene
//! is resolved and rendered headless by the bench
//! (`FTS_BENCH_SCENE=<slug>`) from the golden session under
//! `features/dynamic-template/fixtures/golden/`, and both halves of what
//! comes out are committed beside it under `scenes/`:
//!
//! - **the row list** (`<slug>.rows`) — which rows survived the folds,
//!   how deep each sits and how wide it opens. This is what the scene
//!   *means*, it is the same on every machine, and it is compared **byte
//!   for byte**. A changed rule moves it.
//! - **the picture** (`<slug>.png`) at 2560x1440 — what the scene *looks
//!   like*. Compared structurally, with the repo's own threshold
//!   (`scripts/ui-stress/imagediff.py`: a channel difference over 24 of
//!   255 is a difference, anything under it is rasterisation). Two GPUs —
//!   and the same GPU reached through a different driver, which is what
//!   CI does — disagree about the last few bits of antialiasing:
//!   measured here, up to 15 of 255 on four per cent of pixels, with no
//!   structural difference at all. A moved strip, a changed fold or a
//!   different colour is not that: it is thousands of pixels past the
//!   threshold, and the negative control below pins that.
//!
//! `just daw-scenes` rewrites both.

use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

/// The size every picture is committed at.
const SIZE: &str = "2560x1440";
/// A per-channel difference this large or smaller is rasterisation, not
/// a different picture — `scripts/ui-stress/imagediff.py`'s own default.
const STRUCTURAL: i16 = 24;
/// How many pixels may differ structurally: none.
const STRUCTURAL_TOLERANCE: usize = 0;
/// Below this many colours, a render is a blank window rather than a
/// scene.
const MIN_COLOURS: usize = 64;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features/dynamic-template/fixtures/golden")
}

/// The project a scene renders from: the vocal template for the vocal
/// scenes, the maximal session for everything else.
fn project_for(slug: &str) -> PathBuf {
    let file = if slug.starts_with("lead-vocal") {
        "vocal-fx.rpp"
    } else {
        "template.rpp"
    };
    fixtures().join(file)
}

/// Render one scene through the bench, writing the picture to `png` and
/// the row list to `rows`.
// r[impl flow.scenes.render]
fn render(slug: &str, png: &Path, rows: &Path) -> Result<()> {
    let bench = env!("CARGO_BIN_EXE_bench");
    let output = Command::new(bench)
        .arg(project_for(slug))
        .env("FTS_BENCH_MIXER", png)
        .env("FTS_BENCH_ROWS", rows)
        .env("FTS_BENCH_SCENE", slug)
        .env("FTS_BENCH_SIZE", SIZE)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "bench failed rendering {slug} ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    if !png.is_file() || !rows.is_file() {
        return Err(format!(
            "bench wrote nothing for {slug}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

/// The pixels of a PNG as RGBA8, with its size.
fn pixels(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let image = image::open(path)?.into_rgba8();
    let (w, h) = image.dimensions();
    Ok((w, h, image.into_raw()))
}

/// How two renders differ: the count of pixels past the structural
/// threshold, and the largest per-channel difference among all of them.
fn compare(a: &[u8], b: &[u8]) -> (usize, i16) {
    let mut structural = 0_usize;
    let mut worst = 0_i16;
    for (pa, pb) in a.chunks(4).zip(b.chunks(4)) {
        let delta = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| i16::from(*x).saturating_sub(i16::from(*y)).saturating_abs())
            .max()
            .unwrap_or(0);
        if delta > STRUCTURAL {
            structural = structural.saturating_add(1);
        }
        worst = worst.max(delta);
    }
    (structural, worst)
}

/// How many distinct colours a render uses.
///
/// A scene that resolves to nothing visible renders as the window's
/// background and little else, and a picture of nothing could be
/// committed and never looked at again. Counting colours is the cheap
/// way to tell a window full of strips from an empty one.
fn colours(pixels: &[u8]) -> usize {
    pixels
        .chunks(4)
        .map(<[u8]>::to_vec)
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// Every scene in the table, against its row list and its picture —
/// which is how the drum scenes are verified: what Drum Tracking shows,
/// and what the mixing scenes show, is the committed fixture.
///
/// r[verify flow.scenes.render]
/// r[verify flow.drums.tracking.full]
/// r[verify flow.drums.mixing.scenes]
#[test]
fn every_scene_renders_to_its_committed_fixture() -> Result<()> {
    let update = std::env::var_os("FTS_UPDATE_GOLDEN").is_some();
    // The media the projects reference is generated, not committed, so a
    // fresh checkout has projects with nothing to play until this runs.
    dynamic_template::golden_session::write_media(&fixtures())?;
    let scenes_dir = fixtures().join("scenes");
    let scratch = tempfile::tempdir()?;
    let mut failures = Vec::new();
    for scene in &session_daw::plan::SCENES {
        let committed_png = scenes_dir.join(format!("{}.png", scene.slug));
        let committed_rows = scenes_dir.join(format!("{}.rows", scene.slug));
        let fresh_png = scratch.path().join(format!("{}.png", scene.slug));
        let fresh_rows = scratch.path().join(format!("{}.rows", scene.slug));
        render(scene.slug, &fresh_png, &fresh_rows)?;
        if update {
            std::fs::copy(&fresh_png, &committed_png)?;
            std::fs::copy(&fresh_rows, &committed_rows)?;
            continue;
        }
        if !committed_png.is_file() || !committed_rows.is_file() {
            failures.push(format!(
                "{}: no committed fixture beside {}",
                scene.slug,
                committed_png.display()
            ));
            continue;
        }

        // The row list, byte for byte.
        let fresh = std::fs::read_to_string(&fresh_rows)?;
        let committed = std::fs::read_to_string(&committed_rows)?;
        if fresh != committed {
            let (n, a, b) = fresh
                .lines()
                .zip(committed.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map_or((0, "", ""), |(n, (a, b))| (n.saturating_add(1), a, b));
            failures.push(format!(
                "{}: the scene resolves to a different row list ({} rows, committed {}); \
                 first difference at row {n}:\n    resolved:  {a}\n    committed: {b}",
                scene.slug,
                fresh.lines().count(),
                committed.lines().count()
            ));
        }
        if fresh.lines().count() == 0 {
            failures.push(format!(
                "{}: the scene resolves to no rows at all",
                scene.slug
            ));
        }

        // And the picture, structurally.
        let (w, h, a) = pixels(&fresh_png)?;
        let (cw, ch, b) = pixels(&committed_png)?;
        if (w, h) != (cw, ch) {
            failures.push(format!(
                "{}: rendered {w}x{h}, committed {cw}x{ch}",
                scene.slug
            ));
            continue;
        }
        let used = colours(&a);
        if used < MIN_COLOURS {
            failures.push(format!(
                "{}: rendered {used} colours — a scene that resolves to nothing visible",
                scene.slug
            ));
            continue;
        }
        let (structural, worst) = compare(&a, &b);
        if structural > STRUCTURAL_TOLERANCE {
            // Kept where the fixtures are NOT: a failing run must not
            // leave six hundred kilobytes of untracked PNG beside the
            // committed ones, where the next `add -A` would sweep them
            // in.
            let kept = std::env::temp_dir().join(format!("fts-scene-{}.fresh.png", scene.slug));
            std::fs::copy(&fresh_png, &kept)?;
            failures.push(format!(
                "{}: {structural} pixels differ structurally (worst channel by {worst} of 255); \
                 fresh render kept at {}",
                scene.slug,
                kept.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "scene fixtures drifted — if the change is intended, run `just daw-scenes` and \
         commit the result:\n{}",
        failures.join("\n")
    );
    Ok(())
}

/// The negative control: two scenes of the same project are different
/// pictures and different row lists by far more than the tolerances, so
/// neither comparison above could pass by being lenient.
#[test]
fn two_scenes_differ_by_more_than_the_tolerances() -> Result<()> {
    let scenes_dir = fixtures().join("scenes");
    let (_, _, a) = pixels(&scenes_dir.join("drum-tracking.png"))?;
    let (_, _, b) = pixels(&scenes_dir.join("drum-mixing.png"))?;
    let (structural, worst) = compare(&a, &b);
    assert!(
        structural > 10_000,
        "{structural} pixels differ structurally"
    );
    assert!(worst > STRUCTURAL, "worst {worst}");
    let rows_a = std::fs::read_to_string(scenes_dir.join("drum-tracking.rows"))?;
    let rows_b = std::fs::read_to_string(scenes_dir.join("drum-mixing.rows"))?;
    assert_ne!(rows_a, rows_b);
    Ok(())
}

/// Every scene has a committed picture at the committed size and a row
/// list with rows in it, and none of the pictures is a blank window.
#[test]
fn every_scene_has_a_fixture_with_something_in_it() -> Result<()> {
    for scene in &session_daw::plan::SCENES {
        let dir = fixtures().join("scenes");
        let (w, h, committed) = pixels(&dir.join(format!("{}.png", scene.slug)))?;
        assert_eq!((w, h), (2560, 1440), "{}", scene.slug);
        let used = colours(&committed);
        assert!(used >= MIN_COLOURS, "{} uses {used} colours", scene.slug);
        let rows = std::fs::read_to_string(dir.join(format!("{}.rows", scene.slug)))?;
        assert!(rows.lines().count() > 0, "{} has no rows", scene.slug);
    }
    Ok(())
}

/// The negative control for that: a picture of nothing has one colour,
/// so the count above could not pass an empty scene.
#[test]
fn a_blank_picture_reads_as_blank() {
    let blank = vec![0_u8; 2560 * 4];
    assert_eq!(colours(&blank), 1);
    assert!(colours(&blank) < MIN_COLOURS);
}
