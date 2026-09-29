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
    /// The song's lyrics, the whole window.
    Lyrics,
    /// The mixer alone, the whole window.
    Mixer,
    /// The audio and MIDI editor (the expression editor, brought in
    /// next): a place held for it.
    Editor,
}

impl View {
    pub const ALL: [Self; 8] = [
        Self::Performance,
        Self::Overview,
        Self::Chart,
        Self::Lyrics,
        Self::Daw,
        Self::Mixer,
        Self::Editor,
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
            Self::Lyrics => "Lyrics",
            Self::Mixer => "Mixer",
            Self::Editor => "Editor",
        }
    }

    /// The word under the view's icon in the bottom bar: the name, cut to
    /// a verb where the name is long.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Performance => "Perform",
            Self::Daw => "Arrange",
            other => other.name(),
        }
    }
}

/// Back out of the set to where sets are picked (the start screen, and
/// its library), when the host has one: a context the host provides while
/// a set is open. The top bar shows a Back button while it is there.
#[derive(Clone, Copy, PartialEq)]
pub struct Back(pub Callback<()>);

/// The Editor view, until the audio and MIDI editor (the expression
/// editor) is brought in: a place held for it, saying so.
#[component]
pub fn EditorComing() -> Element {
    rsx! {
        div {
            style: "position:absolute; inset:0; display:flex; flex-direction:column; align-items:center; \
                    justify-content:center; gap:10px; color:{DIM}; font-family:system-ui, sans-serif;",
            lucide_dioxus::AudioWaveform { size: 40, color: "currentColor" }
            span { style: "font-size:16px; font-weight:650; color:{TEXT};", "Editor" }
            span {
                style: "font-size:13px; max-width:420px; text-align:center; line-height:1.5;",
                "The audio and MIDI editor comes here: notes, audio, automation, \
                 edited up close for the selected item."
            }
        }
    }
}

/// What stays on screen over every view — the song's progress along the
/// top, the performance transport along the foot — each a setting a
/// button in the bottom bar (and a switch on the Setup page) turns on;
/// a context the shell provides.
#[derive(Clone, Copy, PartialEq)]
pub struct Pins {
    pub progress: Signal<bool>,
    pub transport: Signal<bool>,
    /// Locked: drags scroll, and move nothing (`crate::options::LOCKING`,
    /// which this mirrors so the buttons that show it re-render).
    pub lock: Signal<bool>,
    /// The inspector down the arrangement's left: the selected track's
    /// strip, and its folder's.
    pub inspector: Signal<bool>,
}

impl Pins {
    #[must_use]
    pub fn new() -> Self {
        Self {
            progress: Signal::new(false),
            transport: Signal::new(false),
            lock: Signal::new(crate::options::LOCKING.get()),
            inspector: Signal::new(true),
        }
    }
}

impl Default for Pins {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a view carries the progress bar and the transport already
/// (Performance, Overview), or is not about the song (Setup).
const fn carries_its_own(view: View) -> bool {
    matches!(view, View::Performance | View::Overview | View::Setup)
}

/// The song's progress bar over a view, when it is pinned: where the song
/// is and what comes next, whatever else is being looked at — and a
/// section pressed on it plays from there, from any view.
#[component]
pub fn PinnedProgress(view: View) -> Element {
    let pinned = try_use_context::<Pins>().is_some_and(|pins| (pins.progress)());
    if !pinned || carries_its_own(view) {
        return rsx! {};
    }
    rsx! {
        div {
            style: "flex:none; padding:8px 10px 0;",
            crate::progress::ProgressBar { height: "3rem".to_owned() }
        }
    }
}

/// The performance transport under a view, when it is pinned: back a
/// section, play, loop (record, in Record mode), on a section.
#[component]
pub fn PinnedTransport(view: View) -> Element {
    let pinned = try_use_context::<Pins>().is_some_and(|pins| (pins.transport)());
    let record = try_use_context::<Signal<Mode>>().is_some_and(|mode| mode() == Mode::Record);
    if !pinned || carries_its_own(view) {
        return rsx! {};
    }
    rsx! {
        div {
            style: "flex:none; padding:6px 10px 8px;",
            crate::progress::TransportButtons { record }
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

/// How tall the bottom bar is: an icon with its word under it, for a
/// finger.
pub const BOTTOM_H: f64 = 54.0;

/// How wide the bottom bar's left end is — the menu, the inspector, Setup
/// and the view's own controls there — kept the same whatever the view,
/// so the views beside it stay put as they are switched between. Narrow,
/// the view's controls there go (they are in the menu too).
const LEFT_W: f64 = 244.0;
const LEFT_W_NARROW: f64 = 150.0;

/// How wide a view's button is, with its word under the icon.
const VIEW_W: f64 = 56.0;

/// How wide the mode picker is.
const MODE_W: f64 = 96.0;

/// Below this the bar is narrow (a tablet held upright).
const BOTTOM_WIDE: f64 = 1000.0;

/// The bottom bar: the views as icons across the foot of the window, the
/// way the top bar runs across its head — Logic's iPad layout, where what
/// you look at is picked at the bottom. Each view an icon with its word
/// under it, in the middle; at the left the menu (the window's switches:
/// lock, progress, transport), the inspector and Setup; at the right the
/// mode. Either side of the views is the view's own: the arrangement's
/// transport, where it is and its tempo and key; the chart's pages; the
/// mixer's folders ([`LeftContext`], [`RightContext`]).
///
/// On a touchscreen there is no Overview: it is every view at once, which
/// a tablet or a phone has no room for, and each of its parts is a view
/// of its own here.
#[component]
pub fn BottomBar(
    view: Signal<View>,
    mode: Signal<Mode>,
    /// The host's transport, for the arrangement's context — sized for a
    /// finger (`big`).
    #[props(default)]
    transport: Option<Element>,
    /// How wide the bar is, in logical pixels, when the host knows.
    #[props(default)]
    width: Option<f64>,
) -> Element {
    let touch = crate::touch::use_touch();
    let pins = try_use_context::<Pins>();
    let mut menu = use_signal(|| false);
    let mut picking = use_signal(|| false);
    let views: Vec<View> = View::ALL
        .into_iter()
        .filter(|v| *v != View::Setup && !(touch && *v == View::Overview))
        .collect();
    let wide = width.is_none_or(|w| w >= BOTTOM_WIDE);
    let left_w = if wide { LEFT_W } else { LEFT_W_NARROW };
    // What the right side's controls have, and so how much of the
    // transport is spelled out there.
    #[allow(clippy::cast_precision_loss)]
    let room = width.map_or(f64::INFINITY, |w| {
        w - 24.0 - left_w - views.len() as f64 * (VIEW_W + 4.0) - MODE_W - 20.0
    });
    let density = if room >= 510.0 {
        Density::Full
    } else if room >= 400.0 {
        Density::Compact
    } else {
        Density::Narrow
    };
    let mut shared = use_context_provider(|| Signal::new(density));
    use_effect(use_reactive!(|density| {
        if *shared.peek() != density {
            shared.set(density);
        }
    }));
    let button = move |each: View| {
        rsx! {
            button {
                key: "{each.name()}",
                title: each.name(),
                style: bottom_button(view() == each, true),
                onclick: move |_| view.set(each),
                ViewIcon { view: each }
                span { style: LABEL, "{each.label()}" }
            }
        }
    };
    let showing = view();
    rsx! {
        div {
            style: "position:relative; height:{BOTTOM_H}px; flex:none; display:flex; align-items:center; \
                    gap:4px; padding:0 12px; background:{BAR_BG}; border-top:1px solid {RULE};",
            // The menu (the window's switches), the inspector, Setup (the
            // set and the song's details, set up before the views are
            // used), and the view's own controls.
            div {
                style: "flex:none; width:{left_w}px; display:flex; align-items:center; gap:4px; overflow:hidden;",
                button {
                    title: "Settings",
                    style: bottom_button(menu(), true),
                    onclick: move |_| {
                        menu.toggle();
                        picking.set(false);
                    },
                    lucide_dioxus::Menu { size: 19, color: "currentColor" }
                    span { style: LABEL, "Menu" }
                }
                if let Some(pins) = pins {
                    button {
                        title: "The inspector: the selected track's strip, down the arrangement's left",
                        style: bottom_button((pins.inspector)(), true),
                        onclick: move |_| {
                            let mut on = pins.inspector;
                            on.toggle();
                        },
                        lucide_dioxus::PanelLeft { size: 19, color: "currentColor" }
                        span { style: LABEL, "Inspect" }
                    }
                }
                {button(View::Setup)}
                if wide {
                    Divider {}
                    LeftContext { view: showing }
                }
            }
            div {
                style: "flex:none; display:flex; align-items:center; gap:4px;",
                for each in views {
                    {button(each)}
                }
            }
            // The view's own controls, then the mode.
            div {
                style: "flex:1; min-width:0; display:flex; justify-content:flex-end; align-items:center; \
                        gap:6px; overflow:hidden;",
                RightContext { view: showing, transport }
            }
            Divider {}
            ModePicker { mode, picking, menu }
            if menu() || picking() {
                // A press outside closes it.
                div {
                    style: "position:fixed; top:0; left:0; width:100vw; height:100vh; z-index:40;",
                    onclick: move |_| {
                        menu.set(false);
                        picking.set(false);
                    },
                }
            }
            if menu() && let Some(pins) = pins {
                div {
                    style: "position:absolute; left:10px; bottom:{BOTTOM_H + 6.0}px; z-index:41; width:300px; \
                            padding:6px; border-radius:12px; background:{BAR_BG}; border:1px solid {RULE}; \
                            box-shadow:0 10px 30px rgba(0,0,0,0.5); display:flex; flex-direction:column; gap:2px;",
                    MenuSwitch {
                        on: pins.lock,
                        label: "Lock",
                        detail: "A drag scrolls and moves nothing — items, faders, knobs",
                        what: Pinned::Lock,
                    }
                    MenuSwitch {
                        on: pins.progress,
                        label: "Song progress on every view",
                        detail: "The sections across the top — press one to play from it",
                        what: Pinned::Progress,
                    }
                    MenuSwitch {
                        on: pins.transport,
                        label: "Transport on every view",
                        detail: "Back, Play, Loop and Advance along the foot",
                        what: Pinned::Transport,
                    }
                }
            }
        }
    }
}

/// A word under a bottom-bar icon.
const LABEL: &str = "font-size:10px; line-height:12px; font-weight:600; white-space:nowrap;";

/// A thin rule between the bottom bar's parts.
#[component]
fn Divider() -> Element {
    rsx! {
        div { style: "flex:none; width:1px; height:30px; margin:0 4px; background:{RULE};" }
    }
}

/// The mode, at the bottom bar's right end: what it is, and every mode
/// to pick from over it.
#[component]
fn ModePicker(mode: Signal<Mode>, picking: Signal<bool>, menu: Signal<bool>) -> Element {
    let mut mode = mode;
    let mut picking = picking;
    let mut menu = menu;
    rsx! {
        div {
            style: "position:relative; flex:none; width:{MODE_W}px; display:flex; justify-content:flex-end;",
            button {
                title: "The mode: what the window is set up for",
                style: "height:44px; min-width:88px; box-sizing:border-box; display:flex; flex-direction:column; \
                        align-items:center; justify-content:center; gap:2px; padding:0 10px; border-radius:9px; \
                        border:1px solid {RULE}; background:#0f1012; color:{TEXT}; cursor:pointer; \
                        font-family:inherit;",
                onclick: move |_| {
                    picking.toggle();
                    menu.set(false);
                },
                span { style: "font-size:13px; line-height:15px; font-weight:700;", "{mode().display_name()}" }
                span { style: "{LABEL} color:{DIM};", "Mode" }
            }
            if picking() {
                div {
                    style: "position:absolute; right:0; bottom:{BOTTOM_H - 2.0}px; z-index:41; \
                            min-width:180px; padding:6px; background:{BAR_BG}; \
                            border:1px solid {RULE}; border-radius:12px; display:flex; \
                            flex-direction:column; gap:2px; box-shadow:0 10px 30px rgba(0,0,0,0.5);",
                    for each in Mode::ALL {
                        button {
                            style: mode_option(mode() == each),
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

/// A mode in the picker: a row a finger can hit.
fn mode_option(on: bool) -> String {
    let (bg, fg) = if on {
        ("#2a2d33", TEXT)
    } else {
        ("transparent", DIM)
    };
    format!(
        "min-height:44px; padding:0 14px; border:none; border-radius:8px; text-align:left; \
         background:{bg}; color:{fg}; font-family:inherit; font-size:14px; font-weight:600; cursor:pointer;"
    )
}

/// The view's own controls at the bottom bar's left, beside Setup: the
/// lock where things can be dragged (the arrangement, the mixer).
#[component]
fn LeftContext(view: View) -> Element {
    let Some(pins) = try_use_context::<Pins>() else {
        return rsx! {};
    };
    match view {
        View::Daw | View::Mixer => rsx! { LockButton { lock: pins.lock } },
        _ => rsx! {},
    }
}

/// The lock, as a button: locked, a drag scrolls and moves nothing.
#[component]
fn LockButton(lock: Signal<bool>) -> Element {
    let mut lock = lock;
    let on = lock();
    rsx! {
        button {
            title: "Locked: a drag scrolls and moves nothing",
            style: bottom_button(on, true),
            onclick: move |_| {
                lock.toggle();
                crate::options::LOCKING.set(lock());
            },
            if on {
                lucide_dioxus::Lock { size: 19, color: "currentColor" }
            } else {
                lucide_dioxus::LockOpen { size: 19, color: "currentColor" }
            }
            span { style: LABEL, "Lock" }
        }
    }
}

/// The view's own controls at the bottom bar's right, beside the mode:
/// the arrangement's transport, where it is and its tempo and key; the
/// chart's pages; the mixer's folders.
#[component]
fn RightContext(view: View, transport: Option<Element>) -> Element {
    match view {
        View::Daw => rsx! {
            if let Some(transport) = transport {
                div {
                    style: "flex:none; display:flex; align-items:center; height:44px;",
                    {transport}
                }
            }
        },
        View::Chart => rsx! { ChartPages {} },
        View::Mixer => rsx! { FolderSwitch {} },
        _ => rsx! {},
    }
}

/// The chart's pages: the one before, back to the one being played, the
/// one after.
#[component]
fn ChartPages() -> Element {
    use crate::chart_panel::{Turn, turn};
    rsx! {
        button {
            title: "The page before",
            style: bottom_button(false, true),
            onclick: move |_| turn(Turn::Back),
            lucide_dioxus::ChevronLeft { size: 22, color: "currentColor" }
            span { style: LABEL, "Page" }
        }
        button {
            title: "Back to the page being played, following the song",
            style: bottom_button(false, true),
            onclick: move |_| turn(Turn::Follow),
            lucide_dioxus::Crosshair { size: 19, color: "currentColor" }
            span { style: LABEL, "Follow" }
        }
        button {
            title: "The page after",
            style: bottom_button(false, true),
            onclick: move |_| turn(Turn::On),
            lucide_dioxus::ChevronRight { size: 22, color: "currentColor" }
            span { style: LABEL, "Page" }
        }
    }
}

/// Every top-level folder folded, or all of them open (`crate::folds`).
#[component]
fn FolderSwitch() -> Element {
    let mut folded = use_signal(crate::folds::tops);
    rsx! {
        button {
            title: if folded() { "Open every folder" } else { "Fold every top-level folder" },
            style: bottom_button(folded(), true),
            onclick: move |_| {
                let on = !folded();
                crate::folds::set_tops(on);
                folded.set(on);
            },
            if folded() {
                lucide_dioxus::FolderClosed { size: 19, color: "currentColor" }
            } else {
                lucide_dioxus::FolderOpen { size: 19, color: "currentColor" }
            }
            span { style: LABEL, if folded() { "Folded" } else { "Open" } }
        }
    }
}

/// What a switch in the bottom bar's menu switches.
#[derive(Clone, Copy, PartialEq)]
enum Pinned {
    Progress,
    Transport,
    Lock,
}

/// A switch in the bottom bar's menu: its words, and a switch a finger can
/// hit.
#[component]
fn MenuSwitch(
    on: Signal<bool>,
    label: &'static str,
    detail: &'static str,
    what: Pinned,
) -> Element {
    let mut on = on;
    let lit = on();
    let (track, knob) = if lit {
        ("#2563eb", "18px")
    } else {
        ("#3a3d44", "2px")
    };
    rsx! {
        button {
            style: "display:flex; align-items:center; gap:12px; width:100%; min-height:48px; padding:6px 10px; \
                    border:none; border-radius:8px; background:transparent; color:{TEXT}; text-align:left; \
                    font-family:inherit; cursor:pointer;",
            onclick: move |_| {
                on.toggle();
                if what == Pinned::Lock {
                    crate::options::LOCKING.set(on());
                }
            },
            div {
                style: "flex:1; min-width:0; display:flex; flex-direction:column; gap:2px;",
                span { style: "font-size:13px; font-weight:600;", "{label}" }
                span { style: "font-size:11px; color:{DIM}; line-height:1.4;", "{detail}" }
            }
            div {
                style: "position:relative; flex:none; width:38px; height:22px; border-radius:11px; background:{track};",
                div { style: "position:absolute; top:2px; left:{knob}; width:18px; height:18px; border-radius:9px; background:#f3f4f6;" }
            }
        }
    }
}

/// A bottom-bar button: a grey icon with its word under it, or, for what
/// is showing, white on a light pill; `label` false, the icon alone.
fn bottom_button(on: bool, label: bool) -> String {
    let (fg, bg) = if on {
        (TEXT, "#3a3d44")
    } else {
        (DIM, "transparent")
    };
    let shape = if label {
        "min-width:52px; padding:0 6px; flex-direction:column; gap:3px;"
    } else {
        "width:44px;"
    };
    format!(
        "{shape} height:46px; flex:none; box-sizing:border-box; display:flex; align-items:center; \
         justify-content:center; border:none; border-radius:9px; background:{bg}; color:{fg}; \
         cursor:pointer; font-family:inherit;"
    )
}

/// A view's icon, as the bottom bar shows it.
#[component]
fn ViewIcon(view: View) -> Element {
    use lucide_dioxus::{
        ChartNoAxesGantt, FileMusic, LayoutDashboard, ListMusic, MicVocal, Settings,
        SlidersVertical,
    };
    let size = 19;
    match view {
        View::Performance => rsx! { ListMusic { size, color: "currentColor" } },
        View::Overview => rsx! { LayoutDashboard { size, color: "currentColor" } },
        View::Chart => rsx! { FileMusic { size, color: "currentColor" } },
        View::Lyrics => rsx! { MicVocal { size, color: "currentColor" } },
        View::Editor => rsx! { lucide_dioxus::AudioWaveform { size, color: "currentColor" } },
        View::Daw => rsx! { ChartNoAxesGantt { size, color: "currentColor" } },
        View::Mixer => rsx! { SlidersVertical { size, color: "currentColor" } },
        View::Setup => rsx! { Settings { size, color: "currentColor" } },
    }
}

/// The top bar: back to the sets, the host's badges (who is here, where
/// the sound comes from), and the setlist's tabs. The transport and the
/// mode are in the bottom bar ([`BottomBar`]).
///
/// `lights` is how much of the left end belongs to the window's own
/// controls (the traffic lights on macOS, nothing in a page), and
/// `on_drag` / `on_zoom` are what a window does with a press on the bar
/// and a double click — a page passes neither.
#[component]
pub fn TopBar(
    /// The host's badges (the collaboration bar).
    badges: Element,
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
    let density = width.map_or(Density::Full, Density::for_width);
    let mut shared = use_context_provider(|| Signal::new(density));
    use_effect(use_reactive!(|density| {
        if *shared.peek() != density {
            shared.set(density);
        }
    }));
    let back = try_use_context::<Back>();
    let controls = rsx! {
        // A row: whatever the host puts here (the collaboration bar).
        div {
            style: "flex:none; display:flex; align-items:center;",
            onmousedown: move |event| event.stop_propagation(),
            {badges}
        }
        // Where the sound comes from: Engine / Cue / Remote.
        AudioBadge { density }
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
            if let Some(Back(back)) = back {
                button {
                    title: "Back to your sets",
                    style: "flex:none; width:32px; height:28px; display:flex; align-items:center; \
                            justify-content:center; padding:0; border-radius:7px; border:1px solid {RULE}; \
                            background:#0f1012; color:{TEXT}; cursor:pointer;",
                    onmousedown: move |event| event.stop_propagation(),
                    onclick: move |_| back.call(()),
                    lucide_dioxus::ChevronLeft { size: 18, color: "currentColor" }
                }
            }
            // The controls first, where they stay put, and the setlist
            // after them filling whatever the bar has left — a longer set
            // grows into the right, not into the transport.
            {controls}
            SongTabs { on_pick, on_color, max_shown: tabs_shown(width, crate::touch::use_touch()) }
        }
    }
}

/// How many song tabs the top bar shows at once: on a touchscreen, five
/// across a tablet on its side, three held upright, the current and next
/// on a phone; with a mouse, as many as fit at a readable width.
fn tabs_shown(width: Option<f64>, touch: bool) -> usize {
    /// The narrowest a tab is and still names its song.
    const TAB_MIN: f64 = 110.0;
    /// What the bar's other controls take.
    const CONTROLS: f64 = 560.0;
    let Some(width) = width else { return 0 };
    if width < 700.0 {
        2
    } else if touch {
        if width >= 1000.0 { 5 } else { 3 }
    } else {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a tab count"
        )]
        let fit = ((width - CONTROLS) / TAB_MIN).floor().max(3.0) as usize;
        fit
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
    /// The most tabs shown at once: a longer set shows the ones around the
    /// current song, with the previous and next song a press away either
    /// side and the whole set in a menu. 0 shows every song.
    #[props(default)]
    max_shown: usize,
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
    let mut listing = use_signal(|| false);
    let list = setlist();
    if list.songs.is_empty() {
        return rsx! { div { style: "flex:1;" } };
    }
    let at = reading().at;
    // The songs still on their way have tabs too, so the row keeps its
    // shape as they arrive.
    let count = list.songs.len() + list.pending.len();
    // Which tabs are shown: every one, or — a set longer than the room —
    // a window round the current song: the one before it, it, and those
    // after, as many as fit.
    let cap = if max_shown == 0 {
        count
    } else {
        max_shown.min(count)
    };
    let windowed = cap < count;
    let start = if windowed {
        let before = if cap <= 2 { 0 } else { (cap - 1) / 2 };
        list.at.saturating_sub(before).min(count - cap)
    } else {
        0
    };
    let shown = start..start + cap;
    let min_w = cap * 24 + 6;
    let pick = move |index: usize| {
        if let Some(pick) = on_pick {
            pick.call(index);
        }
    };
    let songs_len = list.songs.len();
    let current = list.at;
    let arrow = |enabled: bool| {
        let ink = if enabled { TEXT } else { "#3a3d44" };
        format!(
            "flex:none; width:28px; height:30px; display:flex; align-items:center; \
             justify-content:center; border-radius:8px; border:1px solid {RULE}; \
             background:#0f1012; color:{ink}; cursor:pointer; padding:0;"
        )
    };
    let tabs = rsx! {
        div {
            // Never narrower than a dot (and who is there) per song: the
            // names give way first, then the bar's other controls.
            style: "position:relative; flex:1; min-width:{min_w}px; display:flex; height:30px; \
                    padding:2px; gap:0; align-items:stretch; background:#0f1012; \
                    border:1px solid {RULE}; border-radius:9px;",
            onmousedown: move |event| event.stop_propagation(),
            for (index, song) in list.songs.iter().cloned().enumerate().filter(|(i, _)| shown.contains(i)) {
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
            for title in list.pending.iter().cloned().enumerate().filter(|(i, _)| shown.contains(&(songs_len + i))).map(|(_, t)| t) {
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
                    left: format!(
                        "{:.3}%",
                        (index.saturating_sub(start) as f64 + 0.5) / cap as f64 * 100.0
                    ),
                    on_pick: move |choice: Option<String>| {
                        coloring.set(None);
                        if let Some(color) = on_color {
                            color.call((index, choice));
                        }
                    },
                }
            }
        }
    };
    if !windowed {
        return tabs;
    }
    rsx! {
        div {
            style: "position:relative; flex:1; min-width:0; display:flex; align-items:center; gap:4px;",
            onmousedown: move |event| event.stop_propagation(),
            button {
                title: "The song before",
                style: arrow(current > 0),
                onclick: move |_| {
                    if current > 0 {
                        pick(current - 1);
                    }
                },
                lucide_dioxus::ChevronLeft { size: 16, color: "currentColor" }
            }
            {tabs}
            button {
                title: "The song after",
                style: arrow(current + 1 < songs_len),
                onclick: move |_| {
                    if current + 1 < songs_len {
                        pick(current + 1);
                    }
                },
                lucide_dioxus::ChevronRight { size: 16, color: "currentColor" }
            }
            button {
                title: "The whole set",
                style: arrow(true),
                onclick: move |_| listing.toggle(),
                lucide_dioxus::ChevronDown { size: 16, color: "currentColor" }
            }
            if listing() {
                div {
                    style: "position:fixed; top:0; left:0; width:100vw; height:100vh; z-index:44;",
                    onclick: move |_| listing.set(false),
                }
                div {
                    style: "position:absolute; right:0; top:36px; z-index:45; width:280px; max-height:60vh; \
                            overflow-y:auto; padding:4px; background:{BAR_BG}; border:1px solid {RULE}; \
                            border-radius:10px; box-shadow:0 10px 30px rgba(0,0,0,0.5); \
                            display:flex; flex-direction:column; gap:2px;",
                    for (index, song) in list.songs.iter().cloned().enumerate() {
                        button {
                            key: "{song.project}",
                            style: {
                                let (bg, ink) = if index == current { ("#2b2e35", TEXT) } else { ("transparent", DIM) };
                                format!("display:flex; align-items:center; gap:10px; min-height:40px; padding:0 10px; \
                                         border:none; border-radius:7px; background:{bg}; color:{ink}; \
                                         font-family:inherit; font-size:13px; font-weight:600; text-align:left; cursor:pointer;")
                            },
                            onclick: move |_| {
                                listing.set(false);
                                pick(index);
                            },
                            span { style: "flex:none; width:18px; color:#6b7280; font-size:11px;", "{index + 1}" }
                            div { style: "flex:none; width:9px; height:9px; border-radius:5px; background:{song.color};" }
                            span { style: "flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", "{song.name}" }
                        }
                    }
                    for title in list.pending.iter().cloned() {
                        div {
                            key: "list-pending-{title}",
                            style: "display:flex; align-items:center; gap:10px; min-height:36px; padding:0 10px 0 38px; \
                                    color:#6b7280; font-size:13px; opacity:0.6;",
                            "{title} — loading"
                        }
                    }
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
