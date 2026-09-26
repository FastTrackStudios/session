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

/// How much room the top bar has, and so how much of it is spelled out.
///
/// - **Full** — everything, as on a wide screen: the views as a segmented
///   control, tempo and key each labelled, the clock beside the bar.beat.
/// - **Compact** — half a wide screen: the views fold into a menu, tempo
///   and key into one small pill.
/// - **Narrow** — smaller still: the clock and go-to-start/end go too
///   (Home / End on the keyboard, the tabs for the songs).
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Density {
    Narrow,
    Compact,
    Full,
}

impl Density {
    /// For a bar `width` logical pixels wide.
    #[must_use]
    pub fn for_width(width: f64) -> Self {
        if width >= 1700.0 {
            Self::Full
        } else if width >= 1250.0 {
            Self::Compact
        } else {
            Self::Narrow
        }
    }
}

/// The bar's density, for what sits in it (the transport, the
/// collaboration button); `Full` outside a bar.
#[must_use]
pub fn use_density() -> Density {
    try_use_context::<Signal<Density>>().map_or(Density::Full, |d| d())
}

/// The views the bars switch between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Setup,
    Performance,
    /// The arrangement, and the mixer docked under it when it is open.
    Daw,
    Overview,
    /// The chart alone, the whole window.
    Chart,
    /// The mixer alone, the whole window.
    Mixer,
}

impl View {
    pub const ALL: [Self; 6] = [
        Self::Performance,
        Self::Overview,
        Self::Chart,
        Self::Daw,
        Self::Mixer,
        Self::Setup,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Performance => "Performance",
            Self::Daw => "Arrangement",
            Self::Overview => "Overview",
            Self::Chart => "Chart",
            Self::Mixer => "Mixer",
        }
    }
}

/// The app's outer box, inset from a phone's or a tablet's safe areas
/// (the notch, the home indicator, a landscape screen's rounded sides) and
/// filled round them with the bars' colour: in a page, where
/// `viewport-fit=cover` runs the page under them. Blitz keeps them out of
/// the page already, and there the insets are nothing.
pub const SAFE_AREA: &str = "position:absolute; top:0; left:0; width:100vw; height:100vh; \
    box-sizing:border-box; display:flex; background:#17181b; \
    padding:env(safe-area-inset-top, 0px) env(safe-area-inset-right, 0px) \
    env(safe-area-inset-bottom, 0px) env(safe-area-inset-left, 0px);";

/// How tall the bottom bar is.
pub const BOTTOM_H: f64 = 34.0;

/// The bottom bar: the views as icons across the foot of the window, the
/// way the top bar runs across its head — Logic's iPad layout, where what
/// you look at is picked at the bottom and what you play at the top.
/// Icons alone, as Logic's are, with the view's name for a tooltip; the
/// views in the middle, Setup to the right on its own. The left is kept
/// for the panels a view can show beside itself (the inspector).
#[component]
pub fn BottomBar(view: Signal<View>) -> Element {
    let button = move |each: View| {
        rsx! {
            button {
                key: "{each.name()}",
                title: each.name(),
                style: bottom_button(view() == each),
                onclick: move |_| view.set(each),
                ViewIcon { view: each }
            }
        }
    };
    rsx! {
        div {
            style: "height:{BOTTOM_H}px; flex:none; display:flex; align-items:center; \
                    padding:0 12px; background:{BAR_BG}; border-top:1px solid {RULE};",
            div { style: "flex:1;" }
            div {
                style: "flex:none; display:flex; align-items:center; gap:6px;",
                for each in View::ALL.into_iter().filter(|v| *v != View::Setup) {
                    {button(each)}
                }
            }
            div {
                style: "flex:1; display:flex; justify-content:flex-end;",
                {button(View::Setup)}
            }
        }
    }
}

/// A bottom-bar button: a grey icon, or the view showing's, white on a
/// light square.
fn bottom_button(on: bool) -> String {
    let (fg, bg) = if on {
        (TEXT, "#3a3d44")
    } else {
        (DIM, "transparent")
    };
    format!(
        "width:40px; height:28px; display:flex; align-items:center; justify-content:center; \
         border:none; border-radius:8px; background:{bg}; color:{fg}; cursor:pointer;"
    )
}

/// A view's icon, as the bottom bar shows it.
#[component]
fn ViewIcon(view: View) -> Element {
    use lucide_dioxus::{
        ChartNoAxesGantt, FileMusic, LayoutDashboard, ListMusic, Settings, SlidersVertical,
    };
    let size = 19;
    match view {
        View::Performance => rsx! { ListMusic { size, color: "currentColor" } },
        View::Overview => rsx! { LayoutDashboard { size, color: "currentColor" } },
        View::Chart => rsx! { FileMusic { size, color: "currentColor" } },
        View::Daw => rsx! { ChartNoAxesGantt { size, color: "currentColor" } },
        View::Mixer => rsx! { SlidersVertical { size, color: "currentColor" } },
        View::Setup => rsx! { Settings { size, color: "currentColor" } },
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
    /// How wide the bar is, in logical pixels, when the host knows (a
    /// window does; `None` lays it out in full).
    #[props(default)]
    width: Option<f64>,
) -> Element {
    let mut picking = use_signal(|| false);
    let density = width.map_or(Density::Full, Density::for_width);
    let mut shared = use_context_provider(|| Signal::new(density));
    use_effect(use_reactive!(|density| {
        if *shared.peek() != density {
            shared.set(density);
        }
    }));
    let mode_label = if density == Density::Narrow {
        ""
    } else {
        "Mode"
    };
    rsx! {
        div {
            style: "position:relative; height:{BAR_H}px; flex:none; display:flex; \
                    align-items:center; gap:8px; padding-left:{lights}px; \
                    padding-right:10px; background:{BAR_BG}; \
                    border-bottom:1px solid {RULE}; z-index:30;",
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
            // The setlist, filling whatever the bar has left.
            SongTabs { on_pick, on_color }
            // A row: the transport, and anything the host puts beside it
            // (the collaboration bar).
            div {
                style: "flex:none; display:flex; align-items:center;",
                onmousedown: move |event| event.stop_propagation(),
                {transport}
            }
            // Where the sound comes from: Engine / Cue / Remote.
            AudioBadge { density }
            // The mode, visible in every view.
            div {
                style: "position:relative; flex:none;",
                onmousedown: move |event| event.stop_propagation(),
                button {
                    style: "display:flex; align-items:center; gap:6px; height:26px; \
                            padding:0 10px; border-radius:6px; border:1px solid {RULE}; \
                            background:#0f1012; color:{TEXT}; font-size:12px; cursor:pointer;",
                    onclick: move |_| picking.toggle(),
                    if !mode_label.is_empty() {
                        span { style: "color:{DIM};", "{mode_label}" }
                    }
                    span { style: "font-weight:600;", "{mode().display_name()}" }
                }
                if picking() {
                    div {
                        style: "position:absolute; right:0; top:30px; z-index:40; \
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

/// The audio mode, in the bar: a dot in the colour of what is possible now
/// (Engine green, Cue amber, Remote blue) and as many words as the bar has
/// room for — `Remote · REAPER`, `Engine · loading 12/38` — and, pressed, a
/// menu that says what was asked for, what is driven, how far loading has
/// got, and offers the three modes (see [`crate::audio_mode`]).
#[component]
pub fn AudioBadge(density: Density) -> Element {
    let mut open = use_signal(|| false);
    // A note under the picker: a mode that needs another launch.
    let mut note = use_signal(|| None::<String>);
    // The mode changes off the UI thread (a loader, the attach), so it is
    // polled — cheaply, by revision.
    let revision = use_signal(crate::audio_mode::revision);
    #[cfg(feature = "native")]
    use_future(move || async move {
        let mut revision = revision;
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
            let now = crate::audio_mode::revision();
            if *revision.peek() != now {
                revision.set(now);
            }
        }
    });
    let _ = revision();
    let state = crate::audio_mode::state();
    let effective = state.effective();
    let loading =
        state.requested == crate::audio_mode::AudioMode::Engine && !state.assets.complete();
    let label = match density {
        Density::Full => state.label(2),
        Density::Compact if loading => format!(
            "{} {}/{}",
            effective.name(),
            state.assets.loaded,
            state.assets.total
        ),
        Density::Compact => state.label(1),
        Density::Narrow if loading => format!("{}/{}", state.assets.loaded, state.assets.total),
        Density::Narrow => String::new(),
    };
    let dot = effective.color();
    let target = state
        .target
        .as_ref()
        .map(crate::audio_mode::RemoteTarget::describe);
    let title = match &target {
        Some(target) => format!("Audio: {} — driving {target}", state.label(2)),
        None => format!("Audio: {}", state.label(2)),
    };
    rsx! {
        div {
            style: "position:relative; flex:none;",
            onmousedown: move |event| event.stop_propagation(),
            button {
                title: "{title}",
                style: "display:flex; align-items:center; gap:6px; height:26px; \
                        padding:0 9px; border-radius:6px; border:1px solid {RULE}; \
                        background:#0f1012; color:{TEXT}; font-size:12px; cursor:pointer; \
                        white-space:nowrap;",
                onclick: move |_| open.toggle(),
                span { style: "flex:none; width:8px; height:8px; border-radius:4px; background:{dot};" }
                if !label.is_empty() {
                    span { style: "font-weight:600;", "{label}" }
                }
            }
            if open() {
                div {
                    style: "position:absolute; right:0; top:30px; z-index:40; \
                            width:260px; padding:6px; background:{BAR_BG}; \
                            border:1px solid {RULE}; border-radius:8px; \
                            box-shadow:0 8px 24px rgba(0,0,0,0.5); font-size:12px;",
                    div {
                        style: "padding:4px 10px 6px; color:{DIM}; display:flex; flex-direction:column; gap:3px;",
                        div {
                            "Now "
                            span { style: "color:{TEXT}; font-weight:600;", "{effective.name()}" }
                            if effective != state.requested {
                                " — asked for {state.requested.name()}"
                            }
                        }
                        if let Some(target) = target.clone() {
                            div { "Driving " span { style: "color:{TEXT};", "{target}" } }
                        }
                        if loading {
                            div { "Loading media {state.assets.loaded} of {state.assets.total} — silent until each lands" }
                        }
                        if state.requested == crate::audio_mode::AudioMode::Cue && !state.cue_ready {
                            div { "No cue engine yet: Cue plays as Remote" }
                        }
                    }
                    div { style: "height:1px; margin:2px 4px 4px; background:{RULE};" }
                    for each in crate::audio_mode::AudioMode::ALL {
                        div {
                            style: option(state.requested == each),
                            onclick: move |_| {
                                let applied = pick_audio_mode(each);
                                note.set((!applied).then(|| format!("{} applies on the next launch", each.name())));
                            },
                            div { style: "font-weight:600;", "{each.name()}" }
                            div { style: "font-size:11px; color:{DIM};", "{each.blurb()}" }
                        }
                    }
                    if let Some(text) = note() {
                        div { style: "padding:6px 10px 2px; color:#e3b341; font-size:11px;", "{text}" }
                    }
                }
            }
        }
    }
}

/// Ask for `mode`: at once when it drives the same backend (Remote ⇄ Cue),
/// else remembered for the next launch (`false`).
#[cfg(feature = "native")]
fn pick_audio_mode(mode: crate::audio_mode::AudioMode) -> bool {
    crate::open::request_mode(mode)
}

/// A page cannot change what it is: it is shown, not picked.
#[cfg(not(feature = "native"))]
fn pick_audio_mode(mode: crate::audio_mode::AudioMode) -> bool {
    crate::audio_mode::state().requested == mode
}

/// A small down chevron, for a button that opens a menu.
#[component]
pub fn Chevron() -> Element {
    rsx! {
        svg {
            width: "10",
            height: "10",
            view_box: "0 0 10 10",
            fill: "none",
            path { d: "M2 3.5 L5 6.5 L8 3.5", stroke: DIM, stroke_width: "1.5", stroke_linecap: "round", stroke_linejoin: "round" }
        }
    }
}

/// Who else in the session is on a song, for its tab.
#[cfg(feature = "native")]
fn peer_dots(project: String) -> Element {
    rsx! { crate::collab_bar::PeerDots { project } }
}

#[cfg(not(feature = "native"))]
fn peer_dots(_project: String) -> Element {
    rsx! {}
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
    // The songs still on their way have tabs too, so the row keeps its
    // shape as they arrive.
    let count = list.songs.len() + list.pending.len();
    let min_w = count * 24 + 6;
    rsx! {
        div {
            // Never narrower than a dot (and who is there) per song: the
            // names give way first, then the bar's other controls.
            style: "position:relative; flex:1; min-width:{min_w}px; display:flex; height:30px; \
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
                    // A set of many songs in a narrow window: tabs too
                    // small for the usual inset keep their dot in view.
                    let pad = if count > 5 { 4 } else { 12 };
                    rsx! {
                        div {
                            key: "{song.project}",
                            style: "position:relative; flex:1; min-width:0; display:flex; \
                                    align-items:center; justify-content:center; gap:7px; \
                                    padding:0 {pad}px; border-radius:7px; background:{ground}; \
                                    border-left:1px solid {divider}; cursor:default; \
                                    overflow:hidden;",
                            onclick: move |_| {
                                if let Some(pick) = on_pick {
                                    pick.call(index);
                                }
                            },
                            // The song's colour; a click here recolours it —
                            // where a song can be recoloured, else it picks
                            // the tab like the rest of it (a page).
                            div {
                                style: "flex:none; width:9px; height:9px; border-radius:5px; \
                                        background:{color}; cursor:pointer;",
                                onclick: move |event| {
                                    if on_color.is_none() {
                                        return;
                                    }
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
                            {peer_dots(song.project.clone())}
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
            // Songs still on their way (a page opens its first song, then
            // the rest behind it): dim, named, a ring pulsing where the
            // colour dot will be.
            if !list.pending.is_empty() {
                style { "@keyframes fts-tab-pulse {{ 0%, 100% {{ opacity: .3 }} 50% {{ opacity: 1 }} }}" }
            }
            for title in list.pending.iter().cloned() {
                div {
                    key: "pending-{title}",
                    title: "{title} — loading",
                    style: "position:relative; flex:1; min-width:0; display:flex; align-items:center; \
                            justify-content:center; gap:7px; padding:0 4px; border-radius:7px; \
                            opacity:0.5; cursor:default; overflow:hidden;",
                    div {
                        style: "flex:none; width:9px; height:9px; border-radius:5px; box-sizing:border-box; \
                                border:1.5px solid {DIM}; animation:fts-tab-pulse 1.2s ease-in-out infinite;",
                    }
                    span {
                        style: "min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; \
                                color:{DIM}; font-size:12px; font-weight:600;",
                        "{title}"
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
        format!(
            "aspect-ratio:{};",
            crate::chart_panel::FITTED_WIDTH_OVER_HEIGHT
        )
    } else {
        "height:100%;".to_owned()
    };
    let chart_w = if editor.is_some() {
        "width:30%; min-width:240px;"
    } else {
        "width:38%; min-width:280px;"
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; \
                    flex-direction:column; gap:10px; padding:10px;",
            div { style: "flex:none;", onmounted: move |e| region("progress", &e), {progress} }
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

/// Whatever it holds, over one song: that song's session, as context — what
/// every panel below it reads.
#[component]
pub fn WithSong(session: crate::studio::StudioSession, children: Element) -> Element {
    use_context_provider(|| session);
    children
}

/// A view button, on or off.
#[must_use]
pub fn segment(on: bool) -> String {
    let (bg, fg) = if on {
        (ACCENT, "#0b0c0e")
    } else {
        ("transparent", DIM)
    };
    format!(
        "height:24px; padding:0 12px; border:none; border-radius:5px; cursor:pointer; \
         background:{bg}; color:{fg}; font-size:12px; font-weight:600;"
    )
}

/// A row in the mode menu.
#[must_use]
pub fn option(on: bool) -> String {
    let (bg, fg) = if on {
        ("#23262c", TEXT)
    } else {
        ("transparent", DIM)
    };
    format!(
        "padding:6px 10px; border-radius:5px; cursor:pointer; background:{bg}; color:{fg}; font-size:12px;"
    )
}
