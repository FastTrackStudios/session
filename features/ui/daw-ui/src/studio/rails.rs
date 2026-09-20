//! The frame around the panel: three rails of buttons.
//!
//! The top rail says which mode you are in, the left which scene and
//! mix phase, the right how things behave. All three are the same thing
//! — a plate that is lit or not, with a word on it — so they are one
//! component and three lists.
//!
//! # Why this one is components before it is anything else
//!
//! The lanes and the ruler were converted for the renderer's sake. The
//! rails are converted for the button's. A rail button is the only part
//! of this window that is genuinely INTERACTIVE — it hovers, it presses,
//! it is the thing a keyboard tabs to and a screen reader announces —
//! and every one of those was hand-drawn state in the painted version: a
//! hover was a colour computed at paint time from a pointer position the
//! renderer had to be told about.
//!
//! As components it is a `button` with `:hover`, focus and a name. The
//! accessibility comes from the element rather than from anything
//! written here, which is the whole argument for the migration stated in
//! one control.
//!
//! # Flat, like everything that repeats
//!
//! A rail holds a dozen plates and they are on screen the whole time, so
//! they follow the same rule the lanes do: flat fills, square corners,
//! one element. Measured, a gradient and a radius on a control that
//! repeats costs more in paint than the vector art it replaced.

use std::sync::Arc;

use crate::prelude::*;

use super::lanes::{Colors, FONT};

/// How wide the side rails are.
pub const SIDE: f64 = 44.0;

/// How tall the top one is.
pub const TOP: f64 = 30.0;

/// The height of one side-rail button, including the gap under it.
pub const ITEM_H: f64 = 40.0;

/// The width of one button in the top rail.
///
/// Wider than a side rail's because it holds words rather than icons —
/// the modes are named things and "Organize" abbreviates badly.
pub const TOP_ITEM_W: f64 = 74.0;

/// The size a rail label is written at when it fits.
pub const LABEL_SIZE: f64 = 10.0;

/// One button on a rail.
#[derive(Clone, PartialEq)]
pub struct Item {
    /// What it says. Rails are narrow, so this is a word, not a phrase.
    pub label: String,
    /// Whether it is the current one (a phase) or switched on (a
    /// setting). Both are drawn the same way on purpose: "where you
    /// are" and "what is on" are the same question asked of different
    /// things.
    pub on: bool,
    /// How big to write it, when the host has already worked out that
    /// the word needs shrinking to fit the plate.
    ///
    /// `None` is the ordinary case and means [`LABEL_SIZE`]. The fitting
    /// is the host's because measuring a string in a face is the host's:
    /// a component that guessed would be guessing differently on every
    /// renderer, which is the one thing this migration exists to stop.
    pub size: Option<f64>,
}

impl Item {
    /// A plate with a word on it.
    #[must_use]
    pub fn new(label: impl Into<String>, on: bool) -> Self {
        Self {
            label: label.into(),
            on,
            size: None,
        }
    }
}

/// Which rail a button is on, which is what decides where it sits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Left,
    Right,
}

/// The three rails, and the frame they draw around everything.
///
/// The rails are drawn OVER the panel rather than under it: the panel
/// scrolls, and a frame that scrolled with it would let the session
/// slide out from under the window's own edge.
#[component]
pub fn Rails(
    width: f64,
    height: f64,
    colors: Colors,
    top: Arc<[Item]>,
    left: Arc<[Item]>,
    right: Arc<[Item]>,
    /// What a press does. The rail does not know what its buttons mean —
    /// it hands back which one, and on which rail.
    #[props(default)]
    on_press: EventHandler<(Side, usize)>,
) -> Element {
    let ground = colors.tcp_gutter.clone();
    let rule = colors.rule.clone();

    rsx! {
        div {
            style: "position:absolute; left:0; top:0; width:{width}px; height:{height}px; \
                    pointer-events:none; font-family:{FONT};",
            "data-testid": "studio-rails",

            // The three grounds, and a rule on each inner edge so a rail
            // reads as a frame around the panel rather than as more
            // panel.
            Ground { left: 0.0, top: 0.0, width, height: TOP, fill: ground.clone() }
            Ground { left: 0.0, top: 0.0, width: SIDE, height, fill: ground.clone() }
            Ground { left: width - SIDE, top: 0.0, width: SIDE, height, fill: ground.clone() }
            Ground { left: 0.0, top: TOP, width, height: 1.0, fill: rule.clone() }
            Ground { left: SIDE, top: TOP, width: 1.0, height, fill: rule.clone() }
            Ground {
                left: width - SIDE - 1.0,
                top: TOP,
                width: 1.0,
                height,
                fill: rule.clone(),
            }

            for (index, item) in top.iter().enumerate() {
                if let Some(slot) = top_slot(index, width) {
                    Plate {
                        key: "top-{index}",
                        slot,
                        item: item.clone(),
                        colors: colors.clone(),
                        onpress: move |()| on_press.call((Side::Top, index)),
                    }
                }
            }
            for (index, item) in left.iter().enumerate() {
                if let Some(slot) = side_slot(index, height, false, width) {
                    Plate {
                        key: "left-{index}",
                        slot,
                        item: item.clone(),
                        colors: colors.clone(),
                        onpress: move |()| on_press.call((Side::Left, index)),
                    }
                }
            }
            for (index, item) in right.iter().enumerate() {
                if let Some(slot) = side_slot(index, height, true, width) {
                    Plate {
                        key: "right-{index}",
                        slot,
                        item: item.clone(),
                        colors: colors.clone(),
                        onpress: move |()| on_press.call((Side::Right, index)),
                    }
                }
            }
        }
    }
}

/// A rectangle of one colour: a rail's ground, or a rule on its edge.
#[component]
fn Ground(left: f64, top: f64, width: f64, height: f64, fill: String) -> Element {
    rsx! {
        div {
            style: "position:absolute; left:{left}px; top:{top}px; width:{width}px; \
                    height:{height}px; background:{fill};",
        }
    }
}

/// Where a button sits: left, top, width, height.
pub type Slot = (f64, f64, f64, f64);

/// Where the `index`th button in the top rail sits, if it fits.
#[must_use]
pub fn top_slot(index: usize, width: f64) -> Option<Slot> {
    let left = TOP_ITEM_W.mul_add(count(index), SIDE + 4.0);
    if left + TOP_ITEM_W > width - SIDE {
        return None;
    }
    Some((left, 3.0, TOP_ITEM_W - 3.0, TOP - 6.0))
}

/// And in a side rail.
///
/// `None` past the bottom rather than drawing off the end: a rail with
/// more items than height is a real state — a short window, a long phase
/// list — and it has to degrade by showing fewer, not by painting into
/// the panel.
#[must_use]
pub fn side_slot(index: usize, height: f64, right: bool, width: f64) -> Option<Slot> {
    let top = ITEM_H.mul_add(count(index), TOP + 4.0);
    if top + ITEM_H > height {
        return None;
    }
    let x = if right { width - SIDE } else { 0.0 };
    Some((x + 3.0, top, SIDE - 6.0, ITEM_H - 3.0))
}

/// An index as a coordinate.
fn count(index: usize) -> f64 {
    f64::from(u32::try_from(index).unwrap_or(u32::MAX))
}

/// One rail button.
#[component]
fn Plate(slot: Slot, item: Item, colors: Colors, onpress: EventHandler<()>) -> Element {
    let (left, top, width, height) = slot;
    let size = item.size.unwrap_or(LABEL_SIZE);
    // A lit plate takes the accent and dark ink on it; an unlit one is
    // the panel's own button colour with dim ink.
    let (face, ink) = if item.on {
        (colors.accent.clone(), colors.ink_on_accent.clone())
    } else {
        (colors.button.clone(), colors.text_dim.clone())
    };
    rsx! {
        button {
            style: "position:absolute; left:{left}px; top:{top}px; width:{width}px; \
                    height:{height}px; background:{face}; color:{ink}; border:0; \
                    padding:0; margin:0; font-family:inherit; font-size:{size}px; \
                    line-height:{height}px; text-align:center; white-space:nowrap; \
                    overflow:hidden; pointer-events:auto; cursor:pointer;",
            // Pressed and current are the same picture, so the state has
            // to be said rather than shown — this is the attribute a
            // screen reader reads, and the painted version had nothing
            // to say it with.
            "aria-pressed": "{item.on}",
            onclick: move |_| onpress.call(()),
            "{item.label}"
        }
    }
}

/// The mode selector, in the corner the ruler leaves above the panel.
///
/// The one piece of chrome a mode does not re-populate: it is how you
/// CHANGE the mode, so it has to say the same thing in every one of
/// them. Laid out by division rather than by a fixed width — ten modes
/// share the corner, and a mode added to the list narrows them all
/// rather than pushing one off the end.
#[component]
pub fn ModeBar(
    /// How wide the corner is: the track panel's width.
    width: f64,
    /// And how tall: the ruler's height.
    height: f64,
    colors: Colors,
    modes: Arc<[Item]>,
    #[props(default)] on_press: EventHandler<usize>,
) -> Element {
    let ground = colors.tcp_gutter.clone();
    let each = width / count(modes.len()).max(1.0);
    rsx! {
        div {
            style: "position:absolute; left:{SIDE}px; top:{TOP}px; width:{width}px; \
                    height:{height}px; background:{ground}; font-family:{FONT};",
            "data-testid": "studio-modes",
            for (index, mode) in modes.iter().enumerate() {
                Plate {
                    key: "mode-{index}",
                    slot: (
                        count(index).mul_add(each, 1.0),
                        2.0,
                        each - 2.0,
                        height - 4.0,
                    ),
                    item: mode.clone(),
                    colors: colors.clone(),
                    onpress: move |()| on_press.call(index),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ITEM_H, SIDE, TOP, TOP_ITEM_W, side_slot, top_slot};

    /// The buttons stack from the top of the rail, one pitch each.
    #[test]
    fn a_side_rail_stacks_from_the_top() {
        let first = side_slot(0, 1440.0, false, 2560.0).expect("a first slot");
        let second = side_slot(1, 1440.0, false, 2560.0).expect("a second slot");
        assert!((first.1 - (TOP + 4.0)).abs() < f64::EPSILON);
        assert!((second.1 - first.1 - ITEM_H).abs() < f64::EPSILON);
        assert!(first.0 > 0.0 && first.0 + first.2 <= SIDE);
    }

    /// The right rail is the left one, at the other edge.
    #[test]
    fn the_right_rail_is_at_the_right() {
        let left = side_slot(0, 1440.0, false, 2560.0).expect("a left slot");
        let right = side_slot(0, 1440.0, true, 2560.0).expect("a right slot");
        assert!((left.1 - right.1).abs() < f64::EPSILON, "not level");
        assert!(right.0 > 2560.0 - SIDE - 1.0, "not at the right edge");
    }

    /// A rail with more items than height shows fewer, rather than
    /// painting into the panel.
    #[test]
    fn a_rail_runs_out_rather_than_overflowing() {
        // A window tall enough for two and no more.
        let height = TOP + 4.0 + ITEM_H * 2.0;
        assert!(side_slot(0, height, false, 2560.0).is_some());
        assert!(side_slot(1, height, false, 2560.0).is_some());
        assert!(
            side_slot(2, height, false, 2560.0).is_none(),
            "it drew past the bottom of the rail"
        );
    }

    /// The top rail runs out the same way, across.
    #[test]
    fn the_top_rail_runs_out_across() {
        let width = SIDE * 2.0 + 4.0 + TOP_ITEM_W * 2.0;
        assert!(top_slot(0, width).is_some());
        assert!(top_slot(1, width).is_some());
        assert!(top_slot(2, width).is_none(), "it drew under the right rail");
    }
}
