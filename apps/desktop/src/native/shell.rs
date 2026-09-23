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
    use session_daw::shell::{TopBar, View};

    // `FTS_SESSION_VIEW` opens on a view other than the DAW.
    let view = use_signal(|| match std::env::var("FTS_SESSION_VIEW").as_deref() {
        Ok("performance") => View::Performance,
        Ok("overview") => View::Overview,
        Ok("setup") => View::Setup,
        _ => View::Daw,
    });
    // Live unless `FTS_SESSION_MODE` names another (`organize`, …): the
    // docked mixer's compact strips are the Live ones.
    let mode = use_signal(|| {
        std::env::var("FTS_SESSION_MODE")
            .ok()
            .and_then(|name| Mode::ALL.into_iter().find(|m| m.display_name().eq_ignore_ascii_case(&name)))
            .unwrap_or(Mode::Live)
    });
    // The mode, for the panels that change with it (the mixer's strips
    // are live-mode strips in Live).
    use_context_provider(|| mode);
    // The chart's text editor, beside the chart in the Overview: open in
    // Organize, closed in the other modes, and the button on the chart's
    // corner opens or closes it in any of them. Out here, not per song, so
    // picking another song keeps it as it was.
    let mut editor_open = use_signal(|| *mode.peek() == Mode::Organize);
    use_effect(move || editor_open.set(mode() == Mode::Organize));
    // Whether the mixer is open, per mode (Organize starts closed) — above
    // the songs, so it holds across them.
    use_context_provider(session_daw::mixer_panel::MixerMemory::new);
    // The songs, as the launch opened them — a signal from here on, which
    // the tabs read and a pick or a recolour writes.
    let opened: session_daw::setlist::Setlist = use_context();
    let mut setlist = use_context_provider(|| Signal::new(opened));
    use_live_advance(setlist, mode);
    // Space plays and stops whatever has the focus.
    session_daw::keys::use_window_transport_keys();
    let window = dioxus_native::use_window();
    let (dragging, zooming) = (window.clone(), window);
    let current = setlist.read().current().cloned();
    rsx! {
        style { {TAILWIND} }
        div {
            style: "position:absolute; top:0; left:0; width:100vw; height:100vh; display:flex; flex-direction:column; \
                    background:#0f1012; color:{TEXT}; font-family:system-ui, sans-serif;",
            // A press anywhere but the chart editor (which stops it) gives
            // the keyboard back to the transport.
            onmousedown: move |_| session_daw::keys::set_editing(false),
            TopBar {
                view,
                mode,
                lights: LIGHTS_W,
                // The transport reads the song it drives, so it is mounted
                // per song too — the tabs beside it are not.
                transport: rsx! {
                    if let Some(song) = current.clone() {
                        WithSong {
                            key: "{song.project}",
                            session: song.session.clone(),
                            session_daw::transport_bar::TransportBar {}
                        }
                    }
                },
                // Anywhere on the bar that is not a control drags the
                // window; a double click zooms it.
                on_drag: move |()| {
                    if let Err(e) = dragging.drag_window() {
                        tracing::debug!(error = %e, "window drag refused");
                    }
                },
                on_zoom: move |()| zooming.set_maximized(!zooming.is_maximized()),
                // A tab picked: that song is current, and the audio moves to
                // it. Where the one it replaces had got to is kept on its tab.
                on_pick: move |index: usize| {
                    let at = session_daw::engine::Transport::shared().map_or(0.0, |t| t.read().0);
                    let picked = setlist.write().pick(index, at).map(|song| song.project.clone());
                    if let Some(project) = picked {
                        session_daw::open::switch_song(&project);
                    }
                },
                on_color: move |(index, color): (usize, Option<String>)| {
                    setlist.write().recolor(index, color);
                },
            }
            if let Some(song) = current {
                // Keyed by the song: picking another remounts every panel
                // on that song's session rather than patching the last one's.
                SongViews { key: "{song.project}", session: song.session.clone(), view, editor_open }
            }
        }
    }
}

/// Whatever it holds, over one song: that song's session, as context.
#[component]
fn WithSong(session: session_daw::studio::StudioSession, children: Element) -> Element {
    use_context_provider(|| session);
    children
}

/// The views, over one song: its session is what every panel below reads.
#[component]
fn SongViews(
    session: session_daw::studio::StudioSession,
    view: Signal<session_daw::shell::View>,
    editor_open: Signal<bool>,
) -> Element {
    use session_daw::chart_editor::{ChartEditor, EditorToggle};
    use session_daw::shell::{OverviewLayout, View};
    use_context_provider(|| session);
    rsx! {
        div {
            style: "position:relative; flex:1; min-height:0;",
            match view() {
                View::Setup => rsx! { session_daw::setup::SetupView {} },
                View::Daw => rsx! { Arrangement {} },
                View::Performance => rsx! { PerformanceView {} },
                View::Overview => rsx! {
                    OverviewLayout {
                        progress: rsx! { session_daw::progress::ProgressBar {} },
                        editor: editor_open().then(|| rsx! { ChartEditor {} }),
                        chart: rsx! { session_daw::chart_panel::Chart { paged: true } },
                        chart_corner: rsx! { EditorToggle { open: editor_open } },
                        panels: rsx! { Arrangement { docked: true } },
                    }
                },
            }
        }
    }
}

/// Live mode runs the set: when the song playing reaches its end, the next
/// one is picked and plays from its count-in. Checked each frame — the
/// window redraws while the transport moves — and only in Live, so working
/// on a song in the other modes never jumps away from it.
fn use_live_advance(mut setlist: Signal<session_daw::setlist::Setlist>, mode: Signal<Mode>) {
    dioxus_native::use_window_event(move |event, _| {
        if !matches!(event, winit::event::WindowEvent::RedrawRequested) || mode() != Mode::Live {
            return;
        }
        let Some((at, playing)) = session_daw::engine::Transport::shared().map(|t| t.read()) else {
            return;
        };
        let next = {
            let list = setlist.peek();
            match (playing, list.current(), list.next()) {
                (true, Some(song), Some(next)) if song.ended(at) => Some(next),
                _ => None,
            }
        };
        let Some(next) = next else { return };
        let picked = setlist.write().pick(next, at).map(|song| song.project.clone());
        if let Some(project) = picked {
            tracing::info!(setlist.next = next, "live: the song ended; the next one plays");
            session_daw::open::switch_song(&project);
            session_daw::engine::transport(session_daw::engine::Move::PlayFrom, 0.0);
        }
    });
}

/// The arrangement, with the mixer docked under it on `x` (or from the
/// start, `docked`, in the Overview). In Organize the Organize toolbar sits
/// over it — markers, sections, time signatures — in whichever view it is.
/// The transport is in the top bar.
#[component]
fn Arrangement(#[props(default)] docked: bool) -> Element {
    let mode: Signal<Mode> = use_context();
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; flex-direction:column;",
            if mode() == Mode::Organize {
                session_daw::organize::OrganizeToolbar {}
            }
            div {
                style: "position:relative; flex:1; min-height:0;",
                session_daw::mixer_panel::DawPanels { docked, mode: Some(mode()) }
            }
        }
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
                style: "position:relative; flex:1; min-height:0; display:flex; gap:16px;",
                div {
                    style: "position:relative; flex:1; min-width:0; height:100%; border-radius:8px; \
                            overflow:hidden; border:1px solid {RULE};",
                    session_daw::chart_panel::Chart {}
                }
                div {
                    style: "position:relative; width:38%; min-width:320px; height:100%; border-radius:8px; \
                            overflow:hidden; border:1px solid {RULE};",
                    session_daw::lyrics_panel::LyricsPanel {}
                }
            }
            session_daw::progress::TransportButtons {}
        }
    }
}
