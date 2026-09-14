//! The studio window — a full DAW UI over the `daw` facade.
//!
//! Opens a REAPER session and lets you move around inside it: the track
//! control panel down the left, the arrangement beside it, the transport
//! above both, and (once mounted) the mixer down the right. It targets a
//! WRY WebView, which is the renderer the Session desktop app ships on,
//! and it is written for a browser engine rather than ported to one.
//!
//! # The four rules
//!
//! A DAW window is not a page with a lot of nodes on it. It is a
//! *continuously moving* surface over a data set of a size that makes
//! every per-frame cost visible. Everything below follows from four
//! rules, and each one exists because breaking it is what makes a UI
//! like this slow in a way profiling a component tree will not show:
//!
//! 1. **Panning must not re-render.** One scroll container; the TCP
//!    column is `position: sticky` in its left column and the ruler
//!    `sticky` in its top row. Nothing subscribes to scroll offset,
//!    because nothing needs to — see [`css`].
//! 2. **Zooming must not re-render.** Every position is
//!    `calc(var(--t0) * var(--pps) * 1px)`. A zoom writes one custom
//!    property through the page ([`clock`]) and several thousand
//!    elements move in the style engine.
//! 3. **Playback must not re-render.** The playhead is extrapolated by a
//!    `requestAnimationFrame` loop in the page from anchors Rust
//!    publishes a couple of times a second, so it moves every composited
//!    frame and costs no dioxus work at all.
//! 4. **Nothing is copied that could be shared.** Dioxus hands a
//!    component its props by value on every render, so project data
//!    travels behind [`ProjectRef`]/[`RowsRef`], which compare by
//!    pointer — a plain `Arc<Project>` would deep-compare the whole
//!    session every time dioxus checked whether to memoise a child.
//!
//! What is left over — a click, a fold, a mute — is allowed to render,
//! because it is a discrete act and the frame it happens on has budget
//! for it.
//!
//! # Measuring
//!
//! [`fps::FrameMeter`] reports the rate the window's own compositor
//! presented at, and the worst frame in each window beside it. Read it
//! from the toolbar, or off a terminal:
//!
//! ```sh
//! RUST_LOG=daw_ui::studio::fps=info
//! ```
//!
//! Renders, DOM mutations and engine ticks are different quantities and
//! must never be reported as this one.

pub mod arrange;
pub mod autoscroll;
pub mod canvas;
pub mod census;
pub mod clock;
pub mod css;
pub mod fps;
pub mod probe;
pub mod project;
pub mod tcp;
pub mod theme;
pub mod transport;

use std::ops::Deref;
use std::sync::Arc;

use crate::components::folders::FolderState;
use crate::controls::{ControlSync, MeterFeed, use_daw_tracks, use_track_store};
use crate::prelude::*;
use crate::theming::use_theme;
use daw_proto::Track;
use dioxus::html::geometry::WheelDelta;

pub use clock::Clock;
pub use project::Project;

/// The project, compared by pointer.
///
/// This exists because dioxus memoises a child by `PartialEq` on its
/// props, and `Arc<T>`'s `PartialEq` compares the *contents* unless `T:
/// Eq` — which `Project` is not, holding floats. A bare `Arc<Project>`
/// prop therefore deep-compares an entire session on every render of the
/// parent, which is precisely the work the `Arc` was reached for to
/// avoid. Two handles on the same snapshot are the same snapshot; a new
/// snapshot is a new allocation.
#[derive(Clone)]
pub struct ProjectRef(pub Arc<Project>);

impl PartialEq for ProjectRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for ProjectRef {
    type Target = Project;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The visible track list with each track's folder depth, compared by
/// pointer for the same reason as [`ProjectRef`]. Rebuilt only when the
/// project lands or a folder is opened or closed.
#[derive(Clone)]
pub struct RowsRef(pub Arc<Vec<(Track, u32)>>);

impl PartialEq for RowsRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for RowsRef {
    type Target = Vec<(Track, u32)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The whole window. Takes no props: a root component has to be
/// launchable as a bare `fn() -> Element`, and everything it needs comes
/// from the facade or from context it provides itself.
#[component]
pub fn Studio() -> Element {
    // Every colour in the window comes from here. `use_theme` returns
    // whatever a `ThemeProvider` above us supplied — the FTS dark
    // default, or a real `.ReaperTheme` loaded through
    // `theming::reaper_import` — and `studio::theme` turns it into the
    // custom properties the stylesheet reads. Nothing below invents a
    // colour of its own.
    let probes = use_hook(|| {
        let p = probe::Probes::from_env();
        p.announce();
        p
    });
    let theme = use_theme().theme;
    let clock = use_hook(Clock::new);
    use_context_provider(|| clock.clone());

    // What the TCP's controls are wired to. The store is the live track
    // state every control reads and writes; `use_daw_tracks` seeds it and
    // keeps it current from the facade's track events. Provided HERE
    // rather than per row, because a strip's controls must share one
    // store or they disagree with each other about the same track.
    let store = use_track_store();
    use_daw_tracks(store);

    let mut folders = use_signal(FolderState::default);
    let playing = use_signal(|| false);
    let rate = use_signal(fps::FrameRate::default);
    let mut project = use_signal(|| None::<ProjectRef>);

    // The window opens on nothing and fills in behind itself. Parsing a
    // real session and standing up its backend takes seconds, and none
    // of it needs to happen before there is a window — so the loader
    // runs on a worker thread and this waits for the facade to appear.
    use_future({
        let clock = clock.clone();
        move || {
            let clock = clock.clone();
            async move {
                let loaded = project::fetch_when_ready().await;
                // The one line that says the WINDOW has the project, as
                // distinct from the loader having parsed it. They are
                // different events and only this one means anything is
                // drawable.
                tracing::info!(
                    studio.tracks = loaded.tracks.len(),
                    studio.items = loaded.item_count,
                    studio.sections = loaded.sections.len(),
                    studio.markers = loaded.markers.len(),
                    studio.length_secs = loaded.length_secs,
                    studio.bpm = loaded.bpm,
                    "project mounted"
                );
                clock.set_bpm(loaded.bpm);
                // The animation's duration — see `Clock::set_length`.
                clock.set_length(loaded.length_secs);
                project.set(Some(ProjectRef(loaded)));
            }
        }
    });

    let open = project();
    // The visible row list, in a memo — and the memo is the point.
    //
    // [`RowsRef`] compares by POINTER so a child can be skipped when the
    // rows have not changed. Building a fresh `Arc` in the render body
    // defeats that completely: every render minted a new pointer, the
    // equality always failed, and every child re-rendered — `Lanes` with
    // its eight hundred items and `Ruler` with its four hundred bar
    // labels included. A memo that only re-runs when the project or the
    // folder state actually changes is what makes the pointer mean
    // something.
    let rows = use_memo(move || {
        project().map(|p| {
            let (visible, depths) = folders.read().visible(&p.tracks);
            RowsRef(Arc::new(visible.into_iter().zip(depths).collect()))
        })
    });
    let rows = rows();
    let (tracks, items, bpm) = open
        .as_ref()
        .map_or((0, 0, 120.0), |p| (p.tracks.len(), p.item_count, p.bpm));

    // Bar lines are a repeating gradient whose period is one bar, so the
    // grid needs the tempo but no elements. `--len` is the song's length
    // in seconds; the ruler and the lanes take their width from it and
    // the zoom together.
    let secs_per_beat = 60.0 / bpm.max(1.0);
    let secs_per_bar = 4.0 * secs_per_beat;
    let length = open.as_ref().map_or(60.0, |p| p.length_secs);
    let palette = theme::css_variables(&theme);
    // The column's width and the row pitch are the TCP's own measured
    // numbers (`daw_theme_art::geometry::tcp`), not values chosen here.
    // The lanes take the same pitch, which is the whole alignment
    // contract between the two columns.
    let tcp_w = tcp::COLUMN_W;
    let row_h = daw_theme_art::geometry::tcp::ROW_H;
    let row_pitch = tcp::ROW_PITCH;

    // Level 0: the bare shell. A plain scroller of empty rows — no grid,
    // no sticky columns, no panels, no engine machinery. This is the
    // floor the rest is measured against, and it should scroll exactly
    // as well as a hand-written HTML page does.
    if probes.build_level() == Some(0) {
        let count = rows.as_ref().map_or(0, |r| r.len()).max(1);
        return rsx! {
            div { class: "studio",
                StudioStyles {}
                fps::FrameMeter { rate }
                if let Some(axis) = use_hook(|| std::env::var(autoscroll::ENV).ok()) {
                    autoscroll::AutoScroll { axis }
                }
                div { class: "studio-scroll",
                    div { style: "width: 343px;",
                        for i in 0..count {
                            div { key: "{i}", class: "studio-row" }
                        }
                    }
                }
            }
        };
    }

    rsx! {
        div {
            class: {
                let mut c = String::from("studio");
                for (switch, class) in [
                    ("no-sticky", " no-sticky"),
                    ("lanes:plain", " lanes-plain"),
                    ("lanes:nogrid", " lanes-nogrid"),
                    ("lanes:noflex", " lanes-noflex"),
                    ("lanes:nohead", " lanes-nohead"),
                ] {
                    if probes.has(switch) {
                        c.push_str(class);
                    }
                }
                c
            },
            "data-testid": "studio",
            // The theme, as one inline custom-property block. Inline so
            // it beats the stylesheet's fallbacks, and in one place so a
            // theme change is a single attribute write rather than a
            // re-render of everything that draws a colour.
            style: "{palette} --tcp-w: {tcp_w}px; --row-h: {row_h}px; \
                    --row-pitch: {row_pitch}px;",
            // The transport keys hands expect on minute one. On the root
            // and focused at mount, so they work before anything has
            // been clicked. The full keymap — crates/input's profiles —
            // is the follow-up, not a reason to ship without these.
            tabindex: 0,
            autofocus: true,
            onkeydown: {
                let clock = clock.clone();
                move |event: KeyboardEvent| {
                    match event.key() {
                        Key::Character(c) if c == " " => {
                            event.prevent_default();
                            toggle_play(&clock, playing);
                        }
                        Key::Home => {
                            event.prevent_default();
                            seek(&clock, 0.0);
                        }
                        Key::Character(c) if c == "=" || c == "+" => {
                            event.prevent_default();
                            clock.set_pps(clock.pps() * 1.3);
                        }
                        Key::Character(c) if c == "-" || c == "_" => {
                            event.prevent_default();
                            clock.set_pps(clock.pps() / 1.3);
                        }
                        _ => {}
                    }
                }
            },
            StudioStyles {}
            // The engine-facing pair, once for the whole window: drafts
            // flush to the facade at 30 Hz, meter frames feed every
            // meter. Mounted high, per their own docs — one timer for
            // the window rather than one per control.
            // `no-sync` drops the engine-facing pair: the 30 Hz draft
            // flush and the meter feed. Both run regardless of what is
            // on screen, which makes them suspects the moment an EMPTY
            // window scrolls badly.
            if !probes.has("no-sync") && probes.at_least(5) {
                ControlSync {}
                MeterFeed {}
            }
            fps::FrameMeter { rate }
            census::Census {}
            // A self-driven scroll, for measuring on the real window —
            // see `autoscroll`. Off unless asked for.
            if let Some(axis) = use_hook(|| {
                std::env::var(autoscroll::ENV).ok()
            }) {
                autoscroll::AutoScroll { axis }
            }
            // A/B probe: `FTS_STUDIO_NO_CLOCK=1` drops the whole clock
            // machinery — the page loop, its evals, and the playhead's
            // running Web Animation — to see what it costs a scroll.
            if !probes.has("no-clock") && probes.at_least(6) {
                clock::ClockDriver { clock: clock.clone() }
            }
            if !probes.has("no-transport") && probes.at_least(7) {
                transport::TransportFollower { playing }
            }
            if probes.at_least(7) {
                transport::TransportBar { playing, tracks, items, bpm, rate }
            }

            div {
                class: "studio-scroll",
                "data-testid": "studio-scroll",
                // Ctrl-wheel zooms, which is where every DAW puts it.
                // Exponential in the wheel delta so the gesture feels the
                // same however far in you already are.
                onwheel: {
                    let clock = clock.clone();
                    move |event: WheelEvent| {
                        if event.modifiers().ctrl() {
                            event.prevent_default();
                            clock.set_pps(clock.pps() * (-wheel_notches(&event.delta()) * 0.12).exp());
                        }
                    }
                },
                div {
                    class: "studio-grid",
                    style: "--len: {length}; --secs-per-bar: {secs_per_bar}; \
                            --secs-per-beat: {secs_per_beat};",
                    div { class: "studio-corner" }
                    match (&open, &rows) {
                        // A/B probe: `FTS_STUDIO_TCP_ONLY=1` mounts the
                        // track panel and nothing beside it, so a scroll
                        // measures the TCP alone. The window is useless
                        // like this; it is here to split the column from
                        // the timeline, which are very different work.
                        (Some(_), Some(rows)) if probes.has("tcp-only") => rsx! {
                            div {}
                            tcp::TcpColumn { rows: rows.clone(), folders }
                        },
                        (Some(p), Some(rows)) => rsx! {
                            if probes.at_least(3) {
                                arrange::Ruler { project: p.clone(), clock: clock.clone() }
                            } else {
                                div {}
                            }
                            tcp::TcpColumn { rows: rows.clone(), folders }
                            if probes.at_least(4) {
                                arrange::Lanes {
                                    project: p.clone(),
                                    rows: rows.clone(),
                                    clock: clock.clone(),
                                }
                            }
                        },
                        _ => rsx! {
                            div {
                                class: "studio-empty",
                                "data-testid": "studio-loading",
                                "Opening the project — tracks and items land first, \
                                 waveforms stream in behind them…"
                            }
                        },
                    }
                }
            }
        }
    }
}

/// A wheel gesture in units the zoom can use.
///
/// A mouse reports pixels, a trackpad may report lines, and a page-mode
/// wheel reports pages. Stripping the units and treating all three as
/// pixels is why a zoom feels right on one machine and unusably fast or
/// slow on the next — so each is converted to *notches*, roughly one
/// detent of a wheel, before it reaches the exponent.
fn wheel_notches(delta: &WheelDelta) -> f64 {
    match delta {
        WheelDelta::Pixels(v) => v.y / 53.0,
        WheelDelta::Lines(v) => v.y,
        WheelDelta::Pages(v) => v.y * 12.0,
    }
}

/// The sheet, in its own props-less component.
///
/// `document::Style` warns — per render — when its props differ from the
/// ones it first saw, because the head cannot be re-styled in place.
/// Sitting directly in the root it would do exactly that on every
/// render. Nothing can invalidate a component with no props, so this
/// renders once for the life of the window.
#[component]
fn StudioStyles() -> Element {
    rsx! {
        document::Style { {css::STUDIO_CSS} }
    }
}

/// Move the playhead, in the UI and then in the engine.
///
/// In that order deliberately: the UI's own value is the truthful one
/// until the round trip completes, and waiting on the facade to redraw a
/// playhead is how a transport comes to feel laggy on a backend that is
/// otherwise instant.
pub fn seek(clock: &Clock, secs: f64) {
    clock.seek(secs);
    spawn(async move {
        if let Some(daw) = daw_control::Daw::try_get()
            && let Ok(project) = daw.current_project().await
        {
            let _ = project.transport().set_position(secs).await;
        }
    });
}

pub fn toggle_play(clock: &Clock, mut playing: Signal<bool>) {
    let next = !*playing.peek();
    playing.set(next);
    clock.set_playing(next);
    spawn(async move {
        if let Some(daw) = daw_control::Daw::try_get()
            && let Ok(project) = daw.current_project().await
        {
            let _ = project.transport().play_pause().await;
        }
    });
}

pub fn stop(clock: &Clock, mut playing: Signal<bool>) {
    playing.set(false);
    clock.set_playing(false);
    spawn(async move {
        if let Some(daw) = daw_control::Daw::try_get()
            && let Ok(project) = daw.current_project().await
        {
            let _ = project.transport().stop().await;
        }
    });
}
