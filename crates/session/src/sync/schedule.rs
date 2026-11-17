//! Timed events an audio thread reads: Signal's patch changes, Ignition's
//! cues when its output is clocked by the engine.
//!
//! Built off the audio thread from the session's items (sorted, resolved to
//! seconds under the tempo map), handed to the audio thread whole by the
//! engine's own command queue, and read there with no lock and no
//! allocation: each buffer asks what falls in its span, a locate asks what
//! state to chase. A tempo-map or item edit builds a new schedule; the old
//! one is dropped off the audio thread (the engine's job, as for any
//! buffer it hands over).

/// Events at seconds, sorted. Read-only once built.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule<T> {
    events: Vec<(f64, T)>,
}

impl<T> Default for Schedule<T> {
    fn default() -> Self {
        Self { events: Vec::new() }
    }
}

impl<T> Schedule<T> {
    /// A schedule of `events` (any order; NaN times are dropped). Not for
    /// the audio thread: it sorts.
    #[must_use]
    pub fn new(mut events: Vec<(f64, T)>) -> Self {
        events.retain(|(at, _)| !at.is_nan());
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { events }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The events at `from <= t < to`, in order: what a buffer spanning
    /// those seconds plays. Binary search over a slice: no lock, no
    /// allocation.
    #[must_use]
    pub fn between(&self, from: f64, to: f64) -> &[(f64, T)] {
        let start = self.events.partition_point(|(at, _)| *at < from);
        let end = self.events.partition_point(|(at, _)| *at < to);
        self.events.get(start..end).unwrap_or(&[])
    }

    /// The last event at or before `t`: the state to chase after a locate
    /// (the patch that should be on at the new playhead). No allocation.
    #[must_use]
    pub fn last_at_or_before(&self, t: f64) -> Option<&(f64, T)> {
        let end = self.events.partition_point(|(at, _)| *at <= t);
        end.checked_sub(1).and_then(|i| self.events.get(i))
    }
}
