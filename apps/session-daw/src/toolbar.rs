//! The DAW view's main toolbar: the corner left of the ruler, above the
//! track panel — where REAPER keeps its main toolbar.
//!
//! The toggles are REAPER's main-toolbar set: metronome, auto crossfade,
//! item grouping, ripple per track, grid lines, snapping, locking. The
//! editing ones switch [`crate::options`]; the metronome is the session's
//! own click — the generated Click track, whose mute it flips — because the
//! FTS click is the guide's, not the DAW's built-in metronome.
//!
//! The icons are drawn here as SVG rather than taken from REAPER's theme:
//! that art is REAPER's, and this is a GPL tree.

use dioxus::prelude::*;

use crate::options;

const ON_BG: &str = "#1f5f55";
const ON_FG: &str = "#e8fff9";
const OFF_FG: &str = "#9aa0a8";
const HOVER_RULE: &str = "#3a3f47";

/// Which toggle a button is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Toggle {
    Metronome,
    AutoCrossfade,
    Grouping,
    Ripple,
    Grid,
    Snap,
    Lock,
}

impl Toggle {
    const ROWS: [&'static [Self]; 2] = [
        &[Self::Metronome, Self::AutoCrossfade, Self::Grouping, Self::Ripple],
        &[Self::Grid, Self::Snap, Self::Lock],
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Metronome => "Metronome (the Click track)",
            Self::AutoCrossfade => "Auto crossfade",
            Self::Grouping => "Item grouping",
            Self::Ripple => "Ripple editing per track",
            Self::Grid => "Grid lines",
            Self::Snap => "Snapping",
            Self::Lock => "Locking",
        }
    }

    const fn option(self) -> Option<&'static options::Option> {
        match self {
            Self::Metronome => None,
            Self::AutoCrossfade => Some(&options::AUTO_CROSSFADE),
            Self::Grouping => Some(&options::GROUPING),
            Self::Ripple => Some(&options::RIPPLE),
            Self::Grid => Some(&options::GRID),
            Self::Snap => Some(&options::SNAP),
            Self::Lock => Some(&options::LOCKING),
        }
    }

    /// The icon's strokes, on a 24-unit square.
    const fn paths(self) -> &'static [&'static str] {
        match self {
            // A metronome: the body and the arm.
            Self::Metronome => &["M8 21 L10.5 3 H13.5 L16 21 Z", "M12 16 L17.5 6.5", "M6 21 H18"],
            // Two fades crossing.
            Self::AutoCrossfade => &["M3 19 C9 19 15 5 21 5", "M3 5 C9 5 15 19 21 19"],
            // Two links of a chain.
            Self::Grouping => &[
                "M10 14 L8.5 15.5 A3.5 3.5 0 0 1 3.5 10.5 L6.5 7.5 A3.5 3.5 0 0 1 11 7",
                "M14 10 L15.5 8.5 A3.5 3.5 0 0 1 20.5 13.5 L17.5 16.5 A3.5 3.5 0 0 1 13 17",
                "M9 15 L15 9",
            ],
            // An item pushing the ones after it along one track.
            Self::Ripple => &[
                "M3 9 H9 V15 H3 Z",
                "M12 12 H20",
                "M17 9 L20 12 L17 15",
            ],
            // Grid lines.
            Self::Grid => &["M4 4 V20", "M10 4 V20", "M16 4 V20", "M4 8 H20", "M4 16 H20"],
            // A magnet.
            Self::Snap => &[
                "M6 4 V11 A6 6 0 0 0 18 11 V4",
                "M6 8 H10",
                "M14 8 H18",
                "M10 4 V11 A2 2 0 0 0 14 11 V4",
            ],
            // A padlock.
            Self::Lock => &["M6 11 H18 V20 H6 Z", "M8.5 11 V8 A3.5 3.5 0 0 1 15.5 8 V11"],
        }
    }
}

/// The toolbar. `click` is the generated Click track's guid, if the session
/// has one; `edits` is the arrangement's queue, which carries the mute.
#[component]
pub fn MainToolbar(
    click: Option<String>,
    click_muted: bool,
    edits: crate::studio::Edits,
    width: f64,
) -> Element {
    // Each option's state as last set here; the options themselves are the
    // truth, this is what re-renders the button.
    let mut shown = use_signal(|| {
        Toggle::ROWS
            .iter()
            .flat_map(|row| row.iter())
            .map(|t| (*t, t.option().is_none_or(options::Option::get)))
            .collect::<Vec<_>>()
    });
    let mut metronome = use_signal(|| !click_muted);
    let is_on = move |t: Toggle| match t {
        Toggle::Metronome => metronome(),
        other => shown.read().iter().any(|(each, on)| *each == other && *on),
    };
    rsx! {
        div {
            style: "position:absolute; left:0; top:0; width:{width}px; padding:6px 8px; \
                    display:flex; flex-direction:column; gap:4px;",
            for row in Toggle::ROWS {
                div {
                    style: "display:flex; gap:4px;",
                    for toggle in row.iter().copied() {
                        button {
                            key: "{toggle:?}",
                            title: toggle.title(),
                            style: button_style(is_on(toggle)),
                            onclick: {
                                let edits = edits.clone();
                                let click = click.clone();
                                move |_| match toggle {
                                    Toggle::Metronome => {
                                        if let Some(guid) = click.clone() {
                                            edits.0.borrow_mut().push(crate::engine::Edit::ToggleMute(guid));
                                            metronome.toggle();
                                        }
                                    }
                                    other => {
                                        if let Some(option) = other.option() {
                                            let now = option.toggle();
                                            for (each, on) in shown.write().iter_mut() {
                                                if *each == other {
                                                    *on = now;
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            Icon { toggle, on: is_on(toggle) }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Icon(toggle: Toggle, on: bool) -> Element {
    let color = if on { ON_FG } else { OFF_FG };
    rsx! {
        svg {
            width: "18",
            height: "18",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: color,
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            for d in toggle.paths().iter().copied() {
                path { d }
            }
        }
    }
}

fn button_style(on: bool) -> String {
    let (bg, rule) = if on { (ON_BG, ON_BG) } else { ("transparent", HOVER_RULE) };
    format!(
        "width:30px; height:26px; padding:0; display:flex; align-items:center; \
         justify-content:center; border-radius:4px; border:1px solid {rule}; background:{bg};"
    )
}
