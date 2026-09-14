//! Every scene renders from the golden session to its committed picture.
//!
//! `plan::SCENES` is the visual track manager's scene table; each scene
//! is rendered headless by the bench (`FTS_BENCH_SCENE=<slug>`) from the
//! golden session under `features/dynamic-template/fixtures/golden/`,
//! and the picture is committed beside it under `scenes/`. A change to
//! what a scene shows is a diff in a PR, not a surprise.
//!
//! The comparison is per pixel with a one-LSB tolerance on a handful of
//! pixels rather than byte for byte: Vello rasterises on the GPU, and
//! across two processes the same frame can differ by one unit in one
//! channel on one or two pixels (measured: the same project rendered
//! three times gave two hashes, two pixels apart, each off by one).
//! Anything a scene rule could change — a strip's width, a fold, a
//! colour — moves thousands of pixels by far more than one.
//!
//! `FTS_UPDATE_GOLDEN=1 cargo test -p session-daw --test golden_scenes`
//! rewrites the pictures; so does `just daw-scenes`.

use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

/// The size every picture is committed at.
const SIZE: &str = "2560x1440";
/// The largest per-channel difference a pixel may show.
const LSB_TOLERANCE: i16 = 1;
/// How many pixels may differ at all.
const PIXEL_TOLERANCE: usize = 16;

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

/// Render one scene through the bench to `out`.
// r[impl flow.scenes.render]
fn render(slug: &str, out: &Path) -> Result<()> {
    let bench = env!("CARGO_BIN_EXE_bench");
    let output = Command::new(bench)
        .arg(project_for(slug))
        .env("FTS_BENCH_MIXER", out)
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
    if !out.is_file() {
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

/// How two renders differ: the count of pixels that differ at all, and
/// the largest per-channel difference among them.
fn compare(a: &[u8], b: &[u8]) -> (usize, i16) {
    let mut differing = 0;
    let mut worst = 0_i16;
    for (pa, pb) in a.chunks(4).zip(b.chunks(4)) {
        let delta = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| (i16::from(*x) - i16::from(*y)).abs())
            .max()
            .unwrap_or(0);
        if delta > 0 {
            differing += 1;
            worst = worst.max(delta);
        }
    }
    (differing, worst)
}

/// Every scene in the table, against its picture — which is how the
/// drum scenes are verified: what Drum Tracking shows, and what the
/// mixing scenes show, is the committed picture.
///
/// r[verify flow.scenes.render]
/// r[verify flow.drums.tracking.full]
/// r[verify flow.drums.mixing.scenes]
#[test]
fn every_scene_renders_to_its_committed_picture() -> Result<()> {
    let update = std::env::var_os("FTS_UPDATE_GOLDEN").is_some();
    let scenes_dir = fixtures().join("scenes");
    let scratch = tempfile::tempdir()?;
    let mut failures = Vec::new();
    for scene in &session_daw::plan::SCENES {
        let committed = scenes_dir.join(format!("{}.png", scene.slug));
        let fresh = scratch.path().join(format!("{}.png", scene.slug));
        render(scene.slug, &fresh)?;
        if update {
            std::fs::copy(&fresh, &committed)?;
            continue;
        }
        if !committed.is_file() {
            failures.push(format!(
                "{}: no committed picture at {}",
                scene.slug,
                committed.display()
            ));
            continue;
        }
        let (w, h, a) = pixels(&fresh)?;
        let (cw, ch, b) = pixels(&committed)?;
        if (w, h) != (cw, ch) {
            failures.push(format!(
                "{}: rendered {w}x{h}, committed {cw}x{ch}",
                scene.slug
            ));
            continue;
        }
        let (differing, worst) = compare(&a, &b);
        if differing > PIXEL_TOLERANCE || worst > LSB_TOLERANCE {
            let kept = scenes_dir.join(format!("{}.fresh.png", scene.slug));
            std::fs::copy(&fresh, &kept)?;
            failures.push(format!(
                "{}: {differing} pixels differ (worst by {worst}); fresh render kept at {}",
                scene.slug,
                kept.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "scene renders drifted from their committed pictures — if the change is intended, \
         run `just daw-scenes` and commit the pictures:\n{}",
        failures.join("\n")
    );
    Ok(())
}

/// The negative control: two scenes of the same project are different
/// pictures by far more than the tolerance, so the comparison above
/// could not pass by being lenient.
#[test]
fn two_scenes_differ_by_more_than_the_tolerance() -> Result<()> {
    let scenes_dir = fixtures().join("scenes");
    let (_, _, a) = pixels(&scenes_dir.join("drum-tracking.png"))?;
    let (_, _, b) = pixels(&scenes_dir.join("drum-mixing.png"))?;
    let (differing, worst) = compare(&a, &b);
    assert!(differing > PIXEL_TOLERANCE * 1000, "{differing} pixels");
    assert!(worst > LSB_TOLERANCE, "worst {worst}");
    Ok(())
}

/// Every scene has a committed picture at the committed size.
#[test]
fn every_scene_has_a_picture_at_the_committed_size() -> Result<()> {
    for scene in &session_daw::plan::SCENES {
        let path = fixtures()
            .join("scenes")
            .join(format!("{}.png", scene.slug));
        let (w, h, _) = pixels(&path)?;
        assert_eq!((w, h), (2560, 1440), "{}", scene.slug);
    }
    Ok(())
}
