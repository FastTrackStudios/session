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

/// How much of the LANES may still differ at that threshold.
///
/// Not zero: text is lettered by two different stacks and no threshold
/// makes a glyph edge agree with a different glyph edge. Measured at
/// 0.31%, essentially all of it the item titles.
const LANES_TOLERANCE: f64 = 0.5;

/// And how much of the RULER, which is allowed more.
///
/// Not because it is held to a lower standard — because of what it is.
/// A lane is mostly picture with a name on it; a ruler is mostly
/// lettering, so the share of it that two text stacks cannot agree on is
/// larger for the same quality of match. Measured at 0.50%, and every
/// pixel of that is a glyph: the bands, the flags, the ticks and the
/// rules are identical.
///
/// Neither number is close to a real defect. The missing waveform this
/// comparison was written after finding measured over 8%.
const RULER_TOLERANCE: f64 = 1.0;

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

/// The component renderer, which draws one part of the window.
fn components(part: &str, out: &Path) -> Result<()> {
    let status = Command::new(built("blitz_shot")?)
        .current_dir(root())
        .arg(fixture())
        .arg(out)
        .env("FTS_BLITZ_SIZE", SIZE)
        .env("FTS_BLITZ_PART", part)
        .status()?;
    if !status.success() {
        return Err(format!("the component renderer failed drawing the {part}").into());
    }
    Ok(())
}

/// Where the ruler sits, and how much of it can be compared.
///
/// The strip runs the full width between the rails, but its left column
/// — where the rows are named — cannot be compared: the reference draws
/// the window's rails OVER it, so that part of the picture is the rails'
/// and not the ruler's. What is compared is the timeline, which is the
/// part the ruler actually owns.
fn ruler_rect() -> (u32, u32, u32, u32) {
    let x = session_daw::rails::SIDE + session_daw::arrangement::TCP_WIDTH;
    let y = session_daw::rails::TOP;
    let w = 2560.0 - x - session_daw::rails::SIDE;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "a rectangle inside a 2560x1440 window"
    )]
    let rect = (
        x as u32,
        y as u32,
        w as u32,
        session_daw::ruler::RULER_H as u32,
    );
    rect
}

/// Crop, forcing truecolour output.
///
/// A region with few colours is written as a palette PNG, which the
/// comparator cannot read — it reads pixels, not palettes. The rails are
/// exactly that kind of region: a ground, a rule and one accent.
fn crop_true(from: &Path, to: &Path, geometry: &str) -> Result<()> {
    let status = Command::new("magick")
        .arg(from)
        .args(["-crop", geometry, "+repage"])
        .arg(format!("PNG24:{}", to.display()))
        .status()?;
    if !status.success() {
        return Err(format!("could not crop {}", from.display()).into());
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
    components("lanes", &blitz)?;

    let difference = differs(&vello, &blitz)?;
    assert!(
        difference <= LANES_TOLERANCE,
        "the component lanes are not the recorded scene: {difference:.3}% of the \
         picture differs past a threshold antialiasing cannot reach (allowed \
         {LANES_TOLERANCE}%).\n  {}\n  {}",
        vello.display(),
        blitz.display()
    );
    Ok(())
}

/// The two renderers draw the same ruler.
///
/// Compared over the timeline alone — see [`ruler_rect`] for why the
/// column of row names is not the ruler's to be judged on.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn the_components_draw_the_ruler() -> Result<()> {
    let dir = scratch()?;
    let whole = dir.join("whole.png");
    if !whole.exists() {
        reference(&dir.join("vello.png"))?;
    }
    let (x, y, w, h) = ruler_rect();
    let vello = dir.join("ruler-vello.png");
    crop(&whole, &vello, &format!("{w}x{h}+{x}+{y}"))?;

    // The component ruler draws the whole strip, rails' column and all,
    // so it is cut to the same timeline the reference was cut to.
    let drawn = dir.join("ruler-drawn.png");
    components("ruler", &drawn)?;
    let blitz = dir.join("ruler-components.png");
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "the panel's width, which is a small positive integer"
    )]
    let names = session_daw::arrangement::TCP_WIDTH as u32;
    crop(&drawn, &blitz, &format!("{w}x{h}+{names}+0"))?;

    let difference = differs(&vello, &blitz)?;
    assert!(
        difference <= RULER_TOLERANCE,
        "the component ruler is not the recorded one: {difference:.3}% differs \
         (allowed {RULER_TOLERANCE}%).\n  {}\n  {}",
        vello.display(),
        blitz.display()
    );
    Ok(())
}

/// The two renderers draw the same frame: three rails and the mode bar.
///
/// Region by region rather than as one picture, because the rails and
/// the panel between them are drawn by different things and a single
/// figure over the whole window would let one hide inside the other.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn the_components_draw_the_rails() -> Result<()> {
    let dir = scratch()?;
    let whole = dir.join("whole.png");
    if !whole.exists() {
        reference(&dir.join("vello.png"))?;
    }
    let drawn = dir.join("rails-drawn.png");
    components("rails", &drawn)?;

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "window geometry, which is small positive integers"
    )]
    let (side, top, panel, ruler) = (
        session_daw::rails::SIDE as u32,
        session_daw::rails::TOP as u32,
        session_daw::arrangement::TCP_WIDTH as u32,
        session_daw::ruler::RULER_H as u32,
    );
    // Each region, and what it is allowed. The rails that hold words get
    // the lettering allowance; the ones that are frame and nothing else
    // are held to zero, because there is nothing in them for two text
    // stacks to disagree about.
    let regions: [(&str, String, f64); 4] = [
        ("left rail", format!("{side}x1440+0+0"), RULER_TOLERANCE),
        ("right rail", format!("{side}x1440+{}+0", 2560 - side), 0.0),
        ("top rail", format!("2560x{top}+0+0"), 0.0),
        (
            "mode bar",
            format!("{panel}x{ruler}+{side}+{top}"),
            RULER_TOLERANCE,
        ),
    ];

    for (name, geometry, allowed) in regions {
        let slug = name.replace(' ', "-");
        let (a, b) = (
            dir.join(format!("{slug}-vello.png")),
            dir.join(format!("{slug}-components.png")),
        );
        // Forced to truecolour: a region with few colours comes back
        // palettised, and the comparator reads pixels rather than a
        // palette.
        crop_true(&whole, &a, &geometry)?;
        crop_true(&drawn, &b, &geometry)?;
        let difference = differs(&a, &b)?;
        assert!(
            difference <= allowed,
            "the component {name} is not the recorded one: {difference:.3}% differs \
             (allowed {allowed}%).\n  {}\n  {}",
            a.display(),
            b.display()
        );
    }
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
        difference > RULER_TOLERANCE,
        "the comparison cannot tell a moved picture from a matching one: \
         four pixels of shift read as {difference:.3}% different, inside the \
         {RULER_TOLERANCE}% the loosest real test allows"
    );
    Ok(())
}
