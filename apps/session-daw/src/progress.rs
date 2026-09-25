//! The performance panels: the ProgressBar and the transport buttons under
//! the view.
//!
//! Read off the open session rather than the setlist engine for now: the
//! SONG region (lane 0) is the song's span, the SECTIONS regions (lane 1)
//! are its parts — the regions `build_from_chart` stamps. The bar and the
//! buttons are `session-ui`'s, unchanged; what is here is the data and the
//! transport they drive. The same panels in the desktop app and the web
//! demo: only how the reading is polled differs.

use dioxus::prelude::*;

use crate::engine::{Move, Reading, Transport, transport};
use crate::studio::StudioSession;
use session_ui::components::progress::{ProgressSection, SongProgressBar};
use session_ui::components::transport_controls::TransportControlBar;

/// The song as the performance panels (and the navigator) see it.
#[derive(Clone, PartialEq)]
pub(crate) struct Song {
    pub(crate) start: f64,
    pub(crate) end: f64,
    /// Each section's span, in order.
    pub(crate) sections: Vec<(f64, f64)>,
    pub(crate) bar: Vec<ProgressSection>,
}

impl Song {
    pub(crate) fn of(session: &StudioSession) -> Option<Self> {
        let regions = &session.project.sections;
        let (start, end) = regions
            .iter()
            .find(|r| r.lane == 0)
            .map(|r| (r.start, r.end))
            .or_else(|| {
                let start = regions
                    .iter()
                    .map(|r| r.start)
                    .fold(f64::INFINITY, f64::min);
                let end = regions.iter().map(|r| r.end).fold(0.0, f64::max);
                (end > start).then_some((start, end))
            })?;
        let span = (end - start).max(f64::EPSILON);
        let percent = |t: f64| ((t - start) / span * 100.0).clamp(0.0, 100.0);
        let mut parts: Vec<_> = regions
            .iter()
            .filter(|r| r.lane == crate::ruler::SECTIONS_ROW as u32)
            .collect();
        parts.sort_by(|a, b| a.start.total_cmp(&b.start));
        Some(Self {
            start,
            end,
            sections: parts.iter().map(|r| (r.start, r.end)).collect(),
            bar: parts
                .iter()
                .map(|r| ProgressSection {
                    start_percent: percent(r.start),
                    end_percent: percent(r.end),
                    color: r.color.clone().unwrap_or_else(|| "#4b5563".to_owned()),
                    name: r.name.clone(),
                    short_name: r.name.clone(),
                    comment: None,
                })
                .collect(),
        })
    }

    /// The section the play position is in, if any.
    pub(crate) fn current(&self, at: f64) -> Option<usize> {
        self.sections
            .iter()
            .rposition(|(from, _)| *from <= at + 1e-6)
    }

    fn progress(&self, at: f64) -> f64 {
        ((at - self.start) / (self.end - self.start).max(f64::EPSILON) * 100.0).clamp(0.0, 100.0)
    }
}

/// The transport's reading, republished when the position moves a
/// twentieth of a second or a flag changes — enough for a song-wide bar,
/// where a frame's travel is under a pixel.
pub(crate) fn use_reading() -> Signal<Reading> {
    let mut reading = use_signal(Reading::default);
    let mut publish = move || {
        let Some(now) = Transport::shared().map(Transport::reading) else {
            return;
        };
        let was = *reading.peek();
        if (now.at - was.at).abs() > 0.05
            || now.playing != was.playing
            || now.looping != was.looping
            || now.recording != was.recording
        {
            reading.set(now);
        }
    };
    #[cfg(feature = "native")]
    dioxus_native::use_window_event(move |event, _| {
        if matches!(event, winit::event::WindowEvent::RedrawRequested) {
            publish();
        }
    });
    #[cfg(all(feature = "web", not(feature = "native")))]
    use_future(move || async move {
        loop {
            publish();
            gloo_timers::future::TimeoutFuture::new(33).await;
        }
    });
    reading
}

/// The ProgressBar panel. A click on a section plays from it.
#[component]
pub fn ProgressBar(
    /// Its height as CSS — slim on a phone (5rem unless given).
    #[props(default)]
    height: Option<String>,
    /// Whether the sections are named on it (not on an upright phone).
    #[props(default = true)]
    labels: bool,
) -> Element {
    let session: StudioSession = use_context();
    let song = use_hook(|| Song::of(&session));
    let reading = use_reading();
    let Some(song) = song else {
        return rsx! {
            div { style: "color:#8b9099; font-size:12px;", "No song regions in this session." }
        };
    };
    let starts: Vec<f64> = song.sections.iter().map(|(from, _)| *from).collect();
    // Others' pointers over the bar are a time in the song, placed on this
    // bar wherever it is and however wide.
    #[cfg(feature = "native")]
    use_hook({
        let span = (song.start, song.end);
        move || crate::ghosts::register_anchor("progress", std::rc::Rc::new(SongAnchor { span }))
    });
    rsx! {
        SongProgressBar {
            height,
            labels,
            progress: song.progress(reading().at),
            sections: song.bar.clone(),
            on_section_click: move |index: usize| {
                if let Some(at) = starts.get(index) {
                    transport(Move::Seek, *at);
                }
            },
        }
    }
}

/// The performance transport: back a section, play/stop, loop, on a section.
/// `compact` is a small screen's: the four icons alone, `height` pixels tall
/// (44 unless given).
#[component]
pub fn TransportButtons(
    #[props(default)] compact: bool,
    #[props(default)] height: Option<u32>,
) -> Element {
    let session: StudioSession = use_context();
    let song = use_hook(|| Song::of(&session));
    let reading = use_reading();
    let r = reading();
    let back = song.clone();
    let on = song;
    let height = height.unwrap_or(if compact { 44 } else { 64 });
    rsx! {
        div {
            style: "height:{height}px; flex:none; overflow:hidden; border-radius:10px;",
            TransportControlBar {
                icons_only: compact,
                is_playing: r.playing,
                is_looping: r.looping,
                is_recording: false,
                is_armed: false,
                show_recording: false,
                on_play_pause: move |()| transport(Move::PlayStop, 0.0),
                on_loop_toggle: move |()| transport(Move::ToggleLoop, 0.0),
                on_record_toggle: move |()| {},
                on_arm_toggle: move |()| {},
                // Back: to the start of this section, or — within a second
                // of it — to the one before, which is what a second press
                // of a back button means.
                on_back: move |()| {
                    let Some(song) = back.as_ref() else { return };
                    let at = Transport::shared().map_or(0.0, |t| t.read().0);
                    let to = match song.current(at) {
                        Some(i) if at - song.sections[i].0 < 1.0 && i > 0 => song.sections[i - 1].0,
                        Some(i) => song.sections[i].0,
                        None => song.start,
                    };
                    transport(Move::Seek, to);
                },
                on_forward: move |()| {
                    let Some(song) = on.as_ref() else { return };
                    let at = Transport::shared().map_or(0.0, |t| t.read().0);
                    let next = song.sections.iter().map(|(from, _)| *from).find(|from| *from > at + 1e-3);
                    if let Some(to) = next {
                        transport(Move::Seek, to);
                    }
                },
            }
        }
    }
}

/// A pointer over the progress bar, anchored to the song: `u` is a time,
/// in project seconds; `v` how far down the bar.
#[cfg(feature = "native")]
struct SongAnchor {
    span: (f64, f64),
}

#[cfg(feature = "native")]
impl crate::ghosts::Anchor for SongAnchor {
    fn anchor(&self, x: f64, y: f64, (w, h): (f64, f64)) -> Option<(String, f64, f64)> {
        let (start, end) = self.span;
        let at = (x / w.max(1.0)).mul_add(end - start, start);
        Some(("song".to_owned(), at, y / h.max(1.0)))
    }

    fn place(&self, key: &str, u: f64, v: f64, (w, h): (f64, f64)) -> Option<(f64, f64)> {
        let (start, end) = self.span;
        (key == "song").then(|| ((u - start) / (end - start).max(f64::EPSILON) * w, v * h))
    }
}
