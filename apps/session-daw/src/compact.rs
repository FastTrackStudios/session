//! The app on a small screen — a phone, or a window made that small: the
//! same views and the same components as the wide layout
//! ([`crate::shell`]), rearranged for one hand and a finger.
//!
//! What moves where:
//!
//! - **The top bar** becomes one thin line: the song, and a caret that pulls
//!   down the rest of what the wide bar shows (who is here, the audio mode,
//!   the mode) as a drawer.
//! - **The views** are five, one at a time, picked from a tab bar at the
//!   bottom (a rail down the left in landscape): Control, Chart, Lyrics,
//!   Arrangement, Mixer.
//! - **The transport** sits over the tabs: the song's section bar, slim,
//!   and the performance buttons, compact.
//! - **Control** is the setlist navigator (`session-ui`'s): each song a
//!   bar filled as far as the set has got, the one playing opened into its
//!   sections, each filled as it plays — the vertical progress bar — and
//!   the place another song is picked.
//!
//! A host decides which layout by the window's shape ([`Form::of`]) and
//! passes what is host-shaped: the other four views' panels, and what goes
//! in the drawer.

use dioxus::prelude::*;

use crate::setlist::Setlist;
use crate::shell::{ACCENT, BAR_BG, DIM, RULE, TEXT};

/// The window's shape, and so which layout the app takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    /// Room for the wide layout: the top bar and the docked views.
    Wide,
    /// A phone held upright, or a window that narrow.
    Portrait,
    /// A phone on its side, or a window that short.
    Landscape,
}

impl Form {
    /// Narrower than this (and taller than wide) is a phone held upright.
    pub const NARROW: f64 = 700.0;
    /// Shorter than this (and wider than tall) is a phone on its side.
    pub const SHORT: f64 = 500.0;

    /// The form of a window `width` × `height` logical pixels.
    #[must_use]
    pub fn of(width: f64, height: f64) -> Self {
        if width < Self::NARROW && width <= height {
            Self::Portrait
        } else if height < Self::SHORT && width > height {
            Self::Landscape
        } else {
            Self::Wide
        }
    }

    /// Whether this is the small-screen layout.
    #[must_use]
    pub const fn compact(self) -> bool {
        !matches!(self, Self::Wide)
    }
}

/// The window's form, as the host measured it (wide when it did not say).
#[must_use]
pub fn use_form() -> Form {
    try_use_context::<Signal<Form>>().map_or(Form::Wide, |form| form())
}

/// The views of the small-screen layout, in the tab bar's order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhoneView {
    Control,
    Chart,
    Lyrics,
    Arrangement,
    Mixer,
}

impl PhoneView {
    pub const ALL: [Self; 5] = [
        Self::Control,
        Self::Chart,
        Self::Lyrics,
        Self::Arrangement,
        Self::Mixer,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Control => "Control",
            Self::Chart => "Chart",
            Self::Lyrics => "Lyrics",
            Self::Arrangement => "Arrangement",
            Self::Mixer => "Mixer",
        }
    }
}

/// The height of the song line at the top.
const LINE_H: f64 = 36.0;

/// The small-screen frame, over the songs in the setlist (a
/// `Signal<Setlist>` in context, as both hosts keep it).
///
/// `body` is the current view's panel for every view but Control — the host
/// renders it (its chart, its arrangement are host-shaped) — and `drawer`
/// is what the host adds to the pulled-down top (the session bar, the audio
/// mode). `on_pick` is picking a song, as the wide layout's tabs do.
#[component]
pub fn CompactShell(
    form: Form,
    view: Signal<PhoneView>,
    on_pick: EventHandler<usize>,
    drawer: Element,
    body: Element,
) -> Element {
    let setlist: Signal<Setlist> = use_context();
    let open = use_signal(|| false);
    let current = setlist.read().current().cloned();
    let landscape = form == Form::Landscape;
    let direction = if landscape { "row" } else { "column" };
    let panel = rsx! {
        div {
            style: "position:relative; flex:1; min-height:0; min-width:0; overflow:hidden;",
            if view() == PhoneView::Control {
                crate::navigator::Navigator { on_pick }
            } else {
                {body}
            }
        }
    };
    // The section bar and the performance buttons, over the song playing.
    // In Control the navigator is the progress, and the buttons are the
    // view's own, full size.
    let control = view() == PhoneView::Control;
    let strip = match &current {
        Some(song) => rsx! {
            crate::shell::WithSong {
                key: "{song.project}",
                session: song.session.clone(),
                div {
                    style: "flex:none; display:flex; flex-direction:column; gap:4px; padding:6px 8px; \
                            background:{BAR_BG}; border-top:1px solid {RULE};",
                    if !control {
                        // Upright, the sections are too narrow to name.
                        crate::progress::ProgressBar {
                            height: "1.75rem".to_owned(),
                            labels: landscape,
                        }
                    }
                    crate::progress::TransportButtons {
                        compact: true,
                        height: if control { 60 } else { 44 },
                    }
                }
            }
        },
        None => rsx! {},
    };
    rsx! {
        // An explicit size, not insets: Blitz gives an absolutely placed box
        // stretched by `top`/`bottom` no height, and the column collapses to
        // its content (the wide shell does the same). On a phone the viewport
        // is already the safe area: Blitz's shell keeps the notch and the
        // home indicator out of it.
        div {
            style: "position:absolute; top:0; left:0; width:100vw; height:100vh; box-sizing:border-box; \
                    display:flex; \
                    flex-direction:{direction}; \
                    background:#0f1012; color:{TEXT}; font-family:system-ui, sans-serif;",
            if landscape {
                Tabs { view, rail: true }
                div {
                    style: "position:relative; flex:1; min-width:0; display:flex; flex-direction:column;",
                    SongLine { open }
                    {panel}
                    {strip}
                    if open() {
                        Drawer { open, {drawer} }
                    }
                }
            } else {
                SongLine { open }
                div {
                    style: "position:relative; flex:1; min-height:0; display:flex; flex-direction:column;",
                    {panel}
                    {strip}
                    if open() {
                        Drawer { open, {drawer} }
                    }
                }
                Tabs { view, rail: false }
            }
        }
    }
}

/// The one thin line at the top: the song, where it is in the set, and the
/// caret that pulls the rest down.
#[component]
fn SongLine(open: Signal<bool>) -> Element {
    let setlist: Signal<Setlist> = use_context();
    let list = setlist.read();
    let (name, color) = list.current().map_or((String::new(), DIM.to_owned()), |s| {
        (s.name.clone(), s.color.clone())
    });
    let place = format!("{}/{}", list.at + 1, list.songs.len() + list.pending.len());
    rsx! {
        button {
            style: "flex:none; height:{LINE_H}px; display:flex; align-items:center; gap:8px; \
                    padding:0 12px; border:none; border-bottom:1px solid {RULE}; \
                    background:{BAR_BG}; color:{TEXT}; font-family:inherit; cursor:pointer; \
                    width:100%; text-align:left;",
            onclick: move |_| open.toggle(),
            span { style: "flex:none; width:8px; height:8px; border-radius:4px; background:{color};" }
            span {
                style: "font-size:14px; font-weight:650; white-space:nowrap; overflow:hidden; \
                        text-overflow:ellipsis; min-width:0;",
                "{name}"
            }
            span { style: "flex:none; font-size:11px; color:{DIM};", "{place}" }
            span { style: "flex:1;" }
            Caret { up: open() }
        }
    }
}

/// The top pulled down: what the wide layout's bar shows beside the
/// transport, and the host's own.
#[component]
fn Drawer(open: Signal<bool>, children: Element) -> Element {
    rsx! {
        // A press outside puts it back.
        div {
            style: "position:absolute; top:{LINE_H}px; left:0; width:100%; height:calc(100% - {LINE_H}px); \
                    z-index:40; background:rgba(0,0,0,0.5);",
            onclick: move |_| open.set(false),
        }
        div {
            style: "position:absolute; top:{LINE_H}px; left:0; right:0; z-index:41; \
                    display:flex; flex-direction:column; gap:10px; padding:12px; \
                    background:{BAR_BG}; border-bottom:1px solid {RULE}; \
                    border-radius:0 0 14px 14px; box-shadow:0 16px 30px rgba(0,0,0,0.55);",
            {children}
        }
    }
}

/// The views, picked: a tab bar along the bottom, or a rail down the left.
#[component]
fn Tabs(view: Signal<PhoneView>, rail: bool) -> Element {
    let bar = if rail {
        format!(
            "flex:none; width:60px; display:flex; flex-direction:column; justify-content:center; \
             gap:4px; padding:4px; background:{BAR_BG}; border-right:1px solid {RULE};"
        )
    } else {
        format!(
            "flex:none; display:flex; padding:2px 4px 6px; background:{BAR_BG}; \
             border-top:1px solid {RULE};"
        )
    };
    rsx! {
        div {
            style: "{bar}",
            for each in PhoneView::ALL {
                button {
                    style: tab(view() == each, rail),
                    onclick: move |_| view.set(each),
                    ViewIcon { view: each }
                    if !rail {
                        span { style: "font-size:10px;", "{each.name()}" }
                    }
                }
            }
        }
    }
}

fn tab(on: bool, rail: bool) -> String {
    let fg = if on { ACCENT } else { DIM };
    let bg = if on && rail { "#1f2a3a" } else { "transparent" };
    let shape = if rail {
        "height:46px; border-radius:10px;"
    } else {
        "flex:1; min-width:0; height:50px;"
    };
    format!(
        "{shape} display:flex; flex-direction:column; align-items:center; justify-content:center; \
         gap:3px; border:none; background:{bg}; color:{fg}; font-family:inherit; \
         font-weight:{}; cursor:pointer;",
        if on { 650 } else { 500 }
    )
}

#[component]
fn ViewIcon(view: PhoneView) -> Element {
    use lucide_dioxus::{ChartNoAxesGantt, FileMusic, ListMusic, MicVocal, SlidersVertical};
    let size = 22;
    match view {
        PhoneView::Control => rsx! { ListMusic { size, color: "currentColor" } },
        PhoneView::Chart => rsx! { FileMusic { size, color: "currentColor" } },
        PhoneView::Lyrics => rsx! { MicVocal { size, color: "currentColor" } },
        PhoneView::Arrangement => rsx! { ChartNoAxesGantt { size, color: "currentColor" } },
        PhoneView::Mixer => rsx! { SlidersVertical { size, color: "currentColor" } },
    }
}

#[component]
fn Caret(up: bool) -> Element {
    let d = if up {
        "M5 12 L10 7 L15 12"
    } else {
        "M5 8 L10 13 L15 8"
    };
    rsx! {
        svg {
            width: "20",
            height: "20",
            view_box: "0 0 20 20",
            fill: "none",
            path { d, stroke: TEXT, stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Form;

    #[test]
    fn a_phone_is_compact_either_way_up_and_a_desktop_window_is_wide() {
        assert_eq!(Form::of(390.0, 844.0), Form::Portrait);
        assert_eq!(Form::of(844.0, 390.0), Form::Landscape);
        assert_eq!(Form::of(1440.0, 900.0), Form::Wide);
        // An iPad either way up, and a desktop window made narrow but tall.
        assert_eq!(Form::of(820.0, 1180.0), Form::Wide);
        assert_eq!(Form::of(1180.0, 820.0), Form::Wide);
        assert_eq!(Form::of(600.0, 900.0), Form::Portrait);
    }
}
