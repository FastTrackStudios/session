//! The track control panel, drawn into the recorded scene.
//!
//! # Why this is not the DOM row
//!
//! `daw_ui::components::tcp::TrackRow` is the REAPER-matched panel, and
//! it stays the reference: every coordinate here comes from
//! `daw_theme_art::geometry::tcp`, the same measured constants that row
//! positions its controls with. Neither file is allowed to invent a
//! number. When REAPER's layout is re-measured, both move, because both
//! read the same constants.
//!
//! What differs is the RENDERING, and only because it has to. The DOM
//! row is a tree of elements with SVG controls inside it; this is a list
//! of fills and glyph runs recorded once and replayed under a transform.
//! The panel had to come onto the canvas because it could not otherwise
//! stay level with the lanes — native scroll moves DOM content on the
//! compositor while a canvas redraws on the main thread, and the two
//! visibly tore apart while scrolling.
//!
//! # What is drawn, and what is not yet
//!
//! Drawn: the tint, the left column and its rule, the track number, the
//! name field with the record arm on it, the name, volume and pan, the
//! routing and FX buttons, the envelope button, the FX slot, the record
//! input combo, mute and solo in their gutter, phase, and the meter.
//!
//! Not yet: the meter's live level (it draws its well, because at rest
//! REAPER's shows nothing), the fixed-lanes button, and hit testing.
//! Those are the next pieces, and they are listed here rather than left
//! to be discovered as absences.

use daw_proto::Track;
use daw_theme_art::geometry::tcp as g;
use vello::kurbo::{Affine, Rect, RoundedRect, Vec2};
use vello::peniko::{Color, Fill};

use daw_theme_art::paint::tcp as art;
use daw_theme_art::vector_controls::Interaction;

use crate::arrangement::Palette;
use crate::text::Font;

/// A row's full height, without the divider under it.
///
/// Stated rather than converted from `g::ROW_H`, because `f64::from` is
/// not callable in a const and a cast is not allowed to hide in one. The
/// assertion below is what keeps the two honest: change the measured
/// height and this fails to compile rather than drifting.
pub const ROW_H: f64 = 70.0;
const _: () = assert!(
    g::ROW_H.to_bits() == 70.0_f32.to_bits(),
    "tcp::ROW_H must match the measured geometry"
);

/// How far a folder's children are indented per level.
///
/// REAPER indents the row's CONTENT, not the row, so the tint still
/// reaches the panel's edge and only the field and the number move.
const INDENT: f64 = 10.0;
/// Past this, an indent would push the name field into the volume knob.
/// Deep templates nest further than the panel is wide, and a clamp is
/// what stops a section eight folders down from drawing on top of its
/// own controls.
const MAX_INDENT: f64 = 60.0;

/// Draw one row at `y`, in panel content space.
///
/// Everything is positioned relative to `y`, so a caller only has to
/// know the row pitch — which is the single fact keeping the panel level
/// with the lanes beside it.
pub fn draw_row(
    scene: &mut anyrender::Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    depth: i32,
    y: f64,
) {
    let indent = (f64::from(depth.max(0)) * INDENT).min(MAX_INDENT);
    let tint = row_tint(palette, track);

    // ── The row's ground ──
    //
    // The tint runs the full width and the meter section is painted over
    // its right end, which is how REAPER's `meterRight` reads: one row,
    // with a gutter at the end of it, not two panels side by side.
    rect(scene, tint, 0.0, y, f64::from(g::ROW_W), y + ROW_H);
    rect(
        scene,
        palette.tcp_gutter,
        f64::from(g::TINT_W),
        y,
        f64::from(g::ROW_W),
        y + ROW_H,
    );
    // The left column, and the one-pixel rule closing it.
    rect(scene, palette.tcp_column, 0.0, y, f64::from(g::COLUMN_RULE_X), y + ROW_H);
    rect(
        scene,
        palette.tcp_rule,
        f64::from(g::COLUMN_RULE_X),
        y,
        f64::from(g::COLUMN_RULE_X) + 1.0,
        y + ROW_H,
    );
    // The panel's own right edge — the boundary with the arrange view,
    // and the last thing missing from the row's silhouette.
    rect(
        scene,
        palette.tcp_rule,
        f64::from(g::ROW_W) - 2.0,
        y,
        f64::from(g::ROW_W) - 1.0,
        y + ROW_H,
    );

    // The track number, in the left column.
    glyphs(
        scene,
        font,
        palette.text_faint,
        &track.index.saturating_add(1).to_string(),
        9.0,
        y + ROW_H / 2.0 + 4.0,
        11.0,
    );

    row_one(scene, palette, font, track, indent, y);
    row_two(scene, palette, font, track, y);
    gutter(scene, palette, font, track, y);
}

/// The name field and everything that sits on it.
fn row_one(
    scene: &mut anyrender::Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    indent: f64,
    y: f64,
) {
    // ── Row one: the name field, and what sits on it ──
    //
    // One long box with the record arm and the volume knob ON its two
    // ends, not three boxes in a line: drawn separately they had grey
    // gutters either side that REAPER does not have.
    let field_x = f64::from(g::NAME_FIELD_X) + indent;
    let field_w = (f64::from(g::NAME_FIELD_W) - indent).max(0.0);
    let field_top = y + f64::from(g::ROW_ONE);
    let field_h = f64::from(g::NAME_FIELD_H);
    scene_fill(
        scene,
        palette.tcp_field,
        &RoundedRect::new(
            field_x,
            field_top,
            field_x + field_w,
            field_top + field_h,
            (field_h / 2.0, 0.0, 0.0, field_h / 2.0),
        ),
    );

    // The record arm, on the field's left end. Lit when armed, which is
    // the one control on this row that has to be readable at a glance
    // from across a room.
    crate::art::place(
        scene,
        &art::record_arm(&palette.chrome, track.armed, Interaction::Normal),
        font,
        field_x + 3.0,
        field_top + 3.0,
    );

    // The name, between the arm and the knob, cut short rather than
    // wrapped or shrunk — REAPER truncates here too.
    let name_x = 58.0 + indent;
    let name_w = (f64::from(g::NAME_FIELD_X) + f64::from(g::NAME_FIELD_W) - 58.0 - indent).max(0.0);
    let ink = if track.selected { palette.text } else { palette.text_dim };
    glyphs(
        scene,
        font,
        ink,
        &font.elide(&track.name, 11.5, name_w),
        name_x,
        field_top + 16.0,
        11.5,
    );

    // Volume, on the field's right end, and pan outside it — both the
    // measured drawings, not a circle with a dot on it.
    // Scaled so the knob's 22 body CAPS the 24-tall field it straddles.
    // At its authored size the field's square right-hand corners showed
    // past the circle, which read as the name box poking out from under
    // the knob rather than the knob closing it.
    let knob_scale = field_h / 22.0;
    let knob_box = 24.0 * knob_scale;
    crate::art::scaled(
        scene,
        &art::volume_knob(&palette.chrome, gain_fraction(track.volume), Interaction::Normal),
        font,
        // Centred on the field's right edge, which is where REAPER seats
        // it: half on the field, half on the tint.
        f64::from(g::NAME_FIELD_X) + f64::from(g::NAME_FIELD_W) - knob_box / 2.0,
        field_top + (field_h - knob_box) / 2.0,
        knob_scale,
    );
    crate::art::place(
        scene,
        &art::pan_knob(&palette.chrome, pan_position(track.pan)),
        font,
        f64::from(g::PAN_KNOB_X),
        y + 5.0,
    );

    // Routing, then the FX pill. Both get a FACE — lanes on the routing
    // widget, a label on the pill. Drawn as bare plates they read as
    // holes in the row rather than as controls, which is exactly what
    // they looked like.
    crate::art::place(
        scene,
        &art::routing(
            &palette.chrome,
            art::Routing {
                parent_send: track.parent_send,
                // Sends and receives are not on `Track` — they live in
                // the routing model this window has not read yet, so
                // they draw as the unlit slots they are rather than as
                // a guess.
                sends: false,
                receives: false,
            },
            // The source art's own choices: the output lane is the
            // accent, sends are the warn amber, receives the danger red.
            art::RouteInk {
                out: to_theme(palette.accent),
                send: to_theme(palette.meter_warn),
                recv: to_theme(palette.meter_danger),
            },
            Interaction::Normal,
        ),
        font,
        f64::from(g::ROUTING_X),
        y + 6.0,
    );
    crate::art::place(
        scene,
        // The chain's state is not on `Track` — it lives in the FX model
        // this window has not read yet — so the pill draws its empty
        // slot rather than claiming the chain is running.
        &art::fx_pill(&palette.chrome, art::Chain::Empty, Interaction::Normal),
        font,
        f64::from(g::FX_IN_X),
        y + 5.0,
    );

}

/// Envelope, and — only while armed — the input FX slot and the record
/// input combo.
fn row_two(
    scene: &mut anyrender::Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    y: f64,
) {
    let two = y + f64::from(g::ROW_TWO);
    let fh = f64::from(g::FIELD_H);
    crate::art::place(scene, &art::envelope(&palette.chrome, Interaction::Normal), font, 29.0, two);
    // The input FX slot and the record-input combo belong to RECORDING,
    // so they appear when the track is armed and not before. On a
    // 2,000-track orchestral template almost nothing is armed, and
    // drawing an input selector reading "None" on every row filled the
    // panel with the one thing none of those tracks are doing.
    if track.armed {
        rect(scene, palette.tcp_field, 56.0, two, 90.0, two + fh);
            text_centered(scene, font, palette.text_faint, "FX", (56.0, 90.0), two + 14.0, 10.0);
        rect(scene, palette.tcp_combo, 91.0, two, 286.0, two + fh);
        text_centered(
            scene,
            font,
            palette.text_dim,
            &font.elide(&record_input(track), 11.0, 180.0),
            (91.0, 286.0),
            two + 14.0,
            11.0,
        );
        // The combo's caret — the shared triangle, not a glyph.
        caret(scene, palette.text_faint, 91.0 + 181.0, two + 8.0);
    }

}

/// The meter section: the meter, then mute over solo, then phase.
fn gutter(
    scene: &mut anyrender::Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    y: f64,
) {
    // ── The meter section: the meter, then mute over solo ──
    //
    // The meter comes FIRST, a vertical strip against the tint, with
    // mute and solo to its right. Drawn the other way round it lands in
    // the middle of the row.
    rect(
        scene,
        palette.tcp_meter_well,
        f64::from(g::TINT_W) + f64::from(g::METER_X),
        y + 2.0,
        f64::from(g::TINT_W) + f64::from(g::METER_X) + f64::from(g::METER_W),
        y + ROW_H - 2.0,
    );

    let button_x = f64::from(g::TINT_W) + f64::from(g::GUTTER_BUTTON_X);
    crate::art::place(
        scene,
        &art::gutter_button(&palette.chrome, "M", track.muted, mute_lit(palette), Interaction::Normal),
        font,
        button_x,
        y + 3.0,
    );
    crate::art::place(
        scene,
        &art::gutter_button(&palette.chrome, "S", track.soloed, solo_lit(palette), Interaction::Normal),
        font,
        button_x,
        y + f64::from(g::SOLO_TOP),
    );

    // Phase, in the corner. Hidden on rows too short for it, exactly as
    // the theme's own formula hides it — the row's shape must not depend
    // on its height.
    if ROW_H >= f64::from(g::PHASE_HIDE_H) {
        crate::art::place(
            scene,
            &art::phase(&palette.chrome, track.phase_inverted, Interaction::Normal),
            font,
            button_x + 3.0,
            y + ROW_H - f64::from(g::PHASE_FROM_FLOOR),
        );
    }
}

/// A track's own colour, or the palette's neutral when it has none.
///
/// Shared with the lanes so an item and its row cannot disagree about
/// what colour the track is.
#[must_use]
pub fn track_color(palette: &Palette, track: &Track) -> Color {
    track.color.map_or(palette.text_faint, |rgb| {
        Color::from_rgba8(
            u8::try_from((rgb >> 16) & 0xff).unwrap_or(0),
            u8::try_from((rgb >> 8) & 0xff).unwrap_or(0),
            u8::try_from(rgb & 0xff).unwrap_or(0),
            0xff,
        )
    })
}

/// The row's background, tinted toward the track's own colour.
///
/// REAPER tints the whole row rather than showing a colour chip, which
/// is what makes a session readable by section at a glance. The strength
/// is the theme's, not a number chosen here.
fn row_tint(palette: &Palette, track: &Track) -> Color {
    if track.color.is_none() {
        return palette.tcp_tint;
    }
    mix(palette.tcp_tint, track_color(palette, track), palette.track_tint)
}

/// Blend `b` into `a` by `t`.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let [ar, ag, ab, aa] = a.components;
    let [br, bg, bb, _] = b.components;
    Color::new([
        (br - ar).mul_add(t, ar),
        (bg - ag).mul_add(t, ag),
        (bb - ab).mul_add(t, ab),
        aa,
    ])
}

/// Where the volume ring fills to, from a gain.
///
/// Unity is halfway up the sweep rather than at the end, because a fader
/// at 0 dB is the resting position a mix is read against and REAPER's
/// range runs past it.
const fn gain_fraction(volume: f64) -> f64 {
    (volume / 2.0).clamp(0.0, 1.0)
}

/// The pan pointer's position, -1..1.
const fn pan_position(pan: f64) -> f64 {
    pan.clamp(-1.0, 1.0)
}

/// Mute and solo take their lit colour from the resolved theme, so a
/// REAPER theme's own mute red reaches the canvas rather than the art
/// crate's default.
fn mute_lit(palette: &Palette) -> daw_theme::Color {
    to_theme(palette.mute)
}

fn solo_lit(palette: &Palette) -> daw_theme::Color {
    to_theme(palette.solo)
}

fn to_theme(color: Color) -> daw_theme::Color {
    let [red, green, blue, alpha] = color.to_rgba8().to_u8_array();
    daw_theme::Color {
        r: red,
        g: green,
        b: blue,
        a: alpha,
    }
}

fn record_input(track: &Track) -> String {
    format!("{:?}", track.record_input)
}

fn rect(scene: &mut anyrender::Scene, color: Color, x0: f64, y0: f64, x1: f64, y1: f64) {
    scene_fill(scene, color, &Rect::new(x0, y0, x1, y1));
}

fn scene_fill(scene: &mut anyrender::Scene, color: Color, shape: &impl vello::kurbo::Shape) {
    use anyrender::PaintScene as _;
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, shape);
}

/// A knob: its body, and a dot showing where it points.
///
/// A dot rather than a rotated pointer line, because the recorded scene
/// The combo's disclosure triangle.
fn caret(scene: &mut anyrender::Scene, color: Color, x: f64, y: f64) {
    use vello::kurbo::BezPath;
    let mut path = BezPath::new();
    path.move_to((x, y));
    path.line_to((x + 7.0, y));
    path.line_to((x + 3.5, y + 4.0));
    path.close_path();
    scene_fill(scene, color, &path);
}

/// A run of text with its baseline at `baseline`.
///
/// Public because `crate::art` draws the labels inside ported control
/// drawings, and those have to come out in the same font as the track
/// names beside them.
pub fn glyphs(
    scene: &mut anyrender::Scene,
    font: &Font,
    color: Color,
    body: &str,
    x: f64,
    baseline: f64,
    size: f32,
) {
    use anyrender::PaintScene as _;
    if body.is_empty() {
        return;
    }
    let glyphs = font.layout(body, size);
    if glyphs.is_empty() {
        return;
    }
    scene.draw_glyphs(
        font.data(),
        size,
        true,
        &[],
        Vec2::ZERO,
        Fill::NonZero,
        color,
        1.0,
        Affine::translate((x, baseline)),
        None,
        glyphs.into_iter(),
    );
}

/// The same, centred between `left` and `right`.
fn text_centered(
    scene: &mut anyrender::Scene,
    font: &Font,
    color: Color,
    body: &str,
    span: (f64, f64),
    baseline: f64,
    size: f32,
) {
    let (left, right) = span;
    let w = font.width(body, size);
    glyphs(scene, font, color, body, left + (right - left - w) / 2.0, baseline, size);
}
