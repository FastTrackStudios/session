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

use anyrender::PaintScene;
use daw_proto::Track;
use daw_theme_art::geometry::tcp as g;
use vello::kurbo::{Affine, Rect, RoundedRect, Vec2};
use vello::peniko::{Color, Fill};

use daw_theme_art::paint::tcp as art;
use daw_theme_art::vector_controls::Interaction;

use crate::arrangement::Palette;
use crate::text::Font;

/// How much of a track panel fits in the height it has.
///
/// REAPER stops shrinking tracks well before a session of two thousand
/// fits on a screen, which is the wrong trade when the question is
/// "where is everything". So rows here go as small as
/// [`crate::heights::MIN`], and the panel sheds controls on the way down
/// rather than drawing them on top of each other.
///
/// The thresholds are what each tier actually NEEDS, not taste: `Full`
/// is row one at 6 plus its 24, row two at 34 plus its 20, and the
/// phase button hanging off the bottom.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Density {
    /// Everything REAPER's row has.
    Full,
    /// The whole control row — name field, record arm, volume, pan,
    /// routing, FX, mute and solo — flattened into whatever height the
    /// row has.
    ///
    /// Controls are SQUASHED here, not dropped. A track at fourteen
    /// pixels still has everything a track at seventy has, just flatter,
    /// and every one of them stays in the same column: a panel where
    /// controls appear and vanish as tracks resize cannot be read down,
    /// and hitting a control would depend on how tall its track happens
    /// to be. What row two and the stacked gutter cost is HEIGHT, and
    /// that is the one thing this tier does not have, so mute and solo
    /// lie down beside the row instead of stacking above it.
    Compact,
    /// A coloured band. At this height a glyph would not be legible and
    /// a control would not be hittable, so the row is its tint and its
    /// divider — which is exactly what a session zoomed out to fit is
    /// being read for.
    Bar,
}

/// The colours the ported controls light up in, from the resolved theme.
///
/// One place, so a control cannot pick a different blue from the one
/// beside it — and so that changing the theme changes the controls,
/// which was the whole reason the art stopped carrying its own hex.
#[must_use]
pub fn lit(palette: &Palette) -> art::Lit {
    art::Lit {
        volume: to_theme(palette.accent),
        pan: to_theme(palette.pan),
        rec: to_theme(palette.rec),
        bypass: to_theme(palette.mute),
    }
}

/// Mute and solo, at the one size they are drawn at everywhere.
///
/// The theme's measured 21 by 20 — and that size on every track, which
/// is why `Layout::min` is what it is: the shortest settable row has to
/// hold the controls, rather than the controls shrinking to fit each
/// row. A control that is a different shape on every track cannot be
/// built on — no shared hit target, no drag across a column, no "the
/// mute column" for anything else to address.
pub const BUTTON: (f64, f64) = (21.0, 20.0);
/// Between the two, so they read as two controls.
const BUTTON_GAP: f64 = 1.0;

/// Below this tall, volume and pan stop being knobs.
///
/// A knob says its value with the angle of a ring, and an angle needs a
/// circle big enough to have angles in it — at fourteen pixels the ring
/// is three pixels of arc. A fader and a line keep saying it at any
/// height, which matters most exactly here: this is the height tracks
/// sit at once a session is collapsed enough to see all of it.
const KNOB_LEGIBLE: f64 = 20.0;

/// Below this many pixels ON SCREEN, a row is drawn as a band.
///
/// The same threshold [`Density::at`] uses, named separately because the
/// replay applies it to the row's height AFTER the zoom while the
/// recording applies it before — see `Arrangement::panel_bar`.
pub const BAND_BELOW: f64 = 11.0;

impl Density {
    /// What fits in `height`.
    #[must_use]
    pub fn at(height: f64) -> Self {
        if height >= 58.0 {
            Self::Full
        } else if height >= BAND_BELOW {
            // Below this a squashed control is a smear and a name is
            // not legible at any size, so there is nothing left to draw
            // but the band.
            Self::Compact
        } else {
            Self::Bar
        }
    }
}

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
    h: f64,
) {
    let indent = (f64::from(depth.max(0)) * INDENT).min(MAX_INDENT);
    let tint = row_tint(palette, track);
    let density = Density::at(h);
    let rail = f64::from(g::COLUMN_RULE_X);

    // ── The row's ground ──
    //
    // The tint runs the full width and the meter section is painted over
    // its right end, which is how REAPER's `meterRight` reads: one row,
    // with a gutter at the end of it, not two panels side by side.
    rect(scene, tint, 0.0, y, f64::from(g::ROW_W), y + h);
    rect(
        scene,
        palette.tcp_gutter,
        f64::from(g::TINT_W),
        y,
        f64::from(g::ROW_W),
        y + h,
    );

    if density == Density::Bar {
        // Five rectangles a row is nothing at 70px and everything at
        // two: a session zoomed to fit is two thousand rows at once, and
        // culling cannot help when all of them are visible. The column
        // strip, its rule and the panel's right edge are the three that
        // stop carrying information at this height — a one-pixel rule on
        // a two-pixel row is not a rule — so the band keeps the two that
        // say which track this is and how it is coloured.
        return;
    }

    // The left column, and the one-pixel rule closing it. The whole
    // rail moves with the indent rather than staying put while the
    // content slides out from under it: the rail IS the row's left
    // edge, and a nested track whose edge did not move read as a track
    // with a wide gutter rather than as a child.
    rect(scene, palette.tcp_column, 0.0, y, indent + rail, y + h);
    rect(
        scene,
        palette.tcp_rule,
        indent + rail,
        y,
        indent + rail + 1.0,
        y + h,
    );
    // The panel's own right edge — the boundary with the arrange view,
    // and the last thing missing from the row's silhouette.
    rect(
        scene,
        palette.tcp_rule,
        f64::from(g::ROW_W) - 2.0,
        y,
        f64::from(g::ROW_W) - 1.0,
        y + h,
    );

    // The left rail: the folder mark at its top, the track number under
    // it. REAPER puts the mark at the BOTTOM of this column; at the top
    // it lines up with the controls beside it, which is what makes a
    // folder readable while scanning a collapsed session rather than
    // something you find by looking down.
    let mark_h = if track.is_folder {
        // In the rail, which has already moved with the indent.
        let mark_scale = (h / 18.0).clamp(0.4, 1.0);
        crate::art::scaled(
            scene,
            &art::folder_mark(to_theme(palette.text_dim)),
            font,
            indent + 9.0_f64.mul_add(-mark_scale, rail) / 2.0,
            2.0_f64.mul_add(mark_scale, y),
            mark_scale,
        );
        (7.0 + 2.0) * mark_scale
    } else {
        0.0
    };

    // The track number, under whatever the rail already holds. It gets
    // the space that is left, and gives way entirely when a folder's
    // mark has taken the rail — the mark is the fact worth keeping when
    // only one of the two fits.
    if h - mark_h >= 11.0 {
        glyphs(
            scene,
            font,
            palette.text_faint,
            &track.index.saturating_add(1).to_string(),
            indent + 9.0,
            y + mark_h + (h - mark_h) / 2.0 + 4.0,
            11.0,
        );
    }

    match density {
        Density::Full => {
            let band_top = y + f64::from(g::ROW_ONE);
            row_one(scene, palette, font, track, indent, band_top, 24.0);
            row_two(scene, palette, font, track, y);
            gutter(scene, palette, font, track, y, h);
            stacked(scene, palette, font, track, band_top, 24.0);
        }
        // The controls get the row, less a pixel top and bottom so they
        // are not flush against the dividers.
        Density::Compact => {
            let band_h = (h - 2.0).max(1.0);
            row_one(scene, palette, font, track, indent, y + 1.0, band_h);
            stacked(scene, palette, font, track, y + 1.0, band_h);
        }
        Density::Bar => {}
    }
}

/// Mute and solo, side by side in the gutter.
///
/// One position AND one size at every height. They used to flatten with
/// the row, which meant a track's mute was a different shape on every
/// track — fine to look at, useless to build on: a hit target, a drag
/// across several tracks' mutes, or anything that wants to say "the
/// mute column" needs the control to be one thing everywhere.
///
/// So the size is [`BUTTON`], fixed, and it fits the shortest row a
/// track can be set to rather than being scaled down to fit each one.
///
/// Positioned against the CONTROL BAND rather than the row, so they sit
/// beside the name field on a 160-pixel track instead of floating in the
/// middle of its gutter.
///
/// REAPER stacks them, which needs 45 of height. A collapsed track has
/// sixteen, so stacking there left two five-pixel squares whose letters
/// were unreadable — the arrangement was preserved and the controls were
/// not. Turned a quarter turn they keep their width at any height, and
/// the gutter is 47 wide, which is exactly two of them.
///
/// That costs the meter its well: it sat at 297..316, which is where
/// mute now is. Nothing is lost yet, because the meter has no level to
/// show — when it does, this gutter needs re-measuring rather than
/// re-stacking, since REAPER solves the same problem by widening the
/// panel.
fn stacked(
    scene: &mut anyrender::Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    band_top: f64,
    band_h: f64,
) {
    // Centred in the band, at its own fixed size — which fits the
    // shortest row a track can be set to, so it never has to shrink.
    let top = band_top + (band_h - BUTTON.1) / 2.0;
    let x = f64::from(g::TINT_W) + 2.0;

    for (i, (label, on, lit)) in [
        ("M", track.muted, mute_lit(palette)),
        ("S", track.soloed, solo_lit(palette)),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = if i == 0 { 0.0 } else { BUTTON.0 + BUTTON_GAP };
        crate::art::place(
            scene,
            &art::gutter_button(&palette.chrome, label, on, lit, Interaction::Normal),
            font,
            x + offset,
            top,
        );
    }
}

/// The control row: the name field and everything on it, flattened to
/// whatever height the row has.
///
/// `band` is the vertical space the controls get, and every control in
/// it is drawn at ONE SIZE on every track — centred in the band rather
/// than scaled to it.
///
/// Two earlier versions scaled: first squashed, then uniformly. Both
/// looked reasonable and both were wrong for the same reason. A control
/// that is a different size on every track cannot be built on — there is
/// no shared hit target, no dragging a value across a column of tracks,
/// and nothing else can address "the pan column" because the pan column
/// is a different shape in every row. Consistency across tracks is worth
/// more than filling a tall track's band.
///
/// That is why the shortest settable row is the one that has to hold the
/// controls at their authored size, rather than the controls having to
/// fit whatever a row happens to be — see `Layout::min`.
///
/// The name field is the exception: it is a box, not a control, so it
/// takes the whole band. A field that stayed one height would leave the
/// name floating in a gap on a tall track.
fn row_one(
    scene: &mut anyrender::Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    indent: f64,
    y: f64,
    band: f64,
) {
    /// The height row one is authored at — the name field's.
    const AUTHORED: f64 = 24.0;

    // One long box with the record arm and the volume knob ON its two
    // ends, not three boxes in a line: drawn separately they had grey
    // gutters either side that REAPER does not have.
    // The field takes the band; the controls take their own size,
    // centred in it.
    let field_h = band;
    let control_top = y + (band - AUTHORED) / 2.0;
    let field_top = y;
    let field_x = f64::from(g::NAME_FIELD_X) + indent;
    let field_w = (f64::from(g::NAME_FIELD_W) - indent).max(0.0);
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
    // from across a room — and gone once the row is too short for that
    // to be true, because a five-pixel ring is neither readable nor
    // hittable and the volume and pan indicators are what a collapsed
    // row is being read for.
    if band >= KNOB_LEGIBLE {
        crate::art::place(
            scene,
            &art::record_arm(
                &palette.chrome,
                lit(palette).rec,
                track.armed,
                Interaction::Normal,
                art::Arm::Panel,
                // No housing on the strip, so the ring's hole shows the
                // name field it is seated on.
                to_theme(palette.tcp_field),
            ),
            font,
            field_x + 3.0,
            control_top + (AUTHORED - 20.0) / 2.0,
        );
    }

    // The name, between the arm and the knob, cut short rather than
    // wrapped or shrunk — REAPER truncates here too. The type shrinks
    // with the row rather than being squashed with it: a flattened glyph
    // is unreadable where a smaller one is merely small.
    let name_x = 58.0 + indent;
    let name_w = (f64::from(g::NAME_FIELD_X) + f64::from(g::NAME_FIELD_W) - 58.0 - indent).max(0.0);
    let ink = if track.selected { palette.text } else { palette.text_dim };
    let size = name_size(field_h);
    glyphs(
        scene,
        font,
        ink,
        &font.elide(&track.name, size, name_w),
        name_x,
        f64::from(size).mul_add(0.35, field_top + field_h / 2.0),
        size,
    );

    // Volume, on the field's right end, and pan outside it — both the
    // measured drawings, not a circle with a dot on it. The knob's 22
    // body CAPS the field: at its authored size the field's square
    // right-hand corners showed past the circle, which read as the name
    // box poking out from under the knob rather than the knob closing it.
    level(scene, palette, font, track, control_top, AUTHORED, band);

    // These two flatten rather than shrink. A knob has to stay round —
    // its pointer means nothing once the circle is an ellipse — but the
    // routing widget is three bars and the FX pill is a label, and both
    // stay legible squashed while shrinking would make them narrower
    // than the column they head and leave the label unreadable.
    let plate_top = control_top + (AUTHORED - 22.0) / 2.0;
    crate::art::place(
        scene,
        &art::routing(
            &palette.chrome,
            art::Axis::Horizontal,
            art::Routing {
                parent_send: track.parent_send,
                // Sends and receives are not on `Track` — they live in
                // the routing model this window has not read yet — so
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
        plate_top,
    );
    crate::art::place(
        scene,
        // The chain's state is not on `Track` — it lives in the FX model
        // this window has not read yet — so the pill draws its empty
        // slot rather than claiming the chain is running.
        &art::fx_pill(&palette.chrome, lit(palette), art::Chain::Empty, Interaction::Normal),
        font,
        f64::from(g::FX_IN_X),
        plate_top,
    );
}

/// Volume and pan, as knobs or as bars.
///
/// Knobs while there is a circle big enough to read an angle off, bars
/// below that — see [`KNOB_LEGIBLE`]. Both forms sit in the same two
/// columns, so a row can change which it shows without anything moving.
fn level(
    scene: &mut anyrender::Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    field_top: f64,
    field_h: f64,
    band: f64,
) {
    let volume_x = f64::from(g::NAME_FIELD_X) + f64::from(g::NAME_FIELD_W);
    if band >= KNOB_LEGIBLE {
        // The knob's 22 body caps the field: at its authored size the
        // field's square right-hand corners showed past the circle,
        // which read as the name box poking out from under the knob
        // rather than the knob closing it.
        let knob_scale = field_h / 22.0;
        let knob_box = 24.0 * knob_scale;
        crate::art::scaled(
            scene,
            &art::volume_knob(
                &palette.chrome,
                lit(palette).volume,
                volume_fraction(track.volume),
                Interaction::Normal,
                field_h,
            ),
            font,
            // Centred on the field's right edge, which is where REAPER
            // seats it: half on the field, half on the tint.
            volume_x - knob_box / 2.0,
            field_top + (field_h - knob_box) / 2.0,
            knob_scale,
        );
        let pan_scale = (field_h / 25.0).min(1.0);
        crate::art::scaled(
            scene,
            &art::pan_knob(
                &palette.chrome,
                pan_position(track.pan),
                to_theme(palette.pan),
            ),
            font,
            f64::from(g::PAN_KNOB_X),
            field_top + 25.0_f64.mul_add(-pan_scale, field_h) / 2.0,
            pan_scale,
        );
    } else {
        // Flattened, not shrunk: both are bars whose LENGTH is the
        // value, so the axis being squashed carries no meaning and they
        // keep saying what they say all the way down.
        crate::art::squashed(
            scene,
            &art::volume_fader(&palette.chrome, lit(palette).volume, volume_fraction(track.volume)),
            font,
            // Straddling the field's right edge, where the knob it
            // replaces is centred — the column has to hold whichever of
            // the two a row is showing.
            volume_x - 12.0,
            field_top,
            1.0,
            field_h / 24.0,
        );
        crate::art::squashed(
            scene,
            &art::pan_line(
                &palette.chrome,
                pan_position(track.pan),
                to_theme(palette.pan),
            ),
            font,
            f64::from(g::PAN_KNOB_X),
            field_top,
            1.0,
            field_h / 24.0,
        );
    }
}

/// The name's type size for a field of `height`.
///
/// Shrinks with the row and stops: past the small end it is unreadable,
/// past the large end it is bigger than REAPER sets it.
#[must_use]
fn name_size(height: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::as_conversions,
        reason = "a type size in points, which every text API takes as f32"
    )]
    let size = (height * 0.48).clamp(6.5, 11.5) as f32;
    size
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
    // The automation button is hidden for now — it comes back with the
    // envelope model behind it, and an automation control that cannot
    // report or change a mode is a button that lies about what it does.
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
    h: f64,
) {
    // ── The meter section: the meter, then mute over solo ──
    //
    // The meter comes FIRST, a vertical strip against the tint, with
    // mute and solo to its right. Drawn the other way round it lands in
    // the middle of the row.
    // Phase, in the corner. Hidden on rows too short for it, exactly as
    // the theme's own formula hides it — the row's shape must not depend
    // on its height.
    if h >= f64::from(g::PHASE_HIDE_H) {
        crate::art::place(
            scene,
            &art::phase(&palette.chrome, track.phase_inverted, Interaction::Normal),
            font,
            f64::from(g::TINT_W) + f64::from(g::GUTTER_BUTTON_X) + 3.0,
            y + h - f64::from(g::PHASE_FROM_FLOOR),
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
/// Shared with the band the replay substitutes at small zooms, so the
/// two cannot disagree about what colour a track is.
///
/// REAPER tints the whole row rather than showing a colour chip, which
/// is what makes a session readable by section at a glance. The strength
/// is the theme's, not a number chosen here.
#[must_use]
pub fn row_tint(palette: &Palette, track: &Track) -> Color {
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

/// Where the volume control fills to, from a gain.
///
/// This used to be `volume / 2.0` — linear in GAIN, which put unity at
/// the middle and made the position mean nothing: half the travel was
/// the top 6 dB and the bottom half covered everything from −6 dB to
/// silence. A fader you cannot read a level off is a handle.
///
/// It now goes through the measured taper — see
/// `daw_theme_art::paint::tcp::gain_norm` — which is linear in dB, so
/// the dB scale beside it labels the positions it actually uses.
///
/// **Known limit**: that taper tops out at unity, because the labels it
/// was fitted from run −6 to −54 and extrapolating the fit lands 0 dB
/// exactly on the groove's top pixel. REAPER's boost region was not
/// visible in the capture, so a track above unity currently pegs at the
/// top rather than showing how far above. That wants a second
/// measurement with a boosted track rather than a guess at `+12`.
#[must_use]
pub fn volume_fraction(volume: f64) -> f64 {
    daw_theme_art::paint::tcp::gain_norm(volume)
}

/// The pan pointer's position, -1..1.
const fn pan_position(pan: f64) -> f64 {
    pan.clamp(-1.0, 1.0)
}

/// Mute and solo take their lit colour from the resolved theme, so a
/// REAPER theme's own mute red reaches the canvas rather than the art
/// crate's default.
#[must_use]
pub fn mute_lit(palette: &Palette) -> daw_theme::Color {
    to_theme(palette.mute)
}

#[must_use]
pub fn solo_lit(palette: &Palette) -> daw_theme::Color {
    to_theme(palette.solo)
}

#[must_use]
pub fn to_theme(color: Color) -> daw_theme::Color {
    let [red, green, blue, alpha] = color.to_rgba8().to_u8_array();
    daw_theme::Color {
        r: red,
        g: green,
        b: blue,
        a: alpha,
    }
}

/// What a track records from, as the panel prints it.
///
/// `daw_ui`'s own formatter rather than a `Debug` of the enum: this is a
/// label a person reads off a strip, and "Audio { channel: 0 }" is a
/// dump of a data structure. One formatter, so the mixer and the panel
/// name an input the same way.
#[must_use]
pub fn record_input(track: &Track) -> String {
    daw_ui::controls::record_input_name(track)
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
pub fn caret(scene: &mut anyrender::Scene, color: Color, x: f64, y: f64) {
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
    scene: &mut impl PaintScene,
    font: &Font,
    color: Color,
    body: &str,
    x: f64,
    baseline: f64,
    size: f32,
) {
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
    scene: &mut impl PaintScene,
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
