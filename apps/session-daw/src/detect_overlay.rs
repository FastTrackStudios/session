//! What the drum detection found, for the arrangement to draw over the
//! kit while the settings are tuned.
//!
//! Perfect Timing! draws a line at every hit it detects, so turning the
//! sensitivity shows at once which hits come and go. This is the same,
//! on the arrangement itself: each look at the detection
//! (`DRUMS_DETECT`, `DRUMS_EDIT_SET`) puts its moments here, and the
//! window — in Edit mode — draws them over the drum rows, coloured by
//! what a slip would do with each, with the fills shaded. A slip clears
//! them: the hits have moved, and a look after it says where to.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What a slip would do with a hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Move it to its target.
    Move,
    /// Leave it: it is on its target already.
    Stay,
    /// Leave it: no target is near enough to be sure of — to be listened
    /// to.
    Check,
    /// Leave it: it is in a fill.
    Fill,
}

/// One detected moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub time: f64,
    pub target: Option<f64>,
    pub mark: Mark,
}

/// One look's findings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    /// In time order.
    pub lines: Vec<Line>,
    /// The fills, `(from, to)` seconds.
    pub fills: Vec<(f64, f64)>,
}

impl Overlay {
    /// The lines between `from` and `to` seconds.
    #[must_use]
    pub fn between(&self, from: f64, to: f64) -> &[Line] {
        let a = self.lines.partition_point(|l| l.time < from);
        let b = self.lines.partition_point(|l| l.time <= to);
        self.lines.get(a..b.max(a)).unwrap_or_default()
    }
}

static SHOWN: Mutex<Option<Arc<Overlay>>> = Mutex::new(None);
static VERSION: AtomicU64 = AtomicU64::new(0);

/// Show `overlay` (its lines sorted here).
pub fn publish(mut overlay: Overlay) {
    overlay.lines.sort_by(|a, b| a.time.total_cmp(&b.time));
    if let Ok(mut shown) = SHOWN.lock() {
        *shown = Some(Arc::new(overlay));
    }
    VERSION.fetch_add(1, Ordering::Relaxed);
}

/// Show nothing.
pub fn clear() {
    if let Ok(mut shown) = SHOWN.lock()
        && shown.take().is_some()
    {
        VERSION.fetch_add(1, Ordering::Relaxed);
    }
}

/// What is shown, if anything.
#[must_use]
pub fn current() -> Option<Arc<Overlay>> {
    SHOWN.lock().ok().and_then(|s| s.clone())
}

/// Goes up on every change: a view compares it with the one it drew.
#[must_use]
pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_finds_the_lines_in_view() {
        let line = |time| Line {
            time,
            target: None,
            mark: Mark::Stay,
        };
        let overlay = Overlay {
            lines: vec![line(1.0), line(2.0), line(3.0), line(4.0)],
            fills: Vec::new(),
        };
        let times: Vec<f64> = overlay.between(1.5, 3.0).iter().map(|l| l.time).collect();
        assert_eq!(times, [2.0, 3.0]);
        assert!(overlay.between(5.0, 6.0).is_empty());
        assert!(
            overlay.between(3.0, 1.0).is_empty(),
            "a span the wrong way round"
        );
    }
}
