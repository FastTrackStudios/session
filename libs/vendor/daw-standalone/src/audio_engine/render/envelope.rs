//! Per-frame envelope evaluation for the render hot path.

use daw_proto::automation::{EnvelopePoint, EnvelopeShape};

/// Stateful per-frame envelope cursor. `eval_at(t)` advances through
/// the points list without rescanning from the start each call.
/// Construct fresh per render block.
///
/// Time *must* be monotonically non-decreasing across `eval_at` calls
/// — meets the audio callback contract where frames march forward.
pub(crate) struct EnvelopeCursor<'a> {
    points: &'a [EnvelopePoint],
    /// Index of the next point ahead of the cursor. Once
    /// `points[next].time > t`, the active segment is
    /// `points[next-1]..points[next]`.
    next: usize,
}

impl<'a> EnvelopeCursor<'a> {
    pub(crate) fn new(points: &'a [EnvelopePoint]) -> Self {
        Self { points, next: 1 }
    }

    /// Evaluate the envelope at `time_seconds`. Returns `None` only
    /// if the envelope is empty.
    pub(crate) fn eval_at(&mut self, time_seconds: f64) -> Option<f64> {
        if self.points.is_empty() {
            return None;
        }
        // Clamp left of first point.
        let first = &self.points[0];
        if time_seconds <= first.time.as_seconds() {
            return Some(first.value);
        }
        // Clamp right of last point.
        let last = self.points.last().unwrap();
        if time_seconds >= last.time.as_seconds() {
            return Some(last.value);
        }
        // Advance `next` while it's still in the past.
        while self.next < self.points.len()
            && self.points[self.next].time.as_seconds() < time_seconds
        {
            self.next += 1;
        }
        if self.next == 0 {
            // Shouldn't happen given the clamp above, but be safe.
            self.next = 1;
        }
        let a = &self.points[self.next - 1];
        let b = &self.points[self.next];
        match a.shape {
            EnvelopeShape::Square => Some(a.value),
            _ => {
                let ta = a.time.as_seconds();
                let tb = b.time.as_seconds();
                let span = tb - ta;
                if span <= 0.0 {
                    return Some(b.value);
                }
                let f = (time_seconds - ta) / span;
                Some(a.value + (b.value - a.value) * f)
            }
        }
    }
}
