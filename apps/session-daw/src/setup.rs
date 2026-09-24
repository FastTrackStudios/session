//! The Setup view: the setlist, and the routing — what is done before a
//! service rather than during one.
//!
//! Plain DOM, no widget: both hosts show the same page, and what it edits
//! is the [`Setlist`](crate::setlist::Setlist) the window holds. Picking a
//! song here is picking the song everywhere — the host follows the
//! setlist's `at` and tells the engine which project is current.

use dioxus::prelude::*;

use crate::setlist::Setlist;
use crate::shell::{DIM, RULE, TEXT};

/// The Setup page.
#[component]
pub fn SetupView(
    /// Picking a song, when the host can switch to one.
    on_pick: Option<EventHandler<usize>>,
) -> Element {
    let setlist = try_use_context::<Signal<Setlist>>();
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; overflow:auto; \
                    padding:20px; display:flex; gap:20px; align-items:flex-start; \
                    color:{TEXT}; font-size:13px;",
            Panel { title: "Setlist".to_owned(), width: "minmax(420px, 1fr)".to_owned(),
                match setlist {
                    Some(setlist) => rsx! { SongList { setlist, on_pick } },
                    None => rsx! {
                        div { style: "color:{DIM};", "No setlist — this window opened one session." }
                    },
                }
            }
            Panel { title: "Routing".to_owned(), width: "minmax(280px, 0.6fr)".to_owned(),
                div {
                    style: "color:{DIM}; line-height:1.6;",
                    "The engine's outputs, and what goes to them, belong here: the mains, \
                     the in-ear mixes, and which of the session's buses feed each."
                    div {
                        style: "margin-top:10px; padding:8px 10px; border:1px dashed {RULE}; \
                                border-radius:6px;",
                        "Not wired yet — the browser plays one stereo output, and the \
                         desktop engine's device routing is next."
                    }
                }
            }
        }
    }
}

/// A titled panel on the Setup page.
#[component]
fn Panel(title: String, width: String, children: Element) -> Element {
    rsx! {
        section {
            style: "flex:1 1 {width}; min-width:0; background:#131417; border:1px solid {RULE}; \
                    border-radius:10px; padding:14px 16px;",
            h2 {
                style: "margin:0 0 12px; font-size:13px; font-weight:600; color:{TEXT}; \
                        letter-spacing:0.02em;",
                "{title}"
            }
            {children}
        }
    }
}

/// The songs, in order: which is up, and what each one is.
#[component]
fn SongList(setlist: Signal<Setlist>, on_pick: Option<EventHandler<usize>>) -> Element {
    let list = setlist();
    if list.songs.is_empty() {
        return rsx! {
            div { style: "color:{DIM};", "No songs loaded yet." }
        };
    }
    let count = list.songs.len();
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            for (index, song) in list.songs.iter().cloned().enumerate() {
                {
                    let current = index == list.at;
                    let (background, border) = if current {
                        ("#1b1e24", song.color.clone())
                    } else {
                        ("#0f1012", RULE.to_owned())
                    };
                    let minutes = (song.span.1 - song.span.0).max(0.0);
                    let length = format!("{}:{:02}", (minutes / 60.0) as u32, (minutes % 60.0) as u32);
                    rsx! {
                        div {
                            style: "display:flex; align-items:center; gap:10px; padding:8px 10px; \
                                    background:{background}; border:1px solid {border}; \
                                    border-radius:8px;",
                            div {
                                style: "width:8px; height:28px; border-radius:3px; \
                                        background:{song.color}; flex:none;",
                            }
                            div {
                                style: "flex:1; min-width:0; cursor:pointer;",
                                onclick: move |_| {
                                    if let Some(pick) = on_pick {
                                        pick.call(index);
                                    }
                                },
                                div {
                                    style: "font-weight:600; overflow:hidden; \
                                            text-overflow:ellipsis; white-space:nowrap;",
                                    "{song.name}"
                                }
                                div { style: "color:{DIM}; font-size:11px;", "{length}" }
                            }
                            Step { label: "↑".to_owned(), enabled: index > 0,
                                on_press: move |()| setlist.write().reorder(index, index - 1) }
                            Step { label: "↓".to_owned(), enabled: index + 1 < count,
                                on_press: move |()| setlist.write().reorder(index, index + 1) }
                            Step { label: "✕".to_owned(), enabled: true,
                                on_press: move |()| setlist.write().remove(index) }
                        }
                    }
                }
            }
        }
    }
}

/// One of a row's small buttons.
#[component]
fn Step(label: String, enabled: bool, on_press: EventHandler<()>) -> Element {
    let color = if enabled { DIM } else { "#3a3d44" };
    rsx! {
        button {
            style: "width:26px; height:26px; flex:none; border-radius:6px; \
                    border:1px solid {RULE}; background:#0f1012; color:{color}; \
                    font-size:12px; cursor:pointer;",
            disabled: !enabled,
            onclick: move |_| {
                if enabled {
                    on_press.call(());
                }
            },
            "{label}"
        }
    }
}
