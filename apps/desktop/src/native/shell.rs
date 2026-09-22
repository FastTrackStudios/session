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

/// How much of the bar's left end the traffic lights take on macOS.
#[cfg(target_os = "macos")]
const LIGHTS_W: f64 = 78.0;
#[cfg(not(target_os = "macos"))]
const LIGHTS_W: f64 = 12.0;

const RULE: &str = "#2a2c31";
const TEXT: &str = "#e5e7eb";

/// The whole window.
#[component]
pub fn Shell() -> Element {
    use session_daw::shell::{OverviewLayout, TopBar, View};

    // `FTS_SESSION_VIEW` opens on a view other than the DAW.
    let view = use_signal(|| match std::env::var("FTS_SESSION_VIEW").as_deref() {
        Ok("performance") => View::Performance,
        Ok("overview") => View::Overview,
        Ok("setup") => View::Setup,
        _ => View::Daw,
    });
    // Live for now: the docked mixer's compact strips are the Live ones.
    let mode = use_signal(|| Mode::Live);
    // The mode, for the panels that change with it (the mixer's strips
    // are live-mode strips in Live).
    use_context_provider(|| mode);
    let window = dioxus_native::use_window();
    let (dragging, zooming) = (window.clone(), window);
    rsx! {
        style { {TAILWIND} }
        div {
            style: "position:absolute; top:0; left:0; width:100vw; height:100vh; display:flex; flex-direction:column; \
                    background:#0f1012; color:{TEXT}; font-family:system-ui, sans-serif;",
            TopBar {
                view,
                mode,
                lights: LIGHTS_W,
                transport: rsx! { session_daw::transport_bar::TransportBar {} },
                // Anywhere on the bar that is not a control drags the
                // window; a double click zooms it.
                on_drag: move |()| {
                    if let Err(e) = dragging.drag_window() {
                        tracing::debug!(error = %e, "window drag refused");
                    }
                },
                on_zoom: move |()| zooming.set_maximized(!zooming.is_maximized()),
            }
            div {
                style: "position:relative; flex:1; min-height:0;",
                match view() {
                    View::Setup => rsx! { session_daw::setup::SetupView {} },
                    View::Daw => rsx! { DawView {} },
                    View::Performance => rsx! { PerformanceView {} },
                    View::Overview => rsx! {
                        OverviewLayout {
                            progress: rsx! { session_daw::progress::ProgressBar {} },
                            chart: rsx! { session_daw::chart_panel::Chart {} },
                            panels: rsx! { session_daw::mixer_panel::DawPanels { docked: true } },
                        }
                    },
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
