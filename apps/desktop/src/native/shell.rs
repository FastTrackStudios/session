//! The app's frame: one top bar over whichever view is up.
//!
//! The top bar is also the window's title bar. On macOS the traffic lights
//! sit in its left end (the window's own title bar is transparent — see
//! `super::window_attributes`), then the views, then the mode; the rest of
//! it is a drag surface. The mode lives here and not in the DAW's toolbar,
//! so it is visible whatever the view.
//!
//! Inline styles throughout: Blitz and external style sheets do not mix
//! (see the repo's CLAUDE.md). The compiled Tailwind the performance panels
//! were written against goes in once, as a plain `<style>` element.

use dioxus::prelude::*;

use session::modes::Mode;

/// The Session app's compiled Tailwind — what `session-ui`'s panels are
/// styled with. A plain `style { }` element, which Blitz reads; a
/// `document::Style` goes through a window head (see `docs/app-on-blitz.md`).
const TAILWIND: &str = include_str!("../../assets/tailwind-signal.css");

/// How tall the top bar is — room for the traffic lights with air around.
const BAR_H: f64 = 40.0;

/// How much of the bar's left end the traffic lights take on macOS.
#[cfg(target_os = "macos")]
const LIGHTS_W: f64 = 78.0;
#[cfg(not(target_os = "macos"))]
const LIGHTS_W: f64 = 12.0;

const BAR_BG: &str = "#17181b";
const RULE: &str = "#2a2c31";
const TEXT: &str = "#e5e7eb";
const DIM: &str = "#8b9099";
const ACCENT: &str = "#3aa0ff";

/// The views the top bar switches between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Performance,
    Daw,
}

impl View {
    const ALL: [Self; 2] = [Self::Performance, Self::Daw];

    const fn name(self) -> &'static str {
        match self {
            Self::Performance => "Performance",
            Self::Daw => "DAW",
        }
    }
}

/// The whole window.
#[component]
pub fn Shell() -> Element {
    // `FTS_SESSION_VIEW=performance` opens on the performance view.
    let view = use_signal(|| match std::env::var("FTS_SESSION_VIEW").as_deref() {
        Ok("performance") => View::Performance,
        _ => View::Daw,
    });
    // Live for now: the docked mixer's compact strips are the Live ones.
    let mode = use_signal(|| Mode::Live);
    // The mode, for the panels that change with it (the mixer's strips
    // are live-mode strips in Live).
    use_context_provider(|| mode);
    rsx! {
        style { {TAILWIND} }
        div {
            style: "position:absolute; top:0; left:0; width:100vw; height:100vh; display:flex; flex-direction:column; \
                    background:#0f1012; color:{TEXT}; font-family:system-ui, sans-serif;",
            TopBar { view, mode }
            div {
                style: "position:relative; flex:1; min-height:0;",
                match view() {
                    View::Daw => rsx! { DawView {} },
                    View::Performance => rsx! { PerformanceView {} },
                }
            }
        }
    }
}

/// The DAW view: the arrangement, with the mixer docked under it on `x`.
/// The transport is in the top bar; the editor joins as a panel once the
/// dock is in (phase 3).
#[component]
fn DawView() -> Element {
    rsx! {
        session_daw::mixer_panel::DawPanels {}
    }
}

/// The performance view: the song's progress, its chart live with the
/// playhead, and — once it exists — the lyrics beside it.
#[component]
fn PerformanceView() -> Element {
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; \
                    flex-direction:column; gap:16px; padding:16px;",
            session_daw::progress::ProgressBar {}
            div {
                style: "position:relative; flex:1; min-height:0; border-radius:8px; \
                        overflow:hidden; border:1px solid {RULE};",
                session_daw::chart_panel::Chart {}
            }
            session_daw::progress::TransportButtons {}
        }
    }
}

/// The top bar: the lights' corner, the views, the mode, settings. Anywhere
/// that is not a control drags the window; a double click zooms it.
#[component]
fn TopBar(view: Signal<View>, mode: Signal<Mode>) -> Element {
    let window = dioxus_native::use_window();
    let dragging = window.clone();
    let zooming = window;
    let mut picking = use_signal(|| false);
    rsx! {
        div {
            style: "position:relative; height:{BAR_H}px; flex:none; display:flex; \
                    align-items:center; gap:8px; padding-left:{LIGHTS_W}px; \
                    padding-right:10px; background:{BAR_BG}; \
                    border-bottom:1px solid {RULE};",
            // The drag surface: the bar itself. Controls stop the press
            // from reaching it.
            onmousedown: move |_| {
                if let Err(e) = dragging.drag_window() {
                    tracing::debug!(error = %e, "window drag refused");
                }
            },
            ondoubleclick: move |_| zooming.set_maximized(!zooming.is_maximized()),
            span {
                style: "font-size:13px; font-weight:600; color:{TEXT}; margin-right:8px;",
                "Session"
            }
            // The views, as a segmented control.
            div {
                style: "display:flex; gap:2px; padding:2px; background:#0f1012; \
                        border:1px solid {RULE}; border-radius:7px;",
                for each in View::ALL {
                    button {
                        style: segment(view() == each),
                        onmousedown: move |event| event.stop_propagation(),
                        onclick: move |_| view.set(each),
                        "{each.name()}"
                    }
                }
            }
            div { style: "flex:1;" }
            // The transport, right-aligned with the mode and Settings —
            // one cluster at the end of the bar, rather than the bar's own
            // title/view switch. In the bar rather than along the foot of
            // a view: one place for it whatever the view, and no bottom
            // rail.
            session_daw::transport_bar::TransportBar {}
            // The mode, visible in every view.
            div {
                style: "position:relative;",
                onmousedown: move |event| event.stop_propagation(),
                button {
                    style: "display:flex; align-items:center; gap:6px; height:26px; \
                            padding:0 10px; border-radius:6px; border:1px solid {RULE}; \
                            background:#0f1012; color:{TEXT}; font-size:12px;",
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
            button {
                style: "height:26px; padding:0 10px; border-radius:6px; border:1px solid {RULE}; \
                        background:#0f1012; color:{DIM}; font-size:12px;",
                onmousedown: move |event| event.stop_propagation(),
                "Settings"
            }
        }
    }
}

fn segment(on: bool) -> String {
    let (bg, fg) = if on { (ACCENT, "#0b0c0e") } else { ("transparent", DIM) };
    format!(
        "height:24px; padding:0 12px; border:none; border-radius:5px; \
         background:{bg}; color:{fg}; font-size:12px; font-weight:600;"
    )
}

fn option(on: bool) -> String {
    let (bg, fg) = if on { ("#23262c", TEXT) } else { ("transparent", DIM) };
    format!("padding:6px 10px; border-radius:5px; background:{bg}; color:{fg}; font-size:12px;")
}
