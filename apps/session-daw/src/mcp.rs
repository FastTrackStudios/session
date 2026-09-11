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

/// How tall one level of the folder bracket is.
pub const BAND_H: f64 = 16.0;

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

/// A folder, as a bracket over the strips it contains.
struct Band {
    /// Nesting level; row zero is the outermost.
    depth: usize,
    /// Strip indices `from..to`, inclusive of `from`.
    from: usize,
    to: usize,
    name: String,
    color: Color,
}

/// The mixer, recorded.
pub struct Mixer {
    /// The strips, at x = index * [`PITCH`].
    strips: Scene,
    /// The folder brackets above them.
    bands: Scene,
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
    /// How deep the nesting goes — the height the bands occupy.
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
    ) -> Self {
        let depth_seen = rows
            .iter()
            .map(|(_, depth)| usize::try_from(*depth).unwrap_or(0))
            .max()
            .map_or(0, |deepest| deepest.saturating_add(1));
        let bands_h = crate::num::coord(depth_seen) * BAND_H;
        let height = (height - bands_h).max(1.0);

        let mut strips = Scene::new();
        let mut index = Vec::with_capacity(rows.len());
        let mut offsets = Vec::with_capacity(rows.len().saturating_add(1));
        let mut x = 0.0_f64;
        let mut open: Vec<(usize, usize, String, Color)> = Vec::new();
        let mut bands = Vec::new();

        for (i, (track, depth)) in rows.iter().enumerate() {
            let depth = usize::try_from(*depth).unwrap_or(0);

            // Close every bracket this strip has left.
            while open.len() > depth {
                if let Some((from, at_depth, name, color)) = open.pop() {
                    bands.push(Band {
                        depth: at_depth,
                        from,
                        to: i.saturating_sub(1),
                        name,
                        color,
                    });
                }
            }
            if track.is_folder {
                open.push((
                    i,
                    depth,
                    track.name.clone(),
                    crate::tcp::track_color(palette, track),
                ));
            }

            let from = u32::try_from(strips.commands.len()).unwrap_or(u32::MAX);
            offsets.push(x);
            let w = layout.width_of(track.width);
            strip(&mut strips, palette, font, track, x, w, height);
            index.push(from..u32::try_from(strips.commands.len()).unwrap_or(u32::MAX));
            x += w + STRIP_GAP;
        }
        // Anything still open runs to the end.
        while let Some((from, at_depth, name, color)) = open.pop() {
            bands.push(Band {
                depth: at_depth,
                from,
                to: rows.len().saturating_sub(1),
                name,
                color,
            });
        }

        offsets.push(x);

        let mut band_scene = Scene::new();
        for band in &bands {
            draw_band(&mut band_scene, palette, font, band, &offsets);
        }
        let _ = project;

        Self {
            strips,
            bands: band_scene,
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

    /// How tall the brackets are, above the strips.
    #[must_use]
    pub fn bands_height(&self) -> f64 {
        crate::num::coord(self.depth) * BAND_H
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

    /// Replay the folder brackets.
    ///
    /// Not culled: there are a few dozen at most, and a bracket spans
    /// many strips so "which are visible" is a different question from
    /// the one the strip index answers. Culling them would cost more to
    /// decide than to draw.
    pub fn replay_bands(&self, painter: &mut impl PaintScene, transform: Affine) -> Counts {
        crate::arrangement::replay_all(painter, &self.bands, transform)
    }
}

/// One folder's bracket: a bar over its strips, with its name on it.
fn draw_band(scene: &mut Scene, palette: &Palette, font: &Font, band: &Band, offsets: &[f64]) {
    let y = crate::num::coord(band.depth) * BAND_H;
    // The bracket spans from its first strip's left edge to its last
    // one's right, which is the next strip's left less the gap. Read off
    // the offsets rather than multiplied out, because strips are not one
    // width any more.
    let x0 = offsets.get(band.from).copied().unwrap_or(0.0);
    let x1 = offsets
        .get(band.to.saturating_add(1))
        .copied()
        .unwrap_or_else(|| offsets.last().copied().unwrap_or(0.0))
        - STRIP_GAP;
    fill(
        scene,
        band.color,
        Rect::new(x0, y + 1.0, x1, y + BAND_H - 2.0),
    );
    // The name, cut to the bracket rather than overflowing into the next
    // one — a folder wider than its label reads fine, one narrower than
    // its label reads as the wrong folder.
    let inner = (x1 - x0 - 8.0).max(0.0);
    crate::tcp::glyphs(
        scene,
        font,
        palette.tcp_gutter,
        &font.elide(&band.name, 10.0, inner),
        x0 + 4.0,
        y + BAND_H / 2.0 + 3.5,
        10.0,
    );
}

/// One channel strip, at `x`.
///
/// The section heights are [`Collapse`]'s, which resolves them against
/// REAPER's own thresholds — the same model the DOM mixer uses, so the
/// two collapse at the same heights rather than at two sets of numbers
/// that happen to agree today.
fn strip(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    x: f64,
    w: f64,
    h: f64,
) {
    // `Collapse` is written in the f32 the theme's geometry is, and a
    // strip height is a few hundred pixels — exact either way.
    let shape = Collapse::at(f64_to_f32(h));

    // The strip's ground, and the track's colour as a band across it.
    fill(scene, palette.tcp_tint, Rect::new(x, 0.0, x + w, h));

    let fx_section = f64::from(daw_theme_art::collapse::FX_SECTION);
    let pan_band = f64::from(shape.pan_band);
    let input_band = f64::from(shape.input_band);
    let stretch_h = f64::from(shape.stretch);

    let squeeze = Squeeze::at(w);

    // ── The FX section ──
    if squeeze.head() {
        crate::art::place(
            scene,
            &art::fx_pill(&palette.chrome, art::Chain::Empty, Interaction::Normal),
            font,
            x + 7.0,
            f64::from(g::FX_PILL_TOP),
        );
    }

    // ── The tinted band: pan, then the record input ──
    let band_top = fx_section;
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
    if squeeze.head() && shape.show_record_input {
        fill(
            scene,
            palette.tcp_combo,
            Rect::new(
                x + f64::from(g::RECINPUT_X),
                band_top + pan_band + 2.0,
                x + f64::from(g::RECINPUT_X) + f64::from(g::RECINPUT_W),
                band_top + pan_band + 2.0 + f64::from(g::INPUT_FIELD_H),
            ),
        );
    }

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
    );

    // ── The bottom: the name plate, then the index ──
    bottom(scene, palette, font, track, x, w, h);
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
                &art::fader(&palette.chrome, value, fader_w, stretch),
                font,
                fader_x,
                stretch_top,
            );
            // The cap is its own traced drawing rather than part of the
            // groove — one definition, placed where the groove says.
            let (cap_y, cap_h) = art::fader_cap_at(value, fader_w, stretch);
            crate::art::scaled(
                scene,
                // The grip is silver in the art — #9d to #d9 down its face.
                // The panel's mark colour is a dim grey by comparison,
                // which made the cap read as a dark block with lines
                // scratched into it rather than as a metal grip.
                &art::fader_cap(&palette.chrome, palette.chrome.hardware_mark.shade(0.45)),
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
            top: stretch_top,
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
    let mut at = stretch_top + 4.0;
    if squeeze.columns() {
        crate::art::place(
            scene,
            &art::record_arm(&palette.chrome, track.armed, Interaction::Normal),
            font,
            x + f64::from(g::ARM_LEFT),
            at,
        );
        at += f64::from(g::RECMON_FROM_ARM);
    }
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
