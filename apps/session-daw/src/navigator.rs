//! The setlist navigator: the small-screen layout's Control view
//! ([`crate::compact`]).
//!
//! `session-ui`'s own navigator rows ([`SongItem`]): each song a bar filled
//! as far as the set has got through it, and the one playing opened into
//! its sections — each a bar of its own, filled as it plays, with the song's
//! progress running down beside them. A vertical progress bar, and the place
//! another song is picked: a press on a song picks it, a press on a section
//! of the song playing goes there.

use dioxus::prelude::*;
use session_ui::components::sidebar_items::{SectionItem, SongItem, SongItemData};

use crate::engine::{Move, transport};
use crate::setlist::{Setlist, Song};
use crate::shell::{DIM, RULE};

/// The navigator, over the setlist in context. `on_pick` picks a song, as
/// the wide layout's tabs do.
#[component]
pub fn Navigator(on_pick: EventHandler<usize>) -> Element {
    let setlist: Signal<Setlist> = use_context();
    let reading = crate::progress::use_reading();
    let r = reading();
    let list = setlist.read();
    let rows: Vec<(usize, SongItemData, Option<usize>, Vec<f64>)> = list
        .songs
        .iter()
        .enumerate()
        .map(|(index, song)| {
            let playing = index == list.at;
            let at = if playing { r.at } else { song.left_at };
            let (data, current, starts) = song_item(song, at, list.progress_of(index, r.at));
            (index, data, playing.then_some(current).flatten(), starts)
        })
        .collect();
    let at_song = list.at;
    let pending = list.pending.clone();
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; overflow-y:auto; \
                    padding:10px 10px 16px; display:flex; flex-direction:column; gap:6px;",
            for (index, data, current, starts) in rows {
                SongItem {
                    key: "{index}",
                    song_data: data,
                    index,
                    is_expanded: index == at_song,
                    is_playing: index == at_song && r.playing,
                    current_section_index: current,
                    on_song_click: move |()| on_pick.call(index),
                    on_section_click: move |section: usize| {
                        if let Some(from) = starts.get(section) {
                            transport(Move::Seek, *from);
                        }
                    },
                }
            }
            // Songs still opening: their place in the set, greyed.
            for title in pending {
                div {
                    key: "pending-{title}",
                    style: "height:3rem; flex:none; display:flex; align-items:center; justify-content:center; \
                            border-radius:8px; border:1px dashed {RULE}; color:{DIM}; font-size:14px;",
                    "{title}"
                }
            }
        }
    }
}

/// One song as a navigator row: its sections, each filled as far as `at`
/// (seconds, in the song's project), and the song filled to `progress`
/// (0–1). Also which section `at` is in, and where each starts.
fn song_item(song: &Song, at: f64, progress: f64) -> (SongItemData, Option<usize>, Vec<f64>) {
    let parts = crate::progress::Song::of(&song.session);
    let (sections, current, starts) = parts.map_or_else(
        || (Vec::new(), None, Vec::new()),
        |parts| {
            let sections = parts
                .sections
                .iter()
                .zip(&parts.bar)
                .map(|((from, to), bar)| SectionItem {
                    label: bar.name.clone(),
                    progress: ((at - from) / (to - from).max(f64::EPSILON)).clamp(0.0, 1.0) * 100.0,
                    bright_color: bar.color.clone(),
                    muted_color: bar.color.clone(),
                    comment: None,
                })
                .collect();
            let starts = parts.sections.iter().map(|(from, _)| *from).collect();
            (sections, parts.current(at), starts)
        },
    );
    let data = SongItemData {
        label: song.name.clone(),
        progress: progress * 100.0,
        bright_color: song.color.clone(),
        muted_color: song.color.clone(),
        sections,
        key_label: None,
    };
    (data, current, starts)
}
