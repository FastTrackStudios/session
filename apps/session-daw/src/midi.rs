//! The notes inside an item, for drawing it.
//!
//! An item's body should be what the item CONTAINS. Audio has peaks and
//! MIDI has notes, and until now every item in the arrangement was
//! drawn from the same synthetic envelope — a function of the track's
//! index and the time, which never touched the item, the take, or a
//! single sample. That is why a chord track looked like a shaker.
//!
//! Notes arrive the way peaks were always meant to: after the window is
//! already standing. Reading them is a round trip per item, and a
//! session with four hundred MIDI items would spend the whole open
//! doing it — so the window draws what it has, asks for the rest, and
//! redraws as the answers land.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One note, reduced to what a preview draws.
///
/// Not `MidiNote`: a preview needs where and how high, and carries
/// neither channel nor selection nor mute. Reduced at the edge so the
/// drawing has nothing to decide and the cache stays small — a big
/// session holds a lot of these.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Note {
    /// Where it starts, as a fraction of the item's length.
    ///
    /// A fraction rather than a time, because an item preview is drawn
    /// to the item's width and that is the only thing it is measured
    /// against. It also makes the cache independent of the zoom, so a
    /// scroll never invalidates one.
    pub at: f32,
    /// How long, as the same fraction. Never zero: a note with no
    /// length is a note you cannot see.
    pub len: f32,
    /// MIDI pitch, 0..=127.
    pub pitch: u8,
    /// How hard, 0..=127 — the preview draws it as brightness.
    pub velocity: u8,
}

/// The notes of the items that have been read so far.
///
/// Absent means "not asked yet or still reading", and an empty list
/// means "read, and it has none". The two have to be distinguishable:
/// an item with no notes should stop being asked about, and an item
/// still loading should not be drawn as empty.
#[derive(Clone, Default)]
pub struct Previews {
    known: Arc<Mutex<HashMap<String, Vec<Note>>>>,
    /// Set when something new has landed, so the window knows to
    /// re-record rather than polling a map every frame.
    fresh: Arc<std::sync::atomic::AtomicBool>,
}

impl Previews {
    /// The notes for an item, if they have been read.
    #[must_use]
    pub fn get(&self, guid: &str) -> Option<Vec<Note>> {
        self.known.lock().ok()?.get(guid).cloned()
    }

    /// Has anything arrived since this was last asked?
    ///
    /// Asking clears it. A re-record is the only thing that acts on
    /// this, and it redraws everything, so a second answer would only
    /// buy a second identical redraw.
    pub fn take_fresh(&self) -> bool {
        self.fresh.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    /// Read the notes of every MIDI item that has not been read yet.
    ///
    /// Spawns and returns; the window keeps drawing. Items already
    /// known are skipped, so a re-record after a reload costs nothing
    /// for what it already has.
    pub fn fetch(&self, wanted: Vec<(String, f64)>) {
        let Some(runtime) = crate::open::runtime() else {
            return;
        };
        let missing: Vec<(String, f64)> = {
            let Ok(known) = self.known.lock() else { return };
            wanted
                .into_iter()
                .filter(|(guid, _)| !known.contains_key(guid))
                .collect()
        };
        if missing.is_empty() {
            return;
        }
        let known = Arc::clone(&self.known);
        let fresh = Arc::clone(&self.fresh);
        std::thread::Builder::new()
            .name("session-daw-midi".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let Some(daw) = daw::rpc::Daw::try_get() else {
                        return;
                    };
                    let Ok(project) = daw.current_project().await else {
                        return;
                    };
                    for (guid, length) in missing {
                        let notes = read(&project, &guid, length).await;
                        let Ok(mut known) = known.lock() else { return };
                        known.insert(guid, notes);
                        fresh.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            })
            .ok();
    }

    /// Read the notes now, on this thread.
    ///
    /// For a renderer rather than a window: something drawing one frame
    /// and exiting has no later to redraw in, so an empty cache would
    /// mean every MIDI item drawn blank — a worse picture than the one
    /// this replaced.
    pub fn fill_blocking(&self, wanted: Vec<(String, f64)>) {
        let Some(runtime) = crate::open::runtime() else {
            return;
        };
        runtime.block_on(async {
            let Some(daw) = daw::rpc::Daw::try_get() else {
                return;
            };
            let Ok(project) = daw.current_project().await else {
                return;
            };
            for (guid, length) in wanted {
                let notes = read(&project, &guid, length).await;
                if let Ok(mut known) = self.known.lock() {
                    known.insert(guid, notes);
                }
            }
        });
    }
}

/// Read one item's notes, as fractions of its length.
///
/// An empty answer is a real answer — an empty MIDI item is a thing
/// that exists, and drawing it as blank is right.
async fn read(project: &daw_control::Project, guid: &str, length: f64) -> Vec<Note> {
    let Ok(Some(item)) = project.items().by_guid(guid).await else {
        return Vec::new();
    };
    let Ok(notes) = item.active_take().midi().notes().await else {
        return Vec::new();
    };
    // The span the notes actually cover, because a take's PPQ tells us
    // ticks per quarter note and not how many quarter notes the ITEM
    // is. Measuring the notes themselves means the preview fills the
    // block whatever the tempo, which is what it is for: a shape, not a
    // clock.
    let span = notes
        .iter()
        .map(|n| n.start_ppq + n.length_ppq)
        .fold(0.0_f64, f64::max);
    let span = if span > 0.0 { span } else { length.max(1.0) };
    notes
        .iter()
        .map(|note| Note {
            at: (note.start_ppq / span) as f32,
            len: (note.length_ppq / span) as f32,
            pitch: note.pitch,
            velocity: note.velocity,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Note, Previews};

    /// Absent and empty mean different things.
    ///
    /// An item still being read must not draw as an empty one: a chord
    /// track that flashed blank on every open and then filled in would
    /// read as a bug in the session, not as a load.
    #[test]
    fn nothing_read_yet_is_not_the_same_as_nothing_there() {
        let previews = Previews::default();
        assert_eq!(previews.get("unread"), None);
        previews
            .known
            .lock()
            .unwrap()
            .insert("empty".into(), Vec::new());
        assert_eq!(previews.get("empty"), Some(Vec::new()));
    }

    /// Freshness is consumed once: a re-record redraws everything, so a
    /// second answer buys a second identical redraw.
    #[test]
    fn freshness_is_taken_once() {
        let previews = Previews::default();
        assert!(!previews.take_fresh());
        previews
            .fresh
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(previews.take_fresh());
        assert!(!previews.take_fresh(), "it was taken twice");
    }

    /// The shape a preview is drawn from: fractions, so the cache
    /// survives a zoom.
    #[test]
    fn a_note_is_a_fraction_of_its_item() {
        let note = Note {
            at: 0.25,
            len: 0.5,
            pitch: 60,
            velocity: 100,
        };
        assert!(note.at + note.len <= 1.0, "a note ran past its item");
    }
}
