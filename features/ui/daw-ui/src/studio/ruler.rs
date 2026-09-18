//! The ruler over the arrangement, as components.
//!
//! Five rows, top to bottom: the song, its sections, its marks, the
//! tempo, and the bar numbers. Everything on the timeline's own axis, so
//! it pans and zooms with the material below rather than beside it.
//!
//! Built to the same three rules as [`super::lanes`], and panned by the
//! same mechanism: the rows are laid out in content space inside a
//! window wider than the screen, and one transform slides that window.
//! Nothing here subscribes to the scroll except the node that moves.
//!
//! # What this does not compute
//!
//! Where the bar numbers fall, and what they say. That is a walk through
//! the tempo map — a bar is however many beats the signature says, and
//! after a change a multiplied grid lands off the line it names — and it
//! belongs to whoever owns the map, not to a component. So a [`Tick`] is
//! a position and a label, resolved before it gets here, and the same is
//! true of a tempo [`Reading`].
//!
//! That is not squeamishness about arithmetic. It is what lets this
//! render a session, a take under review, or a fixture, without the UI
//! crate depending on a tempo map at all.

use std::sync::Arc;

use crate::prelude::*;

use super::lanes::{Built, Colors, View, Zoom};
use super::project::{Marker, Section};

/// How tall the bar-number row is.
pub const BARS_H: f64 = 28.0;

/// One named row's height.
pub const LANE_H: f64 = 15.0;

/// How many named rows there are, over the tempo and the bars.
pub const LANES: usize = 3;

/// What each is called, in the column beside them.
pub const LANE_NAMES: [&str; LANES] = ["SONG", "SECTIONS", "MARKS"];

/// How tall the tempo row is.
pub const TEMPO_H: f64 = 13.0;

/// The whole strip.
pub const RULER_H: f64 = BARS_H + TEMPO_H + LANE_H * 3.0;

/// How wide the column of row names is.
///
/// The track panel's width, because the names sit over the panel and the
/// timeline starts where the lanes do — a ruler whose axis did not line
/// up with the lanes under it would be a ruler for a different session.
pub const NAMES_W: f64 = 343.0;

/// A bar number: where it goes, and what it says.
#[derive(Clone, PartialEq)]
pub struct Tick {
    /// When, in seconds.
    pub at: f64,
    pub label: String,
}

/// A tempo change as it reads — "120 4/4" — and when it happens.
#[derive(Clone, PartialEq)]
pub struct Reading {
    pub at: f64,
    pub text: String,
}

/// The ticks and readings of a session, shared by pointer.
#[derive(Clone, Default)]
pub struct Marks {
    pub ticks: Arc<[Tick]>,
    pub tempo: Arc<[Reading]>,
    /// What the tempo already was when the view begins, if the first
    /// change is off to the left. Without it a session scrolled past its
    /// opening tempo says nothing about what it is playing.
    pub tempo_before: Option<String>,
}

impl PartialEq for Marks {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.ticks, &other.ticks)
            && Arc::ptr_eq(&self.tempo, &other.tempo)
            && self.tempo_before == other.tempo_before
    }
}

/// The type size everything but the bar numbers is written at.
const SIZE: f64 = 8.0;

/// And the bar numbers, which are the thing you read at arm's length.
const BARS_SIZE: f64 = 11.0;

/// How far the face's baseline sits below the top of its em, as a
/// fraction — DejaVu Sans's own ascent.
const ASCENT: f64 = 0.928;

/// And how far its descent reaches below the baseline.
const DESCENT: f64 = 0.236;

/// The line height that puts a baseline exactly `baseline` below the top
/// of the box.
///
/// CSS has no way to say "baseline here". It has a line box, and the
/// baseline sits at `(line-height - (ascent + descent)) / 2 + ascent`
/// inside it — so this is that solved for the line height. Everything in
/// a ruler is positioned by its baseline, because that is what makes a
/// number look like it is sitting ON the line it names.
#[must_use]
pub fn line_box(size: f64, baseline: f64) -> f64 {
    ASCENT
        .mul_add(-size, baseline)
        .mul_add(2.0, (ASCENT + DESCENT) * size)
}

/// Which row a lane number draws on.
///
/// One-based in the project, and clamped: a lane past the last one is a
/// project written by something that knew about more lanes than this
/// does, and the mark still has to appear somewhere.
#[must_use]
pub fn lane_row(lane: u32) -> usize {
    usize::try_from(lane.saturating_sub(1))
        .unwrap_or(0)
        .min(LANES.saturating_sub(1))
}

/// The ruler.
#[component]
pub fn Ruler(
    view: View,
    colors: Colors,
    marks: Marks,
    sections: Arc<[Section]>,
    markers: Arc<[Marker]>,
    scroll: ReadSignal<f64>,
    /// How far in the timeline is magnified — a signal, for the reason
    /// [`super::lanes::Lanes`] gives at length: read as a prop, a zoom
    /// re-rendered every bar number, every section band and every mark
    /// on every frame of the gesture. The strip is built at a snapped
    /// zoom and the leftover rides on `--sx`.
    #[props(default)]
    zoom: ReadSignal<Zoom>,
) -> Element {
    let (sx, _) = zoom().residual();
    let built_view = View {
        pps: view.pps * zoom().quantised().x,
        ..view
    };
    let built = use_memo(move || {
        let (sx, _) = zoom().residual();
        let view = View {
            pps: view.pps * zoom().quantised().x,
            ..view
        };
        Built::around((scroll() - NAMES_W).max(0.0) / sx, view)
    });
    let ground = colors.ruler_bg.clone();
    let rule = colors.rule.clone();
    let faint = colors.faint.clone();

    rsx! {
        div {
            style: "position:relative; width:{view.width}px; height:{RULER_H}px; \
                    overflow:hidden; background:{ground}; \
                    font-family:{super::lanes::FONT}; --sx:{sx:.6};",
            "data-testid": "studio-ruler",

            // The rows' own grounds and names, which do not move with
            // the timeline: they label the strip, not the session.
            for (row, name) in LANE_NAMES.iter().enumerate() {
                Named { row, name: (*name).to_owned(), rule: rule.clone(), faint: faint.clone() }
            }
            Named {
                row: LANES,
                name: "TEMPO".to_owned(),
                rule: rule.clone(),
                faint: faint.clone(),
                height: TEMPO_H,
            }

            // Everything that lives on the timeline.
            RulerPan {
                scroll,
                built,
                zoom,
                view: built_view,
                colors: colors.clone(),
                marks,
                sections,
                markers,
            }

            // The strip's own bottom edge, over everything.
            div {
                style: "position:absolute; left:0; right:0; bottom:0; height:1px; \
                        background:{rule};",
            }
        }
    }
}

/// One named row: its rule, and its name in the column.
#[component]
fn Named(
    row: usize,
    name: String,
    rule: String,
    faint: String,
    #[props(default = LANE_H)] height: f64,
) -> Element {
    let top = LANE_H * row_index(row);
    // The name sits four pixels off the bottom of its own row — three
    // for the tempo row, which is shorter and would otherwise have its
    // name crowding the bar numbers under it.
    let baseline = height
        - if (height - LANE_H).abs() < f64::EPSILON {
            4.0
        } else {
            3.0
        };
    let line = line_box(SIZE, baseline);
    rsx! {
        // The row's rule is its own bottom border and its name is its own
        // text: one node where there were three.
        div {
            style: "position:absolute; left:0; top:{top}px; width:100%; \
                    height:{height}px; box-sizing:border-box; \
                    border-bottom:1px solid {rule}; padding-left:8px; \
                    font-size:{SIZE}px; line-height:{line}px; color:{faint}; \
                    white-space:nowrap;",
            "{name}"
        }
    }
}

/// A row index as a coordinate.
fn row_index(row: usize) -> f64 {
    f64::from(u32::try_from(row).unwrap_or(0))
}

/// Everything on the timeline, and the transform that pans it.
#[component]
fn RulerPan(
    scroll: ReadSignal<f64>,
    built: Memo<Built>,
    zoom: ReadSignal<Zoom>,
    view: View,
    colors: Colors,
    marks: Marks,
    sections: Arc<[Section]>,
    markers: Arc<[Marker]>,
) -> Element {
    let window = built();
    // The column of row names is not on the timeline — the names label
    // the strip, not the session — so the offset it puts under everything
    // lives HERE, on the node that moves, rather than inside every
    // coordinate below. Otherwise it would be scaled by the zoom along
    // with them, and the ruler would start somewhere new at every zoom.
    let (sx, _) = zoom().residual();
    let offset = scroll() - window.from.mul_add(sx, NAMES_W);
    rsx! {
        div {
            style: "position:absolute; left:0; top:0; width:100%; height:100%; \
                    transform: translateX({-offset}px);",
            OnTheLine {
                view,
                colors,
                marks,
                sections,
                markers,
                built: window,
            }
        }
    }
}

/// The sections, marks, tempo changes and bar numbers.
#[component]
fn OnTheLine(
    view: View,
    colors: Colors,
    marks: Marks,
    sections: Arc<[Section]>,
    markers: Arc<[Marker]>,
    built: Built,
) -> Element {
    let pps = view.pps.max(1e-9);
    // Where a moment lands, measured from the built window's left edge,
    // in the pixels the SNAPPED zoom makes. The names column's width is
    // not in here any more: it is a fixed offset on the node that pans,
    // because it does not zoom.
    let x_of = move |at: f64| at.mul_add(pps, -built.from);
    let (from, to) = (built.from / pps, built.to / pps);

    let tempo_top = RULER_H - BARS_H - TEMPO_H;
    let bars_top = RULER_H - BARS_H;

    rsx! {
        div { style: "position:absolute; inset:0;",

            // Sections: a band, with its own colour down its leading
            // edge and its name inside it.
            for section in sections.iter() {
                {
                    let (x0, x1) = (x_of(section.start), x_of(section.end));
                    if x1 <= x0 || section.end < from || section.start > to {
                        return rsx! {};
                    }
                    let top = LANE_H * row_index(lane_row(section.lane)) + 2.0;
                    let tint = section.color.clone().unwrap_or_else(|| colors.accent.clone());
                    rsx! {
                        Band {
                            key: "{section.start}-{section.name}",
                            left: x0,
                            width: x1 - x0,
                            top,
                            tint,
                            name: section.name.clone(),
                            text: colors.text.clone(),
                        }
                    }
                }
            }

            // Marks: a flag on its lane, named to the right of it.
            for marker in markers.iter() {
                {
                    let x = x_of(marker.at);
                    if marker.at < from || marker.at > to {
                        return rsx! {};
                    }
                    let top = LANE_H * row_index(lane_row(marker.lane)) + 2.0;
                    let tint = marker.color.clone().unwrap_or_else(|| colors.ruler_fg.clone());
                    rsx! {
                        Flag {
                            key: "{marker.idx}-{marker.at}",
                            left: x,
                            top,
                            tint,
                            name: marker.name.clone(),
                            text: colors.text.clone(),
                        }
                    }
                }
            }

            // The tempo, where it changes — and what it already was, if
            // the change that set it is off to the left.
            if let Some(before) = marks.tempo_before.as_ref() {
                div {
                    style: "position:absolute; \
                            left:calc({x_of(from.max(0.0))}px * var(--sx, 1) + 4px); \
                            top:{tempo_top}px; font-size:{SIZE}px; \
                            line-height:{line_box(SIZE, TEMPO_H - 3.0)}px; \
                            color:{colors.faint}; white-space:nowrap;",
                    "{before}"
                }
            }
            for reading in marks.tempo.iter() {
                {
                    if reading.at < from || reading.at > to {
                        return rsx! {};
                    }
                    let x = x_of(reading.at);
                    rsx! {
                        // The reading lives INSIDE its mark rather than
                        // beside it: one node fewer, one key, and the
                        // text is free to overflow a one-pixel parent
                        // because nothing here clips.
                        div {
                            key: "{reading.at}",
                            style: "position:absolute; \
                                    left:calc({x}px * var(--sx, 1)); \
                                    top:{tempo_top + 1.0}px; width:1px; \
                                    height:{TEMPO_H - 2.0}px; background:{colors.accent};",
                            div {
                                style: "position:absolute; left:4px; top:-1px; \
                                        font-size:{SIZE}px; \
                                        line-height:{line_box(SIZE, TEMPO_H - 3.0)}px; \
                                        color:{colors.text}; white-space:nowrap;",
                                "{reading.text}"
                            }
                        }
                    }
                }
            }

            // The bar numbers, each on its own tick.
            for tick in marks.ticks.iter() {
                {
                    if tick.at < from || tick.at > to {
                        return rsx! {};
                    }
                    let x = x_of(tick.at);
                    rsx! {
                        // Likewise the bar number, inside its own tick.
                        div {
                            key: "{tick.at}",
                            style: "position:absolute; \
                                    left:calc({x}px * var(--sx, 1)); \
                                    top:{bars_top + BARS_H - 9.0}px; width:1px; \
                                    height:9px; background:{colors.grid};",
                            div {
                                style: "position:absolute; left:2px; \
                                        top:{9.0 - BARS_H}px; font-size:{BARS_SIZE}px; \
                                        line-height:{line_box(BARS_SIZE, 14.0)}px; \
                                        color:{colors.ruler_fg}; white-space:nowrap;",
                                "{tick.label}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A section's band.
#[component]
fn Band(left: f64, width: f64, top: f64, tint: String, name: String, text: String) -> Element {
    /// How much room a name needs before it is written at all.
    const ROOM: f64 = 24.0;
    let height = LANE_H - 4.0;
    rsx! {
        div {
            style: "position:absolute; left:calc({left}px * var(--sx, 1)); top:{top}px; \
                    width:calc({width}px * var(--sx, 1)); \
                    height:{height}px; background:{tint}; opacity:0.45; overflow:hidden;",
        }
        // The leading edge at full strength, so a band's START is
        // findable when the band itself is a wash. Two pixels at any
        // zoom, because it is an edge rather than a span.
        div {
            style: "position:absolute; left:calc({left}px * var(--sx, 1)); top:{top}px; \
                    width:{width.min(2.0)}px; height:{height}px; background:{tint};",
        }
        if width > ROOM {
            div {
                style: "position:absolute; \
                        left:calc({left}px * var(--sx, 1) + 5px); top:{top}px; \
                        width:calc({width}px * var(--sx, 1) - 8px); \
                        height:{height}px; font-size:{SIZE}px; \
                        line-height:{line_box(SIZE, LANE_H - 6.0)}px; color:{text}; \
                        white-space:nowrap; overflow:hidden;",
                "{name}"
            }
        }
    }
}

/// A marker's flag.
#[component]
fn Flag(left: f64, top: f64, tint: String, name: String, text: String) -> Element {
    let height = LANE_H - 4.0;
    // A flag is a POSITION, not a span: its staff, its pennant and its
    // name keep their size at every zoom and only their place moves.
    rsx! {
        div {
            style: "position:absolute; left:calc({left}px * var(--sx, 1)); top:{top}px; \
                    width:2px; height:{height}px; background:{tint};",
        }
        div {
            style: "position:absolute; left:calc({left}px * var(--sx, 1)); top:{top}px; \
                    width:8px; height:4px; background:{tint};",
        }
        div {
            style: "position:absolute; left:calc({left}px * var(--sx, 1) + 5px); \
                    top:{top}px; \
                    font-size:{SIZE}px; line-height:{line_box(SIZE, LANE_H - 6.0)}px; \
                    color:{text}; white-space:nowrap;",
            "{name}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ASCENT, DESCENT, LANES, RULER_H, lane_row, line_box};

    /// The line box puts the baseline where it was asked for.
    #[test]
    fn a_line_box_lands_its_baseline() {
        for (size, baseline) in [(8.0, 11.0), (8.0, 9.0), (11.0, 14.0)] {
            let line: f64 = line_box(size, baseline);
            // Where CSS will actually put it, from the same definition.
            let landed = (line - (ASCENT + DESCENT) * size).mul_add(0.5, ASCENT * size);
            assert!(
                (landed - baseline).abs() < 1e-9,
                "{size}px asked for {baseline} and got {landed}"
            );
        }
    }

    /// Lanes are one-based in the project, and a lane past the last one
    /// still has to draw somewhere.
    #[test]
    fn a_lane_number_finds_a_row() {
        assert_eq!(lane_row(1), 0);
        assert_eq!(lane_row(2), 1);
        assert_eq!(lane_row(3), 2);
        assert_eq!(lane_row(0), 0, "a zero lane fell off the top");
        assert_eq!(lane_row(99), LANES - 1, "a lane past the last one vanished");
    }

    /// The strip is as tall as its parts, which is what every other
    /// module lays itself out under.
    #[test]
    fn the_strip_is_the_sum_of_its_rows() {
        assert!((RULER_H - (28.0 + 13.0 + 15.0 * 3.0)).abs() < f64::EPSILON);
    }
}
