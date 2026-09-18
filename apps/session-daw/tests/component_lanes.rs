//! The component lanes draw the same picture the recorded scene draws.
//!
//! Two renderers, one fixture, one comparison. `bin/bench`'s
//! `FTS_BENCH_SHOT` paints the arrangement straight into Vello;
//! `bin/blitz_shot` builds it out of `daw_ui::studio::lanes` components
//! and lets Blitz paint it. Both are pointed at the golden session at
//! the same size, the same zoom and the same scroll, and what comes out
//! has to be the same picture.
//!
//! # Why the threshold is not the golden scenes' threshold
//!
//! `golden_scenes.rs` compares one renderer against ITSELF on another
//! machine, so it can demand that nothing structural differs at all.
//! This compares two DIFFERENT renderers. They agree on geometry and
//! colour — that is the point of the test — but they cannot agree on the
//! last bits of an antialiased edge, because one fills a path into Vello
//! directly and the other hands an `<svg>` to usvg first, and they
//! letter text through different stacks entirely.
//!
//! So the difference is measured where antialiasing cannot reach. The
//! signature of an edge is that it vanishes as the threshold rises; the
//! signature of a real difference is that it does not. Measured on the
//! golden session, with the renderers agreeing:
//!
//! | threshold | differing |
//! |---|---|
//! | 24 | 4.8% |
//! | 96 | 0.31% |
//! | 160 | 0.04% |
//!
//! and with one item's waveform missing — a real defect, and the one
//! this test was written after finding — the figure at 96 was over 8%.
//! The gate is set between the two, and [`a_moved_picture_fails`] is the
//! negative control that says the gate can still fail.

use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

/// The window both renderers are pointed at.
const SIZE: &str = "2560x1440";

/// Past this, a difference is not an antialiased edge.
const THRESHOLD: &str = "96";

/// How much of the picture may still differ at that threshold.
///
/// Not zero: text is lettered by two different stacks and no threshold
/// makes a glyph edge agree with a different glyph edge. Two thirds of
/// what is left at this threshold is the titles.
const TOLERANCE: f64 = 0.5;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture() -> PathBuf {
    root().join("features/dynamic-template/fixtures/golden/template.rpp")
}

/// Where the lane rect sits inside the frame, which is what the
/// reference has to be cropped to.
///
/// Read from the same constants both renderers lay out from, so a rail
/// or a ruler that changes height moves the crop with it rather than
/// silently comparing two different parts of the session.
fn lane_rect() -> (u32, u32, u32, u32) {
    let x = session_daw::rails::SIDE + session_daw::arrangement::TCP_WIDTH;
    let y = session_daw::rails::TOP + session_daw::ruler::RULER_H;
    let w = 2560.0 - x - session_daw::rails::SIDE;
    let h = 1440.0 - y;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "a rectangle inside a 2560x1440 window"
    )]
    let rect = (x as u32, y as u32, w as u32, h as u32);
    rect
}

/// Build a binary once and hand back its path.
fn built(bin: &str) -> Result<PathBuf> {
    let status = Command::new(env!("CARGO"))
        .current_dir(root())
        .args(["build", "--release", "-p", "session-daw", "--bin", bin])
        .status()?;
    if !status.success() {
        return Err(format!("could not build {bin}").into());
    }
    Ok(root().join("target/release").join(bin))
}

/// The reference: the recorded scene, cropped to the lane rect.
fn reference(out: &Path) -> Result<()> {
    let whole = out.with_file_name("whole.png");
    let status = Command::new(built("bench")?)
        .current_dir(root())
        .arg(fixture())
        .env("FTS_BENCH_SHOT", &whole)
        .env("FTS_BENCH_SIZE", SIZE)
        .status()?;
    if !status.success() {
        return Err("the reference renderer failed".into());
    }
    let (x, y, w, h) = lane_rect();
    crop(&whole, out, &format!("{w}x{h}+{x}+{y}"))
}

/// The component renderer, which draws the lane rect and nothing else.
fn components(out: &Path) -> Result<()> {
    let status = Command::new(built("blitz_shot")?)
        .current_dir(root())
        .arg(fixture())
        .arg(out)
        .env("FTS_BLITZ_SIZE", SIZE)
        .status()?;
    if !status.success() {
        return Err("the component renderer failed".into());
    }
    Ok(())
}

fn crop(from: &Path, to: &Path, geometry: &str) -> Result<()> {
    let status = Command::new("magick")
        .arg(from)
        .args(["-crop", geometry, "+repage"])
        .arg(to)
        .status()?;
    if !status.success() {
        return Err(format!("could not crop {}", from.display()).into());
    }
    Ok(())
}

/// How much of one picture differs from the other, as a percentage.
fn differs(a: &Path, b: &Path) -> Result<f64> {
    let out = Command::new("python3")
        .current_dir(root())
        .arg("scripts/ui-stress/imagediff.py")
        .arg(a)
        .arg(b)
        .arg(THRESHOLD)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let percent = text
        .lines()
        .find_map(|line| {
            let (_, rest) = line.split_once('(')?;
            let (number, _) = rest.split_once("%)")?;
            number.trim().parse::<f64>().ok()
        })
        .ok_or_else(|| format!("could not read a difference out of:\n{text}"))?;
    Ok(percent)
}

/// A scratch directory that survives a failure, so the pictures can be
/// looked at rather than described.
fn scratch() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("fts-component-lanes");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The two renderers draw the same lanes.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn the_components_draw_the_recorded_scene() -> Result<()> {
    let dir = scratch()?;
    let (vello, blitz) = (dir.join("vello.png"), dir.join("components.png"));
    reference(&vello)?;
    components(&blitz)?;

    let difference = differs(&vello, &blitz)?;
    assert!(
        difference <= TOLERANCE,
        "the component lanes are not the recorded scene: {difference:.3}% of the \
         picture differs past a threshold antialiasing cannot reach (allowed \
         {TOLERANCE}%).\n  {}\n  {}",
        vello.display(),
        blitz.display()
    );
    Ok(())
}

/// The negative control: a picture moved four pixels fails.
///
/// Without this the test above says only that something was rendered
/// twice. Four pixels is less than a row and less than a bar, so a gate
/// that catches it catches anything worth catching — and it is the size
/// of the body-margin bug that this comparison found on its first run.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn a_moved_picture_fails() -> Result<()> {
    let dir = scratch()?;
    let (vello, moved) = (dir.join("vello.png"), dir.join("moved.png"));
    if !vello.exists() {
        reference(&vello)?;
    }
    let (_, _, w, h) = lane_rect();
    crop(
        &vello,
        &moved,
        &format!("{}x{}+0+4", w, h.saturating_sub(4)),
    )?;
    let cut = dir.join("cut.png");
    crop(&vello, &cut, &format!("{}x{}+0+0", w, h.saturating_sub(4)))?;

    let difference = differs(&cut, &moved)?;
    assert!(
        difference > TOLERANCE,
        "the comparison cannot tell a moved picture from a matching one: \
         four pixels of shift read as {difference:.3}% different, inside the \
         {TOLERANCE}% the real test allows"
    );
    Ok(())
}
