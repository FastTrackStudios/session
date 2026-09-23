//! The app's frame, shared by the window and the page: the main views, the
//! setlist's own tabs, the transport and the mode, and the Overview's
//! layout.
//!
//! Both hosts draw the same bar over their own panels — the desktop app
//! adds the window's drag surface and the traffic lights' corner, the page
//! adds neither, and everything else here is the same in both. What a host
//! passes in is only what is host-shaped: its transport bar, and the panels
//! a view is made of.
//!
//! Four views, in the order a service goes through them:
//!
//! - **Setup** — the setlist, and the routing. What is done before.
//! - **Performance** — the chart and the progress, for playing from.
//! - **DAW** — the arrangement and the mixer, for working on.
//! - **Overview** — all of it at once: progress across the top, the chart
//!   down the left, the arrangement with the mixer docked under it on the
//!   right. The first of the docked views.

use dioxus::prelude::*;
use session::modes::Mode;

use crate::setlist::Setlist;

/// How tall the top bar is — room for the traffic lights with air around.
pub const BAR_H: f64 = 40.0;

pub const BAR_BG: &str = "#17181b";
pub const RULE: &str = "#2a2c31";
pub const TEXT: &str = "#e5e7eb";
pub const DIM: &str = "#8b9099";
pub const ACCENT: &str = "#3aa0ff";

/// The views the top bar switches between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Setup,
    Performance,
    Daw,
    Overview,
}

impl View {
    pub const ALL: [Self; 4] = [Self::Setup, Self::Performance, Self::Daw, Self::Overview];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Performance => "Performance",
            Self::Daw => "DAW",
            Self::Overview => "Overview",
        }
    }
}

/// The top bar: the views, the setlist, the transport, the mode.
///
/// `lights` is how much of the left end belongs to the window's own
/// controls (the traffic lights on macOS, nothing in a page), and
/// `on_drag` / `on_zoom` are what a window does with a press on the bar
/// and a double click — a page passes neither.
#[component]
pub fn TopBar(
    view: Signal<View>,
    mode: Signal<Mode>,
    /// The host's transport bar.
    transport: Element,
    #[props(default = 12.0)] lights: f64,
    on_drag: Option<EventHandler<()>>,
    on_zoom: Option<EventHandler<()>>,
    /// Picking a song from the setlist tabs.
    on_pick: Option<EventHandler<usize>>,
    /// Recolouring a song from its tab (see [`SongTabs`]).
    on_color: Option<EventHandler<(usize, Option<String>)>>,
) -> Element {
    let mut picking = use_signal(|| false);
    rsx! {
        div {
            style: "position:relative; height:{BAR_H}px; flex:none; display:flex; \
                    align-items:center; gap:8px; padding-left:{lights}px; \
                    padding-right:10px; background:{BAR_BG}; \
                    border-bottom:1px solid {RULE};",
            onmousedown: move |_| {
                if let Some(drag) = on_drag {
                    drag.call(());
                }
            },
            ondoubleclick: move |_| {
                if let Some(zoom) = on_zoom {
                    zoom.call(());
                }
            },
            // The views, as a segmented control.
            div {
                style: "display:flex; gap:2px; padding:2px; background:#0f1012; \
                        border:1px solid {RULE}; border-radius:7px; flex:none;",
                for each in View::ALL {
                    button {
                        style: segment(view() == each),
                        onmousedown: move |event| event.stop_propagation(),
                        onclick: move |_| view.set(each),
                        "{each.name()}"
                    }
                }
            }
            // The setlist, filling whatever the bar has left.
            SongTabs { on_pick, on_color }
            // A row: the transport, and anything the host puts beside it
            // (the collaboration bar).
            div {
                style: "flex:none; display:flex; align-items:center;",
                onmousedown: move |event| event.stop_propagation(),
                {transport}
            }
            // The mode, visible in every view.
            div {
                style: "position:relative; flex:none;",
                onmousedown: move |event| event.stop_propagation(),
                button {
                    style: "display:flex; align-items:center; gap:6px; height:26px; \
                            padding:0 10px; border-radius:6px; border:1px solid {RULE}; \
                            background:#0f1012; color:{TEXT}; font-size:12px; cursor:pointer;",
                    onclick: move |_| picking.toggle(),
                    span { style: "color:{DIM};", "Mode" }
                    span { style: "font-weight:600;", "{mode().display_name()}" }
                }
                if picking() {
                    div {
                        style: "position:absolute; right:0; top:30px; z-index:10; \
                                min-width:160px; padding:4px; background:{BAR_BG}; \
                                border:1px solid {RULE}; border-radius:8px; \
                                box-shadow:0 8px 24px rgba(0,0,0,0.5);",
                        for each in Mode::ALL {
                            div {
                                style: option(mode() == each),
                                onclick: move |_| {
                                    mode.set(each);
                                    picking.set(false);
                                },
                                "{each.display_name()}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The setlist across the bar, as Safari lays out its tabs: one rounded
/// strip in the middle of the bar, a tab per song sharing its width, the
/// current one raised.
///
/// Each tab carries its song's colour — a dot beside the title, and a
/// progress line along its foot filled to how far through the song the
/// set is (the playhead in the current song, where it was left in the
/// others). The colour is the song's own: set by hand from the dot, or
/// derived from its title ([`crate::setlist::title_color`]).
///
/// The tabs SHARE the width rather than each taking a fixed size — three
/// songs are three wide tabs, twelve are twelve narrow ones, and either way
/// the set is one glance rather than a list to read.
#[component]
pub fn SongTabs(
    on_pick: Option<EventHandler<usize>>,
    /// A colour picked for a song by hand: its index and the CSS colour,
    /// or `None` for "back to the title's colour".
    on_color: Option<EventHandler<(usize, Option<String>)>>,
) -> Element {
    let Some(setlist) = try_use_context::<Signal<Setlist>>() else {
        // No setlist (a single session opened straight): nothing to show,
        // and the bar's spare width goes to the transport.
        return rsx! { div { style: "flex:1;" } };
    };
    let reading = crate::progress::use_reading();
    let mut coloring = use_signal(|| None::<usize>);
    let list = setlist();
    if list.songs.is_empty() {
        return rsx! { div { style: "flex:1;" } };
    }
    let at = reading().at;
    let count = list.songs.len();
    rsx! {
        div {
            style: "position:relative; flex:1; min-width:0; display:flex; height:30px; \
                    padding:2px; gap:0; align-items:stretch; background:#0f1012; \
                    border:1px solid {RULE}; border-radius:9px;",
            onmousedown: move |event| event.stop_propagation(),
            for (index, song) in list.songs.iter().cloned().enumerate() {
                {
                    let current = index == list.at;
                    let percent = list.progress_of(index, at) * 100.0;
                    let ground = if current { "#2b2e35" } else { "transparent" };
                    let ink = if current { TEXT } else { DIM };
                    // A hairline between two tabs that are not raised, as
                    // Safari draws it; none beside the current one.
                    let divider = if index > 0 && !current && index != list.at + 1 {
                        RULE
                    } else {
                        "transparent"
                    };
                    let color = song.color.clone();
                    rsx! {
                        div {
                            key: "{song.project}",
                            style: "position:relative; flex:1; min-width:0; display:flex; \
                                    align-items:center; justify-content:center; gap:7px; \
                                    padding:0 12px; border-radius:7px; background:{ground}; \
                                    border-left:1px solid {divider}; cursor:default;",
                            onclick: move |_| {
                                if let Some(pick) = on_pick {
                                    pick.call(index);
                                }
                            },
                            // The song's colour; a click here recolours it.
                            div {
                                style: "flex:none; width:9px; height:9px; border-radius:5px; \
                                        background:{color}; cursor:pointer;",
                                onclick: move |event| {
                                    event.stop_propagation();
                                    coloring.set(if coloring() == Some(index) { None } else { Some(index) });
                                },
                            }
                            span {
                                style: "min-width:0; overflow:hidden; text-overflow:ellipsis; \
                                        white-space:nowrap; color:{ink}; font-size:12px; \
                                        font-weight:600;",
                                "{song.name}"
                            }
                            // How far through the song: a line along the foot.
                            div {
                                style: "position:absolute; left:10px; right:10px; bottom:2px; \
                                        height:2px; border-radius:1px; background:#ffffff14;",
                                div {
                                    style: "width:{percent}%; height:2px; border-radius:1px; \
                                            background:{color};",
                                }
                            }
                        }
                    }
                }
            }
            if let Some(index) = coloring() {
                ColorMenu {
                    // Under the tab it belongs to.
                    left: format!("{:.3}%", (index as f64 + 0.5) / count as f64 * 100.0),
                    on_pick: move |choice: Option<String>| {
                        coloring.set(None);
                        if let Some(color) = on_color {
                            color.call((index, choice));
                        }
                    },
                }
            }
        }
    }
}

/// The colours a song can be given by hand, and "Auto" — back to the
/// colour its title gives it.
#[component]
fn ColorMenu(left: String, on_pick: EventHandler<Option<String>>) -> Element {
    rsx! {
        div {
            style: "position:absolute; top:34px; left:{left}; margin-left:-86px; z-index:20; \
                    width:172px; padding:8px; display:flex; flex-wrap:wrap; gap:6px; \
                    background:{BAR_BG}; border:1px solid {RULE}; border-radius:9px; \
                    box-shadow:0 8px 24px rgba(0,0,0,0.5);",
            onmousedown: move |event| event.stop_propagation(),
            for color in crate::setlist::COLORS {
                div {
                    style: "width:34px; height:20px; border-radius:5px; background:{color}; \
                            cursor:pointer;",
                    onclick: move |_| on_pick.call(Some(color.to_owned())),
                }
            }
            div {
                style: "flex:1; min-width:100%; height:22px; display:flex; align-items:center; \
                        justify-content:center; border-radius:5px; border:1px solid {RULE}; \
                        color:{DIM}; font-size:11px; cursor:pointer;",
                onclick: move |_| on_pick.call(None),
                "Auto — from the title"
            }
        }
    }
}

/// The Overview: the progress across the top, the chart down the left, and
/// the arrangement (with the mixer docked under it) on the right.
///
/// One page that answers the three questions a service asks at once —
/// where are we in the song, what is coming, and what is playing — which
/// is why it is the first of the docked views rather than a fourth panel
/// in a stack.
///
/// `editor`, when a host passes one, is the chart as text: a column left of
/// the chart, which then gives up some of its width. `chart_corner` sits
/// over the chart's top-left corner — the desktop's button that opens and
/// closes that editor.
#[component]
pub fn OverviewLayout(
    progress: Element,
    chart: Element,
    panels: Element,
    #[props(default)] editor: Option<Element>,
    #[props(default)] chart_corner: Option<Element>,
    #[props(default)] under_chart: Option<Element>,
) -> Element {
    // The chart takes the column's full width, and — with something under
    // it — the larger share of its height; the rest goes under it.
    let chart_h = if under_chart.is_some() {
        format!("aspect-ratio:{};", crate::chart_panel::FITTED_WIDTH_OVER_HEIGHT)
    } else {
        "height:100%;".to_owned()
    };
    let chart_w = if editor.is_some() { "width:30%; min-width:240px;" } else { "width:38%; min-width:280px;" };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; \
                    flex-direction:column; gap:10px; padding:10px;",
            div { style: "flex:none;", {progress} }
            div {
                style: "flex:1; min-height:0; display:flex; gap:10px;",
                if let Some(editor) = editor {
                    // A chart's lines are short: the editor needs a column,
                    // not a share of the window.
                    // Every column is `height:100%`, not the row's stretch:
                    // what is inside is placed absolutely, and Blitz laid it
                    // out against the column's height BEFORE the stretch —
                    // 2px, the editor a one-line sliver — until something
                    // (a click) laid the pane out again.
                    div {
                        style: "position:relative; flex:none; width:380px; height:100%; \
                                border-radius:8px; overflow:hidden; border:1px solid {RULE};",
                        onmounted: move |e| region("editor", &e),
                        {editor}
                    }
                }
                // The chart over whatever the host puts under it — the
                // lyrics, in the room a 16:9 screen leaves below a page —
                // as one pane: the chart exactly its fitted shape across
                // the column's width, the rest below it the lyrics'.
                div {
                    style: "position:relative; {chart_w} height:100%; display:flex; \
                            flex-direction:column; border-radius:8px; overflow:hidden; \
                            border:1px solid {RULE};",
                    div {
                        style: "position:relative; flex:none; width:100%; {chart_h}",
                        onmounted: move |e| region("chart", &e),
                        {chart}
                        if let Some(corner) = chart_corner {
                            div { style: "position:absolute; top:8px; left:8px;", {corner} }
                        }
                    }
                    if let Some(under) = under_chart {
                        div {
                            style: "position:relative; flex:1; min-height:0; width:100%; \
                                    border-top:1px solid {RULE};",
                            onmounted: move |e| region("lyrics", &e),
                            {under}
                        }
                    }
                }
                div {
                    style: "position:relative; flex:1; min-width:0; height:100%; border-radius:8px; \
                            overflow:hidden; border:1px solid {RULE};",
                    onmounted: move |e| region("panels", &e),
                    {panels}
                }
            }
        }
    }
}

/// A pane of the Overview is a place others' pointers can be shown in
/// (see `collab_pointers`).
#[allow(clippy::needless_pass_by_value)]
fn region(id: &str, event: &Event<MountedData>) {
    #[cfg(feature = "native")]
    crate::ghosts::region_mounted(id, event.data());
    #[cfg(not(feature = "native"))]
    let _ = (id, event);
}

/// A view button, on or off.
#[must_use]
pub fn segment(on: bool) -> String {
    let (bg, fg) = if on { (ACCENT, "#0b0c0e") } else { ("transparent", DIM) };
    format!(
        "height:24px; padding:0 12px; border:none; border-radius:5px; cursor:pointer; \
         background:{bg}; color:{fg}; font-size:12px; font-weight:600;"
    )
}

/// A row in the mode menu.
#[must_use]
pub fn option(on: bool) -> String {
    let (bg, fg) = if on { ("#23262c", TEXT) } else { ("transparent", DIM) };
    format!("padding:6px 10px; border-radius:5px; cursor:pointer; background:{bg}; color:{fg}; font-size:12px;")
}
