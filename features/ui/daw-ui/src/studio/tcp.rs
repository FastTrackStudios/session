//! The track control panel column.
//!
//! The rows themselves are [`crate::components::tcp::TrackRow`] — the
//! real TCP, laid out at coordinates measured off REAPER and drawing the
//! same `daw_theme_art` vector controls that the REAPER theme's art is
//! rasterised from. Routing, FX, phase, envelope, record arm, monitor,
//! pan, volume, the meter, and mute/solo in their own gutter: all of it,
//! because all of it is what a track panel is for.
//!
//! This module is only the column around them. Writing a simpler row
//! here would not have been a simplification — it would have been a
//! second, worse track panel sitting beside the one that was matched to
//! REAPER on purpose, and the two would have drifted from the first edit
//! onwards.
//!
//! # What the rows need above them
//!
//! The controls are self-wired: each reads its track from
//! [`crate::controls::TrackStore`] and writes through `daw_control` on
//! its own, which is why nothing here threads state down. But that only
//! works if the store, its flush loop and the meter feed exist — see
//! [`super::Studio`], which mounts `use_daw_tracks`,
//! [`crate::controls::ControlSync`] and [`crate::controls::MeterFeed`]
//! once for the window.
//!
//! # Culling: the engine's, not ours
//!
//! The whole track list is mounted and `content-visibility: auto` (see
//! [`super::css`]) lets the engine skip style, layout and paint for the
//! rows that are off screen. Nothing is virtualised in Rust, and there
//! is no scroll listener — panning re-renders nothing at all.
//!
//! There WAS a recycling pool here, on the theory that 154 nodes a row
//! of SVG vector control, times sixty-five tracks, was what cost the
//! frames. It was not. A framework-free page with this column's exact
//! shape — sixty-five sticky rows, a 24,000px gradient surface, nine
//! hundred positioned blocks AND 845 inline SVGs — scrolls at a locked
//! 62 fps in plain MiniBrowser on the same WebKit, GTK and GPU. The
//! pool halved the DOM (12,397 nodes to 5,472) and bought nothing:
//! scrolling stayed at 2.4 fps.
//!
//! So it is gone, and with it the scroll listener it needed — which was
//! a dioxus event crossing the IPC on every scroll frame, for a
//! virtualiser that was solving a problem this window does not have.

use crate::prelude::*;
use daw_theme_art::geometry::tcp::{ROW_H, ROW_W};

/// Render only part of a row, to find what a scroll is actually paying
/// for. `FTS_STUDIO_PROBE=row:` one of:
///
/// - `none`   — an empty row. The floor: 65 plain divs in this app.
/// - `mute`   — two toggle buttons.
/// - `fader`  — the volume fader.
/// - `knob`   — the pan knob.
/// - `meter`  — the track meter.
/// - anything else (or unset) — the real [`TrackRow`].
///
/// A framework-free page with this column's shape and 845 inline SVGs
/// holds 62 fps; the real column manages 9.2. The difference is inside
/// one component, so this splits it by control family rather than by
/// guesswork — the same shape of experiment that found the column in
/// the first place.

/// A row plus the one-pixel divider under it — the pitch the lanes
/// beside the column have to match, and the single fact that keeps the
/// two columns level.
pub const ROW_PITCH: f32 = ROW_H + 1.0;

/// The column's width, from the panel's own measured geometry.
pub const COLUMN_W: f32 = ROW_W;

/// The column. `rows` is the folder-filtered track list with its depths,
/// shared rather than copied — it is re-read whenever the window
/// re-renders and nothing here mutates it.
#[component]
pub fn TcpColumn(
    rows: super::RowsRef,
    folders: Signal<crate::components::folders::FolderState>,
) -> Element {
    rsx! {
        div { class: "studio-tcp", "data-testid": "studio-tcp",
            for (track, depth) in rows.iter() {
                {
                    let guid = track.guid.clone();
                    let collapsed = folders.read().is_collapsed(&guid);
                    let probes = use_hook(super::probe::Probes::from_env);
                    let probe = if probes.at_least(2) {
                        probes.row().map(str::to_owned)
                    } else {
                        Some("none".to_owned())
                    };
                    rsx! {
                        div { key: "{guid}", class: "studio-row",
                            match probe.as_deref() {
                                Some("none") => rsx! {},
                                Some("mute") => rsx! {
                                    crate::controls::MuteButton { track: guid.clone() }
                                    crate::controls::SoloButton { track: guid.clone() }
                                },
                                Some("fader") => rsx! {
                                    crate::controls::VolumeFader { track: guid.clone() }
                                },
                                Some("knob") => rsx! {
                                    crate::controls::PanKnob { track: guid.clone() }
                                },
                                Some("meter") => rsx! {
                                    crate::controls::TrackMeter { track: guid.clone() }
                                },
                                _ => rsx! {
                                    crate::components::tcp::TrackRow {
                                        track: track.clone(),
                                        index: track.index,
                                        height: ROW_H,
                                        depth: *depth,
                                        selected: track.selected,
                                        collapsed,
                                        onfoldertoggle: move |()| folders.write().toggle(&guid),
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}
