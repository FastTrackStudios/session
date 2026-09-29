//! The Setup view: the set and the routing — what
//! is done before a service rather than during one — and the way back to
//! the sets.
//!
//! Plain DOM, no widget: both hosts show the same page, and what it edits
//! is the [`Setlist`](crate::setlist::Setlist) the window holds. Flat, as
//! the bars are: a header strip, the set as a table of hairline rows, and
//! the routing beside it — sections, not cards. The window's switches are
//! the bottom bar's (transport, progress) and the views' own (the lock).

use dioxus::prelude::*;

use crate::setlist::Setlist;
use crate::shell::{DIM, RAISED, RULE, TEXT};

/// The Setup page.
#[component]
pub fn SetupView(
    /// Picking a song, when the host can switch to one.
    on_pick: Option<EventHandler<usize>>,
) -> Element {
    let setlist = try_use_context::<Signal<Setlist>>();
    let back = try_use_context::<crate::shell::Back>();
    let summary = setlist.map(|setlist| {
        let list = setlist();
        let secs: f64 = list.songs.iter().map(|s| (s.span.1 - s.span.0).max(0.0)).sum();
        let songs = list.songs.len() + list.pending.len();
        format!("{songs} songs · {}", length(secs))
    });
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; \
                    flex-direction:column; color:{TEXT}; font-size:13px; background:#0f1012;",
            // The header strip: back out of the set, and what the set is.
            div {
                style: "flex:none; height:52px; display:flex; align-items:stretch; \
                        border-bottom:1px solid {RULE}; background:#131417;",
                if let Some(crate::shell::Back(back)) = back {
                    button {
                        title: "Back to your sets",
                        style: "flex:none; display:flex; align-items:center; gap:6px; padding:0 16px 0 10px; \
                                border:none; border-right:1px solid {RULE}; background:transparent; \
                                color:{TEXT}; font-family:inherit; font-size:14px; font-weight:600; cursor:pointer;",
                        onclick: move |_| back.call(()),
                        lucide_dioxus::ChevronLeft { size: 20, color: "currentColor" }
                        "Sets"
                    }
                }
                div {
                    style: "flex:1; min-width:0; display:flex; align-items:baseline; gap:12px; padding:0 18px; \
                            align-self:center;",
                    span { style: "font-size:17px; font-weight:700;", "Setlist" }
                    if let Some(summary) = summary {
                        span { style: "font-size:13px; color:{DIM};", "{summary}" }
                    }
                }
            }
            div {
                style: "flex:1; min-height:0; display:flex; align-items:stretch;",
                // The set.
                div {
                    style: "flex:1; min-width:0; overflow-y:auto;",
                    match setlist {
                        Some(setlist) => rsx! { SongTable { setlist, on_pick } },
                        None => rsx! {
                            div { style: "padding:18px; color:{DIM};", "No setlist — this window opened one session." }
                        },
                    }
                }
                // The routing.
                div {
                    style: "flex:none; width:340px; overflow-y:auto; border-left:1px solid {RULE}; \
                            background:#131417;",
                    Heading { title: "Routing" }
                    div {
                        style: "padding:10px 18px 18px; color:{DIM}; line-height:1.6;",
                        "The engine's outputs, and what goes to them: the mains, the in-ear \
                         mixes, and which of the session's buses feed each. Not wired yet — the \
                         browser plays one stereo output, and the desktop engine's device \
                         routing is next."
                    }
                }
            }
        }
    }
}

/// A length as a person reads it: `4:05`, `1:02:30`.
fn length(secs: f64) -> String {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "whole seconds")]
    let whole = secs.max(0.0).round() as u64;
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A section's title in the side column: small capitals over a hairline.
#[component]
fn Heading(title: &'static str) -> Element {
    rsx! {
        div {
            style: "padding:16px 18px 6px; font-size:11px; font-weight:700; letter-spacing:0.08em; \
                    color:{DIM}; border-bottom:1px solid {RULE};",
            "{title.to_uppercase()}"
        }
    }
}

/// The table's columns, as widths: number, song (the rest), key, tempo,
/// length, mode, and the row's own buttons.
const COLUMNS: [&str; 7] = ["44px", "1fr", "72px", "64px", "72px", "96px", "144px"];

/// A cell's style for column `i`.
fn cell(i: usize, align: &str) -> String {
    let width = COLUMNS[i];
    let size = if width == "1fr" {
        "flex:1; min-width:0;".to_owned()
    } else {
        format!("flex:none; width:{width};")
    };
    format!(
        "{size} display:flex; align-items:center; justify-content:{align}; overflow:hidden; \
         white-space:nowrap;"
    )
}

/// The set, in order, as a table: which is up, what each song is — its
/// key, tempo, length and the mode it is worked in — and moving or taking
/// one out.
#[component]
fn SongTable(setlist: Signal<Setlist>, on_pick: Option<EventHandler<usize>>) -> Element {
    let list = setlist();
    if list.songs.is_empty() && list.pending.is_empty() {
        return rsx! {
            div { style: "padding:18px; color:{DIM};", "No songs loaded yet." }
        };
    }
    let count = list.songs.len();
    let head = "font-size:11px; font-weight:700; letter-spacing:0.08em; color:#6b7280;";
    rsx! {
        div {
            style: "display:flex; flex-direction:column;",
            div {
                style: "display:flex; height:34px; padding:0 8px 0 0; border-bottom:1px solid {RULE}; {head}",
                div { style: cell(0, "center"), "#" }
                div { style: cell(1, "flex-start"), "SONG" }
                div { style: cell(2, "flex-start"), "KEY" }
                div { style: cell(3, "flex-start"), "BPM" }
                div { style: cell(4, "flex-start"), "LENGTH" }
                div { style: cell(5, "flex-start"), "MODE" }
                div { style: cell(6, "flex-end"), "" }
            }
            for (index, song) in list.songs.iter().cloned().enumerate() {
                {
                    let current = index == list.at;
                    let ground = if current { RAISED } else { "transparent" };
                    let ink = if current { TEXT } else { "#c9ccd2" };
                    let project = &song.session.project;
                    let key = crate::arrangement::key_at(project, song.span.0)
                        .map_or_else(|| "—".to_owned(), |k| crate::transport_bar::short_key(&k));
                    let bpm = crate::transport_bar::tempo_text(project.bpm);
                    let long = length(song.span.1 - song.span.0);
                                        let mode = crate::song_modes::of(&song.name)
                        .map_or("—", session::modes::Mode::display_name);
                    let (earlier, later) = (index > 0, index + 1 < count);
                    rsx! {
                        div {
                            key: "{song.project}",
                            style: "position:relative; display:flex; height:56px; padding:0 8px 0 0; \
                                    border-bottom:1px solid {RULE}; background:{ground}; color:{ink};",
                            // The song's colour, down the row's edge.
                            div { style: "position:absolute; left:0; top:0; bottom:0; width:3px; background:{song.color};" }
                            div { style: "{cell(0, \"center\")} color:#6b7280; font-size:13px;", "{index + 1}" }
                            div {
                                style: "{cell(1, \"flex-start\")} gap:10px; cursor:pointer;",
                                onclick: move |_| {
                                    if let Some(pick) = on_pick {
                                        pick.call(index);
                                    }
                                },
                                span {
                                    style: "font-size:15px; font-weight:650; overflow:hidden; text-overflow:ellipsis;",
                                    "{song.name}"
                                }
                                if current {
                                    span { style: "flex:none; font-size:11px; font-weight:700; color:{song.color};", "NOW" }
                                }
                            }
                            div { style: "{cell(2, \"flex-start\")} font-weight:600;", "{key}" }
                            div { style: "{cell(3, \"flex-start\")} font-family:ui-monospace, monospace;", "{bpm}" }
                            div { style: "{cell(4, \"flex-start\")} font-family:ui-monospace, monospace; color:{DIM};", "{long}" }
                            div { style: "{cell(5, \"flex-start\")} color:{DIM};", "{mode}" }
                            div {
                                style: "{cell(6, \"flex-end\")} align-items:stretch;",
                                {row_button("Earlier in the set", earlier, RowIcon::Up,
                                    EventHandler::new(move |()| setlist.write().reorder(index, index - 1)))}
                                {row_button("Later in the set", later, RowIcon::Down,
                                    EventHandler::new(move |()| setlist.write().reorder(index, index + 1)))}
                                {row_button("Take it out of the set", true, RowIcon::Remove,
                                    EventHandler::new(move |()| setlist.write().remove(index)))}
                            }
                        }
                    }
                }
            }
            // Songs still opening: their place in the set, greyed.
            for (i, title) in list.pending.iter().cloned().enumerate() {
                div {
                    key: "pending-{title}",
                    style: "display:flex; height:56px; border-bottom:1px solid {RULE}; color:#6b7280; opacity:0.7;",
                    div { style: cell(0, "center"), "{count + i + 1}" }
                    div { style: cell(1, "flex-start"), "{title} — opening" }
                }
            }
        }
    }
}

/// What a row's button shows.
#[derive(Clone, Copy)]
enum RowIcon {
    Up,
    Down,
    Remove,
}

/// One of a row's buttons: flat, the row's full height, an icon — in its
/// colour outright, not `currentColor`: Blitz resolved an icon's
/// `currentColor` once, so a button made unusable (a row made as the set's
/// last, while it was still arriving) stayed grey once it could be used.
fn row_button(title: &'static str, enabled: bool, icon: RowIcon, on_press: EventHandler<()>) -> Element {
    let color = if enabled { DIM } else { "#34373d" };
    rsx! {
        button {
            title,
            style: "flex:none; width:48px; display:flex; align-items:center; justify-content:center; \
                    border:none; background:transparent; cursor:pointer; padding:0;",
            onclick: move |_| {
                if enabled {
                    on_press.call(());
                }
            },
            match icon {
                RowIcon::Up => rsx! { lucide_dioxus::ChevronUp { size: 18, color } },
                RowIcon::Down => rsx! { lucide_dioxus::ChevronDown { size: 18, color } },
                RowIcon::Remove => rsx! { lucide_dioxus::X { size: 16, color } },
            }
        }
    }
}
