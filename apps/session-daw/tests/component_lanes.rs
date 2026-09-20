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

/// And the threshold that sees everything, including a dim control on a
/// dim ground — which the one above does not.
const FAINT: &str = "24";

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

/// And how much of the PANEL, which is looser again.
///
/// Forty names, forty numbers and four hundred vector shapes in one
/// column: the densest strip in the window in exactly the things two
/// renderers cannot agree on, because Vello fills the art's paths
/// directly where the components hand the same paths to usvg as an
/// image. Measured at 1.27%, falling to 0.31% once the threshold clears
/// an edge — which is antialiasing, not a shape in the wrong place.
const PANEL_TOLERANCE: f64 = 1.75;

/// What each surface may differ by at the FAINT threshold — where a
/// missing control shows even though a missing control is dim.
///
/// Measured: lanes 4.8%, ruler 1.1%, rails under 1%, panel 5.3%. These
/// are larger numbers than the structural ones and mean something
/// different: at this threshold every antialiased edge in the picture
/// counts, so a dense surface has a large figure while matching
/// perfectly. What they bound is something being ABSENT.
const LANES_OVERALL: f64 = 6.0;
const RULER_OVERALL: f64 = 2.0;
const RAILS_OVERALL: f64 = 2.0;

/// What the widget is allowed to differ from the renderer it is made of.
///
/// Near zero, and not by luck: unlike every other comparison here this
/// is not two renderers being asked to agree, it is ONE renderer asked
/// whether going through a DOM node changes what it draws. Same
/// recording, same passes, same font, same Vello. Measured at 0.000% at
/// both thresholds — the pictures are identical — so the allowance is a
/// hair for a different GPU and nothing else.
///
/// Three real defects have already shown up here rather than as a
/// panic: a transform missing the ruler's height, the live controls not
/// being drawn at all, and the recording built with no MIDI previews so
/// every trigger row came out blank.
const WIDGET_TOLERANCE: f64 = 0.05;
const WIDGET_OVERALL: f64 = 0.05;
const PANEL_OVERALL: f64 = 7.0;

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

/// The part of the lane rect the widget answers for.
///
/// Off the right goes the vertical scrollbar and off the bottom the
/// horizontal one together with the strip the widget's node stops short
/// of — it is laid out inside the rails, so the bottom rail's height is
/// the window's and not the arrangement's.
fn widget_inset(w: u32, h: u32) -> (u32, u32) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "small positive constants"
    )]
    let (bar, rail) = (
        session_daw::scrollbar::THICK as u32,
        session_daw::rails::SIDE as u32,
    );
    (w.saturating_sub(bar), h.saturating_sub(rail.max(bar)))
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
///
/// `FTS_BLITZ_WIDGET=0` explicitly: the window paints the arrangement
/// as one node now, and these tests are the reason the tree it replaced
/// is still there. Asking for it by name is the point of the call.
fn components(part: &str, out: &Path) -> Result<()> {
    let status = Command::new(built("blitz_shot")?)
        .current_dir(root())
        .arg(fixture())
        .arg(out)
        .env("FTS_BLITZ_SIZE", SIZE)
        .env("FTS_BLITZ_PART", part)
        .env("FTS_BLITZ_WIDGET", "0")
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

/// How much of one picture differs from the other, at a threshold.
fn differs_at(a: &Path, b: &Path, threshold: &str) -> Result<f64> {
    let out = Command::new("python3")
        .current_dir(root())
        .arg("scripts/ui-stress/imagediff.py")
        .arg(a)
        .arg(b)
        .arg(threshold)
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

/// Two pictures match, judged at BOTH thresholds.
///
/// Two, because one is not enough, and finding that out cost a whole
/// second row of controls. The high threshold asks "is anything
/// structurally different" and ignores an antialiased edge — but it also
/// ignores a dim control against a dim ground, and the panel's input
/// slot and record-input combo are exactly that. A panel missing both of
/// them measured 0.2% at the high threshold and 11% at the low one.
///
/// So the high threshold bounds what is structurally wrong, and the low
/// one bounds how much of the picture differs AT ALL. Something that is
/// simply absent moves the second even when it cannot move the first.
fn matches(name: &str, a: &Path, b: &Path, structural: f64, overall: f64) -> Result<()> {
    for (threshold, allowed, what) in [
        (THRESHOLD, structural, "structurally"),
        (FAINT, overall, "in total"),
    ] {
        let difference = differs_at(a, b, threshold)?;
        assert!(
            difference <= allowed,
            "the component {name} is not the recorded one: {difference:.3}% differs \
             {what} at a threshold of {threshold} (allowed {allowed}%).\n  {}\n  {}",
            a.display(),
            b.display()
        );
    }
    Ok(())
}

/// A scratch directory that survives a failure, so the pictures can be
/// looked at rather than described.
fn scratch() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("fts-component-lanes");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The WIDGET draws the same window as the renderer it is made of.
///
/// A different question from the ones below, and a stricter one. Those
/// ask whether a component tree can reproduce a painted picture, and
/// allow for two text stacks disagreeing about a pixel. This asks
/// whether the arrangement painted through one DOM node is the
/// arrangement painted through no DOM at all — the same recording, the
/// same five passes, the same font — so the only honest answer is
/// "almost exactly", and what is left is the chrome around it that the
/// widget does not draw and the reference does.
///
/// It is the gate that matters now. The widget is what the window will
/// use, and it is put together by hand: three passes that are not in
/// the recording — the live controls, the section bands, the item
/// titles — and a transform that has to carry the ruler's height. Every
/// one of those was missing or wrong at some point, and each showed up
/// as a picture rather than as a panic.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn the_widget_draws_the_painted_window() -> Result<()> {
    let dir = scratch()?;
    let (painted, widget) = (dir.join("painted.png"), dir.join("widget.png"));
    reference(&painted)?;
    let status = Command::new(built("blitz_shot")?)
        .current_dir(root())
        .arg(fixture())
        .arg(&widget)
        .env("FTS_BLITZ_SIZE", SIZE)
        .env("FTS_BLITZ_PART", "all")
        .env("FTS_BLITZ_WIDGET", "1")
        .status()?;
    if !status.success() {
        return Err("the widget renderer failed".into());
    }
    // `reference` already hands back the lane rect, so only the
    // widget's whole-window shot needs cutting — to the same rectangle,
    // because outside it the reference paints rails and a mode bar the
    // widget leaves to components.
    //
    // Then both are cut again, by [`widget_inset`], to the part of that
    // rectangle the widget is actually responsible for. Two strips are
    // not: the scrollbars, which the reference paints itself and the
    // window builds as components, and the band under the widget's
    // node, which is where the bottom rail goes. Comparing those would
    // be comparing a picture against a picture of something else.
    let (x, y, w, h) = lane_rect();
    let cropped = dir.join("w-widget.png");
    crop_true(&widget, &cropped, &format!("{w}x{h}+{x}+{y}"))?;

    let (w, h) = widget_inset(w, h);
    let (a, b) = (dir.join("w-vello-in.png"), dir.join("w-widget-in.png"));
    let inset = format!("{w}x{h}+0+0");
    crop_true(&painted, &a, &inset)?;
    crop_true(&cropped, &b, &inset)?;
    matches("widget", &a, &b, WIDGET_TOLERANCE, WIDGET_OVERALL)?;
    Ok(())
}

/// The two renderers draw the same lanes.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn the_components_draw_the_recorded_scene() -> Result<()> {
    let dir = scratch()?;
    let (vello, blitz) = (dir.join("vello.png"), dir.join("components.png"));
    reference(&vello)?;
    components("lanes", &blitz)?;

    matches("lanes", &vello, &blitz, LANES_TOLERANCE, LANES_OVERALL)?;
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

    matches("ruler", &vello, &blitz, RULER_TOLERANCE, RULER_OVERALL)?;
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
    //
    // The arrangement's right rail was one of those until it got a
    // switch of its own — the one that says what a shut folder shows —
    // and a rail with a word on it is a rail two text stacks can centre
    // a pixel apart. It takes the lettering allowance now, for the same
    // reason the left one always did.
    let regions: [(&str, String, f64); 4] = [
        ("left rail", format!("{side}x1440+0+0"), RULER_TOLERANCE),
        (
            "right rail",
            format!("{side}x1440+{}+0", 2560 - side),
            RULER_TOLERANCE,
        ),
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
        matches(name, &a, &b, allowed, allowed.max(RAILS_OVERALL))?;
    }
    Ok(())
}

/// Every converted surface at once, in one tree, still matches.
///
/// The parts are proven one at a time above. This is the question those
/// cannot answer: that they still draw the same thing when they are
/// composed — one document, one layout, one paint — rather than each
/// being right alone and wrong together. A stray margin, an inherited
/// font, a positioned ancestor moving the origin: all of them show up
/// here and nowhere else.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn the_whole_window_composes() -> Result<()> {
    let dir = scratch()?;
    let whole = dir.join("whole.png");
    if !whole.exists() {
        reference(&dir.join("vello.png"))?;
    }
    let drawn = dir.join("window.png");
    components("all", &drawn)?;

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "window geometry, which is small positive integers"
    )]
    let (side, top, panel, ruler_h) = (
        session_daw::rails::SIDE as u32,
        session_daw::rails::TOP as u32,
        session_daw::arrangement::TCP_WIDTH as u32,
        session_daw::ruler::RULER_H as u32,
    );
    let (lx, ly, lw, lh) = lane_rect();
    // Each region with BOTH of its figures, rather than one derived
    // from the other: they measure different things, and a multiplier
    // between them is a guess that fails the moment a surface is denser
    // than the one it was guessed from.
    let regions: [(&str, String, f64, f64); 4] = [
        (
            "lanes",
            format!("{lw}x{lh}+{lx}+{ly}"),
            LANES_TOLERANCE,
            LANES_OVERALL,
        ),
        (
            "ruler",
            format!(
                "{}x{ruler_h}+{}+{top}",
                2560 - side - panel - side,
                side + panel
            ),
            RULER_TOLERANCE,
            RULER_OVERALL,
        ),
        (
            "left rail",
            format!("{side}x1440+0+0"),
            RULER_TOLERANCE,
            RAILS_OVERALL,
        ),
        (
            "mode bar",
            format!("{panel}x{ruler_h}+{side}+{top}"),
            RULER_TOLERANCE,
            RAILS_OVERALL,
        ),
    ];

    for (name, geometry, allowed, overall) in regions {
        let slug = name.replace(' ', "-");
        let (a, b) = (
            dir.join(format!("win-{slug}-vello.png")),
            dir.join(format!("win-{slug}-components.png")),
        );
        crop_true(&whole, &a, &geometry)?;
        crop_true(&drawn, &b, &geometry)?;
        matches(&format!("composed {name}"), &a, &b, allowed, overall)?;
    }
    Ok(())
}

/// The two renderers draw the same track panel — all of it.
///
/// This began as the rail alone, because the rest of the row was under
/// live controls the components did not draw. They do now: the record
/// arm, the volume knob, the pan knob, mute and solo, from the same
/// `daw_theme_art` drawings the painted panel uses, turned into SVG. So
/// the comparison is the whole column.
///
/// Held looser than the lanes and about the same as the ruler, for the
/// reason that keeps recurring: this strip is dense with things two
/// renderers letter and antialias differently — forty names, forty
/// numbers, and four hundred vector shapes whose edges Vello fills
/// directly and usvg rasterises through an image. Measured at 1.27%, and
/// it falls to 0.31% as soon as the threshold clears an edge, which is
/// the signature of antialiasing rather than of a shape in the wrong
/// place.
///
/// The RAIL inside it is still checked separately and tightly, because
/// it is geometry and two glyphs: it measures zero differing pixels.
#[test]
#[ignore = "renders the golden session through two renderers; run with --ignored"]
fn the_components_draw_the_panel() -> Result<()> {
    let dir = scratch()?;
    let whole = dir.join("whole.png");
    if !whole.exists() {
        reference(&dir.join("vello.png"))?;
    }
    let drawn = dir.join("panel.png");
    components("panel", &drawn)?;

    let (_, lane_y, _, lane_h) = lane_rect();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "window geometry, which is small positive integers"
    )]
    let (side, rail) = (
        session_daw::rails::SIDE as u32,
        daw_ui::studio::panel::NAME_FIELD_X as u32,
    );
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "the panel's width, which is a small positive integer"
    )]
    let panel = daw_ui::studio::panel::ROW_W as u32;

    for (name, width, allowed) in [
        ("rail", rail, LANES_TOLERANCE),
        ("panel", panel, PANEL_TOLERANCE),
    ] {
        let (a, b) = (
            dir.join(format!("{name}-vello.png")),
            dir.join(format!("{name}-components.png")),
        );
        crop_true(&whole, &a, &format!("{width}x{lane_h}+{side}+{lane_y}"))?;
        crop_true(&drawn, &b, &format!("{width}x{lane_h}+0+0"))?;
        matches(name, &a, &b, allowed, PANEL_OVERALL)?;
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

    let difference = differs_at(&cut, &moved, THRESHOLD)?;
    assert!(
        difference > RULER_TOLERANCE,
        "the comparison cannot tell a moved picture from a matching one: \
         four pixels of shift read as {difference:.3}% different, inside the \
         {RULER_TOLERANCE}% the loosest real test allows"
    );
    Ok(())
}
