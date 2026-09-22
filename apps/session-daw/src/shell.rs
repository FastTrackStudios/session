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
            SongTabs { on_pick }
            div {
                style: "flex:none;",
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

/// The setlist across the bar: a tab per song, each in the song's own
/// colour, filling as it plays.
///
/// The tabs SHARE the bar's spare width rather than each taking a fixed
/// size — three songs are three wide tabs, twelve are twelve narrow ones,
/// and either way the set is one glance rather than a list to read. The
/// current song's tab is filled to where the playhead is in it, so the
/// same glance says how far through it is.
#[component]
pub fn SongTabs(on_pick: Option<EventHandler<usize>>) -> Element {
    let Some(setlist) = try_use_context::<Signal<Setlist>>() else {
        // No setlist (a single session opened straight): nothing to show,
        // and the bar's spare width goes to the transport.
        return rsx! { div { style: "flex:1;" } };
    };
    let reading = crate::progress::use_reading();
    let list = setlist();
    if list.songs.is_empty() {
        return rsx! { div { style: "flex:1;" } };
    }
    let at = reading().at;
    rsx! {
        div {
            style: "flex:1; min-width:0; display:flex; gap:3px; height:28px; \
                    align-items:stretch; overflow:hidden;",
            onmousedown: move |event| event.stop_propagation(),
            for (index, song) in list.songs.iter().cloned().enumerate() {
                {
                    let current = index == list.at;
                    let filled = if current { song.progress(at) } else { f64::from(u8::from(index < list.at)) };
                    let percent = filled * 100.0;
                    // The fill is the song's colour over its own dim
                    // ground, so a tab says which song and how far at once.
                    let ground = if current { "#0f1012" } else { "#131417" };
                    let ink = if current { TEXT } else { DIM };
                    let border = if current { song.color.clone() } else { RULE.to_owned() };
                    rsx! {
                        button {
                            style: "flex:1; min-width:0; padding:0 8px; border-radius:6px; \
                                    border:1px solid {border}; color:{ink}; font-size:12px; \
                                    font-weight:600; text-align:left; cursor:pointer; \
                                    overflow:hidden; text-overflow:ellipsis; white-space:nowrap; \
                                    background:linear-gradient(90deg, {song.color} 0%, \
                                    {song.color} {percent}%, {ground} {percent}%, {ground} 100%);",
                            onclick: move |_| {
                                if let Some(pick) = on_pick {
                                    pick.call(index);
                                }
                            },
                            "{song.name}"
                        }
                    }
                }
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
#[component]
pub fn OverviewLayout(progress: Element, chart: Element, panels: Element) -> Element {
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; \
                    flex-direction:column; gap:10px; padding:10px;",
            div { style: "flex:none;", {progress} }
            div {
                style: "flex:1; min-height:0; display:flex; gap:10px;",
                div {
                    style: "position:relative; width:38%; min-width:280px; border-radius:8px; \
                            overflow:hidden; border:1px solid {RULE};",
                    {chart}
                }
                div {
                    style: "position:relative; flex:1; min-width:0; border-radius:8px; \
                            overflow:hidden; border:1px solid {RULE};",
                    {panels}
                }
            }
        }
    }
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
