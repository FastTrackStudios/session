//! The track control panel, as components.
//!
//! One row per track: its colour, the folders it sits inside, its
//! number, its name. Laid out at the geometry measured off REAPER, the
//! same numbers the painted panel uses, so the two columns line up row
//! for row against the lanes beside them.
//!
//! # What is here and what is not
//!
//! The row's GROUND — tint, gutter, the folders' bands, the rail, the
//! rules, the number, the name field and the name. All of it rectangles
//! and text, which is what the measurements say to build things out of.
//!
//! Not the controls: the record arm, the volume, the pan, the routing,
//! the FX pill, mute and solo and the meter. Every one of those shows a
//! VALUE that changes while the window is open, and several of them are
//! genuinely round — a ring, an arc, a pointer — which is the one shape
//! a rectangle cannot be talked into. They are a separate layer and a
//! separate problem.
//!
//! # Density
//!
//! A row is whatever height the project says, and what fits changes with
//! it. Below [`BAND_BELOW`] there is nothing to draw but the band: a
//! glyph is not legible and a control is not hittable, and a session
//! zoomed out to fit is being read for its colours anyway. That is not a
//! degradation to apologise for — it is what the tier is for, and it is
//! why a two-thousand-track session can be shown at once at all.

use crate::prelude::*;

use daw_theme_art::paint::tcp as art;
use daw_theme_art::vector_controls::Interaction;

use super::art::{Label, Sheet};
use super::lanes::{Built, Colors, DIVIDER, FONT, Offsets, Rows, View, ink_on};
use super::{ProjectRef, RowsRef};

/// A control a row can be pressed on.
///
/// Only the ones that ARE pressable. The sheet draws every control, but
/// a picture cannot be clicked: an `<img>` is one node and one node has
/// one hit box, which is the price of drawing four hundred shapes
/// without spending four hundred nodes. So the controls that do
/// something get a real element over the art — an invisible one, the
/// size of the control, which is a node per interactive control rather
/// than a node per shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    Mute,
    Solo,
    RecArm,
}

/// What a track's live controls are showing.
///
/// Passed in rather than read here: a volume is a value that changes
/// while the window is open, and where it comes from — a store, a meter
/// feed, a fixture — is the host's business, not the panel's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Live {
    pub volume: f64,
    pub pan: f64,
    pub muted: bool,
    pub soloed: bool,
    pub armed: bool,
    /// Whether the track feeds its folder, and whether anything else
    /// feeds it or is fed by it. Three lights on one control, because
    /// "where does this go" is one question.
    pub parent_send: bool,
    pub sends: bool,
    pub receives: bool,
    /// Whether the chain has anything in it.
    pub effects: bool,
    /// Whether polarity is flipped.
    pub phase_inverted: bool,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            // Unity, centred, and nothing switched on — what a track
            // reads as before anything has been done to it.
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            armed: false,
            parent_send: true,
            sends: false,
            receives: false,
            effects: false,
            phase_inverted: false,
        }
    }
}

/// How wide the panel is.
pub const ROW_W: f64 = 343.0;

/// Where the row's tint ends and REAPER's meter gutter begins.
pub const TINT_W: f64 = 296.0;

/// The one-pixel rule between the left column and the row.
pub const RAIL_W: f64 = 20.0;

/// Where the name field starts, and how wide it is.
pub const NAME_FIELD_X: f64 = 33.0;
pub const NAME_FIELD_W: f64 = 136.0;

/// Where the name itself starts — past the record arm on the field.
pub const NAME_X: f64 = 58.0;

/// How far a folder's children are indented per level.
///
/// REAPER indents the row's CONTENT, not the row, so the tint still
/// reaches the panel's edge and only the field and the number move.
pub const INDENT: f64 = 10.0;

/// Past this, an indent would push the name field into the volume knob.
pub const MAX_INDENT: f64 = 60.0;

/// Below this height a row is a band and nothing else.
pub const BAND_BELOW: f64 = 14.0;

/// The height the control band is authored at.
pub const AUTHORED: f64 = 24.0;

/// Where the control band sits in a full-height row.
pub const ROW_ONE: f64 = 6.0;

/// And the height above which a row shows everything REAPER's does.
pub const FULL_ABOVE: f64 = 58.0;

/// Mute and solo, at the one size they are drawn at everywhere.
///
/// That size on EVERY track, which is why the shortest settable row is
/// what it is: the row holds the controls rather than the controls
/// shrinking to fit the row. A control that is a different shape on
/// every track cannot be built on — no shared hit target, no drag down a
/// column, no "the mute column" for anything else to address.
pub const BUTTON: (f64, f64) = (21.0, 20.0);

/// Between the two, so they read as two controls.
pub const BUTTON_GAP: f64 = 1.0;

/// Below this tall, volume and pan stop being knobs.
pub const KNOB_LEGIBLE: f64 = 20.0;

/// Where the pan knob sits.
pub const PAN_KNOB_X: f64 = 184.0;

/// Where the second row sits, and how tall its fields are.
pub const ROW_TWO: f64 = 34.0;
pub const FIELD_H: f64 = 20.0;

/// Where routing sits, and the FX pill after it.
pub const ROUTING_X: f64 = 214.0;
pub const FX_IN_X: f64 = 248.0;

/// How far up from the row's floor polarity sits.
pub const PHASE_FROM_FLOOR: f64 = 24.0;

/// And the height below which it is not drawn at all.
///
/// The theme's own formula rather than a number chosen here: the row's
/// shape must not depend on its height.
pub const PHASE_HIDE_H: f64 = 12.0 + 20.0 + 17.0 + 20.0 + 17.0;

/// The polarity glyph's width — one measurement shared by both panels,
/// because it is the same art in the strip and in the row.
pub const PHASE_W: f64 = 16.0;

/// Where the gutter's buttons start, past the tint.
pub const GUTTER_BUTTON_X: f64 = 21.0;

/// What fits in a row this tall.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Density {
    /// Everything REAPER's row has.
    Full,
    /// The control row flattened into whatever height there is.
    ///
    /// Controls are SQUASHED here, not dropped. A track at fourteen
    /// pixels still has everything a track at seventy has, just flatter,
    /// and every one stays in the same column: a panel where controls
    /// appear and vanish as tracks resize cannot be read down, and
    /// hitting one would depend on how tall its track happened to be.
    Compact,
    /// A coloured band.
    Bar,
}

impl Density {
    /// What fits in `height`.
    #[must_use]
    pub fn at(height: f64) -> Self {
        if height >= FULL_ABOVE {
            Self::Full
        } else if height >= BAND_BELOW {
            Self::Compact
        } else {
            Self::Bar
        }
    }
}

/// How big a name is written at this row height.
///
/// The type shrinks with the row rather than being squashed with it: a
/// flattened glyph is unreadable where a smaller one is merely small.
#[must_use]
pub fn name_size(height: f64) -> f64 {
    (height * 0.48).clamp(6.5, 11.5)
}

/// A track's own colour, mixed into the panel's grey.
///
/// REAPER tints the whole row rather than showing a colour chip, which
/// is what makes a session readable by section at a glance. The strength
/// is the theme's, not a number chosen here.
#[must_use]
pub fn row_tint(colors: &Colors, track: &daw_proto::Track) -> String {
    track.color.map_or_else(
        || colors.tcp_tint.clone(),
        |rgb| mix(&colors.tcp_tint_rgb, rgb, colors.track_tint),
    )
}

/// The colour a folder writes down the left edge of its children.
///
/// The track's own colour, not [`row_tint`]'s. That one mixes a few per
/// cent of the colour into the panel's grey — right for a strip body,
/// where the colour is a hint behind controls you are reading — and
/// hopeless for a ten-pixel band whose ENTIRE job is to be identifiable
/// at a glance across half a screen.
///
/// Still short of the raw colour: pulled toward the panel so a column of
/// bands reads as part of the panel rather than as a stripe of paint
/// down it.
#[must_use]
pub fn folder_band(colors: &Colors, track: &daw_proto::Track) -> String {
    /// How far toward the track's own colour the band goes.
    const STRENGTH: f32 = 0.62;
    track.color.map_or_else(
        || colors.tcp_gutter.clone(),
        |rgb| mix(&colors.tcp_tint_rgb, rgb, STRENGTH),
    )
}

/// Blend a packed `0xRRGGBB` into a base colour.
fn mix(base: &(u8, u8, u8), rgb: u32, t: f32) -> String {
    let t = t.clamp(0.0, 1.0);
    let at = |shift: u32| u8::try_from((rgb >> shift) & 0xff).unwrap_or(0);
    let blend = |a: u8, b: u8| {
        let (a, b) = (f32::from(a), f32::from(b));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions,
            reason = "a blend of two channels, which stays inside 0..255"
        )]
        let out = (b - a).mul_add(t, a).clamp(0.0, 255.0) as u8;
        out
    };
    format!(
        "rgb({}, {}, {})",
        blend(base.0, at(16)),
        blend(base.1, at(8)),
        blend(base.2, at(0))
    )
}

/// Where a row's controls sit, in the row's own coordinates.
///
/// The same arithmetic the painted panel lays out from, so a control is
/// in one place rather than in two places that agree today.
#[derive(Clone, Copy, PartialEq)]
pub struct Band {
    pub top: f64,
    pub height: f64,
    pub indent: f64,
    pub density: Density,
}

impl Band {
    /// The control band of a row this tall, at this depth.
    #[must_use]
    pub fn of(height: f64, depth: usize) -> Self {
        let density = Density::at(height);
        let (top, band) = if density == Density::Full {
            (ROW_ONE, AUTHORED)
        } else {
            (1.0, (height - 2.0).max(1.0))
        };
        Self {
            top,
            height: band,
            indent: (f64::from(u32::try_from(depth).unwrap_or(0)) * INDENT).min(MAX_INDENT),
            density,
        }
    }

    /// Whether volume and pan are knobs, or flattened bars.
    #[must_use]
    pub fn knobs(self) -> bool {
        self.height >= KNOB_LEGIBLE
    }

    /// The record arm, on rows tall enough to read one.
    #[must_use]
    pub fn rec_arm(self) -> Option<(f64, f64, f64)> {
        self.knobs().then(|| {
            (
                NAME_FIELD_X + self.indent + 3.0,
                self.height.mul_add(0.5, self.top) - 10.0,
                20.0,
            )
        })
    }

    /// The volume knob: centred on the name field's right edge, and
    /// scaled to the band it straddles.
    #[must_use]
    pub fn volume(self) -> (f64, f64, f64) {
        let scale = if self.knobs() {
            self.height / 22.0
        } else {
            1.0
        };
        let w = 24.0 * scale;
        (NAME_FIELD_X + NAME_FIELD_W - w / 2.0, self.top, scale)
    }

    /// The pan knob.
    ///
    /// Authored at twenty-five rather than the volume knob's twenty-two,
    /// and never drawn larger than authored — a knob scaled past its own
    /// art is a blur where every other control is crisp.
    #[must_use]
    pub fn pan(self) -> (f64, f64, f64) {
        (PAN_KNOB_X, self.top, (self.height / 25.0).min(1.0))
    }

    /// Routing, on rows with room for the whole band.
    #[must_use]
    pub fn routing(self) -> Option<(f64, f64)> {
        (self.density == Density::Full).then_some((ROUTING_X, self.top))
    }

    /// The FX pill, likewise.
    #[must_use]
    pub fn fx(self) -> Option<(f64, f64)> {
        (self.density == Density::Full).then_some((FX_IN_X, self.top))
    }

    /// Polarity, in the corner below routing — measured from the row's
    /// FLOOR rather than from the control band, because that is where
    /// the theme puts it.
    #[must_use]
    pub fn phase(self, row_height: f64) -> Option<(f64, f64)> {
        (row_height >= PHASE_HIDE_H).then(|| {
            (
                TINT_W + GUTTER_BUTTON_X + 3.0,
                row_height - PHASE_FROM_FLOOR,
            )
        })
    }

    /// Mute and solo, in the gutter past the tint.
    #[must_use]
    pub fn gutter(self, solo: bool) -> (f64, f64) {
        let x = TINT_W + 2.0 + if solo { BUTTON.0 + BUTTON_GAP } else { 0.0 };
        let top = if self.density == Density::Full {
            ROW_ONE + (AUTHORED - BUTTON.1) / 2.0
        } else {
            let h = self.height.min(BUTTON.1);
            self.top + (self.height - h) / 2.0
        };
        (x, top)
    }
}

/// The panel: a row per visible track.
#[component]
pub fn Panel(
    project: ProjectRef,
    rows: RowsRef,
    view: View,
    colors: Colors,
    /// How far DOWN the session the view is.
    ///
    /// A signal and not a number in the view, for the reason the lanes
    /// give: read as a prop, a vertical scroll re-renders every row, its
    /// controls and its hit targets — forty milliseconds a frame in a
    /// window where a horizontal scroll costs nothing. The rows are built
    /// for a window taller than the screen and a transform slides them.
    #[props(default)]
    scroll_y: ReadSignal<f64>,
    /// The theme the controls are drawn from. Separate from [`Colors`]
    /// because the art takes a `Chrome` and the boxes take CSS strings,
    /// and resolving one from the other per frame would be work for
    /// nothing.
    theme: crate::theming::Theme,
    #[props(default)] sizing: Rows,
    /// What each track's controls are showing, by guid. A track with no
    /// entry gets [`Live::default`], which is what an unread track looks
    /// like rather than a silent one.
    #[props(default)]
    live: std::collections::HashMap<String, Live>,
    /// A control was pressed on a track.
    ///
    /// The panel does not know what mute MEANS — it hands back which
    /// track and which control, and whoever owns the session decides.
    #[props(default)]
    on_press: EventHandler<(String, Control)>,
) -> Element {
    let _ = project;
    let offsets = use_memo({
        let rows = rows.clone();
        move || Offsets::of(&rows, sizing)
    });
    let offsets = offsets();
    // The band of session that is BUILT, which moves a screen at a time
    // rather than a pixel at a time.
    let built = use_memo(move || Built::down(scroll_y(), view.height));
    let built = built();
    let visible = offsets.visible(View {
        scroll_y: built.from,
        height: built.to - built.from,
        ..view
    });

    // The colour of the folder open at each depth, so a row can draw the
    // folders it sits inside down its own left edge. Walked from the top
    // of the visible range rather than from the top of the session: a
    // folder's lineage is whatever is still open above it, and the rows
    // above the screen are what say so.
    let mut lineage: Vec<String> = Vec::new();
    for (track, depth) in rows.iter().take(visible.start) {
        lineage.truncate(usize::try_from(*depth).unwrap_or(0));
        lineage.push(folder_band(&colors, track));
    }

    // Every control on screen, in one sheet. Built here rather than per
    // row because that is the whole point of a sheet: a panel is forty
    // rows of controls, and an `<svg>` each is forty elements and forty
    // usvg parses where one will do.
    // Anchored to the first visible row's own top, so what the sheet
    // says does not change while the view moves over it.
    //
    // This is the difference between a window that scrolls and one that
    // crawls. The sheet is a `data:` image, and its URI is its CONTENT:
    // put the rows where the scroll has left them and the string differs
    // every frame, which makes it a new resource for Blitz to fetch and
    // hand to usvg — several hundred shapes re-parsed per frame. Built
    // where the SESSION says the rows are, the string is the same string
    // until a new row scrolls in, and the scroll is a transform.
    let anchor = built.from;
    let mut sheet = Sheet::new();
    let chrome = super::art::chrome(&theme);
    let lit = super::art::lit(&theme);
    let ink = super::art::route_ink(&theme);
    let buttons = super::art::buttons(&theme);
    for row in visible.clone() {
        let (Some((top, height)), Some((track, depth))) = (offsets.row(row), rows.get(row)) else {
            continue;
        };
        let top = top.mul_add(view.zoom_y, -anchor);
        let height = height * view.zoom_y;
        let body = (height - DIVIDER).max(0.5);
        if Density::at(height) == Density::Bar {
            continue;
        }
        let band = Band::of(height, usize::try_from(*depth).unwrap_or(0));
        let state = live.get(&track.guid).copied().unwrap_or_default();
        controls(
            &mut sheet, &chrome, &lit, &ink, buttons, band, state, top, body,
        );
    }
    // Timed, because this is the one thing in the panel that is not
    // cheap: several hundred shapes written out as markup, percent
    // encoded, and handed to a parser as a NEW resource. If a scroll
    // rebuilds it, a scroll costs that — and this is how to find out
    // rather than assume.
    // As tall as the rows it holds, which reach past the window at both
    // ends by the bleed the visible range carries.
    let sheet_h = offsets
        .row(visible.end.saturating_sub(1))
        .map_or(view.height, |(top, height)| {
            (top + height).mul_add(view.zoom_y, -anchor)
        })
        .max(view.height);
    let art = (!sheet.is_empty()).then(|| sheet.data_uri(ROW_W, sheet_h));
    let labels = sheet.labels().to_vec();

    rsx! {
        div {
            style: "position:relative; width:{ROW_W}px; height:{view.height}px; \
                    overflow:hidden; background:{colors.tcp_gutter}; font-family:{FONT};",
            "data-testid": "studio-panel",
            // The panel's own right edge — the boundary with the arrange
            // view, and OUTSIDE the sliding part because an edge that
            // scrolled with the session would not be an edge. One rule
            // down the whole column rather than a fragment of one per
            // row: it is the PANEL's edge, and every row was drawing the
            // same pixel.
            div {
                style: "position:absolute; left:{ROW_W - 2.0}px; top:0; width:1px; \
                        bottom:0; background:{colors.rule}; z-index:1;",
            }
            // Everything that moves with the session, under one node so
            // that a scroll writes one transform.
            Sliding { scroll_y, anchor, children: rsx! {
            for row in visible.clone() {
                if let (Some((top, height)), Some((track, depth))) =
                    (offsets.row(row), rows.get(row))
                {
                    {
                        let level = usize::try_from(*depth).unwrap_or(0);
                        lineage.truncate(level);
                        let ancestors = lineage.clone();
                        lineage.push(folder_band(&colors, track));
                        rsx! {
                            Row {
                                key: "{track.guid}",
                                track: track.clone(),
                                depth: level,
                                ancestors,
                                top: top.mul_add(view.zoom_y, -anchor),
                                height: height * view.zoom_y,
                                colors: colors.clone(),
                                state: live.get(&track.guid).copied().unwrap_or_default(),
                                on_press,
                            }
                        }
                    }
                }
            }

            // The controls and their labels, moved as one by the scroll
            // rather than rebuilt by it.
            div {
                style: "position:absolute; left:0; top:0; \
                        width:{ROW_W}px; height:{sheet_h}px; pointer-events:none;",
                if let Some(art) = art {
                    Art { source: art, width: ROW_W, height: sheet_h }
                }
                for (index, label) in labels.into_iter().enumerate() {
                    Word { key: "{index}", label }
                }
            }
            } }
        }
    }
}

/// The control sheet: every control on screen, in one image.
///
/// An `<img>` over an inline `<svg>` because Blitz renders an inline one
/// by walking its DOM subtree back into markup — the shapes would have
/// to BE nodes, which is a node per shape and the one thing this effort
/// has spent itself avoiding. An image is one node whatever the sheet
/// holds, and every target has drawn an SVG image for twenty years.
#[component]
fn Art(source: String, width: f64, height: f64) -> Element {
    rsx! {
        img {
            src: "{source}",
            width: "{width:.0}",
            height: "{height:.0}",
            // Sized in CSS as well as by attribute: an SVG image scales
            // to whatever box it is given, so a parent that constrains
            // it does not clip it — it SQUASHES it, and every control
            // drifts further from its row the further down the panel it
            // is. Stating the size twice is what stops that.
            style: "position:absolute; left:0; top:0; width:{width:.0}px; \
                    height:{height:.0}px; max-width:none; pointer-events:none;",
        }
    }
}

/// One of the sheet's labels, written as an element.
///
/// Not as `<text>` inside the sheet: there it would be lettered by
/// usvg's own font stack on Blitz and by the page's on the web, which is
/// two different pictures of the same word.
#[component]
fn Word(label: Label) -> Element {
    let line = super::ruler::line_box(label.size, label.size);
    let (left, shift) = if label.centred {
        (label.x, "transform:translateX(-50%);")
    } else {
        (label.x, "")
    };
    rsx! {
        div {
            style: "position:absolute; left:{left}px; top:{label.baseline - label.size}px; \
                    font-size:{label.size}px; line-height:{line}px; color:{label.color}; \
                    white-space:nowrap; pointer-events:none; {shift}",
            "{label.body}"
        }
    }
}

/// Put one row's controls into the sheet.
///
/// The art's own drawings, placed at the measured rects and scaled to
/// the band — so the shapes are the theme's rather than this module's,
/// and the two renderers cannot drift apart by anyone's judgement.
#[expect(
    clippy::too_many_arguments,
    reason = "a row's controls are its geometry, its values and its \
              palette, and every one of them is needed to place one"
)]
fn controls(
    sheet: &mut Sheet,
    chrome: &daw_theme::Chrome,
    lit: &art::Lit,
    ink: &art::RouteInk,
    buttons: (daw_theme::Color, daw_theme::Color),
    band: Band,
    live: Live,
    row_top: f64,
    row_height: f64,
) {
    let at = Interaction::Normal;
    if let Some((x, y, _)) = band.rec_arm() {
        sheet.place(
            &art::record_arm(
                chrome,
                lit.rec,
                live.armed,
                at,
                art::Arm::Panel,
                chrome.surface_sunken,
            ),
            x,
            row_top + y,
            1.0,
        );
    }
    // Volume and pan, in whichever form the row is showing — a knob
    // where there is room to turn one, a flattened bar where there is
    // not. Both are the same VALUE; only the shape changes.
    let (vx, vy, scale) = band.volume();
    if band.knobs() {
        sheet.place(
            &art::volume_knob(chrome, lit.volume, live.volume, at, band.height),
            vx,
            row_top + vy,
            scale,
        );
    } else {
        sheet.place(
            &art::volume_fader(chrome, lit.volume, live.volume),
            vx,
            row_top + vy,
            1.0,
        );
    }
    let (px, py, pscale) = band.pan();
    if band.knobs() {
        sheet.place(
            // Pan's own colour, not the mark's: it is yellow so that it
            // is never mistaken for volume's blue in the column beside
            // it.
            &art::pan_knob(chrome, live.pan.clamp(-1.0, 1.0), lit.pan, at),
            px,
            row_top + py,
            pscale,
        );
    }
    // Routing, and polarity in the corner below it. Both are live
    // values with their own events — a row that recorded them was right
    // until the first time anything changed one.
    if let Some((x, y)) = band.routing() {
        sheet.place(
            &art::routing(
                chrome,
                art::Axis::Horizontal,
                art::Routing {
                    parent_send: live.parent_send,
                    sends: live.sends,
                    receives: live.receives,
                },
                *ink,
                at,
            ),
            x,
            row_top + y,
            1.0,
        );
    }
    if let Some((x, y)) = band.fx() {
        sheet.place(
            &art::fx_pill(
                chrome,
                *lit,
                if live.effects {
                    art::Chain::Active
                } else {
                    art::Chain::Empty
                },
                at,
            ),
            x,
            row_top + y,
            1.0,
        );
    }
    if let Some((x, y)) = band.phase(row_height) {
        sheet.place(
            &art::phase(chrome, live.phase_inverted, at),
            x,
            row_top + y,
            1.0,
        );
    }
    // The combo's caret — a triangle, which is a shape, so it goes where
    // the shapes go rather than becoming a `clip-path` of its own.
    if band.density == Density::Full && live.armed {
        sheet.place(
            &caret(chrome.text_faint),
            91.0 + 181.0,
            row_top + ROW_TWO + 8.0,
            1.0,
        );
    }

    for (solo, on) in [(false, live.muted), (true, live.soloed)] {
        let (x, y) = band.gutter(solo);
        sheet.place(
            &art::gutter_button(
                chrome,
                if solo { "S" } else { "M" },
                on,
                // Each lights in its OWN colour: solo yellow, mute the
                // bypass shade. They sit next to each other and mean
                // opposite things, so a lit one has to say which it is
                // without being read.
                if solo { buttons.1 } else { buttons.0 },
                at,
            ),
            x,
            row_top + y,
            1.0,
        );
    }
}

/// The combo's caret: the shared triangle, not a glyph.
fn caret(ink: daw_theme::Color) -> daw_theme_art::paint::Drawing {
    let mut drawing = daw_theme_art::paint::Drawing::new(7.0, 4.0);
    drawing.fill(
        daw_theme_art::paint::Shape::Poly(vec![(0.0, 0.0), (7.0, 0.0), (3.5, 4.0)]),
        ink,
    );
    drawing
}

/// The part of the panel that moves with the session.
///
/// One node between the scroll and everything in it, so a scroll writes
/// one transform instead of rebuilding forty rows of controls.
#[component]
fn Sliding(scroll_y: ReadSignal<f64>, anchor: f64, children: Element) -> Element {
    let offset = scroll_y() - anchor;
    rsx! {
        div {
            style: "position:absolute; left:0; top:0; width:100%; height:100%; \
                    transform: translateY({-offset}px);",
            {children}
        }
    }
}

/// One row of the panel.
#[component]
#[expect(
    clippy::too_many_arguments,
    reason = "a row is its track, its place, its palette and its state"
)]
fn Row(
    track: daw_proto::Track,
    depth: usize,
    ancestors: Vec<String>,
    top: f64,
    height: f64,
    colors: Colors,
    state: Live,
    on_press: EventHandler<(String, Control)>,
) -> Element {
    let body = (height - DIVIDER).max(0.5);
    // The tier is decided by the ROW, not by the row less its divider:
    // the divider is inside the height, and a row that is a band at 14
    // has to be a band at 14 in both renderers or the two panels
    // disagree about which tracks show controls.
    let density = Density::at(height);
    let tint = row_tint(&colors, &track);
    let band = folder_band(&colors, &track);
    let indent = (f64::from(u32::try_from(depth).unwrap_or(0)) * INDENT).min(MAX_INDENT);

    // The band tier is the row's tint and its divider and nothing else.
    // Five rectangles a row is nothing at seventy pixels and everything
    // at two, and a session zoomed to fit has every row on screen at
    // once — culling cannot help when nothing is off screen.
    if density == Density::Bar {
        return rsx! {
            div {
                style: "position:absolute; left:0; top:{top}px; width:{TINT_W}px; \
                        height:{height}px; box-sizing:border-box; background:{tint}; \
                        border-bottom:{DIVIDER}px solid {colors.divider};",
            }
        };
    }

    let mark_h = mark_height(&track, body);
    let digits = (track.index + 1).to_string().len();
    // The name sits on the field's middle, which CSS says with a line
    // box the height of the field.
    let field_h = if density == Density::Full {
        AUTHORED
    } else {
        // The band is the row less a pixel top and bottom, so the
        // controls are not flush against the dividers.
        (height - 2.0).max(1.0)
    };
    // Where this row's controls sit — the same arithmetic the sheet
    // placed the art with, so a hit target lands on the control it is
    // for rather than near it.
    let controls = Band::of(height, depth);
    let name_size = name_size(field_h);
    let shows_number = body - mark_h >= 11.0;
    // A rail with nothing above the number can carry it itself.
    let numbered_here = shows_number && !track.is_folder;
    let numbered = if numbered_here {
        format!(
            "line-height:{body}px; text-align:center; font-size:{}px; color:{};",
            number_size(digits),
            ink_on_band(&band)
        )
    } else {
        String::new()
    };
    let field_top = if density == Density::Full {
        ROW_ONE
    } else {
        1.0
    };
    let name_w = (NAME_FIELD_X + NAME_FIELD_W - NAME_X - indent).max(0.0);

    rsx! {
        div {
            style: "position:absolute; left:0; top:{top}px; width:{ROW_W}px; \
                    height:{height}px; box-sizing:border-box; background:{tint}; \
                    border-bottom:{DIVIDER}px solid {colors.divider};",
            "data-track": "{track.guid}",

            // The meter gutter, over the tint's right end: one row with a
            // gutter at the end of it, not two panels side by side.
            div {
                style: "position:absolute; left:{TINT_W}px; top:0; right:0; bottom:0; \
                        background:{colors.tcp_gutter};",
            }

            // The left column, and the folders this row sits inside.
            // Each ancestor paints one step of the indent in its own
            // colour, and because every child of a folder paints it at
            // the same x the steps join top to bottom into one unbroken
            // line — the folder's own left edge running down past
            // everything inside it.
            div {
                style: "position:absolute; left:0; top:0; width:{indent + RAIL_W}px; \
                        bottom:0; background:{colors.tcp_column};",
            }
            for (level, tint) in ancestors.iter().enumerate() {
                {
                    let left = f64::from(u32::try_from(level).unwrap_or(0)) * INDENT;
                    if left >= MAX_INDENT {
                        return rsx! {};
                    }
                    let wide = (left + INDENT).min(MAX_INDENT) - left;
                    rsx! {
                        div {
                            key: "{level}",
                            style: "position:absolute; left:{left}px; top:0; \
                                    width:{wide}px; bottom:0; background:{tint};",
                        }
                    }
                }
            }
            // The rail carries the row's OWN colour, at the same
            // strength — so the whole left edge is the track and its
            // lineage, unbroken from the panel's edge to the row's
            // content, and a folder's stripe starts on the folder's own
            // row rather than on its first child.
            div {
                style: "position:absolute; left:{indent}px; top:0; width:{RAIL_W}px; \
                        bottom:0; background:{band}; \
                        border-right:1px solid {colors.rule}; \
                        box-sizing:border-box; {numbered}",
                // On a row with no folder mark the rail IS the number's
                // box, so the number is its text. A folder's rail has
                // the mark at its top and the number under it, which
                // needs a box of its own — see below.
                if numbered_here {
                    "{track.index + 1}"
                }
            }

            // The folder mark, at the TOP of the rail.
            //
            // REAPER puts it at the bottom; at the top it lines up with
            // the controls beside it, which is what makes a folder
            // readable while scanning a collapsed session rather than
            // something you find by looking down.
            //
            // Two rectangles, because that is what the mark is: a body
            // and a tab. The art draws it as one six-point polygon and
            // the polygon has no curve in it, so this is the same shape
            // rather than an impression of it.
            if track.is_folder {
                {
                    let scale = mark_scale(body);
                    let left = indent + (RAIL_W - MARK_W * scale) / 2.0;
                    let top = 2.0 * scale;
                    rsx! {
                        div {
                            style: "position:absolute; left:{left}px; top:{top}px; \
                                    width:{TAB_W * scale}px; height:{TAB_H * scale}px; \
                                    background:{colors.text_dim};",
                        }
                        div {
                            style: "position:absolute; left:{left}px; \
                                    top:{top + TAB_H * scale}px; \
                                    width:{MARK_W * scale}px; \
                                    height:{(MARK_H - TAB_H) * scale}px; \
                                    background:{colors.text_dim};",
                        }
                    }
                }
            }

            // The track number, under whatever the rail already holds.
            // It gets the space that is left, and gives way entirely
            // when a folder's mark has taken the rail — the mark is the
            // fact worth keeping when only one of the two fits.
            if shows_number && !numbered_here {
                div {
                    style: "position:absolute; left:{indent}px; top:{mark_h}px; \
                            width:{RAIL_W}px; height:{body - mark_h}px; \
                            line-height:{body - mark_h}px; text-align:center; \
                            font-size:{number_size(digits)}px; \
                            color:{ink_on_band(&band)}; overflow:hidden;",
                    "{track.index + 1}"
                }
            }

            // The name field, with the name ON it: a pill on its left
            // end and square on its right, because the record arm sits
            // on the field rather than beside it — and the name is the
            // field's own text, padded past where the arm goes, rather
            // than a second box laid over the first.
            //
            // Clipped rather than wrapped or shrunk; REAPER truncates
            // here too.
            div {
                style: "position:absolute; left:{NAME_FIELD_X + indent}px; \
                        top:{field_top}px; width:{(NAME_FIELD_W - indent).max(0.0)}px; \
                        height:{field_h}px; box-sizing:border-box; \
                        padding-left:{NAME_X - NAME_FIELD_X}px; \
                        background:{colors.tcp_field}; \
                        border-radius:{field_h / 2.0}px 0 0 {field_h / 2.0}px; \
                        line-height:{field_h}px; font-size:{name_size}px; \
                        color:{name_ink(&colors, &track)}; white-space:nowrap; \
                        overflow:hidden;",
                "{track.name}"
            }
            // The hit targets, over the art. Invisible, because the
            // sheet already drew the control — these exist to be
            // pressed, to take focus, and to say what they are to
            // anything that asks.
            for (control, label, on) in [
                (Control::Mute, "Mute", state.muted),
                (Control::Solo, "Solo", state.soloed),
            ] {
                {
                    let (x, y) = controls.gutter(control == Control::Solo);
                    let guid = track.guid.clone();
                    let name = track.name.clone();
                    let tall = controls.height.min(BUTTON.1);
                    rsx! {
                        button {
                            key: "{label}",
                            style: "position:absolute; left:{x}px; top:{y}px; \
                                    width:{BUTTON.0}px; height:{tall}px; \
                                    background:transparent; border:0; padding:0; \
                                    margin:0; cursor:pointer;",
                            "aria-pressed": "{on}",
                            "aria-label": "{label} {name}",
                            onclick: move |_| on_press.call((guid.clone(), control)),
                        }
                    }
                }
            }
            if let Some((x, y, size)) = controls.rec_arm() {
                {
                    let guid = track.guid.clone();
                    let name = track.name.clone();
                    rsx! {
                        button {
                            style: "position:absolute; left:{x}px; top:{y}px; \
                                    width:{size}px; height:{size}px; \
                                    background:transparent; border:0; padding:0; \
                                    margin:0; cursor:pointer;",
                            "aria-pressed": "{state.armed}",
                            "aria-label": "Record arm {name}",
                            onclick: move |_| on_press.call((guid.clone(), Control::RecArm)),
                        }
                    }
                }
            }

            // The second row: the input FX slot and the record-input
            // combo. They belong to RECORDING, so they appear when the
            // track is armed and not before — on a two-thousand-track
            // orchestral template almost nothing is armed, and an input
            // selector reading "None" on every row fills the panel with
            // the one thing none of those tracks are doing.
            if density == Density::Full && track.armed {
                {
                    let two = ROW_TWO;
                    let name = crate::controls::record_input_name(&track);
                    rsx! {
                        div {
                            style: "position:absolute; left:56px; top:{two}px; \
                                    width:34px; height:{FIELD_H}px; \
                                    background:{colors.tcp_field}; text-align:center; \
                                    line-height:{super::ruler::line_box(10.0, 14.0)}px; \
                                    font-size:10px; color:{colors.faint};",
                            "FX"
                        }
                        div {
                            style: "position:absolute; left:91px; top:{two}px; \
                                    width:195px; height:{FIELD_H}px; \
                                    background:{colors.tcp_combo}; text-align:center; \
                                    line-height:{super::ruler::line_box(11.0, 14.0)}px; \
                                    font-size:11px; color:{colors.text_dim}; \
                                    white-space:nowrap; overflow:hidden;",
                            "{name}"
                        }
                    }
                }
            }


        }
    }
}

/// How big the track number is written.
///
/// Sized to the RAIL'S WIDTH, not to the row's height: the rail is what
/// it has to fit inside, and a two-digit number at a fixed size ran past
/// the rule closing it — one track's number crossing into the body of
/// its own row. A three-digit session is not unusual and this is where
/// it shows.
///
/// The advance is the face's own digit width. Digits are tabular in
/// DejaVu, so one number fits them all, which is what lets this be
/// arithmetic instead of a text measurement the component cannot do.
#[must_use]
pub fn number_size(digits: usize) -> f64 {
    /// How wide a digit is, as a fraction of the type size.
    const ADVANCE: f64 = 0.636;
    /// The room the rail gives, less the rule closing it.
    const ROOM: f64 = RAIL_W - 3.0;
    let digits = f64::from(u32::try_from(digits.max(1)).unwrap_or(1));
    (ROOM / (digits * ADVANCE)).clamp(6.0, 11.0)
}

/// How tall the folder mark is, including the gap under it.
///
/// Nought for a track that is not a folder — which is the whole of the
/// rail then going to the number.
#[must_use]
pub fn mark_height(track: &daw_proto::Track, body: f64) -> f64 {
    if track.is_folder {
        (MARK_H + 2.0) * mark_scale(body)
    } else {
        0.0
    }
}

/// How much the mark is scaled down on a short row.
fn mark_scale(body: f64) -> f64 {
    (body / 18.0).clamp(0.4, 1.0)
}

/// The folder mark's authored size.
const MARK_W: f64 = 9.0;
const MARK_H: f64 = 7.0;
/// And its tab, which is the part that makes it a folder rather than a
/// box: four across and two down, out of nine by seven.
const TAB_W: f64 = 4.0;
const TAB_H: f64 = 2.0;

/// Ink that reads on a folder band.
fn ink_on_band(band: &str) -> String {
    // The band is `rgb(r, g, b)` because `mix` made it, or a token
    // colour otherwise — either way the luminance decides.
    band.trim_start_matches("rgb(")
        .trim_start_matches("rgba(")
        .trim_end_matches(')')
        .split(',')
        .take(3)
        .map(|part| part.trim().parse::<f32>().unwrap_or(0.0))
        .collect::<Vec<_>>()
        .split_first()
        .map_or_else(
            || "rgba(232, 232, 234, 1.000)".to_owned(),
            |(r, rest)| {
                let (g, b) = (
                    rest.first().copied().unwrap_or(0.0),
                    rest.get(1).copied().unwrap_or(0.0),
                );
                ink_on(crate::theming::Color {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::as_conversions,
                        reason = "a channel that was parsed out of a channel"
                    )]
                    r: r.clamp(0.0, 255.0) as u8,
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::as_conversions,
                        reason = "a channel that was parsed out of a channel"
                    )]
                    g: g.clamp(0.0, 255.0) as u8,
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::as_conversions,
                        reason = "a channel that was parsed out of a channel"
                    )]
                    b: b.clamp(0.0, 255.0) as u8,
                    a: 255,
                })
            },
        )
}

/// A selected track's name is brighter than the rest.
fn name_ink(colors: &Colors, track: &daw_proto::Track) -> String {
    if track.selected {
        colors.text.clone()
    } else {
        colors.text_dim.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{BAND_BELOW, Density, FULL_ABOVE, MARK_H, RAIL_W, name_size, number_size};

    /// What fits in a row is decided by its height and nothing else.
    #[test]
    fn a_row_shows_what_fits_in_it() {
        assert_eq!(Density::at(70.0), Density::Full);
        assert_eq!(Density::at(FULL_ABOVE), Density::Full);
        assert_eq!(Density::at(FULL_ABOVE - 0.1), Density::Compact);
        assert_eq!(Density::at(32.0), Density::Compact);
        assert_eq!(Density::at(BAND_BELOW), Density::Compact);
        assert_eq!(Density::at(BAND_BELOW - 0.1), Density::Bar);
        assert_eq!(Density::at(1.0), Density::Bar);
    }

    /// The type shrinks with the row, between a floor and a ceiling —
    /// it is never squashed and never grows past what the field holds.
    #[test]
    fn a_name_shrinks_with_its_row_but_only_so_far() {
        assert!(
            (name_size(24.0) - 11.5).abs() < 1e-9,
            "it grew past the field"
        );
        assert!((name_size(200.0) - 11.5).abs() < 1e-9);
        assert!(
            (name_size(1.0) - 6.5).abs() < 1e-9,
            "it shrank past legible"
        );
        assert!(
            name_size(20.0) < name_size(24.0),
            "it did not shrink at all"
        );
    }

    /// The number is sized to the rail, so a long one fits inside it.
    ///
    /// The case that matters is three digits: a session with two hundred
    /// tracks has them, and at a fixed size the third one crossed the
    /// rule closing the rail.
    #[test]
    fn a_number_shrinks_to_fit_its_rail() {
        const ADVANCE: f64 = 0.636;
        for digits in 1..=4 {
            let size = number_size(digits);
            let width = f64::from(u32::try_from(digits).unwrap()) * ADVANCE * size;
            assert!(
                width <= RAIL_W - 3.0 + 1e-9 || size <= 6.0,
                "{digits} digits at {size}px is {width} wide in a {RAIL_W} rail"
            );
        }
        assert!((number_size(1) - 11.0).abs() < 1e-9, "one digit was shrunk");
        assert!(number_size(3) < 11.0, "three digits were not shrunk");
        assert!(number_size(9) >= 6.0, "it shrank past legible");
    }

    /// The mark scales with the row and takes the rail's top.
    #[test]
    fn a_folder_mark_shrinks_with_its_row() {
        let tall = super::mark_scale(70.0);
        let short = super::mark_scale(14.0);
        assert!((tall - 1.0).abs() < 1e-9, "it grew past its authored size");
        assert!(
            short < tall && short >= 0.4,
            "it shrank past visible: {short}"
        );
        assert!(MARK_H > 0.0);
    }
}
