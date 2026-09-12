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

/// How tall the mixer panel opens by default.
///
/// REAPER's own default, and a *default* is all it is: the panel is
/// draggable and a strip fills whatever height it is given. There is no
/// fixed REAPER strip height to match.
///
/// This was briefly 310, measured off a running REAPER — wrongly. That
/// REAPER had "show multiple rows of tracks" ON, so its mixer had
/// wrapped the session into two rows, and what I measured was the ROW
/// PITCH of a wrapped layout, not the height of a strip. With the
/// option off there is one row and each strip is as tall as the panel.
///
/// The width measured the same way IS right — 86, confirmed twice — and
/// that is the difference worth remembering: a strip's width is a real
/// constant, its height is whatever the dock gives it.
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
    /// Record the whole mixer, `height` being the height of its REAPER
    /// part.
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

        // The Tone rack's height, ADDED above every strip — including
        // the ones too narrow to draw one.
        //
        // Added rather than taken out. `height` is what the REAPER
        // controls need — its own MCP is 371 and everything in it is
        // sized against that — so spending half of it on the rack
        // leaves a fader of sixty pixels and a mute you cannot hit. The
        // embedded processing is an ADDITION to a channel strip, not a
        // replacement for most of one, and the strip underneath it has
        // to stay the strip you already know how to use.
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
        // What the strips actually stand in: the controls at their own
        // height, with the rack on top.
        let height = height + rack_h;

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
        // Every strip's width, resolved together — an opened strip
        // borrows from the others rather than adding to the total.
        let widths = widths(rows, layout, tone);

        let mut x = 0.0_f64;
        for (ordinal, (track, depth)) in rows.iter().enumerate() {
            let depth = usize::try_from(*depth).unwrap_or(0);
            let from = u32::try_from(strips.commands.len()).unwrap_or(u32::MAX);
            offsets.push(x);
            let w = widths.get(ordinal).copied().unwrap_or(layout.strip);
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

/// How tall the Tone rack is, as a share of the strip it sits above.
///
/// Measured against the CONTROLS' height rather than the window's, so
/// the rack is proportioned to the strip it belongs to instead of to
/// however tall someone dragged the panel.
///
/// Not a whole strip and not a corner: big enough that three stacked
/// curves each read, small enough that the strip underneath is still
/// the strip you already know how to use.
const RACK_SHARE: f64 = 0.46;

/// How far a strip may be lent down.
///
/// Not to the absolute floor: to the bottom of the TIER it is already
/// in, whichever tier that is. A strip drawing a full rack lends only
/// down to the width that still draws one; a strip drawing curves lends
/// only down to the width that still draws those.
///
/// Without this, opening a strip switched off every OTHER strip's rack.
/// Each lender gave up a handful of pixels, which was enough to carry a
/// strip across a threshold, so one click on a mic blanked the
/// processing on the whole kit. Lending has to be invisible, and a
/// control disappearing is the least invisible thing a layout can do.
///
/// Asking the tier for its own bound rather than naming the thresholds
/// here is not tidiness: the first version of this guarded `LEGIBLE`
/// only, and a strip one tier down walked straight through `SHAPE`
/// instead.
fn lending_floor(width: f64, layout: crate::layout::Layout) -> f64 {
    crate::tone::Rack::at(width)
        .floor()
        .unwrap_or(layout.strip_min)
        .max(layout.strip_min)
}

/// Every strip's width, with the selected ones opened.
///
/// The mics of a piece — a kick's In and Out, a snare's Top and Bottom
/// — are stored narrow, because the tone processing lives on the sum of
/// them and spending a rack's width on each mic would push the kit off
/// the screen. But they are not tracks you never touch: balancing the
/// In against the Out is the mixing move at that level, and sometimes
/// the move is to EQ one of them. So selection is the zoom.
///
/// # An opened strip BORROWS its width
///
/// The extra width does not come from nowhere — it is taken off the
/// other strips, in proportion to how much each has to spare. **The
/// mixer's total width is unchanged by opening a strip.**
///
/// That is the property worth having. The alternative is to let the
/// mixer grow and reserve enough screen for the worst case, which
/// means laying the session out smaller than it needs to be all the
/// time to pay for a zoom you use occasionally — and it still breaks
/// the moment the session is one track bigger than the reserve
/// assumed. Borrowing costs the other strips about four pixels each
/// across a drum kit, which is invisible, and it cannot overflow
/// because there is nothing to overflow WITH.
///
/// Each strip lends in proportion to its headroom above the floor, so a
/// strip already at its minimum lends nothing and no strip is pushed
/// below what its controls need. If the others cannot cover the whole
/// ask, the opened strip gets what there was: the rack then opens to
/// whatever tier that width supports, which is the same graceful path
/// every other width takes.
fn widths(
    rows: &RowsRef,
    layout: crate::layout::Layout,
    tone: bool,
) -> Vec<f64> {
    let mut widths: Vec<f64> = rows
        .iter()
        .map(|(track, _)| layout.width_of(track.width))
        .collect();
    if !tone {
        return widths;
    }

    let open: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, (track, _))| track.selected)
        .map(|(i, _)| i)
        .collect();
    if open.is_empty() {
        return widths;
    }

    // What the opened strips want, and what the rest can lend.
    let asked: f64 = open
        .iter()
        .filter_map(|i| widths.get(*i))
        .map(|w| (crate::tone::WORKING - w).max(0.0))
        .sum();
    if asked <= 0.0 {
        return widths;
    }
    let headroom: Vec<f64> = widths
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if open.contains(&i) {
                0.0
            } else {
                (w - lending_floor(*w, layout)).max(0.0)
            }
        })
        .collect();
    let lent: f64 = headroom.iter().sum();
    let taken = asked.min(lent);
    if taken <= 0.0 {
        return widths;
    }

    // Proportional to headroom, so nobody is pushed under their floor:
    // a strip lends at most `taken/lent` of what it had spare, and
    // `taken <= lent`.
    for (i, width) in widths.iter_mut().enumerate() {
        if let Some(spare) = headroom.get(i) {
            *width -= taken * spare / lent;
        }
    }
    // The openers split what was actually raised, in proportion to what
    // each asked for — so two selected strips both open part way rather
    // than the first taking everything and the second getting nothing.
    //
    // Their own widths are untouched by the loop above (their headroom
    // is zero), so each ask is still the one `asked` was summed from.
    for i in &open {
        let Some(width) = widths.get_mut(*i) else {
            continue;
        };
        let want = (crate::tone::WORKING - *width).max(0.0);
        *width += taken * want / asked;
    }
    widths
}

/// How thick the selected strip's top rule is.
const SELECTED_RULE: f64 = 2.0;

/// And its ceiling, so a tall panel does not turn into three big plots
/// with a channel strip hanging off the bottom.
const RACK_MAX: f64 = 220.0;

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
    // ── The left column: the dB scale, with the meter beside it ──
    //
    // REAPER gives this column to the SCALE. Measured off its mixer,
    // the labels sit at x 9..24 of an 86-wide strip and there is no
    // meter well behind them — so a meter drawn here was occupying the
    // one place the strip had for the numbers, and drawing an empty
    // well while it did it, because nothing hands it a level yet.
    //
    // Without a scale a fader is a handle on an unmarked line: you can
    // see that one track is louder than another and not by how much,
    // which is most of what a mixer is for.
    //
    // The meter keeps a narrow bar hard against the fader's left edge.
    // It is not REAPER's placement — REAPER's MCP meter is not visible
    // at rest in this theme at all — but a console meter beside its
    // fader is a shape everyone reads, and it costs the scale nothing.
    const METER_BAR: f64 = 5.0;
    if squeeze.meter() {
        crate::art::place(
            scene,
            &art::fader_scale(
                fader_x - x - METER_BAR - 4.0,
                stretch,
                crate::tcp::to_theme(palette.meter_warn),
                8.0,
            ),
            font,
            x + 2.0,
            stretch_top,
        );
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
                METER_BAR,
                stretch,
            ),
            font,
            fader_x - METER_BAR - 2.0,
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
    use super::widths;
    use crate::layout::Layout;
    use daw_proto::Track;
    use daw_ui::studio::RowsRef;

    /// `stored` widths, with the strip at `select` selected.
    fn rows(stored: &[u32], select: Option<usize>) -> RowsRef {
        RowsRef(std::sync::Arc::new(
            stored
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    (
                        Track {
                            guid: format!("t{i}"),
                            width: Some(*w),
                            selected: select == Some(i),
                            ..Track::default()
                        },
                        0,
                    )
                })
                .collect(),
        ))
    }

    fn total(widths: &[f64]) -> f64 {
        widths.iter().sum()
    }

    /// The claim the whole design rests on: opening a strip does not
    /// make the mixer wider. If the kit fitted the screen before the
    /// click, it fits after it.
    #[test]
    fn opening_a_strip_does_not_widen_the_mixer() {
        let stored = [60, 195, 86, 86, 30, 30, 195, 195, 60, 86];
        let layout = Layout::default();
        let shut = widths(&rows(&stored, None), layout, true);
        let open = widths(&rows(&stored, Some(2)), layout, true);

        assert!(
            (total(&shut) - total(&open)).abs() < 1e-9,
            "the total moved: {} -> {}",
            total(&shut),
            total(&open)
        );
    }

    /// And the strip you opened actually opened.
    #[test]
    fn the_opened_strip_reaches_the_working_width() {
        let stored = [60, 195, 86, 86, 30, 30, 195, 195, 60, 86];
        let open = widths(&rows(&stored, Some(2)), Layout::default(), true);
        assert!(
            (open[2] - crate::tone::WORKING).abs() < 1e-9,
            "wanted {}, got {}",
            crate::tone::WORKING,
            open[2]
        );
    }

    /// Everyone else gives up a little, and nobody is pushed under the
    /// floor their controls need.
    #[test]
    fn the_others_lend_from_their_headroom() {
        let stored = [60, 195, 86, 86, 30, 30, 195, 195, 60, 86];
        let layout = Layout::default();
        let open = widths(&rows(&stored, Some(2)), layout, true);

        for (i, width) in open.iter().enumerate() {
            assert!(
                *width >= layout.strip_min - 1e-9,
                "strip {i} fell to {width}, under the floor of {}",
                layout.strip_min
            );
        }
        // A strip already AT the floor has nothing to lend and keeps
        // every pixel: lending is proportional to headroom.
        assert!((open[4] - 30.0).abs() < 1e-9, "a floor strip lent: {}", open[4]);
        assert!(open[1] < 195.0, "a wide strip should have lent");
    }

    /// A piece is stored at the width selection opens to, so selecting
    /// one is a no-op — nothing moves under you when you click a track
    /// that is already open.
    #[test]
    fn selecting_an_already_open_strip_changes_nothing() {
        let stored = [60, 195, 86, 86, 30];
        let layout = Layout::default();
        let shut = widths(&rows(&stored, None), layout, true);
        let open = widths(&rows(&stored, Some(1)), layout, true);
        assert_eq!(shut, open);
    }

    /// When the others cannot cover the ask, the opened strip takes
    /// what there was rather than overdrawing — the rack then opens to
    /// whatever tier that width supports.
    #[test]
    fn an_ask_larger_than_the_headroom_is_capped() {
        // Three strips at the floor have nothing to lend.
        let stored = [30, 30, 30];
        let layout = Layout::default();
        let shut = widths(&rows(&stored, None), layout, true);
        let open = widths(&rows(&stored, Some(0)), layout, true);
        assert!((total(&shut) - total(&open)).abs() < 1e-9);
        assert!(
            (open[0] - 30.0).abs() < 1e-9,
            "nothing could be lent, so nothing should have moved: {:?}",
            open
        );
    }

    /// Two selected strips both open part way rather than the first
    /// taking everything.
    #[test]
    fn two_open_strips_share_what_is_raised() {
        let stored = [30, 30, 86, 86];
        let layout = Layout::default();
        let open = widths(&rows(&stored, Some(0)), layout, true);
        let both = {
            let rows = RowsRef(std::sync::Arc::new(
                stored
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        (
                            Track {
                                guid: format!("t{i}"),
                                width: Some(*w),
                                selected: i < 2,
                                ..Track::default()
                            },
                            0,
                        )
                    })
                    .collect(),
            ));
            widths(&rows, layout, true)
        };
        assert!((total(&open) - total(&both)).abs() < 1e-9);
        assert!(
            both[0] > 30.0 && both[1] > 30.0,
            "both should have opened: {both:?}"
        );
        assert!(
            both[0] < open[0],
            "sharing means each gets less than one alone would"
        );
    }

    /// Opening one strip must not switch off anyone else's rack.
    ///
    /// The bug this guards: every lender gave up a few pixels, which
    /// was enough to carry a strip across `LEGIBLE`, so one click on a
    /// mic blanked the processing on the whole kit. Lending has to be
    /// invisible, and a control vanishing is the least invisible thing
    /// a layout can do.
    #[test]
    fn lending_never_costs_a_strip_its_rack() {
        // A kit's worth of pieces sitting just above the threshold,
        // which is where the bug bit.
        let piece = crate::tone::LEGIBLE + 4.0;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions,
            reason = "a strip width in pixels, built for a fixture"
        )]
        let piece = piece as u32;
        let stored = [piece, piece, piece, piece, piece, piece, 86, 86, 30, 30];
        let layout = Layout::default();

        let shut = widths(&rows(&stored, None), layout, true);
        let open = widths(&rows(&stored, Some(6)), layout, true);

        for (i, (before, after)) in shut.iter().zip(&open).enumerate() {
            if i == 6 {
                continue;
            }
            assert_eq!(
                crate::tone::Rack::at(*before),
                crate::tone::Rack::at(*after),
                "strip {i} changed rack tier when another opened: {before} -> {after}"
            );
        }
    }

    /// Outside the Tone sub-mode selection is not a zoom at all.
    #[test]
    fn selection_only_opens_in_tone() {
        let stored = [60, 86, 86];
        let layout = Layout::default();
        let plain = widths(&rows(&stored, Some(1)), layout, false);
        assert_eq!(plain, vec![60.0, 86.0, 86.0]);
    }

    /// What selection opens to is legible — the two constants are set
    /// independently and nothing else would catch them crossing.
    #[test]
    fn an_opened_strip_is_legible() {
        assert!(crate::tone::WORKING >= crate::tone::LEGIBLE);
        assert_eq!(
            crate::tone::Rack::at(crate::tone::WORKING),
            crate::tone::Rack::Full
        );
    }
}
