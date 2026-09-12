//! The toolbars down the sides and across the top.
//!
//! Three rails frame a view: one on the left, one on the right, one
//! across the top. They are the same three in the arrangement and in
//! the mixer, because a window that moved its toolbars when you pressed
//! `x` would be two applications wearing one title bar.
//!
//! What goes IN them differs, and that is the point: the left rail says
//! where you are, the right says how this view behaves, the top carries
//! the things that act on the session as a whole. In the mixer the left
//! rail is the mix phases — Rescue through Overview — so the panel
//! always answers "which pass is this" without being asked.
//!
//! # Why the rails own the frame
//!
//! The panel inside them is recorded once and replayed under a
//! transform. The rails are not: they are a handful of shapes that
//! change when a setting changes, which is orders of magnitude rarer
//! than a frame and orders of magnitude cheaper than a strip. So they
//! are drawn live, over the panel, and the panel is told to cull to the
//! space between them — [`Frame::content`].

use anyrender::PaintScene;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

use crate::arrangement::Palette;
use crate::text::Font;

/// How wide the side rails are.
///
/// Enough for a 22-pixel icon with a margin either side, which is the
/// size REAPER's own toolbar buttons are and the size the ported art is
/// authored at. A rail narrower than its buttons is a rail you miss.
pub const SIDE: f64 = 44.0;

/// And how tall the top one is.
pub const TOP: f64 = 30.0;

/// One button in a rail.
#[derive(Clone, Copy, Debug)]
pub struct Item<'a> {
    /// What it says. Rails are narrow, so this is a word, not a phrase.
    pub label: &'a str,
    /// Whether it is the current one (a phase) or switched on (a
    /// setting). The rails draw both the same way on purpose: "where
    /// you are" and "what is on" are the same question asked of
    /// different things.
    pub on: bool,
}

/// Where a view's rails are, and what is left for its panel.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub width: f64,
    pub height: f64,
}

impl Frame {
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    /// The box the panel gets, inside the rails.
    ///
    /// Returned rather than assumed, because the panel culls to it: a
    /// mixer handed the whole window draws strips underneath the right
    /// rail and pays for every one of them.
    #[must_use]
    pub fn content(self) -> Rect {
        Rect::new(
            SIDE,
            TOP,
            (self.width - SIDE).max(SIDE),
            self.height.max(TOP),
        )
    }

    /// Where the `index`th item in the top rail sits.
    ///
    /// Laid out from the left, after the left rail, so the first mode
    /// starts where the panel does rather than over the rail beside it.
    #[must_use]
    pub fn top_slot(self, index: usize) -> Option<Rect> {
        let left = SIDE + 4.0 + crate::num::coord(index) * TOP_ITEM_W;
        if left + TOP_ITEM_W > self.width - SIDE {
            return None;
        }
        Some(Rect::new(left, 3.0, left + TOP_ITEM_W - 3.0, TOP - 3.0))
    }

    /// The width the panel has.
    #[must_use]
    pub fn content_width(self) -> f64 {
        (self.width - SIDE * 2.0).max(0.0)
    }

    /// The height the panel has.
    #[must_use]
    pub fn content_height(self) -> f64 {
        (self.height - TOP).max(0.0)
    }

    /// Where the `index`th item in a side rail sits.
    ///
    /// Stacked from the top, one [`ITEM_H`] each. Returns `None` past
    /// the bottom of the rail rather than drawing off the end — a rail
    /// with more items than height is a real state (a short window, a
    /// long phase list) and it has to degrade by showing fewer, not by
    /// painting into the panel.
    #[must_use]
    pub fn slot(self, index: usize, right: bool) -> Option<Rect> {
        let top = TOP + 4.0 + crate::num::coord(index) * ITEM_H;
        if top + ITEM_H > self.height {
            return None;
        }
        let x = if right { self.width - SIDE } else { 0.0 };
        Some(Rect::new(x + 3.0, top, x + SIDE - 3.0, top + ITEM_H - 3.0))
    }
}

/// The height of one rail button, including the gap under it.
pub const ITEM_H: f64 = 40.0;

/// The width of one button in the TOP rail.
///
/// Wider than a side rail's because it holds words rather than icons —
/// the modes are named things and "Organize" abbreviates badly.
pub const TOP_ITEM_W: f64 = 74.0;

/// Draw the three rails and their contents.
///
/// `left` and `right` are the buttons; the top rail is drawn empty for
/// now, because what belongs in it is the transport and the mode
/// switcher and neither exists yet. It is here rather than left out so
/// the panel is laid out against its real frame from the start — adding
/// it later would move every other number.
pub fn draw(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    frame: Frame,
    left: &[Item<'_>],
    right: &[Item<'_>],
    top: &[Item<'_>],
) {
    let Frame { width, height } = frame;

    // The rails' ground, drawn over the panel rather than under it: the
    // panel is a recorded scene that does not know where the rails are,
    // so the frame has to be painted last to cover what scrolled under
    // it.
    fill(painter, palette.tcp_gutter, Rect::new(0.0, 0.0, width, TOP));
    fill(painter, palette.tcp_gutter, Rect::new(0.0, 0.0, SIDE, height));
    fill(
        painter,
        palette.tcp_gutter,
        Rect::new(width - SIDE, 0.0, width, height),
    );
    // A rule on each inner edge, so a rail reads as a frame around the
    // panel rather than as more panel.
    fill(painter, palette.tcp_rule, Rect::new(0.0, TOP, width, TOP + 1.0));
    fill(painter, palette.tcp_rule, Rect::new(SIDE, TOP, SIDE + 1.0, height));
    fill(
        painter,
        palette.tcp_rule,
        Rect::new(width - SIDE - 1.0, TOP, width - SIDE, height),
    );

    for (i, item) in top.iter().enumerate() {
        let Some(slot) = frame.top_slot(i) else { break };
        button(painter, palette, font, slot, *item);
    }

    for (side, items) in [(false, left), (true, right)] {
        for (i, item) in items.iter().enumerate() {
            let Some(slot) = frame.slot(i, side) else {
                break;
            };
            button(painter, palette, font, slot, *item);
        }
    }
}

/// One rail button: a plate, and its label wrapped to the rail.
fn button(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    slot: Rect,
    item: Item<'_>,
) {
    fill(
        painter,
        if item.on {
            palette.accent
        } else {
            palette.tcp_button
        },
        slot,
    );
    // The label is the fallback until the toolbar icons are ported:
    // `MixPhase::icon` names the file REAPER ships for each phase, and
    // a rail of words is legible where a rail of nothing is not.
    //
    // Short enough for the rail or it is not drawn at all — a clipped
    // word in a 38-pixel button is a smear, and the plate's lit state
    // already says which one is current.
    let room = slot.width() - 4.0;
    let (label, size) = font.fit(item.label, 10.0, 6.0, room);
    if label.contains('…') {
        return;
    }
    let ink = if item.on {
        crate::tcp::ink_on(palette.accent)
    } else {
        palette.text_dim
    };
    crate::tcp::glyphs(
        painter,
        font,
        ink,
        &label,
        slot.x0 + (slot.width() - font.width(&label, size)) / 2.0,
        slot.y0 + slot.height() / 2.0 + f64::from(size) / 3.0,
        size,
    );
}

/// What the mixer's rails hold.
///
/// The left rail answers "where am I": which visual preset is showing
/// and which mix phase it belongs to. The right answers "how does this
/// behave". Both are lists of one-word buttons until the toolbar icons
/// `MixPhase::icon` names are ported.
#[must_use]
pub fn mixer_left(preset: &str) -> Vec<Item<'static>> {
    let mut items: Vec<Item<'static>> = PRESETS
        .iter()
        .map(|name| Item {
            label: name,
            on: *name == preset,
        })
        .collect();
    items.extend(
        session::mix_phases::MixPhase::ALL
            .iter()
            .map(|phase| Item {
                label: phase.display_name(),
                // Tone is the phase this panel was built for; the rest
                // light up when their rules exist.
                on: matches!(phase, session::mix_phases::MixPhase::Tone),
            }),
    );
    items
}

/// The visual presets, until they come from `dynamic_template`'s
/// `ModeVisibility` — see the note in `mcp`.
pub const PRESETS: [&str; 3] = ["Mix", "Rec", "Over"];

/// The DAW modes, for the arrangement's top rail.
///
/// The mode is the biggest thing about the window — it decides what the
/// toolbars hold and which tracks are worth showing — so it goes across
/// the top, where a thing that governs everything below it belongs.
#[must_use]
pub fn modes(current: session::modes::Mode) -> Vec<Item<'static>> {
    session::modes::Mode::ALL
        .iter()
        .map(|mode| Item {
            label: mode.display_name(),
            on: *mode == current,
        })
        .collect()
}

/// The right rail's switches, showing their state.
#[must_use]
pub fn mixer_right(settings: crate::settings::Settings) -> Vec<Item<'static>> {
    vec![
        Item {
            label: "Focus",
            on: settings.focus_selected,
        },
        Item {
            label: "Steal",
            on: settings.take_focus_width,
        },
    ]
}

fn fill(painter: &mut impl PaintScene, color: Color, rect: Rect) {
    painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
}

#[cfg(test)]
mod tests {
    use super::{Frame, ITEM_H, SIDE, TOP};

    /// The panel is told what it has, and it is what is left.
    #[test]
    fn the_content_box_excludes_the_rails() {
        let frame = Frame::new(2560.0, 1440.0);
        let content = frame.content();
        assert!((content.x0 - SIDE).abs() < f64::EPSILON);
        assert!((content.x1 - (2560.0 - SIDE)).abs() < f64::EPSILON);
        assert!((content.y0 - TOP).abs() < f64::EPSILON);
        assert!((frame.content_width() - (2560.0 - SIDE * 2.0)).abs() < f64::EPSILON);
        assert!((frame.content_height() - (1440.0 - TOP)).abs() < f64::EPSILON);
    }

    /// Slots stack without overlapping, and stop at the bottom rather
    /// than painting into the panel.
    #[test]
    fn slots_stack_and_then_stop() {
        let frame = Frame::new(2560.0, 1440.0);
        let a = frame.slot(0, false).expect("first slot");
        let b = frame.slot(1, false).expect("second slot");
        assert!(b.y0 >= a.y1, "slots overlap: {a:?} then {b:?}");
        assert!((b.y0 - a.y0 - ITEM_H).abs() < f64::EPSILON);

        // A window too short for the eight phases shows what fits.
        let short = Frame::new(2560.0, 120.0);
        assert!(short.slot(0, false).is_some());
        assert!(short.slot(7, false).is_none());
    }

    /// The right rail is on the right.
    #[test]
    fn the_right_rail_is_against_the_right_edge() {
        let frame = Frame::new(2560.0, 1440.0);
        let right = frame.slot(0, true).expect("a right slot");
        assert!(right.x0 >= 2560.0 - SIDE);
        assert!(right.x1 <= 2560.0);
    }

    /// A window narrower than its own rails must not produce a negative
    /// panel — it produces none.
    #[test]
    fn a_tiny_window_has_no_content_rather_than_negative_content() {
        let frame = Frame::new(40.0, 20.0);
        assert!(frame.content_width().abs() < f64::EPSILON);
        assert!(frame.content_height().abs() < f64::EPSILON);
        assert!(frame.content().x1 >= frame.content().x0);
    }
}
