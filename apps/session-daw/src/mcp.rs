//! The mixer, as a recorded scene.
//!
//! The same architecture as the arrangement, turned ninety degrees: a
//! scene recorded once in content space and replayed under a transform,
//! with an index that lets a frame skip the strips it cannot see. There
//! the unit is a row and the scroll is vertical; here it is a strip and
//! the scroll is horizontal. Everything else — the shape layer the
//! controls come from, the culling, the level-of-detail — is shared.
//!
//! # The same session, in the other direction
//!
//! A mixer and a track panel are two views of one list, and the thing
//! that makes them hard to keep honest is that the list is a TREE. The
//! panel says so with indentation, which it has room for because it
//! stacks downward. A strip is 86 wide and cannot be indented without
//! becoming narrower than its own controls.
//!
//! So the nesting goes ABOVE the strips instead, as bands: one row per
//! depth, each band spanning the strips beneath it. A folder reads as a
//! bracket over its children rather than as a gap to their left, and the
//! two views can be compared track for track — which is the whole reason
//! to draw the structure at all.
//!
//! # What is not here yet
//!
//! Live levels. The meter draws its well and whatever level it is
//! handed, and nothing hands it one, so every meter is at rest. That is
//! the same gap the panel has and it closes in the same place.

use anyrender::{PaintScene, Scene};
use daw_proto::Track;
use daw_theme_art::geometry::mcp as g;
use daw_theme_art::paint::tcp as art;
use daw_theme_art::vector_controls::Interaction;
use daw_ui::controls::{Collapse, PanAnchor, VolumeWidget};
use daw_ui::studio::{ProjectRef, RowsRef};
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

use crate::arrangement::Palette;
use crate::profile::Counts;
use crate::text::Font;

/// REAPER's wide strip, and the width everything in the geometry was
/// measured against.
///
/// Narrow mode's 54 is a separate layout rather than this one scaled, so
/// it is not modelled by shrinking this. What IS modelled by shrinking
/// is `Track::width`: a strip narrower than this keeps its controls in
/// their measured columns and loses the room to the right of them.
pub const STRIP_W: f64 = 86.0;
const _: () = assert!(
    g::STRIP_W.to_bits() == 86.0_f32.to_bits(),
    "mcp::STRIP_W must match the measured geometry"
);
/// The gap between strips, so two adjacent ones read as two.
pub const STRIP_GAP: f64 = 1.0;

/// How much shorter each level of nesting makes a strip.
///
/// Folder depth reads off the BOTTOM of the mixer: strips share a top
/// edge and their bottoms step up with depth, so a folder's children sit
/// visibly inside it. Brackets across the top said the same thing and
/// were removed — that band is wanted for something else, and a
/// staircase costs no height that the strips were using.
pub const INDENT_STEP: f64 = 12.0;

/// REAPER's own default MCP height.
pub const DEFAULT_HEIGHT: f64 = 371.0;

/// What fits in a strip of a given width.
///
/// The width counterpart of [`Collapse`], which does the same job for a
/// strip's height. REAPER needs no such thing because every strip there
/// is one width; ours are not, so the measured columns — the button
/// column at 55, the fader at 30 — stop being reachable long before the
/// strip stops being useful.
///
/// The rule is the same one the track panel follows: shed controls in
/// the order they stop earning their space, and never let one overflow
/// its strip. What a narrow strip keeps is what you look at a send or a
/// trigger for — its level, its mute, and which track it is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Squeeze {
    /// Everything REAPER's strip has, at the columns it measures them
    /// to. The button column sits at 55 and the routing widget after it,
    /// so this is the only width where they are reachable.
    Full,
    /// The head of the strip — its FX pill and its pan — with the meter
    /// beside the fader. Routing goes first, because it is the control
    /// you set once and then read off the arrangement.
    Head,
    /// The fader down the middle, mute and solo stacked over it, and the
    /// name. What you look at a send or a trigger for: its level, its
    /// mute, and which track it is.
    Level,
}

impl Squeeze {
    /// What fits in `width`.
    ///
    /// Thresholds are what each tier NEEDS: the button column plus its
    /// own 21 for `Full`, the meter's 26 and the fader's 22 with their
    /// gaps for `Head`.
    #[must_use]
    pub fn at(width: f64) -> Self {
        if width >= 78.0 {
            Self::Full
        } else if width >= 60.0 {
            Self::Head
        } else {
            Self::Level
        }
    }

    /// The pan knob and the FX pill, which share a threshold: both want
    /// a cell and enough either side to read as placed rather than
    /// wedged.
    #[must_use]
    pub fn head(self) -> bool {
        self <= Self::Head
    }

    /// The meter, which shares the stretch with the fader.
    #[must_use]
    pub fn meter(self) -> bool {
        self <= Self::Head
    }

    /// Routing, and the measured button column it sits in.
    #[must_use]
    pub fn columns(self) -> bool {
        self == Self::Full
    }
}

/// The mixer, recorded.
pub struct Mixer {
    /// The strips, at x = index * [`PITCH`].
    strips: Scene,
    /// Command range per strip, so a frame draws only what it can see.
    index: Vec<std::ops::Range<u32>>,
    /// The left edge of every strip, plus the right edge of the last —
    /// so strip `i` occupies `offsets[i]..offsets[i + 1]`.
    ///
    /// Cumulative because strips are no longer one width. What "which
    /// strip is at this x" costs went from a division to a binary
    /// search, which is the same trade the arrangement made when rows
    /// stopped being one height.
    offsets: Vec<f64>,
    /// How many strips there are.
    pub count: usize,
    /// How deep the nesting goes.
    pub depth: usize,
    /// The height a strip was recorded at.
    pub height: f64,
}

impl Mixer {
    /// Record the whole mixer, once, into a box `height` tall.
    ///
    /// The brackets take their share off the top and the strips get the
    /// rest. Depth is measured before anything is recorded, because a
    /// strip resolves its sections against the height it is DRAWN at —
    /// recording at a guessed height and then discovering the nesting
    /// would collapse the wrong sections.
    #[must_use]
    pub fn build(
        palette: &Palette,
        font: &Font,
        project: &ProjectRef,
        rows: &RowsRef,
        height: f64,
        layout: crate::layout::Layout,
        tone: bool,
    ) -> Self {
        let depth_seen = rows
            .iter()
            .map(|(_, depth)| usize::try_from(*depth).unwrap_or(0))
            .max()
            .map_or(0, |deepest| deepest.saturating_add(1));

        // The Tone rack's height, taken off the top of every strip —
        // including the ones too narrow to draw one.
        //
        // Shared rather than per-strip for the same reason the button
        // line is: a mixer is read by scanning ACROSS it, and a rack
        // that started at a different y on each strip would make
        // "which of these is compressed hardest" a question you answer
        // one strip at a time. A narrow strip keeps the blank space,
        // which is the cost of the row staying level.
        let rack_h = if tone {
            (height * RACK_SHARE).min(RACK_MAX)
        } else {
            0.0
        };

        // One section layout for the whole mixer, resolved against the
        // height LEFT OVER — not against each strip's own.
        //
        // Strips are different heights (folder depth shortens them) and
        // different widths, and resolving sections per strip put the
        // mute of one track at a different y from the mute of the next.
        // A mixer is read by scanning ACROSS it: "which of these is
        // muted" has to be answerable with one horizontal look, and that
        // needs the button column on one line. The fader gives way
        // instead — it is shorter on a shortened strip, which is the
        // cost of the indent rather than a second inconsistency.
        let shared = Collapse::at(f64_to_f32((height - rack_h).max(1.0)));
        let buttons_top = rack_h
            + f64::from(daw_theme_art::collapse::FX_SECTION)
            + f64::from(shared.pan_band)
            + f64::from(shared.input_band)
            + 4.0;

        let mut strips = Scene::new();
        let mut index = Vec::with_capacity(rows.len());
        let mut offsets = Vec::with_capacity(rows.len().saturating_add(1));
        let mut x = 0.0_f64;
        for (ordinal, (track, depth)) in rows.iter().enumerate() {
            let depth = usize::try_from(*depth).unwrap_or(0);
            let from = u32::try_from(strips.commands.len()).unwrap_or(u32::MAX);
            offsets.push(x);
            let w = opened(layout.width_of(track.width), track.selected, tone);
            // Nesting shortens the strip from the BOTTOM, so the tops
            // stay level and the bottoms staircase.
            let strip_h = (height - crate::num::coord(depth) * INDENT_STEP).max(1.0);
            strip(
                &mut strips,
                palette,
                font,
                track,
                Slot {
                    x,
                    width: w,
                    height: strip_h,
                    buttons_top,
                    rack_h,
                },
                ordinal,
            );
            index.push(from..u32::try_from(strips.commands.len()).unwrap_or(u32::MAX));
            x += w + STRIP_GAP;
        }
        offsets.push(x);
        let _ = project;

        Self {
            strips,
            index,
            offsets,
            count: rows.len(),
            depth: depth_seen,
            height,
        }
    }

    /// How wide the whole mixer is, in content pixels.
    #[must_use]
    pub fn content_width(&self) -> f64 {
        self.offsets.last().copied().unwrap_or(0.0)
    }

    /// The strips that intersect a viewport `width` wide, scrolled to
    /// `scroll_x`.
    ///
    /// A binary search over [`Mixer::offsets`], because strips are no
    /// longer one width. One strip of bleed either side, so a strip half
    /// off the edge still paints its visible half.
    #[must_use]
    pub fn visible(&self, scroll_x: f64, width: f64) -> std::ops::Range<usize> {
        if self.count == 0 {
            return 0..0;
        }
        // `partition_point` gives the first strip whose LEFT edge is
        // past the boundary; the one before it is the one the edge falls
        // inside.
        let first = self
            .offsets
            .partition_point(|&x| x <= scroll_x)
            .saturating_sub(1);
        let last = self.offsets.partition_point(|&x| x < scroll_x + width);
        first.min(self.count)..last.min(self.count)
    }

    /// Replay the visible strips under `transform`.
    pub fn replay(
        &self,
        painter: &mut impl PaintScene,
        scroll_x: f64,
        width: f64,
        transform: Affine,
    ) -> Counts {
        let mut counts = Counts::default();
        for i in self.visible(scroll_x, width) {
            let Some(span) = self.index.get(i) else {
                continue;
            };
            let start = usize::try_from(span.start).unwrap_or(usize::MAX);
            let end = usize::try_from(span.end).unwrap_or(usize::MAX);
            let Some(cmds) = self
                .strips
                .commands
                .get(start..end.min(self.strips.commands.len()))
            else {
                continue;
            };
            for cmd in cmds {
                counts.replayed = counts.replayed.saturating_add(1);
                if crate::arrangement::submit_command(painter, cmd, transform) {
                    counts.submitted = counts.submitted.saturating_add(1);
                }
            }
        }
        counts
    }

}

/// One channel strip, at `x`.
///
/// The section heights are [`Collapse`]'s, which resolves them against
/// REAPER's own thresholds — the same model the DOM mixer uses, so the
/// two collapse at the same heights rather than at two sets of numbers
/// that happen to agree today.
/// Where one strip sits, and the mixer's shared button line.
#[derive(Clone, Copy)]
struct Slot {
    x: f64,
    width: f64,
    height: f64,
    /// The y every strip puts its button column at — see `Mixer::build`.
    buttons_top: f64,
    /// How much of the top is the Tone rack's; zero when it is off.
    rack_h: f64,
}

/// How much of a strip the Tone rack takes when it is on.
///
/// Not a whole strip and not a corner: the rack has to be big enough
/// that three stacked curves each read, and the fader and the buttons
/// below it have to stay usable, because the point of a channel strip
/// with the processing in it is that you can still MIX on it.
const RACK_SHARE: f64 = 0.46;

/// How wide a strip opens while it is SELECTED.
///
/// The mics of a piece — a kick's In and Out, a snare's Top and Bottom
/// — are stored narrow, because the tone processing lives on the sum of
/// them and spending a rack's width on each mic would push the kit off
/// the screen. But they are not tracks you never touch: balancing the
/// In against the Out is the mixing move at that level, and sometimes
/// the move is to EQ one of them.
///
/// So selection is the zoom. A strip you are working on opens to a
/// working width and closes again when you move on, which means the
/// session can be laid out for the OVERVIEW and still let you go into
/// any one track without re-laying it out.
fn opened(width: f64, selected: bool, tone: bool) -> f64 {
    if tone && selected {
        width.max(crate::tone::WORKING)
    } else {
        width
    }
}

/// How thick the selected strip's top rule is.
const SELECTED_RULE: f64 = 2.0;

/// And its ceiling, so a tall mixer does not turn into three plots.
///
/// Generous, because the fader below it does not need the other half of
/// a 1440-pixel display to be usable and the curves do need the room:
/// the EQ panel is the one you make a decision on, and a decision you
/// squint at is one you get wrong.
const RACK_MAX: f64 = 600.0;

fn strip(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    slot: Slot,
    index: usize,
) {
    let Slot {
        x,
        width: w,
        height: h,
        buttons_top,
        rack_h,
    } = slot;
    // `Collapse` is written in the f32 the theme's geometry is, and a
    // strip height is a few hundred pixels — exact either way.
    // Against the height the strip has BELOW the rack, so a strip that
    // gives half itself to the processing still collapses its remaining
    // sections the way a half-height strip would.
    let shape = Collapse::at(f64_to_f32((h - rack_h).max(1.0)));

    // The strip's ground, and the track's colour as a band across it.
    fill(scene, palette.tcp_tint, Rect::new(x, 0.0, x + w, h));

    // The selected strip says so.
    //
    // Without it, a mic that has opened to a working width is just a
    // strip that is inexplicably wider than the one beside it — the
    // layout would look broken rather than focused. A rule along the
    // top edge rather than a tint over the whole strip: the strip's
    // colour is the TRACK's, and overlaying selection on it would make
    // two tracks of the same colour read as different ones.
    if track.selected {
        fill(
            scene,
            palette.accent,
            Rect::new(x, 0.0, x + w, SELECTED_RULE),
        );
    }

    let fx_section = f64::from(daw_theme_art::collapse::FX_SECTION) + rack_h;
    let pan_band = f64::from(shape.pan_band);
    let input_band = f64::from(shape.input_band);
    let stretch_h = f64::from(shape.stretch);

    let squeeze = Squeeze::at(w);

    // ── The FX section ──
    if squeeze.head() {
        crate::art::place(
            scene,
            &art::fx_pill(
                &palette.chrome,
                crate::tcp::lit(palette),
                art::Chain::Empty,
                Interaction::Normal,
            ),
            font,
            x + 7.0,
            f64::from(g::FX_PILL_TOP),
        );
    }

    // ── The Tone rack ──
    //
    // Between the FX pill and the coloured band: the processing sits
    // above the track's identity, which is the order you read a strip
    // in when you are mixing rather than navigating.
    if rack_h > 0.0 {
        crate::tone::record(
            scene,
            palette,
            font,
            &crate::tone::placeholder(index),
            crate::tone::Panel {
                x: x + 2.0,
                y: f64::from(daw_theme_art::collapse::FX_SECTION),
                width: (w - 4.0).max(0.0),
                height: rack_h - 2.0,
            },
        );
    }

    let band_top = fx_section;
    tinted_band(
        scene,
        palette,
        font,
        track,
        Slot {
            x,
            width: w,
            height: h,
            buttons_top,
            rack_h,
        },
        (band_top, pan_band, input_band),
        &shape,
    );

    stretch(
        scene,
        palette,
        font,
        track,
        Stretch {
            x,
            width: w,
            top: band_top + pan_band + input_band,
            height: stretch_h,
        },
        &shape,
        buttons_top,
    );

    bottom(scene, palette, font, track, x, w, h);
}

/// The coloured band: pan, the record input, and the arm hanging off its
/// bottom edge.
fn tinted_band(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    slot: Slot,
    bands: (f64, f64, f64),
    shape: &Collapse,
) {
    let Slot { x, width: w, .. } = slot;
    let (band_top, pan_band, input_band) = bands;
    let squeeze = Squeeze::at(w);
    fill(
        scene,
        crate::tcp::row_tint(palette, track),
        Rect::new(x, band_top, x + w, band_top + pan_band + input_band),
    );
    // Pan moves into the input area when its own section is gone, which
    // is `Collapse`'s call rather than a height comparison here.
    let pan_top = match shape.pan {
        PanAnchor::PanSection => band_top + 2.0,
        PanAnchor::InputArea => band_top + pan_band + 2.0,
    };
    if squeeze.head() && pan_band + input_band > 26.0 {
        crate::art::place(
            scene,
            &art::pan_knob(
                &palette.chrome,
                track.pan.clamp(-1.0, 1.0),
                crate::tcp::to_theme(palette.pan),
            ),
            font,
            x + (w - f64::from(g::PAN_KNOB_W)) / 2.0,
            pan_top,
        );
    }
    // The record input, on armed tracks only.
    //
    // It is what a track RECORDS FROM, so a track that is not recording
    // has nothing to say with it — the same rule the track panel
    // follows. Drawn unconditionally it was an empty sunken rectangle on
    // every strip in the session: a black box with no label, which reads
    // as a hole rather than as a field nobody has filled in.
    if squeeze.head() && shape.show_record_input && track.armed {
        let field_x = x + f64::from(g::RECINPUT_X);
        let field_w = f64::from(g::RECINPUT_X)
            .mul_add(-2.0, w)
            .min(f64::from(g::RECINPUT_W));
        let top = band_top + pan_band + 2.0;
        let height = f64::from(g::INPUT_FIELD_H);
        fill(
            scene,
            palette.tcp_combo,
            Rect::new(field_x, top, field_x + field_w, top + height),
        );
        crate::tcp::glyphs(
            scene,
            font,
            palette.text_dim,
            &font.elide(&crate::tcp::record_input(track), 10.0, field_w - 14.0),
            field_x + 4.0,
            top + height / 2.0 + 3.5,
            10.0,
        );
        // The field's caret, so it reads as something you can open.
        crate::tcp::caret(
            scene,
            palette.text_faint,
            field_x + field_w - 9.0,
            top + height / 2.0 - 2.0,
        );
    }

    // The record arm hangs off the BOTTOM of the coloured band.
    //
    // Its housing's straight base is meant to be invisible — REAPER
    // sinks it into the dark below the band so only the 45 degree flare
    // emerges into the colour — and `ARM_OVERHANG` is exactly how much
    // of the cell that base is. So the cell's bottom sits that far below
    // the band's edge, which puts the shoulder ON the edge and the
    // flares above it.
    //
    // Positioned against the band rather than the button column: the
    // column's top is the mixer's shared button line, four pixels lower,
    // and placing the arm there put the flares in the dark.
    if Squeeze::at(w).columns() {
        let band_bottom = band_top + pan_band + input_band;
        crate::art::place(
            scene,
            &art::record_arm(
                &palette.chrome,
                crate::tcp::lit(palette).rec,
                track.armed,
                Interaction::Normal,
                art::Arm::Mixer,
                // The strip's own body, so the housing reads as that
                // body growing up into the coloured band rather than as
                // something grey sitting on top of it — which is what
                // makes it a moulding and not a lump.
                crate::tcp::to_theme(palette.tcp_tint),
            ),
            font,
            x + f64::from(g::ARM_LEFT),
            band_bottom + f64::from(g::ARM_OVERHANG) - f64::from(g::ARM_CELL_H),
        );
    }

}

/// Where the stretch section sits, and how tall it is.
#[derive(Clone, Copy)]
struct Stretch {
    x: f64,
    width: f64,
    top: f64,
    height: f64,
}

/// The meter, the volume control and the button column.
fn stretch(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    band: Stretch,
    shape: &Collapse,
    buttons_top: f64,
) {
    let Stretch {
        x,
        width: w,
        top: stretch_top,
        height: stretch,
    } = band;
    let squeeze = Squeeze::at(w);
    // The fader keeps the measured column while there is a meter beside
    // it, and takes the middle of the strip once there is not.
    let fader_w = 22.0;
    let fader_x = if squeeze.meter() {
        x + 30.0
    } else {
        x + (w - fader_w) / 2.0
    };
    let meter_w = f64::from(g::METER_W);
    if squeeze.meter() {
        crate::art::place(
        scene,
        &art::meter(
            &palette.chrome,
            0.0,
            [
                crate::tcp::to_theme(palette.meter_safe),
                crate::tcp::to_theme(palette.meter_warn),
                crate::tcp::to_theme(palette.meter_danger),
            ],
            meter_w,
            stretch,
        ),
        font,
        x + 4.0,
        stretch_top,
        );
    }
    match shape.volume {
        // Below the swap threshold a fader has no travel worth having,
        // so it stops being a fader — REAPER's own rule, and the reason
        // `Collapse` answers this rather than a height comparison here.
        VolumeWidget::Knob => {
            crate::art::place(
                scene,
                &art::volume_knob(
                    &palette.chrome,
                    crate::tcp::lit(palette).volume,
                    crate::tcp::volume_fraction(track.volume),
                    Interaction::Normal,
                    24.0,
                ),
                font,
                fader_x,
                stretch_top + 2.0,
            );
        }
        VolumeWidget::Fader => {
            let value = crate::tcp::volume_fraction(track.volume);
            crate::art::place(
                scene,
                &art::fader(
                    &palette.chrome,
                    crate::tcp::lit(palette).volume,
                    value,
                    fader_w,
                    stretch,
                ),
                font,
                fader_x,
                stretch_top,
            );
            // The cap is its own traced drawing rather than part of the
            // groove — one definition, placed where the groove says.
            let (cap_y, cap_h) = art::fader_cap_at(value, fader_w, stretch);
            crate::art::scaled(
                scene,
                // The grip is silver in the art — #9d to #d9 down its
                // face — which is what `hardware_mark` now carries.
                &art::fader_cap(&palette.chrome, palette.chrome.hardware_mark),
                font,
                fader_x,
                stretch_top + cap_y,
                cap_h / 53.0,
            );
        }
    }

    column(
        scene,
        palette,
        font,
        track,
        Stretch {
            x,
            width: w,
            top: buttons_top,
            height: stretch,
        },
        shape,
    );
}

/// The right-hand column: the record arm, then mute, solo and routing
/// stacked under it.
fn column(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    band: Stretch,
    shape: &Collapse,
) {
    let Stretch {
        x,
        width: w,
        top: stretch_top,
        ..
    } = band;
    let squeeze = Squeeze::at(w);
    // The measured column while the strip is wide enough to hold it;
    // hard against the right edge once it is not, so the buttons stay
    // in the strip rather than in its neighbour.
    let column = if squeeze.columns() {
        x + f64::from(g::COLUMN)
    } else {
        x + (w - f64::from(g::BUTTON_W)).max(0.0) / 2.0
    };
    // `top` here is the mixer's shared button line, not this strip's
    // stretch — see `Mixer::build`.
    //
    // The record arm's slot is reserved whether or not the arm is drawn.
    // Skipping the advance on a narrow strip put its mute twenty pixels
    // above every other mute, which is the same failure the shared line
    // was introduced to fix: a control that moves because of something
    // about ITS track cannot be scanned across tracks.
    // The arm is placed against the coloured band by `strip`, not here:
    // it belongs to that band, and this column's `top` is the mixer's
    // shared button line rather than the band's edge.
    let mut at = stretch_top + f64::from(g::RECMON_FROM_ARM);
    for (label, on, lit) in [
        ("M", track.muted, crate::tcp::mute_lit(palette)),
        ("S", track.soloed, crate::tcp::solo_lit(palette)),
    ] {
        crate::art::place(
            scene,
            &art::gutter_button(&palette.chrome, label, on, lit, Interaction::Normal),
            font,
            column,
            at,
        );
        at += f64::from(g::BUTTON_H) + 1.0;
    }
    if shape.show_io && squeeze.columns() {
        crate::art::place(
            scene,
            // The mixer stacks the lanes; the track panel sets them in
            // a row. That is the whole difference between the theme's
            // two routing images, and the reason the drawing takes an
            // axis rather than being rotated at the call site.
            &art::routing(
                &palette.chrome,
                art::Axis::Vertical,
                art::Routing {
                    parent_send: track.parent_send,
                    sends: false,
                    receives: false,
                },
                art::RouteInk {
                    out: crate::tcp::to_theme(palette.accent),
                    send: crate::tcp::to_theme(palette.meter_warn),
                    recv: crate::tcp::to_theme(palette.meter_danger),
                },
                Interaction::Normal,
            ),
            font,
            column,
            at,
        );
    }

}

/// The name plate and the track number.
fn bottom(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    x: f64,
    w: f64,
    h: f64,
) {
    let bottom = h - f64::from(daw_theme_art::collapse::BOTTOM_SECTION);
    let plate = f64::from(g::NAME_PLATE);
    fill(
        scene,
        palette.tcp_field,
        Rect::new(x + 2.0, bottom, x + w - 2.0, bottom + plate),
    );
    let ink = if track.selected { palette.text } else { palette.text_dim };
    crate::tcp::glyphs(
        scene,
        font,
        ink,
        &font.elide(&track.name, 11.0, w - 10.0),
        x + 5.0,
        bottom + plate / 2.0 + 4.0,
        11.0,
    );
    crate::tcp::glyphs(
        scene,
        font,
        palette.text_faint,
        &track.index.saturating_add(1).to_string(),
        x + 5.0,
        bottom + plate + 14.0,
        10.0,
    );
}

/// A strip height, for the f32 the theme's geometry is written in.
const fn f64_to_f32(value: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::as_conversions,
        reason = "a pixel height; f32 holds it exactly and the theme's API takes one"
    )]
    let narrowed = value as f32;
    narrowed
}

fn fill(scene: &mut Scene, color: Color, rect: Rect) {
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
}

#[cfg(test)]
mod selection_tests {
    use super::opened;

    /// A mic is stored narrow and opens to a working width when you
    /// select it — the whole point of laying the session out for the
    /// overview and still being able to go into one track.
    #[test]
    fn selecting_a_mic_opens_it() {
        let mic = 86.0;
        assert!((opened(mic, false, true) - mic).abs() < f64::EPSILON);
        assert!(opened(mic, true, true) >= crate::tone::LEGIBLE);
    }

    /// A piece is already wide enough, so selecting it must not make it
    /// jump: a strip that resized when you clicked it would move every
    /// strip to its right, which is the one thing a mixer must not do
    /// when you are comparing tracks.
    #[test]
    fn selecting_a_piece_changes_nothing() {
        let piece = 240.0;
        assert!((opened(piece, true, true) - piece).abs() < f64::EPSILON);
    }

    /// And outside the Tone sub-mode selection is not a zoom at all.
    #[test]
    fn selection_only_opens_in_tone() {
        assert!((opened(86.0, true, false) - 86.0).abs() < f64::EPSILON);
    }
}
