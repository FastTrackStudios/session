//! The editing options the DAW view's main toolbar switches — REAPER's
//! main-toolbar toggles: snapping, grid lines, ripple per track, auto
//! crossfade, item grouping, locking.
//!
//! Process-wide, as REAPER's are: they are how the editor behaves, not a
//! property of one panel, and two arrangement panels that disagreed about
//! whether snapping is on would be one editor with two minds.
//!
//! Snapping and grid lines are honoured (`mousemap::resolve`, the widget's
//! grid pass). Ripple, auto crossfade, grouping and locking are held for the
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
/// Locked items cannot be moved.
pub static LOCKING: Option = Option::new(false);
