//! Razor edit areas — rectangles of time by row.
//!
//! A razor is the answer to "I want *this rectangle*", independent of
//! what is selected and independent of where the things inside it
//! begin and end. Two surfaces need exactly that and need to agree
//! about it: the expression editor over notes, and the arrangement
//! over items.
//!
//! What lives here is the rectangle and the set algebra over it —
//! merging, hit testing, translation. What does NOT live here is any
//! operation on contents, because the contents are the one thing the
//! two surfaces do not share. The expression editor slices notes at an
//! area's edges; the arrangement will split items at them. Same
//! rectangle, different verb.
//!
//! The property that makes a razor different from a marquee, and the
//! reason both surfaces want one: an area **slices** at its edges
//! rather than selecting whole things. Dragging a razor over the middle
//! of a held note, or the middle of a take, takes the middle with it.
//! That is what makes it usable for comping, and it is why the AREA is
//! the unit of operation rather than the note or the item.

/// Which axis a razor drag is locked to.
///
/// MRE's `H` and `L`. One enum rather than two booleans because they are
/// mutually exclusive there and two booleans would have a fourth state
/// that means nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RazorAxis {
    /// Time only — the rows the area covers cannot change.
    Horizontal,
    /// Rows only — the span of time cannot change.
    Vertical,
}

/// A rectangular selection over time and rows.
///
/// Rows are inclusive on both ends: a razor over one row has
/// `row_lo == row_hi`, not a zero-height rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RazorArea {
    pub t0: f64,
    pub t1: f64,
    pub row_lo: i32,
    pub row_hi: i32,
}

impl RazorArea {
    #[must_use]
    pub fn new(t0: f64, t1: f64, row_a: i32, row_b: i32) -> Self {
        Self {
            t0: t0.min(t1),
            t1: t0.max(t1),
            row_lo: row_a.min(row_b),
            row_hi: row_a.max(row_b),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.t1 - self.t0 <= 0.0
    }

    #[must_use]
    pub fn width(&self) -> f64 {
        self.t1 - self.t0
    }

    #[must_use]
    pub fn rows(&self) -> i32 {
        self.row_hi - self.row_lo + 1
    }

    #[must_use]
    pub fn contains(&self, t: f64, row: i32) -> bool {
        t >= self.t0 && t <= self.t1 && row >= self.row_lo && row <= self.row_hi
    }

    /// Does this area overlap any part of a thing spanning `start`
    /// to `end` on `row`?
    ///
    /// Takes the span rather than the thing, because the thing is a
    /// note on one surface and a take on the other and this crate
    /// knows about neither. Half-open in time on purpose: something
    /// that merely touches the edge is not inside.
    #[must_use]
    pub fn touches(&self, row: i32, start: f64, end: f64) -> bool {
        row >= self.row_lo && row <= self.row_hi && start < self.t1 && end > self.t0
    }

    #[must_use]
    pub fn translated(&self, dt: f64, drows: i32) -> Self {
        Self {
            t0: self.t0 + dt,
            t1: self.t1 + dt,
            row_lo: self.row_lo + drows,
            row_hi: self.row_hi + drows,
        }
    }
}

/// The razor areas currently active, plus which one the pointer grabbed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RazorSet {
    pub areas: Vec<RazorArea>,
}

impl RazorSet {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    pub fn clear(&mut self) {
        self.areas.clear();
    }

    /// Add an area, merging it into any it overlaps on the same rows.
    ///
    /// Merging keeps the set canonical: two adjacent razors over the
    /// same rows behave as one, so an operation cannot slice a note
    /// twice at an interior seam.
    pub fn add(&mut self, area: RazorArea) {
        if area.is_empty() {
            return;
        }
        let mut merged = area;
        self.areas.retain(|a| {
            let same_rows = a.row_lo == merged.row_lo && a.row_hi == merged.row_hi;
            let overlaps = a.t0 <= merged.t1 && merged.t0 <= a.t1;
            if same_rows && overlaps {
                merged.t0 = merged.t0.min(a.t0);
                merged.t1 = merged.t1.max(a.t1);
                false
            } else {
                true
            }
        });
        self.areas.push(merged);
    }

    /// The area under a point, if any. Later areas win, matching draw
    /// order.
    pub fn at(&self, t: f64, row: i32) -> Option<(usize, RazorArea)> {
        self.areas
            .iter()
            .enumerate()
            .rev()
            .find(|(_, a)| a.contains(t, row))
            .map(|(i, a)| (i, *a))
    }

    pub fn remove_at(&mut self, t: f64, row: i32) -> bool {
        match self.at(t, row) {
            Some((i, _)) => {
                self.areas.remove(i);
                true
            }
            None => false,
        }
    }

    /// Total time span covered, for a readout.
    #[must_use]
    pub fn total_span(&self) -> f64 {
        self.areas.iter().map(|a| a.width()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::{RazorArea, RazorSet};

    #[test]
    fn an_area_is_normalised_whichever_way_it_was_drawn() {
        let up = RazorArea::new(4.0, 1.0, 7, 2);
        assert!((up.t0 - 1.0).abs() < f64::EPSILON);
        assert!((up.t1 - 4.0).abs() < f64::EPSILON);
        assert_eq!((up.row_lo, up.row_hi), (2, 7));
        // Rows are inclusive on both ends: one row is one row, not a
        // rectangle of no height.
        assert_eq!(RazorArea::new(0.0, 1.0, 3, 3).rows(), 1);
    }

    #[test]
    fn touching_is_half_open_in_time() {
        let area = RazorArea::new(2.0, 4.0, 0, 1);
        assert!(area.touches(0, 3.0, 3.5), "inside");
        assert!(area.touches(1, 1.0, 9.0), "straddling is touching");
        assert!(
            !area.touches(2, 3.0, 3.5),
            "on a row the area does not cover"
        );
        // Abutting is not overlapping, or every razor would take the
        // thing that merely ends where it begins.
        assert!(!area.touches(0, 0.0, 2.0));
        assert!(!area.touches(0, 4.0, 6.0));
    }

    #[test]
    fn areas_on_the_same_rows_merge_and_others_do_not() {
        let mut set = RazorSet::default();
        set.add(RazorArea::new(0.0, 2.0, 0, 0));
        set.add(RazorArea::new(1.0, 4.0, 0, 0));
        assert_eq!(
            set.areas.len(),
            1,
            "overlapping on the same rows is one area"
        );
        assert!((set.areas[0].t1 - 4.0).abs() < f64::EPSILON);

        // The same span on different rows is a different area — merging
        // it would silently widen what an operation covers.
        set.add(RazorArea::new(1.0, 4.0, 1, 1));
        assert_eq!(set.areas.len(), 2);

        // And an empty one is not an area at all.
        set.add(RazorArea::new(5.0, 5.0, 0, 0));
        assert_eq!(set.areas.len(), 2);
    }

    #[test]
    fn the_last_area_drawn_is_the_one_under_the_pointer() {
        let mut set = RazorSet::default();
        set.add(RazorArea::new(0.0, 4.0, 0, 0));
        set.add(RazorArea::new(0.0, 4.0, 1, 1));
        let (index, _) = set.at(2.0, 1).expect("an area");
        assert_eq!(index, 1, "later areas win, matching draw order");
        assert!(set.at(2.0, 9).is_none());
        assert!(set.remove_at(2.0, 1));
        assert_eq!(set.areas.len(), 1);
    }

    #[test]
    fn the_span_is_what_an_operation_would_cover() {
        let mut set = RazorSet::default();
        set.add(RazorArea::new(0.0, 2.0, 0, 0));
        set.add(RazorArea::new(0.0, 3.0, 1, 1));
        assert!((set.total_span() - 5.0).abs() < f64::EPSILON);
    }
}
