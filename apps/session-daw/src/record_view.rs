//! Record mode's performance view: recording vocals, with REAPER doing the
//! recording and this — on a tablet, a phone or a page — driving it.
//!
//! - **On top**, the song's bar and, right under it, the section's: the
//!   current section split into its measures, a press on one going there.
//! - **On the right**, a mixer of the five things a singer needs to hear —
//!   Click, Vocal, Drums, Bass, Guitar — made for a finger: big mute and
//!   solo, and a fader that moves only by its cap.
//! - **On the left**, the rating pad (★★★ / ★★ / ★ / ✕): a press marks the
//!   vocal take at the playhead — while he sings, or after, moved there by
//!   the bars ([`crate::take_rating`]).
//! - **When a take ends**, a prompt rates it whole, or offers to go through
//!   it in more detail with the pad.
//! - **At the bottom**, the transport, with Record where Loop is.

use std::rc::Rc;

use dioxus::prelude::*;
use session_ui::components::{MeasureIndicator, SectionProgressBar};

use crate::compact::{Form, use_form};
use crate::engine::{Edit, Move, transport};
use crate::progress::{ProgressBar, Song, TransportButtons, use_reading};
use crate::studio::StudioSession;
use crate::take_rating::{Rating, rate_at, rate_take};

const BG: &str = "#0f1012";
const PANEL: &str = "#17181b";
const RULE: &str = "#2a2c31";
const TEXT: &str = "#e5e7eb";
const DIM: &str = "#8b9099";

/// The record view, over the song in context.
#[component]
pub fn RecordView() -> Element {
    let form = use_form();
    let reading = use_reading();
    // Where the take began, while one is being recorded; the take to ask
    // about once it has stopped.
    let mut began = use_signal(|| None::<f64>);
    let mut ask = use_signal(|| None::<f64>);
    let recording = reading().recording;
    use_effect(use_reactive!(|recording| {
        let at = reading.peek().at;
        match (recording, began()) {
            (true, None) => began.set(Some(at)),
            (false, Some(from)) => {
                began.set(None);
                ask.set(Some(from));
            }
            _ => {}
        }
    }));
    let upright = form == Form::Portrait;
    let body = if upright {
        rsx! {
            div {
                style: "flex:1; min-height:0; display:flex; flex-direction:column; gap:10px; padding:10px;",
                RatingPad { upright }
                div { style: "flex:1; min-height:0; display:flex;", TouchMixer {} }
            }
        }
    } else {
        rsx! {
            div {
                style: "flex:1; min-height:0; display:flex; gap:12px; padding:12px;",
                div { style: "flex:1; min-width:0; display:flex;", RatingPad { upright } }
                div { style: "flex:none; display:flex;", TouchMixer {} }
            }
        }
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; box-sizing:border-box; \
                    display:flex; flex-direction:column; background:{BG}; color:{TEXT}; \
                    font-family:system-ui, sans-serif;",
            div {
                style: "flex:none; display:flex; flex-direction:column; gap:4px; padding:10px 12px 0;",
                ProgressBar { height: "3rem".to_owned(), labels: !upright }
                SectionBar {}
            }
            {body}
            div {
                style: "flex:none; padding:0 12px 12px;",
                TransportButtons { compact: upright, height: 64, record: true }
            }
            if let Some(from) = ask() {
                TakePrompt {
                    on_rate: move |rating: Rating| {
                        rate_take(rating, from);
                        ask.set(None);
                    },
                    on_detail: move |()| {
                        // Back to where the take began, to go through it.
                        transport(Move::Seek, from);
                        ask.set(None);
                    },
                    on_close: move |()| ask.set(None),
                }
            }
        }
    }
}

/// The section's bar: the current section of the song, split into its
/// measures, each filling as it plays; a press on one goes to it.
#[component]
fn SectionBar() -> Element {
    let session: StudioSession = use_context();
    let song = use_hook(|| Song::of(&session));
    let reading = use_reading();
    let Some(song) = song else {
        return rsx! {};
    };
    let at = reading().at;
    let Some(index) = song
        .current(at)
        .or(Some(0))
        .filter(|i| *i < song.sections.len())
    else {
        return rsx! {};
    };
    let (from, to) = song.sections[index];
    let span = (to - from).max(f64::EPSILON);
    // The measures that start inside the section, from the song's tempo
    // map — or, for a song without one, its tempo in 4/4 from the start.
    let fallback = [daw_ui::studio::project::TempoChange {
        at: 0.0,
        bpm: if reading().bpm > 0.0 {
            reading().bpm
        } else {
            120.0
        },
        beats_per_bar: 4,
        beat_unit: 4,
    }];
    let tempo: &[daw_ui::studio::project::TempoChange] = if session.project.0.tempo.is_empty() {
        &fallback
    } else {
        &session.project.0.tempo
    };
    let starts: Vec<(u32, f64)> = crate::ruler::Timeline::new(tempo)
        .beats(to, 20_000)
        .into_iter()
        .filter(|b| b.beat == 1 && b.at >= from - 1e-6 && b.at < to - 1e-6)
        .map(|b| (b.measure, b.at))
        .collect();
    let measures: Vec<MeasureIndicator> = starts
        .iter()
        .map(|(measure, start)| MeasureIndicator {
            position_percent: (start - from) / span * 100.0,
            measure_number: i32::try_from(*measure).unwrap_or(i32::MAX),
            time_signature: None,
            musical_position: daw_proto::MusicalPosition {
                measure: i32::try_from(*measure).unwrap_or(i32::MAX),
                beat: 1,
                subdivision: 0,
            },
        })
        .collect();
    let mut section = song.bar[index].clone();
    section.start_percent = 0.0;
    section.end_percent = 100.0;
    let progress = ((at - from) / span * 100.0).clamp(0.0, 100.0);
    rsx! {
        div {
            style: "height:2.25rem; flex:none;",
            SectionProgressBar {
                progress,
                sections: vec![section],
                measure_indicators: measures,
                on_measure_click: move |position: daw_proto::MusicalPosition| {
                    let found = starts
                        .iter()
                        .find(|(measure, _)| i64::from(*measure) == i64::from(position.measure));
                    if let Some((_, start)) = found {
                        transport(Move::Seek, *start);
                    }
                },
            }
        }
    }
}

/// The rating pad: each press marks the vocal take at the playhead.
#[component]
fn RatingPad(upright: bool) -> Element {
    let reading = use_reading();
    let direction = if upright { "row" } else { "column" };
    rsx! {
        div {
            style: "flex:1; min-width:0; min-height:0; display:flex; flex-direction:column; gap:8px; \
                    padding:12px; border-radius:16px; background:{PANEL}; border:1px solid {RULE};",
            div {
                style: "display:flex; align-items:baseline; gap:8px;",
                span { style: "font-size:13px; font-weight:700; letter-spacing:0.06em; color:{DIM};", "RATE THE TAKE" }
                span { style: "font-size:12px; color:#6b7280;", "at the playhead" }
            }
            div {
                style: "flex:1; min-height:0; display:flex; flex-direction:{direction}; gap:8px;",
                for rating in Rating::ALL {
                    button {
                        key: "{rating.label()}",
                        style: "flex:1; min-width:0; min-height:56px; border-radius:14px; border:1px solid {RULE}; \
                                background:#1f2228; color:{rating.color()}; font-family:inherit; \
                                font-size:28px; font-weight:700; cursor:pointer;",
                        onclick: move |_| rate_at(rating, reading.peek().at),
                        RatingMark { rating, size: 30 }
                    }
                }
            }
        }
    }
}

/// A rating's mark: its stars, or an X for a miss (drawn, since the ✕
/// glyph is missing from the fonts a phone draws with).
#[component]
fn RatingMark(rating: Rating, size: u32) -> Element {
    rsx! {
        div {
            style: "display:flex; align-items:center; justify-content:center; font-size:{size}px; line-height:1;",
            if rating == Rating::Miss {
                lucide_dioxus::X { size: size as usize + 6, color: rating.color(), stroke_width: 3 }
            } else {
                "{rating.label()}"
            }
        }
    }
}

/// The prompt at the end of a take: rate it whole, or go through it.
#[component]
fn TakePrompt(
    on_rate: EventHandler<Rating>,
    on_detail: EventHandler<()>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; z-index:50; display:flex; \
                    align-items:center; justify-content:center; background:rgba(0,0,0,0.6);",
            onclick: move |_| on_close.call(()),
            div {
                style: "width:min(92%, 520px); box-sizing:border-box; padding:20px; border-radius:20px; \
                        background:{PANEL}; border:1px solid {RULE}; display:flex; flex-direction:column; gap:14px;",
                onclick: move |e| e.stop_propagation(),
                span { style: "font-size:20px; font-weight:700;", "How was that take?" }
                div {
                    style: "display:flex; gap:8px;",
                    for rating in Rating::ALL {
                        button {
                            key: "{rating.label()}",
                            style: "flex:1; min-width:0; height:72px; border-radius:14px; border:1px solid {RULE}; \
                                    background:#1f2228; color:{rating.color()}; font-family:inherit; \
                                    font-size:26px; font-weight:700; cursor:pointer;",
                            onclick: move |_| on_rate.call(rating),
                            RatingMark { rating, size: 26 }
                        }
                    }
                }
                button {
                    style: "height:48px; border-radius:12px; border:1px solid {RULE}; background:transparent; \
                            color:{TEXT}; font-family:inherit; font-size:15px; font-weight:600; cursor:pointer;",
                    onclick: move |_| on_detail.call(()),
                    "More detailed — go through it bar by bar"
                }
            }
        }
    }
}

/// The five groups a singer's mix is made of, left to right: each matched
/// to the project's top-level track whose name says it.
const GROUPS: [(&str, &[&str]); 5] = [
    ("Click", &["click"]),
    ("Vocal", &["vocal", "vox"]),
    ("Drums", &["drum"]),
    ("Bass", &["bass"]),
    ("Guitar", &["guitar", "gtr"]),
];

/// A group's track, as the mixer shows it.
#[derive(Clone, Debug, PartialEq)]
struct Strip {
    label: &'static str,
    track: daw_proto::Track,
}

/// The touch mixer: one strip per group the project has.
#[component]
fn TouchMixer() -> Element {
    let applier = use_hook(|| Rc::new(crate::engine::Applier::start()));
    let mut strips = use_signal(Vec::<Strip>::new);
    // A fader held: its strip's live value is ours until it is let go.
    let held = use_signal(|| None::<String>);
    use_future(move || async move {
        loop {
            if let Some(found) = read_strips().await {
                let holding = held.peek().clone();
                let mut next = found;
                if let Some(guid) = holding {
                    // Keep the held fader where the finger has it.
                    let current = strips.peek().clone();
                    for strip in &mut next {
                        if strip.track.guid == guid
                            && let Some(was) = current.iter().find(|s| s.track.guid == guid)
                        {
                            strip.track.volume = was.track.volume;
                        }
                    }
                }
                if *strips.peek() != next {
                    strips.set(next);
                }
            }
            architect::platform::sleep(std::time::Duration::from_millis(300)).await;
        }
    });
    let list = strips();
    rsx! {
        div {
            style: "display:flex; gap:8px; padding:10px; border-radius:16px; background:{PANEL}; \
                    border:1px solid {RULE};",
            if list.is_empty() {
                div {
                    style: "width:220px; display:flex; align-items:center; justify-content:center; \
                            text-align:center; font-size:13px; color:{DIM}; line-height:1.5;",
                    "No Click, Vocal, Drums, Bass or Guitar tracks in this project."
                }
            }
            for (i, strip) in list.into_iter().enumerate() {
                TouchStrip {
                    key: "{strip.track.guid}",
                    strip: strip.clone(),
                    held,
                    on_edit: {
                        let applier = Rc::clone(&applier);
                        move |edit: Edit| {
                            // Shown at once; the engine catches up.
                            apply_locally(&mut strips.write()[i].track, &edit);
                            if let Some(applier) = applier.as_ref() {
                                applier.send(edit);
                            }
                        }
                    },
                }
            }
        }
    }
}

/// An edit, on the strip's copy, so the finger sees it before the engine
/// answers.
fn apply_locally(track: &mut daw_proto::Track, edit: &Edit) {
    match edit {
        Edit::SetVolume(_, gain) => track.volume = *gain,
        Edit::ToggleMute(_) => track.muted = !track.muted,
        Edit::ToggleSolo(_) => track.soloed = !track.soloed,
        _ => {}
    }
}

/// The groups' tracks, from the engine.
async fn read_strips() -> Option<Vec<Strip>> {
    let daw = daw::rpc::Daw::try_get()?;
    let project = daw.current_project().await.ok()?;
    let tracks = project.tracks().all().await.ok()?;
    Some(strips_of(&tracks))
}

/// Each group's top-level track, in the groups' order.
fn strips_of(tracks: &[daw_proto::Track]) -> Vec<Strip> {
    GROUPS
        .iter()
        .filter_map(|(label, words)| {
            let track = tracks.iter().find(|t| {
                let name = t.name.to_lowercase();
                t.parent_guid.is_none() && words.iter().any(|w| name.contains(w))
            })?;
            Some(Strip {
                label,
                track: track.clone(),
            })
        })
        .collect()
}

/// How tall a fader's travel is, in pixels.
const TRAVEL: f64 = 220.0;
/// The fader cap's height.
const CAP_H: f64 = 44.0;

/// One strip: the name, a fader moved only by its cap, mute and solo.
#[component]
fn TouchStrip(strip: Strip, held: Signal<Option<String>>, on_edit: EventHandler<Edit>) -> Element {
    let guid = strip.track.guid.clone();
    let track = strip.track.clone();
    let norm = daw_theme_art::paint::tcp::gain_norm(track.volume).clamp(0.0, 1.0);
    let cap_top = (1.0 - norm) * TRAVEL;
    // Where the drag began: the pointer's y, and the fader then.
    let mut grip = use_signal(|| None::<(f64, daw_proto::Track)>);
    let holding = grip().is_some();
    let db = 20.0 * track.volume.max(1e-9).log10();
    let db_text = if track.volume <= 0.0 {
        "-inf".to_owned()
    } else {
        format!("{db:+.1}")
    };
    let (mute_bg, mute_fg) = if track.muted {
        ("#b45309", "#fff")
    } else {
        ("#23262c", DIM)
    };
    let (solo_bg, solo_fg) = if track.soloed {
        ("#ca8a04", "#111")
    } else {
        ("#23262c", DIM)
    };
    let cap_color = if holding { "#3aa0ff" } else { "#d1d5db" };
    let drag_guid = guid.clone();
    let mute_guid = guid.clone();
    let solo_guid = guid.clone();
    rsx! {
        div {
            style: "width:84px; flex:none; display:flex; flex-direction:column; align-items:center; gap:8px;",
            span { style: "font-size:14px; font-weight:650;", "{strip.label}" }
            span { style: "font-size:11px; color:{DIM}; font-variant-numeric:tabular-nums;", "{db_text} dB" }
            // The fader: its lane takes the moves while the cap is held, so
            // a finger that slides off the cap keeps it; a press on the lane
            // alone does nothing.
            div {
                style: "position:relative; width:84px; height:{TRAVEL + CAP_H}px; flex:none;",
                onmousemove: move |e| {
                    let Some((from_y, was)) = grip() else { return };
                    let y = e.data().client_coordinates().y;
                    let fraction = (from_y - y) / TRAVEL;
                    if let Some(edit) = crate::engine::drag(crate::mcp::Control::Volume, &drag_guid, &was, fraction) {
                        on_edit.call(edit);
                    }
                },
                onmouseup: move |_| {
                    grip.set(None);
                    held.set(None);
                },
                onmouseleave: move |_| {
                    grip.set(None);
                    held.set(None);
                },
                // The slot.
                div {
                    style: "position:absolute; left:39px; top:{CAP_H / 2.0}px; width:6px; height:{TRAVEL}px; \
                            border-radius:3px; background:#2a2d33;",
                }
                // The cap: the only place a fader is taken.
                div {
                    style: "position:absolute; left:12px; top:{cap_top}px; width:60px; height:{CAP_H}px; \
                            border-radius:10px; background:{cap_color}; box-shadow:0 2px 6px rgba(0,0,0,0.5); \
                            display:flex; align-items:center; justify-content:center;",
                    onmousedown: {
                        let guid = guid.clone();
                        let track = track.clone();
                        move |e: MouseEvent| {
                            grip.set(Some((e.data().client_coordinates().y, track.clone())));
                            held.set(Some(guid.clone()));
                        }
                    },
                    div { style: "width:36px; height:3px; border-radius:2px; background:#4b5563;" }
                }
            }
            button {
                style: "width:72px; height:44px; border-radius:10px; border:1px solid {RULE}; \
                        background:{mute_bg}; color:{mute_fg}; font-family:inherit; font-size:15px; \
                        font-weight:700; cursor:pointer;",
                onclick: move |_| on_edit.call(Edit::ToggleMute(mute_guid.clone())),
                "M"
            }
            button {
                style: "width:72px; height:44px; border-radius:10px; border:1px solid {RULE}; \
                        background:{solo_bg}; color:{solo_fg}; font-family:inherit; font-size:15px; \
                        font-weight:700; cursor:pointer;",
                onclick: move |_| on_edit.call(Edit::ToggleSolo(solo_guid.clone())),
                "S"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strips_of;

    fn track(guid: &str, name: &str, parent: Option<&str>) -> daw_proto::Track {
        daw_proto::Track {
            guid: guid.into(),
            name: name.into(),
            parent_guid: parent.map(Into::into),
            ..daw_proto::Track::default()
        }
    }

    #[test]
    fn the_mixer_shows_the_five_groups_in_order_by_their_top_level_tracks() {
        let tracks = [
            track("g", "Guitar", None),
            track("v", "Vocal", None),
            track("k", "Kick", Some("d")),
            track("d", "DRUMS", None),
            track("c", "Click", None),
            track("x", "Keys", None),
        ];
        let labels: Vec<(&str, &str)> = strips_of(&tracks)
            .iter()
            .map(|s| (s.label, s.track.guid.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("Click", "c"),
                ("Vocal", "v"),
                ("Drums", "d"),
                ("Guitar", "g")
            ]
        );
    }
}
