//! Each song's own mode: the one it was last worked in, taken up again
//! whenever it is gone to.
//!
//! A song still being recorded is a Record song; a live-tracks song is a
//! Live one; a song being mixed is a Mix one. Picking it — from its tab,
//! the set menu, the start of the set — puts the window in its mode, and
//! changing the mode while on it is what it is remembered as. Kept by the
//! song's name (a streamed song's engine id is new each time it opens),
//! on this device: a file in the app's data folder, a page's local
//! storage.

use std::collections::HashMap;

use dioxus::prelude::*;
use session::modes::Mode;

use crate::setlist::Setlist;

/// Every song's remembered mode, by name.
fn read() -> HashMap<String, Mode> {
    let Some(text) = load() else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let (mode, name) = line.split_once('\t')?;
            let mode = Mode::ALL.into_iter().find(|m| m.slug() == mode)?;
            Some((name.to_owned(), mode))
        })
        .collect()
}

fn write(modes: &HashMap<String, Mode>) {
    let mut lines: Vec<String> = modes
        .iter()
        .map(|(name, mode)| format!("{}\t{name}", mode.slug()))
        .collect();
    lines.sort();
    store(&lines.join("\n"));
}

/// The mode `song` was last worked in, if it has one.
#[must_use]
pub fn of(song: &str) -> Option<Mode> {
    read().get(song).copied()
}

/// Remember `mode` as `song`'s.
pub fn remember(song: &str, mode: Mode) {
    let mut modes = read();
    if modes.get(song) != Some(&mode) {
        modes.insert(song.to_owned(), mode);
        write(&modes);
    }
}

#[cfg(feature = "native")]
fn path() -> Option<std::path::PathBuf> {
    Some(dirs::data_dir()?.join("Session").join("song-modes.tsv"))
}

#[cfg(feature = "native")]
fn load() -> Option<String> {
    std::fs::read_to_string(path()?).ok()
}

#[cfg(feature = "native")]
fn store(text: &str) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        tracing::warn!(error = %e, "song modes: could not be kept");
    }
}

#[cfg(all(feature = "web", not(feature = "native")))]
const KEY: &str = "fts-session-song-modes";

#[cfg(all(feature = "web", not(feature = "native")))]
fn load() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(KEY)
        .ok()?
}

#[cfg(all(feature = "web", not(feature = "native")))]
fn store(text: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(KEY, text);
    }
}

#[cfg(not(any(feature = "native", feature = "web")))]
fn load() -> Option<String> {
    None
}

#[cfg(not(any(feature = "native", feature = "web")))]
fn store(_text: &str) {}

/// The window's mode follows the song: gone to, a song puts the window in
/// the mode it was last worked in; the mode changed, the song on screen
/// keeps it. For a shell, over its setlist and its mode.
pub fn use_song_modes(setlist: Signal<Setlist>, mode: Signal<Mode>) {
    let mut mode = mode;
    // The song on screen, as it changes.
    let current = use_memo(move || setlist().current().map(|song| song.name.clone()));
    use_effect(move || {
        if let Some(song) = current()
            && let Some(kept) = of(&song)
            && *mode.peek() != kept
        {
            mode.set(kept);
        }
    });
    use_effect(move || {
        let now = mode();
        if let Some(song) = current.peek().clone() {
            remember(&song, now);
        }
    });
}
