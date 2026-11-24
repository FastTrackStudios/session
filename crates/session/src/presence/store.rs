//! Who else is in the session and what they are doing, and what this peer
//! is doing, held for the process: what an app's collaboration driver
//! publishes and sends, what a view paints (its ghosts) and reports this
//! peer's pointer and selection into. State only.

use std::sync::Mutex;

use super::{Pointer, Roster};

pub struct Published {
    pub roster: Roster,
    /// The roster as last published, in words, and when it last changed:
    /// the driver publishes every tick whether or not anyone moved.
    pub said: String,
    pub changed: web_time::Instant,
    /// Local clock → shared clock, milliseconds.
    pub clock_offset_ms: f64,
    /// Whose play cursors to draw (independent transports only).
    pub show_play: bool,
}

pub static ROSTER: Mutex<Option<Published>> = Mutex::new(None);

/// Publish the roster the arrangement should draw. `None` clears it
/// (the session was left).
pub fn publish(roster: Option<Roster>, clock_offset_ms: f64, show_play: bool) {
    if let Ok(mut slot) = ROSTER.lock() {
        *slot = roster.map(|roster| {
            let said = format!("{roster:?}");
            let changed = match slot.as_ref() {
                Some(was) if was.said == said => was.changed,
                _ => web_time::Instant::now(),
            };
            Published {
                roster,
                said,
                changed,
                clock_offset_ms,
                show_play,
            }
        });
    }
}

/// How long after the roster last changed its pointers may still be
/// gliding: their interpolation delay, and some.
pub const GLIDING: std::time::Duration = std::time::Duration::from_millis(600);

/// Someone else in the session, and the song they are on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// Their song's project here (what the tabs know it by).
    pub project: String,
    /// Their song's name everywhere (`Washed`).
    pub song: String,
    pub name: String,
    pub color: u32,
}

/// Everyone else in the session.
pub static EVERYONE: Mutex<Vec<Person>> = Mutex::new(Vec::new());

/// Publish who is on which song.
pub fn publish_everyone(everyone: Vec<Person>) {
    if let Ok(mut slot) = EVERYONE.lock()
        && *slot != everyone
    {
        *slot = everyone;
    }
}

/// Everyone else in the session.
#[must_use]
pub fn everyone() -> Vec<Person> {
    EVERYONE.lock().map(|all| all.clone()).unwrap_or_default()
}

/// The people on the song that is `project` here: (name, colour).
#[must_use]
pub fn on_song(project: &str) -> Vec<(String, u32)> {
    EVERYONE
        .lock()
        .map(|all| {
            all.iter()
                .filter(|p| p.project == project)
                .map(|p| (p.name.clone(), p.color))
                .collect()
        })
        .unwrap_or_default()
}

/// Anyone moving: the arrangement keeps repainting while someone's
/// pointer glides or their play cursor runs — and only then. Someone
/// merely present, still, needs no frames.
#[must_use]
pub fn active() -> bool {
    ROSTER.lock().ok().is_some_and(|slot| {
        slot.as_ref().is_some_and(|p| {
            !p.roster.peers.is_empty()
                && (p.changed.elapsed() < GLIDING
                    || (p.show_play
                        && p.roster
                            .peers
                            .values()
                            .any(|peer| peer.play.as_ref().is_some_and(|play| play.playing))))
        })
    })
}

/// What this peer is doing, as the arrangement last saw it — read by the
/// collaboration driver, which decides what to tell the others and when.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Local {
    /// The mouse: over the lanes a time and a track, anywhere else a
    /// place in a named panel ([`local_window_pointer`]).
    pub pointer: Option<Pointer>,
    pub selected_items: Vec<String>,
    pub selected_tracks: Vec<String>,
}

pub static LOCAL: Mutex<Local> = Mutex::new(Local {
    pointer: None,
    selected_items: Vec::new(),
    selected_tracks: Vec::new(),
});

/// Whether the mouse is over the lanes, where the arrangement reports it
/// as a time and a track — finer than any panel position.
pub static OVER_LANES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The mouse moved over the arrangement: a time and a track over the
/// lanes, `None` off them (the window overlay reports it from there).
/// The panel id of the arrangement's track panel: a pointer over it is
/// anchored to a track row there, and the arrangement paints it (a DOM
/// overlay would draw under the arrangement's canvas).
pub const TCP: &str = "tcp";

/// The arrangement's own rectangle, as a panel: a pointer over it (lanes
/// or track panel) is the arrangement's to publish.
pub const ARRANGEMENT: &str = "arrangement";

/// The pointer is over the arrangement's track panel: on `track` (`None`:
/// its header, above the rows), `u` across the panel, `v` down the row.
pub fn local_tcp_pointer(track: Option<String>, u: f64, v: f64) {
    OVER_LANES.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut local) = LOCAL.lock() {
        local.pointer = Some(Pointer::Anchor {
            panel: TCP.into(),
            key: track.unwrap_or_default(),
            u: u.clamp(0.0, 1.0),
            v,
        });
    }
}

pub fn local_pointer(pointer: Option<(f64, Option<String>, f64)>) {
    OVER_LANES.store(pointer.is_some(), std::sync::atomic::Ordering::Relaxed);
    if let Some((at, track, y)) = pointer
        && let Ok(mut local) = LOCAL.lock()
    {
        local.pointer = Some(Pointer::Timeline { at, track, y });
    }
}

/// A pointer the arrangement paints itself: over its lanes, or over its
/// track panel.
pub fn drawn_by_arrangement(pointer: &Pointer) -> bool {
    match pointer {
        Pointer::Timeline { .. } => true,
        Pointer::Anchor { panel, .. } => panel == TCP,
        _ => false,
    }
}

/// The selection, as drawn this frame: the selected items, and the
/// selected tracks' guids in row order.
pub fn local_selection(
    items: &std::collections::HashSet<String>,
    selected_tracks: impl IntoIterator<Item = String>,
) {
    if let Ok(mut local) = LOCAL.lock() {
        let mut items: Vec<String> = items.iter().cloned().collect();
        items.sort();
        let tracks: Vec<String> = selected_tracks.into_iter().collect();
        if local.selected_items != items {
            local.selected_items = items;
        }
        if local.selected_tracks != tracks {
            local.selected_tracks = tracks;
        }
    }
}

/// The arrangement's track guids and item guids as last drawn (the
/// collaboration puppet points at real ones).
pub static SHOWN: Mutex<(Vec<String>, Vec<String>, Vec<String>)> =
    Mutex::new((Vec::new(), Vec::new(), Vec::new()));

#[must_use]
pub fn local_rows() -> Vec<String> {
    SHOWN.lock().map(|s| s.0.clone()).unwrap_or_default()
}

/// A shown track's name, by guid.
#[must_use]
pub fn local_track_name(guid: &str) -> Option<String> {
    let shown = SHOWN.lock().ok()?;
    let i = shown.0.iter().position(|g| g == guid)?;
    shown.2.get(i).cloned()
}

#[must_use]
pub fn local_item_guids() -> Vec<String> {
    SHOWN.lock().map(|s| s.1.clone()).unwrap_or_default()
}

/// This peer's presence as the arrangement last saw it.
#[must_use]
pub fn local() -> Local {
    LOCAL.lock().map(|l| l.clone()).unwrap_or_default()
}

/// Milliseconds on this machine's clock, for presence timing.
#[must_use]
pub fn now_ms() -> f64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}
