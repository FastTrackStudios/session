//! The editing options the DAW view's main toolbar switches — REAPER's
//! main-toolbar toggles: snapping, grid lines, ripple per track, auto
//! crossfade, item grouping, locking.
//!
//! Process-wide, as REAPER's are: they are how the editor behaves, not a
//! property of one panel, and two arrangement panels that disagreed about
//! whether snapping is on would be one editor with two minds.
//!
//! Snapping and grid lines are honoured (`mousemap::resolve`, the widget's
//! grid pass), and locking (`arrange_edit::Editor::press`, the mixer's
//! holds). Ripple, auto crossfade and grouping are held for the
//! editor passes that do not exist yet — the toolbar shows their state
//! honestly as a setting, not as a behaviour.

use std::sync::atomic::{AtomicBool, Ordering};

/// One option: its switch, and what it starts as.
pub struct Option {
    on: AtomicBool,
}

impl Option {
    const fn new(on: bool) -> Self {
        Self {
            on: AtomicBool::new(on),
        }
    }

    #[must_use]
    pub fn get(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    pub fn set(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
    }

    /// Flip it, and say what it is now.
    pub fn toggle(&self) -> bool {
        !self.on.fetch_xor(true, Ordering::Relaxed)
    }
}

/// Edits land on the grid. Shift still frees a drag.
pub static SNAP: Option = Option::new(true);
/// The grid is drawn through the lanes.
pub static GRID: Option = Option::new(true);
/// Moving or trimming an item moves what comes after it on its track.
pub static RIPPLE: Option = Option::new(false);
/// Overlapping items crossfade on their own.
pub static AUTO_CROSSFADE: Option = Option::new(true);
/// Grouped items move together.
pub static GROUPING: Option = Option::new(true);
/// Locked, nothing is moved by a drag: an arrangement's items stay put
/// and the drag scrolls the view; a mixer's faders, pans and rack
/// controls stay put and the drag scrolls the mixer. Presses still
/// press — mute, solo, arm, select. On by default: on a touchscreen a
/// finger meant to scroll or zoom is the common case, and moving a take
/// by accident mid-service is the costly one.
///
/// Off under test: every editing test would otherwise be a test of the
/// lock. The lock's own tests turn it on.
pub static LOCKING: Option = Option::new(!cfg!(test));
