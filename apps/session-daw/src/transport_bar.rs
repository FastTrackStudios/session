//! The transport: go to start, play/stop, record, loop, go to end, and
//! where the song is — bar.beat, clock time, tempo, key.
//!
//! It lives in the app's top bar, so there is no bottom rail and the
//! transport is in the same place whatever the view. The Performance view
//! also has its own, bigger buttons along its foot (`session-ui`'s
//! `TransportControlBar`); this one is for editing, where the numbers
//! matter more than the size of the buttons.

use dioxus::prelude::*;

use crate::engine::{Move, Transport, transport};
use crate::studio::StudioSession;

const RULE: &str = "#2a2c31";
const TEXT: &str = "#e5e7eb";
const DIM: &str = "#8b9099";
const PLAY: &str = "#34d399";
const REC: &str = "#ef4444";
const LOOP: &str = "#3aa0ff";

/// The bar.
#[cfg(feature = "native")]
#[component]
pub fn TransportBar() -> Element {
    let mut reading = use_signal(crate::engine::Reading::default);
    // A read a frame, published only when something a person can see
    // changed: the clock to the hundredth, the flags, the tempo.
    dioxus_native::use_window_event(move |event, _| {
        if !matches!(event, winit::event::WindowEvent::RedrawRequested) {
            return;
        }
        let Some(now) = Transport::shared().map(Transport::reading) else {
            return;
        };
        publish(&mut reading, now);
    });
    rsx! { TransportBarView { reading: reading() } }
}

/// A new reading, published only when something a person can see changed:
/// the clock to the hundredth, the flags, the tempo.
fn publish(reading: &mut Signal<crate::engine::Reading>, now: crate::engine::Reading) {
    let was = *reading.peek();
    if (now.at - was.at).abs() >= 0.01
        || now.playing != was.playing
        || now.looping != was.looping
        || now.recording != was.recording
        || (now.bpm - was.bpm).abs() > 1e-6
    {
        reading.set(now);
    }
}

/// The bar in a browser: the same view, polled on a timer (a page has no
/// redraw event to hang it on).
#[cfg(feature = "web")]
#[component]
pub fn WebTransportBar() -> Element {
    let mut reading = use_signal(crate::engine::Reading::default);
    use_future(move || async move {
        loop {
            if let Some(now) = Transport::shared().map(Transport::reading) {
                publish(&mut reading, now);
            }
            gloo_timers::future::TimeoutFuture::new(33).await;
        }
    });
    rsx! { TransportBarView { reading: reading() } }
}

/// What the bar shows, for a reading.
#[component]
fn TransportBarView(reading: crate::engine::Reading) -> Element {
    let session: StudioSession = use_context();
    let r = reading;
    let (bar, beat) = crate::ruler::Timeline::new(&session.project.tempo)
        .beats(r.at + 1e-6, 100_000)
        .last()
        .map_or((1, 1), |b| (b.measure, b.beat));
    let key = crate::arrangement::key_at(&session.project, r.at)
        .map(|name| short_key(&name))
        .unwrap_or_else(|| "—".to_owned());
    let minutes = (r.at / 60.0).floor();
    let seconds = r.at - minutes * 60.0;
    rsx! {
        div {
            style: "height:100%; flex:none; display:flex; align-items:center; gap:4px;",
            // A press here is a button, not a drag of the window the bar
            // it sits in is the title bar of.
            onmousedown: move |event| event.stop_propagation(),
            Button { title: "Go to start", on: false, color: TEXT, glyph: Glyph::Home,
                onpress: move |()| transport(Move::Home, 0.0) }
            Button { title: "Play / stop", on: r.playing, color: PLAY,
                glyph: if r.playing { Glyph::Stop } else { Glyph::Play },
                onpress: move |()| transport(Move::PlayStop, 0.0) }
            Button { title: "Record", on: r.recording, color: REC, glyph: Glyph::Record,
                onpress: move |()| transport(Move::ToggleRecord, 0.0) }
            Button { title: "Loop", on: r.looping, color: LOOP, glyph: Glyph::Loop,
                onpress: move |()| transport(Move::ToggleLoop, 0.0) }
            Button { title: "Go to end", on: false, color: TEXT, glyph: Glyph::End,
                onpress: move |()| transport(Move::End, 0.0) }
            div { style: "width:6px;" }
            // Where the song is: the bar and beat first, the clock under it
            // in weight — the bar is what an editor counts in.
            div {
                style: "display:flex; align-items:baseline; gap:10px; padding:3px 12px; \
                        border-radius:5px; background:#0b0c0e; border:1px solid {RULE}; \
                        font-family:ui-monospace, monospace;",
                span { style: "font-size:16px; font-weight:600; color:{TEXT};", "{bar}.{beat}" }
                span { style: "font-size:12px; color:{DIM};", "{minutes:.0}:{seconds:05.2}" }
            }
            // The tempo, as a two-sided pill: a note glyph on the left (a
            // quarter note today — the slot a future eighth-note tempo
            // reading would swap it for) says what unit `bpm` counts,
            // the number sits on the right of its own divider.
            div {
                style: "display:flex; align-items:stretch; border-radius:5px; \
                        background:#0b0c0e; border:1px solid {RULE}; overflow:hidden;",
                div {
                    style: "display:flex; align-items:center; padding:0 8px; \
                            border-right:1px solid {RULE};",
                    NoteGlyph {}
                }
                span {
                    style: "display:flex; align-items:center; padding:0 10px; \
                            font-size:12px; color:{TEXT}; font-family:ui-monospace, monospace;",
                    if r.bpm > 0.0 { "{r.bpm:.1}" } else { "—" }
                }
            }
            // The key where the song is — the KEY track's change in force,
            // shortened: "F", "Fm", never "major".
            div {
                style: "display:flex; align-items:stretch; border-radius:5px; \
                        background:#0b0c0e; border:1px solid {RULE}; overflow:hidden;",
                div {
                    style: "display:flex; align-items:center; padding:0 8px; \
                            border-right:1px solid {RULE};",
                    KeyGlyph {}
                }
                span {
                    style: "display:flex; align-items:center; padding:0 10px; \
                            font-size:12px; font-weight:600; color:{TEXT};",
                    "{key}"
                }
            }
        }
    }
}

/// A quarter note, filled — the tempo pill's unit glyph.
#[component]
fn NoteGlyph() -> Element {
    rsx! {
        svg {
            width: "12",
            height: "14",
            view_box: "0 0 14 18",
            fill: "none",
            ellipse {
                cx: "4", cy: "14.5", rx: "3.6", ry: "2.7",
                fill: TEXT,
                transform: "rotate(-18 4 14.5)",
            }
            path {
                d: "M7.3 14.2 V1.5",
                stroke: TEXT, stroke_width: "1.6", stroke_linecap: "round",
            }
        }
    }
}

/// A key, stroked — the KEY pill's unit glyph.
#[component]
fn KeyGlyph() -> Element {
    rsx! {
        svg {
            width: "16",
            height: "14",
            view_box: "0 0 22 18",
            fill: "none",
            stroke: TEXT,
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            circle { cx: "6", cy: "9", r: "4.4" }
            path { d: "M10.2 9 H20" }
            path { d: "M15.5 9 V12.5" }
            path { d: "M18.5 9 V12.5" }
        }
    }
}

/// The KEY track's item name ("F major", "Eb minor", "F dorian"), shortened
/// for the transport bar: the tonic alone for major ("F"), the tonic with
/// a trailing `m` for minor ("Fm"), and the mode's first three letters for
/// anything else ("F dor") — spelling out "major" every time the song
/// hasn't changed key is wasted width the pill doesn't have.
fn short_key(name: &str) -> String {
    let Some(key) = session::key::parse_key(name) else {
        return name.to_owned();
    };
    let full = session::key::format_key(&key);
    let Some((root, mode)) = full.split_once(' ') else {
        return full;
    };
    match mode {
        "major" => root.to_owned(),
        "minor" => format!("{root}m"),
        other => format!("{root} {}", &other[..other.len().min(3)]),
    }
}

/// What a transport button shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Glyph {
    Home,
    Play,
    Stop,
    Record,
    Loop,
    End,
}

#[component]
fn Button(
    title: &'static str,
    on: bool,
    color: &'static str,
    glyph: Glyph,
    onpress: EventHandler<()>,
) -> Element {
    let fg = if on { color } else { DIM };
    let border = if on { color } else { RULE };
    rsx! {
        button {
            title,
            style: "width:30px; height:24px; padding:0; display:flex; align-items:center; \
                    justify-content:center; border-radius:5px; border:1px solid {border}; \
                    background:#0b0c0e;",
            onclick: move |_| onpress.call(()),
            svg {
                width: "16",
                height: "16",
                view_box: "0 0 24 24",
                match glyph {
                    Glyph::Home => rsx! {
                        path { d: "M6 5 V19", stroke: fg, stroke_width: "2.2", fill: "none" }
                        path { d: "M19 5 L9 12 L19 19 Z", fill: fg }
                    },
                    Glyph::Play => rsx! { path { d: "M7 4 L20 12 L7 20 Z", fill: fg } },
                    Glyph::Stop => rsx! { path { d: "M6 6 H18 V18 H6 Z", fill: fg } },
                    Glyph::Record => rsx! { circle { cx: "12", cy: "12", r: "7", fill: fg } },
                    Glyph::Loop => rsx! {
                        path {
                            d: "M4 12 A6 6 0 0 1 10 6 H18 M15 3 L18 6 L15 9 M20 12 A6 6 0 0 1 14 18 H6 M9 21 L6 18 L9 15",
                            stroke: fg, stroke_width: "2", fill: "none",
                            stroke_linecap: "round", stroke_linejoin: "round",
                        }
                    },
                    Glyph::End => rsx! {
                        path { d: "M5 5 L15 12 L5 19 Z", fill: fg }
                        path { d: "M18 5 V19", stroke: fg, stroke_width: "2.2", fill: "none" }
                    },
                }
            }
        }
    }
}
