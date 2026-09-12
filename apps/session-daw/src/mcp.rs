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
use daw_ui::controls::{Collapse, PanAnchor};
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

/// A name with what its folders already say taken off the front.
///
/// `T1 Trig` sits inside `Tom 1`. The `T1` is not information at that
/// point — the strip is standing on a band of the folder's colour that
/// runs under every one of its children — so on a narrow strip it is
/// two of the four characters you have spent saying something the eye
/// has already been told. Dropped, the strip says `Trig`, which is the
/// whole of what distinguishes it from its neighbour.
///
/// Matching is deliberately loose about how a folder writes its name.
/// `Tom 1` and `T1` are the same thing said long and short, so a name
/// is reduced to its letters and digits, and a folder also answers to
/// its initials with the digits kept: `Tom 1` gives `tom1` and `t1`,
/// and `T1 Trig`'s first word matches the second.
///
/// Returns `None` when nothing can safely come off — including when
/// every word would, because a strip labelled with nothing is worse
/// than one labelled redundantly.
fn shorten(name: &str, ancestors: &[&str]) -> Option<String> {
    fn letters(text: &str) -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    /// A folder's short form: the first letter of each word, with any
    /// digits kept — `Tom 1` -> `t1`, `Hi-Hat` -> `hh`.
    ///
    /// Split on anything that is not alphanumeric, not just spaces, so
    /// `Hi-Hat` is two words rather than one. And a word that is only
    /// digits contributes its digits and no initial — otherwise the `1`
    /// of `Tom 1` counts twice and the key comes out `t11`, matching
    /// nothing.
    fn initials(text: &str) -> String {
        let mut out = String::new();
        for word in text.split(|c: char| !c.is_alphanumeric()) {
            if let Some(first) = word.chars().next() {
                if first.is_alphabetic() {
                    out.extend(first.to_lowercase());
                }
            }
            out.extend(word.chars().filter(char::is_ascii_digit));
        }
        out
    }

    let keys: Vec<String> = ancestors
        .iter()
        .flat_map(|a| [letters(a), initials(a)])
        .filter(|k| !k.is_empty())
        .collect();

    // The longest leading run of words that matches a folder, not just
    // the first word: `Tom 1 Trig` has to lose both `Tom` and `1`, and
    // taking one word at a time stops at `Tom`, which matches nothing
    // on its own.
    let mut words: Vec<&str> = name.split_whitespace().collect();
    loop {
        let Some(take) = (1..words.len()).rev().find(|take| {
            let head = letters(&words[..*take].concat());
            !head.is_empty() && keys.iter().any(|k| *k == head)
        }) else {
            break;
        };
        words.drain(..take);
    }
    let short = words.join(" ");
    (short.len() < name.len()).then_some(short)
}

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
    /// The y every strip puts its button column at.
    ///
    /// Exposed because the LIVE controls have to land on the same line
    /// the recorded chrome was built around. Recomputing it in the
    /// overlay is how the two ended up disagreeing: the recorded strips
    /// share one line and a per-strip recomputation gives each its own.
    pub buttons_top: f64,
    /// How much of a strip the rack took.
    pub rack_h: f64,
    /// Each strip's own height.
    ///
    /// NOT the mixer's: nesting shortens a strip from the bottom, so a
    /// track three levels deep is shorter than the panel by three
    /// steps. `Collapse` resolves a strip's sections against the height
    /// it is DRAWN at, so handing the overlay the panel height instead
    /// of the strip's gave it different bands from the recorded chrome
    /// — which moved the arm, the buttons and the fader down by
    /// whatever the indent came to.
    heights: Vec<f64>,
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
        rack: &[crate::tone::Which],
    ) -> Self {
        let depth_seen = rows
            .iter()
            .map(|(_, depth)| usize::try_from(*depth).unwrap_or(0))
            .max()
            .map_or(0, |deepest| deepest.saturating_add(1));

        // The panel splits into the REAPER strip and the rack above it.
        //
        // `height` is the WHOLE panel, and the strip takes about a third
        // of it off the bottom — which is what REAPER's own mixer looks
        // like on a 1440p screen, and leaves the other two thirds for
        // the processing.
        //
        // This used to be the other way round: the strip was the height
        // and the rack was a 220-pixel band added on top. That reads as
        // a channel strip with a stripe of graphs stuck to it, when the
        // thing being built is a channel you MIX on — where the
        // processing is most of what you are looking at and the fader
        // is the part you reach for after deciding.
        //
        // A rack nobody is wide enough to draw is height nobody should
        // pay for. Two thirds of the panel reserved for graphs that
        // every strip is too narrow to show is what the Overview preset
        // looked like before this: a wall of nothing over a row of
        // faders pushed to the floor.
        //
        // Asked of the widths the rows CARRY rather than of the widths
        // that come back from `widths` below, because that one borrows
        // between strips to open a selection — and a selection wide
        // enough for a rack is a reason to have one, not a reason to
        // have already decided.
        let any_rack = rows
            .iter()
            .any(|(track, _)| layout.width_of(track.width) >= crate::tone::LEGIBLE);
        let tone = !rack.is_empty() && any_rack;
        let control = if tone {
            (height * CONTROL_SHARE).max(CONTROL_MIN).min(height)
        } else {
            height
        };
        let rack_h = height - control;

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

        // The tint of the folder open at each depth, so a strip can
        // draw the colours of everything it sits inside.
        let mut lineage: Vec<Color> = Vec::new();
        let mut lineage_names: Vec<&str> = Vec::new();

        let mut heights = Vec::with_capacity(rows.len());

        let mut x = 0.0_f64;
        for (ordinal, (track, depth)) in rows.iter().enumerate() {
            let depth = usize::try_from(*depth).unwrap_or(0);
            // Everything this track is inside, outermost first. A folder
            // closing simply means the next track is shallower, so the
            // truncate IS the close — no bookkeeping of ends needed.
            lineage.truncate(depth);
            lineage_names.truncate(depth);
            let ancestors = lineage.clone();
            let ancestor_names = lineage_names.clone();
            lineage.push(crate::tcp::folder_band(palette, track));
            lineage_names.push(track.name.as_str());
            let from = u32::try_from(strips.commands.len()).unwrap_or(u32::MAX);
            offsets.push(x);
            let w = widths.get(ordinal).copied().unwrap_or(layout.strip);
            // Nesting shortens the strip from the BOTTOM, so the tops
            // stay level and the bottoms staircase.
            let strip_h = (height - crate::num::coord(depth) * INDENT_STEP).max(1.0);
            heights.push(strip_h);
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
                    mixer_h: height,
                },
                ordinal,
                rack,
                &ancestors,
                &ancestor_names,
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
            buttons_top,
            rack_h,
            heights,
        }
    }

    /// How wide the whole mixer is, in content pixels.
    #[must_use]
    pub fn content_width(&self) -> f64 {
        self.offsets.last().copied().unwrap_or(0.0)
    }

    /// Where a strip is, and how big — for drawing over it.
    ///
    /// The overlay needs this because the strips are RECORDED: a
    /// hovered control cannot be re-recorded without re-recording the
    /// mixer, so it is redrawn live on top, and drawing on top of
    /// something means knowing where it is.
    ///
    /// Returns the strip's left edge in content space, its width, and
    /// its height — the same three numbers `build` handed `strip`.
    #[must_use]
    pub fn strip_box(&self, row: usize) -> Option<(f64, f64, f64)> {
        let left = *self.offsets.get(row)?;
        let right = *self.offsets.get(row.checked_add(1)?)?;
        let width = (right - left - STRIP_GAP).max(0.0);
        Some((left, width, *self.heights.get(row)?))
    }

    /// Which strip is at a content x, if any.
    ///
    /// The binary search `visible` uses, over the same offsets, for the
    /// same reason: the strip you click is the strip you see.
    #[must_use]
    pub fn strip_at(&self, content_x: f64) -> Option<usize> {
        if content_x < 0.0 || self.count == 0 {
            return None;
        }
        let strip = self
            .offsets
            .partition_point(|&x| x <= content_x)
            .checked_sub(1)?;
        (strip < self.count).then_some(strip)
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
    /// The mixer's own height, which the top sections resolve against.
    mixer_h: f64,
}

/// How much of the panel the REAPER strip keeps, with the rack on.
///
/// About a third, off the bottom — which is what REAPER's own mixer
/// looks like on a 1440p screen. The rack gets the other two thirds:
/// it is the reason to open this view, and a processor whose shape you
/// can see is the difference between mixing and guessing.
const CONTROL_SHARE: f64 = 0.34;

/// And the least the strip may be squeezed to.
///
/// Below this the fader has no travel and the buttons crowd, and a
/// channel strip you cannot mix on is not improved by the graphs above
/// it. A short panel gives the rack whatever is left over instead.
const CONTROL_MIN: f64 = 240.0;

/// A control on a strip.
///
/// What a click can land on, as opposed to what it can land IN — the
/// strip is `hit::Target::Track`, and this narrows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    Fx,
    Pan,
    RecArm,
    Mute,
    Solo,
    Routing,
    /// The fader, including its cap. Dragged, not clicked.
    Volume,
    /// The name plate. Double-clicked to rename.
    Name,
}

impl Control {
    /// Whether this control is set by DRAGGING rather than clicking.
    ///
    /// The distinction the gesture layer needs: a press on a knob is
    /// the start of a drag and must not act until the pointer moves or
    /// is released, where a press on a mute is a click waiting to
    /// happen.
    #[must_use]
    pub const fn is_continuous(self) -> bool {
        matches!(self, Self::Volume | Self::Pan)
    }
}

/// Which control is at a point inside a strip.
///
/// `x` and `y` are relative to the STRIP, not the window — the caller
/// has already worked out which strip, and passing window coordinates
/// here would mean this function needed the scroll too.
///
/// Laid out from the same `Columns` and the same `Collapse` the strip
/// was drawn from, so a control is hit where it is drawn. That is the
/// property worth having: these are two readings of one layout, not two
/// layouts that have to be kept in step.
#[must_use]
pub fn control_at(
    width: f64,
    height: f64,
    mixer_h: f64,
    rack_h: f64,
    buttons_top: f64,
    x: f64,
    y: f64,
) -> Option<Control> {
    crate::strip::Strip::new(width, height, mixer_h, rack_h, buttons_top).control_at(x, y)
}

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

fn strip(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    track: &Track,
    slot: Slot,
    index: usize,
    rack: &[crate::tone::Which],
    ancestors: &[Color],
    ancestor_names: &[&str],
) {
    let Slot {
        x,
        width: w,
        height: h,
        buttons_top,
        rack_h,
        ..
    } = slot;
    // `Collapse` is written in the f32 the theme's geometry is, and a
    // strip height is a few hundred pixels — exact either way.
    // Two resolutions, and the difference matters.
    //
    // Nesting shortens a strip from the BOTTOM, so everything anchored
    // to the top — the rack, the coloured band, the pan knob, the arm —
    // is the same on every strip and resolves against the mixer's own
    // height. Only what hangs below is shorter, which is the fader, and
    // that is the cost of the indent rather than a second inconsistency.
    //
    // Resolving the bands per strip instead put the arm of a track
    // three levels deep several pixels above the arm beside it, and a
    // control that moves because of something about ITS track cannot be
    // scanned across tracks.
    let shape = Collapse::at(f64_to_f32((slot.mixer_h - rack_h).max(1.0)));
    let own = Collapse::at(f64_to_f32((h - rack_h).max(1.0)));

    // The strip's ground, and the track's colour as a band across it.
    fill(scene, palette.tcp_tint, Rect::new(x, 0.0, x + w, h));

    // ── The folders this strip sits inside ──
    //
    // Nesting shortens a strip from the bottom, which leaves a
    // staircase of empty space under it — one step per level. Those
    // steps are exactly the shape of the tree, so they carry the
    // colours of the tree: the step immediately under a strip is its
    // parent, the step under that its grandparent, and the bottom step
    // is the outermost folder.
    //
    // Every child of a folder puts that folder's colour at the same y,
    // so the steps join up across the children into one unbroken band —
    // the folder's strip CONTINUING along the bottom of everything it
    // contains, which is how you see what a track is part of without
    // counting indents back to the left.
    //
    // Bare, the staircase said only "this one is deeper than that one".
    // The strip was shortened by one step per level, so the mixer's
    // full height is this strip's plus the steps under it. `ancestors`
    // has exactly one entry per level, which is what makes that true.
    let full = h + crate::num::coord(ancestors.len()) * INDENT_STEP;
    for (level, tint) in ancestors.iter().enumerate() {
        // Level 0 is the OUTERMOST folder, so it sits on the floor; the
        // last is the immediate parent and sits right under the strip.
        //
        // Which is not just an ordering preference — it is what makes
        // the band continuous. A folder's own strip ends one step above
        // its children's, so the band carrying its colour has to run
        // from the child's bottom down to its own, and that is
        // `level + 1` steps off the floor.
        let from_floor = crate::num::coord(level + 1);
        let top = full - from_floor * INDENT_STEP;
        fill(scene, *tint, Rect::new(x, top, x + w, top + INDENT_STEP));
    }

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
    let stretch_h = f64::from(own.stretch);
    let squeeze = Squeeze::at(w);

    // ── The FX section ──
    //
    // Below the rack, at the top of the REAPER strip — not at the top of
    // the whole panel. It is the button that OPENS the chain, so it
    // belongs with the controls rather than floating above the graphs
    // it would open: sitting on top of the embedded FX it read as a
    // label for them, which is the one thing it is not.
    //
    // The pill itself is NOT recorded: whether a track has a chain
    // changes while the window is open (`FxCountChanged`), and a pill
    // baked in at Empty was a button that told the truth once. It is
    // drawn in the live pass — see `overlay::controls` — which is also
    // where it can answer the pointer.

    // ── The Tone rack ──
    //
    // The whole top of the panel, down to where the REAPER strip
    // starts. It used to begin one FX-section below that, leaving a
    // band of empty strip above it — room reserved for the FX pill back
    // when the pill sat up here.
    if rack_h > 0.0 {
        crate::tone::record(
            scene,
            palette,
            font,
            &crate::tone::placeholder(index),
            rack,
            crate::tone::Panel {
                x: x + 2.0,
                y: 2.0,
                width: (w - 4.0).max(0.0),
                height: (rack_h - 4.0).max(0.0),
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
            mixer_h: slot.mixer_h,
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

    bottom(scene, palette, font, track, x, w, h, ancestor_names);
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
    // The pan knob is NOT recorded — it is drawn live, from the value
    // the track has now. See `overlay::controls`.

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

    // The record arm is not recorded either: it is lit or not, and that
    // changes on a click. `overlay::controls` draws it, against the
    // same shared button line this strip was built around.
    let _ = pan_top;
}

/// Where a strip's columns fall, for a strip of this width.
///
/// REAPER has one strip width, so its geometry is a list of fixed
/// offsets. Ours are not one width — a piece is wider than a mic, and a
/// selected strip is wider again — and fixed offsets meant every extra
/// pixel piled up as dead space on the right: the fader stayed a third
/// of the way across, the arm sat where an 86-wide strip puts it, and
/// the meter never grew.
///
/// So the offsets become rules, chosen so that **a strip of exactly
/// [`STRIP_W`] reproduces REAPER's own layout**. That is the constraint
/// that keeps this honest: the measured numbers are still the numbers,
/// they are just expressed as where-they-come-from rather than as
/// where-they-landed.
///
/// | | rule | at 86 | REAPER |
/// |---|---|---|---|
/// | button column | right-aligned, REAPER's margin | 55 | 55 |
/// | fader | centred | 32 | 31 |
/// | scale | left, fixed width | 2 | ~4 |
/// | meter | fills what is left between them | ~0 | not drawn |
#[derive(Clone, Copy, Debug)]
pub struct Columns {
    pub scale_x: f64,
    pub scale_w: f64,
    pub meter_x: f64,
    pub meter_w: f64,
    pub fader_x: f64,
    pub fader_w: f64,
    /// The left edge of the button column.
    pub column_x: f64,
    /// Its centre — what the record arm hangs off.
    pub column_axis: f64,
}

/// The fader's own width. Fixed: a fader is a fader, and a wider strip
/// wants a longer scale and a bigger meter, not a fatter handle.
const FADER_W: f64 = 22.0;

/// The widest the meter grows to — the track panel's own measured
/// meter, so the two views agree once there is room for both.
const METER_MAX: f64 = 26.0;
const _: () = assert!(g::METER_W == 26, "METER_MAX must track the measured meter");

/// How much room the scale's numbers need. Fixed, because `-54-` is
/// `-54-` at any strip width.
const SCALE_W: f64 = 24.0;

impl Columns {
    pub fn at(x: f64, w: f64) -> Self {
        // Right-aligned by REAPER's own right margin, so the buttons
        // keep their distance from the edge instead of their distance
        // from the left.
        let margin = f64::from(g::STRIP_W - g::COLUMN);
        let column_x = if Squeeze::at(w).columns() {
            (x + w - margin).max(x)
        } else {
            x + (w - f64::from(g::BUTTON_W)).max(0.0) / 2.0
        };
        let fader_x = x + (w - FADER_W) / 2.0;
        let scale_x = x + 2.0;
        // Whatever is left between the numbers and the fader. At 86 that
        // is almost nothing, which is why REAPER draws no meter there;
        // on a piece strip it is a real meter.
        let meter_w = (fader_x - (scale_x + SCALE_W) - 4.0).clamp(0.0, METER_MAX);
        Self {
            scale_x,
            scale_w: SCALE_W,
            meter_x: fader_x - 3.0 - meter_w,
            meter_w,
            fader_x,
            fader_w: FADER_W,
            column_x,
            column_axis: column_x + f64::from(g::BUTTON_W) / 2.0,
        }
    }

    /// Whether there is enough width for a meter worth drawing.
    ///
    /// A two-pixel meter is not a small meter, it is a line — and a line
    /// beside a fader reads as part of the fader.
    pub const fn has_meter(self) -> bool {
        self.meter_w >= 4.0
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
    let columns = Columns::at(x, w);

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
    if squeeze.meter() {
        crate::art::place(
            scene,
            &art::fader_scale(
                columns.scale_w,
                stretch,
                crate::tcp::to_theme(palette.meter_warn),
                8.0,
            ),
            font,
            columns.scale_x,
            stretch_top,
        );
    }
    // The meter takes whatever the scale and the fader leave, which on
    // an 86-wide strip is nothing — REAPER draws none there either —
    // and on a piece strip is a meter you can actually read.
    if squeeze.meter() && columns.has_meter() {
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
                columns.meter_w,
                stretch,
            ),
            font,
            columns.meter_x,
            stretch_top,
        );
    }
    // The volume control is drawn live — its cap and its lit travel
    // both move with the value, so recording it would record a fader
    // frozen at whatever the project opened with.

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
    // Right-aligned rather than at a fixed offset — see `Columns`. On
    // an 86-wide strip this IS REAPER's 55; on a wider one the buttons
    // keep their distance from the edge instead of stranding the extra
    // width to their right.
    let column = Columns::at(x, w).column_x;
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
    // Mute and solo are live: they are lit or not, and that is the
    // most common thing to change in a mixer.
    let mut at = stretch_top
        + f64::from(g::RECMON_FROM_ARM)
        + (f64::from(g::BUTTON_H) + 1.0) * 2.0;

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
    ancestor_names: &[&str],
) {
    let bottom = h - f64::from(daw_theme_art::collapse::BOTTOM_SECTION);
    let plate = f64::from(g::NAME_PLATE);
    fill(
        scene,
        palette.tcp_field,
        Rect::new(x + 2.0, bottom, x + w - 2.0, bottom + plate),
    );
    let ink = if track.selected { palette.text } else { palette.text_dim };
    // Three ways to make a name fit, in order of what they cost.
    //
    // Print it, shrink it, then drop what the folders already said —
    // and only cut as a last resort. Dropping comes AFTER shrinking
    // because the full name at nine points tells you more than half of
    // it at eleven; it comes before cutting because `Trig` is a word
    // and `T1 …` is not.
    let room = w - 8.0;
    let (mut label, mut label_size) = font.fit(&track.name, 11.0, 7.0, room);
    if label.contains('…') {
        if let Some(short) = shorten(&track.name, ancestor_names) {
            let (short_label, short_size) = font.fit(&short, 11.0, 7.0, room);
            if !short_label.contains('…') {
                label = short_label;
                label_size = short_size;
            }
        }
    }
    crate::tcp::glyphs(
        scene,
        font,
        ink,
        &label,
        x + 4.0,
        bottom + plate / 2.0 + f64::from(label_size) / 3.0,
        label_size,
    );
    // The number sits on the track's own colour, in a band exactly one
    // indent step tall.
    //
    // That height is the whole point and not a detail. A folder's strip
    // ends one step above its children's, and its children each draw a
    // step of its colour under themselves — so if the folder's own
    // colour band is a different height, its line sits at a different
    // level from the line continuing across everything it contains, and
    // the two read as unrelated marks that happen to share a hue.
    //
    // At one step they are the same line. A red Drum Kit puts red under
    // its number, and that red runs unbroken along the floor beneath
    // every track inside it.
    let number_h = INDENT_STEP;
    let number_top = h - number_h;
    if number_top > bottom + plate - number_h {
        fill(
            scene,
            crate::tcp::folder_band(palette, track),
            Rect::new(x, number_top, x + w, h),
        );
    }
    crate::tcp::glyphs(
        scene,
        font,
        // Against a colour now rather than the panel, so the faintest
        // ink in the palette no longer reads — a number you cannot make
        // out is the same as no number.
        palette.text_dim,
        &track.index.saturating_add(1).to_string(),
        x + 5.0,
        h - 3.0,
        9.0,
    );
}

/// A strip height, for the f32 the theme's geometry is written in.
pub const fn f64_to_f32(value: f64) -> f32 {
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

#[cfg(test)]
mod column_tests {
    use super::{Columns, STRIP_W};
    use daw_theme_art::geometry::mcp as g;

    /// The constraint the whole model rests on: expressing the measured
    /// offsets as RULES must not change what they evaluate to at the
    /// width they were measured at. If this drifts, every number in
    /// `geometry::mcp` has quietly stopped describing what is drawn.
    #[test]
    fn an_86_wide_strip_is_still_reapers_layout() {
        let c = Columns::at(0.0, STRIP_W);
        assert!(
            (c.column_x - f64::from(g::COLUMN)).abs() < 0.01,
            "button column drifted: {} vs REAPER's {}",
            c.column_x,
            g::COLUMN
        );
        assert!(
            (c.column_axis - f64::from(g::COLUMN_AXIS)).abs() < 0.01,
            "column axis drifted: {} vs {}",
            c.column_axis,
            g::COLUMN_AXIS
        );
        let arm = c.column_axis - f64::from(g::ARM_CELL_W) * 0.486;
        assert!(
            (arm - f64::from(g::ARM_LEFT)).abs() < 0.01,
            "record arm drifted: {arm} vs REAPER's {}",
            g::ARM_LEFT
        );
        // REAPER's groove sits at 31; centring puts the cell at 32.
        assert!((c.fader_x - 32.0).abs() < 0.01, "fader at {}", c.fader_x);
        // And REAPER draws no meter at this width, so neither do we.
        assert!(!c.has_meter(), "a meter appeared at REAPER's own width");
    }

    /// The whole point of the change: width goes somewhere useful.
    #[test]
    fn a_wider_strip_spends_the_width() {
        let narrow = Columns::at(0.0, STRIP_W);
        let wide = Columns::at(0.0, 195.0);

        assert!(
            wide.fader_x > narrow.fader_x,
            "the fader should move to the middle, not stay left"
        );
        assert!(
            (wide.fader_x + wide.fader_w / 2.0 - 195.0 / 2.0).abs() < 0.01,
            "the fader should be centred"
        );
        assert!(
            wide.column_axis > narrow.column_axis,
            "the buttons and the arm should travel right with the edge"
        );
        assert!(
            wide.meter_w > narrow.meter_w && wide.has_meter(),
            "the meter should grow: {} -> {}",
            narrow.meter_w,
            wide.meter_w
        );
    }

    /// The right margin is REAPER's, at every width — that is what
    /// "right-aligned" has to mean for the column to look placed rather
    /// than pushed.
    #[test]
    fn the_right_margin_is_constant() {
        let margin = f64::from(STRIP_W - f64::from(g::COLUMN));
        for w in [STRIP_W, 96.0, 130.0, 195.0, 300.0] {
            let c = Columns::at(0.0, w);
            assert!(
                (w - c.column_x - margin).abs() < 0.01,
                "margin at width {w}: {}",
                w - c.column_x
            );
        }
    }

    /// And the meter is capped, so a very wide strip does not turn its
    /// meter into a second fader.
    #[test]
    fn the_meter_stops_growing() {
        assert!(Columns::at(0.0, 600.0).meter_w <= super::METER_MAX);
    }

    /// Nothing escapes the strip it belongs to.
    #[test]
    fn every_column_stays_inside_the_strip() {
        for w in [30.0, 56.0, STRIP_W, 96.0, 130.0, 195.0] {
            let c = Columns::at(10.0, w);
            let right = 10.0 + w;
            for (name, edge) in [
                ("scale", c.scale_x + c.scale_w),
                ("meter", c.meter_x + c.meter_w),
                ("fader", c.fader_x + c.fader_w),
                ("column", c.column_x + f64::from(g::BUTTON_W)),
            ] {
                assert!(edge <= right + 0.01, "{name} runs past the strip at width {w}: {edge} > {right}");
            }
            assert!(c.scale_x >= 10.0 && c.fader_x >= 10.0 && c.column_x >= 10.0);
        }
    }
}

#[cfg(test)]
mod folder_band_tests {
    use super::INDENT_STEP;

    /// A folder's own colour band and the band its children draw for it
    /// must occupy the same pixels, or the "line" is two lines.
    ///
    /// The folder's band is the last [`INDENT_STEP`] of ITS strip; the
    /// child draws the folder's step `level + 1` off the floor. This is
    /// the arithmetic that makes those the same range — get it wrong by
    /// the difference between the band height and the indent step and
    /// the stripes look almost right, which is worse than obviously
    /// wrong because nobody reports it.
    #[test]
    fn a_folders_band_lines_up_with_its_childrens() {
        const FULL: f64 = 460.0;
        for folder_depth in 0..4_usize {
            // The folder's own strip, and its number band at the foot.
            let folder_h = FULL - folder_depth as f64 * INDENT_STEP;
            let folder_band = (folder_h - INDENT_STEP, folder_h);

            // A child one level deeper: the folder is the LAST of its
            // ancestors, so `level` is the folder's own depth.
            let child_depth = folder_depth + 1;
            let child_h = FULL - child_depth as f64 * INDENT_STEP;
            let ancestors = child_depth; // one entry per level above it
            let full_from_child = child_h + ancestors as f64 * INDENT_STEP;
            let from_floor = (folder_depth + 1) as f64;
            let top = full_from_child - from_floor * INDENT_STEP;
            let child_draws = (top, top + INDENT_STEP);

            assert!(
                (folder_band.0 - child_draws.0).abs() < 1e-9
                    && (folder_band.1 - child_draws.1).abs() < 1e-9,
                "depth {folder_depth}: folder's band {folder_band:?} but children draw it at {child_draws:?}"
            );
        }
    }

    /// And a grandchild puts it in the same place as a child does — the
    /// line is continuous across every level below the folder, not just
    /// the one immediately under it.
    #[test]
    fn the_line_survives_deeper_nesting() {
        const FULL: f64 = 460.0;
        let folder_depth = 1_usize;
        let expected = FULL - (folder_depth + 1) as f64 * INDENT_STEP;
        for descendant_depth in (folder_depth + 1)..5 {
            let h = FULL - descendant_depth as f64 * INDENT_STEP;
            let full = h + descendant_depth as f64 * INDENT_STEP;
            let top = full - (folder_depth + 1) as f64 * INDENT_STEP;
            assert!(
                (top - expected).abs() < 1e-9,
                "a descendant at depth {descendant_depth} drew the folder's line at {top}, not {expected}"
            );
        }
    }
}

#[cfg(test)]
mod shorten_tests {
    use super::shorten;

    /// The case that prompted it: a trigger inside the tom it triggers.
    #[test]
    fn a_folder_does_not_need_saying_twice() {
        assert_eq!(
            shorten("T1 Trig", &["Drum Kit", "Toms", "Tom 1"]).as_deref(),
            Some("Trig")
        );
        assert_eq!(
            shorten("T4 Trig", &["Drum Kit", "Toms", "Tom 4"]).as_deref(),
            Some("Trig")
        );
    }

    /// Long and short forms of a folder's name are the same folder —
    /// `Tom 1` has to answer to `T1` or the rule never fires, since the
    /// template writes the folder long and the tracks short.
    #[test]
    fn a_folder_answers_to_its_initials() {
        assert_eq!(shorten("HH Bleed", &["Hi-Hat"]).as_deref(), Some("Bleed"));
        assert_eq!(shorten("Tom 1 Trig", &["Tom 1"]).as_deref(), Some("Trig"));
    }

    /// A name that says nothing its folders said keeps every word.
    #[test]
    fn an_unrelated_name_is_left_alone() {
        assert_eq!(shorten("Stereo L", &["Drum Kit", "Rooms"]), None);
        assert_eq!(shorten("Bottom", &["Snare", "Sum"]), None);
        assert_eq!(shorten("Trig", &["Tom 1"]), None);
    }

    /// And a strip is never left with no label at all — the last word
    /// stays even when the folder said it.
    #[test]
    fn it_never_strips_the_whole_name() {
        assert_eq!(shorten("T1", &["Tom 1"]), None);
        assert_eq!(shorten("Tom 1", &["Tom 1"]), None);
        for short in [shorten("Sum", &["Kick"]), shorten("Kick", &["Kick"])] {
            assert!(short.as_deref().is_none_or(|s| !s.is_empty()));
        }
    }

    /// It only ever gets shorter — a caller that swaps in the result
    /// must never end up with MORE to fit than it started with.
    #[test]
    fn the_result_is_always_shorter() {
        for (name, ancestors) in [
            ("T1 Trig", &["Tom 1"][..]),
            ("Stereo L", &["Rooms"][..]),
            ("Kick In", &["Kick"][..]),
            ("", &["Kick"][..]),
        ] {
            if let Some(short) = shorten(name, ancestors) {
                assert!(short.len() < name.len(), "{name:?} -> {short:?}");
            }
        }
    }
}

