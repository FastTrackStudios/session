//! The arrangement — the ruler and the lanes.
//!
//! Every position in here is expressed in **seconds**, handed to CSS as
//! a custom property, and turned into pixels by `calc(var(--t0) *
//! var(--pps) * 1px)`. That is the load-bearing decision of this module.
//! Zooming a DAW changes one number, and if that number lives in Rust
//! then every item, marker, region and grid line has to be re-rendered
//! to move — for a gesture that changed no data whatsoever. Living in
//! CSS instead, a zoom is a single custom-property write on the root and
//! the browser's own style engine does the rest, at compositor speed.
//!
//! The bar grid has no elements at all: it is one repeating gradient
//! whose period is a bar, so a session's worth of grid lines costs a
//! background instead of thousands of nodes that exist only to be lines.
//!
//! Lanes are mounted whole and culled by `content-visibility: auto`.
//! See [`super::tcp`] for why that beats a hand-written recycler.

use crate::prelude::*;

use super::clock::Clock;
use super::{ProjectRef, RowsRef};

/// Beyond this, bar numbers stop being labels and start being a way to
/// spend a second mounting text nobody can read. The grid lines are a
/// gradient and continue regardless.
const MAX_BAR_LABELS: usize = 2_000;

/// The ruler: region bands, marker flags, bar numbers — all on the
/// timeline's own axis, so they pan and zoom with the material below
/// rather than beside it.
#[component]
pub fn Ruler(project: ProjectRef, clock: Clock) -> Element {
    let secs_per_bar = 4.0 * 60.0 / project.bpm.max(1.0);
    let bars = ((project.length_secs / secs_per_bar).ceil() as usize).min(MAX_BAR_LABELS);

    rsx! {
        div { class: "studio-ruler", "data-testid": "studio-ruler",
            div { class: "studio-region-lane", "data-testid": "studio-regions",
                for section in project.sections.iter() {
                    {
                        let start = section.start;
                        let color = section.color.clone().unwrap_or_else(|| "#5a5a66".to_owned());
                        rsx! {
                            div {
                                key: "{section.start}-{section.name}",
                                class: "studio-region",
                                style: "--t0: {section.start}; --dur: {(section.end - section.start).max(0.0)}; background: {color};",
                                title: "{section.name}",
                                onclick: {
                                let clock = clock.clone();
                                move |_| clock.seek(start)
                            },
                                "{section.name}"
                            }
                        }
                    }
                }
            }
            div { class: "studio-marker-lane", "data-testid": "studio-markers",
                for marker in project.markers.iter() {
                    {
                        let at = marker.at;
                        let color = marker.color.clone().unwrap_or_else(|| "#8a8a92".to_owned());
                        rsx! {
                            div {
                                key: "{marker.idx}-{marker.at}",
                                class: "studio-marker",
                                style: "--t0: {marker.at}; background: {color};",
                                title: "{marker.name}",
                                onclick: {
                                let clock = clock.clone();
                                move |_| clock.seek(at)
                            },
                                "{marker.idx} {marker.name}"
                            }
                        }
                    }
                }
            }
            div { class: "studio-bar-lane",
                for bar in 0..bars {
                    {
                        let at = bar as f64 * secs_per_bar;
                        rsx! {
                            div { key: "{bar}", class: "studio-bar", style: "--t0: {at};", "{bar + 1}" }
                        }
                    }
                }
            }
        }
    }
}

/// The lanes: one per visible track, in the same pitch the TCP column
/// uses, with the playhead over them.
#[component]
pub fn Lanes(project: ProjectRef, rows: RowsRef, clock: Clock) -> Element {
    rsx! {
        div {
            class: "studio-lanes",
            "data-testid": "studio-lanes",
            // The row divs used to give this its height; the canvas
            // draws them now, so the size has to be stated.
            style: "height: {rows.len() as f32 * super::tcp::ROW_PITCH}px;",
            // Clicking empty timeline moves the playhead, which is what
            // every DAW does and what makes a project explorable with a
            // mouse alone.
            onclick: {
                let clock = clock.clone();
                move |event: MouseEvent| {
                    let x = event.element_coordinates().x;
                    clock.seek((x / clock.pps().max(0.001)).max(0.0));
                }
            },
            // The lanes are DRAWN, not built — see `super::canvas`.
            // This element only carries the content size, so the
            // scrollbars tell the truth; the canvas overlays it.
            super::canvas::LaneCanvas { project: project.clone(), rows: rows.clone() }
            div { class: "studio-playhead", "data-testid": "studio-playhead" }
        }
    }
}
