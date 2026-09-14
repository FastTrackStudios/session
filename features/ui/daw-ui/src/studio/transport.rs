//! The transport band, and the wire between it and the engine.
//!
//! The bar itself is small. The interesting part is the subscription
//! below it: the engine emits position ticks at about 30 Hz, and the
//! rule this window is built on is that **a tick must not cause a
//! render**. Ticks re-anchor the page clock ([`super::clock`]) instead
//! of being drawn, and even that is rate-limited — the clock
//! extrapolates between anchors, so it needs correcting a couple of
//! times a second, not thirty.

use crate::prelude::*;
use daw_proto::transport::{PlayState, TransportEvent, TransportStreamEvent};

use super::clock::Clock;
use super::fps::FrameRate;

/// How many engine ticks pass between re-anchors. At the engine's ~30 Hz
/// this corrects the page clock about twice a second, which is far
/// tighter than free-running extrapolation can drift and far cheaper
/// than following every tick.
const RESYNC_EVERY_TICKS: u32 = 15;

/// The transport band: buttons, the clock readout, and the counters.
#[component]
pub fn TransportBar(
    playing: Signal<bool>,
    tracks: usize,
    items: usize,
    bpm: f64,
    rate: Signal<FrameRate>,
) -> Element {
    let clock = use_context::<Clock>();
    let is_playing = playing();
    rsx! {
        div { class: "studio-transport",
            button {
                r#type: "button",
                title: "Return to start (Home)",
                "data-testid": "studio-home",
                onclick: {
                    let clock = clock.clone();
                    move |_| clock.seek(0.0)
                },
                TransportGlyph { shape: Shape::Home }
            }
            button {
                r#type: "button",
                title: "Stop",
                "data-testid": "studio-stop",
                onclick: {
                    let clock = clock.clone();
                    move |_| super::stop(&clock, playing)
                },
                TransportGlyph { shape: Shape::Stop }
            }
            button {
                r#type: "button",
                title: "Play / pause (Space)",
                "data-testid": "studio-play",
                aria_pressed: "{is_playing}",
                onclick: {
                    let clock = clock.clone();
                    move |_| super::toggle_play(&clock, playing)
                },
                TransportGlyph { shape: Shape::Play }
            }
            // Written by the page clock every frame — see `clock`. What
            // is here is only what it reads before the first frame.
            span { class: "studio-clock", "data-testid": "studio-clock", "1.1.00   0:00.000" }
            span { class: "studio-stat", "{bpm:.2} BPM" }
            span { class: "studio-stat", "data-testid": "studio-counts",
                "{tracks} tracks · {items} items"
            }
            super::fps::FrameReadout { rate }
        }
    }
}

/// Which transport symbol to draw.
#[derive(Clone, Copy, PartialEq)]
pub enum Shape {
    Home,
    Stop,
    Play,
}

/// The transport symbols, drawn rather than typed.
///
/// Characters were the obvious way and the wrong one twice over: a font
/// with colour emoji renders ⏮ ⏹ ▶ as pictures that ignore `color`
/// entirely, and a font without those codepoints renders nothing at all
/// — which is how the bar came to have three invisible buttons on it.
/// Geometry has neither problem, and `currentColor` puts the symbol back
/// under the theme where it belongs.
#[component]
fn TransportGlyph(shape: Shape) -> Element {
    rsx! {
        svg {
            width: "11",
            height: "11",
            view_box: "0 0 12 12",
            fill: "currentColor",
            match shape {
                Shape::Home => rsx! {
                    rect { x: "1", y: "1", width: "2", height: "10" }
                    path { d: "M11 1 L11 11 L4 6 Z" }
                },
                Shape::Stop => rsx! {
                    rect { x: "1.5", y: "1.5", width: "9", height: "9" }
                },
                Shape::Play => rsx! {
                    path { d: "M2 1 L11 6 L2 11 Z" }
                },
            }
        }
    }
}

/// Follow the engine's transport, anchoring the page clock.
///
/// Renders nothing, and writes `playing` only when the transport
/// actually changes state — never on a position tick.
#[component]
pub fn TransportFollower(playing: Signal<bool>) -> Element {
    let clock = use_context::<Clock>();
    use_future(move || {
        let clock = clock.clone();
        let mut playing = playing;
        async move {
            loop {
                let Some(daw) = daw_control::Daw::try_get() else {
                    futures_timer::Delay::new(std::time::Duration::from_millis(200)).await;
                    continue;
                };
                let Ok(project) = daw.current_project().await else {
                    futures_timer::Delay::new(std::time::Duration::from_millis(200)).await;
                    continue;
                };
                let guid = project.guid().to_string();
                let mut stream = project.transport().events();
                let mut since_resync = 0_u32;
                while let Ok(Some(event)) = stream.recv().await {
                    match event.get() {
                        TransportStreamEvent::Position(tick) if tick.project_guid == guid => {
                            let Some(secs) = tick.playhead.seconds() else {
                                continue;
                            };
                            since_resync += 1;
                            if since_resync >= RESYNC_EVERY_TICKS {
                                since_resync = 0;
                                clock.resync(secs);
                            }
                        }
                        TransportStreamEvent::State(TransportEvent::PlayStateChanged {
                            project_guid,
                            play_state,
                        }) if *project_guid == guid => {
                            let running =
                                matches!(play_state, PlayState::Playing | PlayState::Recording);
                            clock.set_playing(running);
                            since_resync = 0;
                            // Only when it differs: this is the one place
                            // the transport is allowed to re-render, and
                            // an engine that re-announces its state would
                            // otherwise do it on every announcement.
                            if *playing.peek() != running {
                                playing.set(running);
                            }
                        }
                        _ => {}
                    }
                }
                // The stream ended — the backend went away or was
                // replaced. Re-subscribe rather than leaving the window
                // silently detached from its own transport.
                futures_timer::Delay::new(std::time::Duration::from_millis(400)).await;
            }
        }
    });
    rsx! {}
}
