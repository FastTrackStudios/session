//! The arrangement's scrollbars, and the rules that move the view.
//!
//! Two bars over the lanes — one along the bottom, one up the right —
//! each a track with a thumb whose length is the view's share of the
//! session and whose position is the scroll's. The geometry is here,
//! pure, so a drag on a thumb and the bar the drag is drawn on cannot
//! disagree; the window owns the scroll position and calls in.
//!
//! Also here: the edge autoscroll (a drag held at the edge of the
//! lanes moves the view under it) and the follow-playhead paging (the
//! view turns the page when the playhead reaches the end of it), as
//! rules over the same numbers.

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

use anyrender::PaintScene;

use crate::arrangement::Palette;

/// How thick a bar is.
pub const THICK: f64 = 10.0;

/// The least a thumb may shrink to, so a long session still has one
/// to grab.
const THUMB_MIN: f64 = 24.0;

/// Which way a bar runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// One bar: its track, its thumb, and the numbers they were built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub axis: Axis,
    pub track: Rect,
    pub thumb: Rect,
    /// How far the scroll can go, in content pixels.
    span: f64,
    /// How much is on screen, in content pixels.
    visible: f64,
}

impl Bar {
    /// A bar for a view that shows `visible` of `visible + span`, at
    /// `scroll`, along `track`.
    #[must_use]
    pub fn new(axis: Axis, track: Rect, scroll: f64, span: f64, visible: f64) -> Self {
        let length = match axis {
            Axis::X => track.width(),
            Axis::Y => track.height(),
        };
        let content = (span + visible).max(1.0);
        let thumb_len = (length * visible / content).clamp(THUMB_MIN.min(length), length);
        let travel = (length - thumb_len).max(0.0);
        let at = if span > 0.0 { (scroll / span).clamp(0.0, 1.0) * travel } else { 0.0 };
        let thumb = match axis {
            Axis::X => Rect::new(track.x0 + at, track.y0, track.x0 + at + thumb_len, track.y1),
            Axis::Y => Rect::new(track.x0, track.y0 + at, track.x1, track.y0 + at + thumb_len),
        };
        Self {
            axis,
            track,
            thumb,
            span,
            visible,
        }
    }

    /// How far the SCROLL moves when the thumb is dragged `pixels`
    /// along the track.
    #[must_use]
    pub fn scroll_per_thumb(&self, pixels: f64) -> f64 {
        let length = match self.axis {
            Axis::X => self.track.width(),
            Axis::Y => self.track.height(),
        };
        let thumb_len = match self.axis {
            Axis::X => self.thumb.width(),
            Axis::Y => self.thumb.height(),
        };
        let travel = (length - thumb_len).max(1.0);
        pixels * self.span / travel
    }

    /// One page, in content pixels: what a click on the track beside
    /// the thumb moves by.
    #[must_use]
    pub fn page(&self) -> f64 {
        self.visible * 0.9
    }

    /// What a press at a point on the bar does: nothing (off it), a
    /// thumb grab, or a page in one direction.
    #[must_use]
    pub fn press(&self, x: f64, y: f64) -> Press {
        if !contains(self.track, x, y) {
            return Press::Miss;
        }
        if contains(self.thumb, x, y) {
            return Press::Thumb;
        }
        let before = match self.axis {
            Axis::X => x < self.thumb.x0,
            Axis::Y => y < self.thumb.y0,
        };
        if before { Press::PageBack } else { Press::PageForward }
    }
}

/// What a press on a bar meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Miss,
    Thumb,
    PageBack,
    PageForward,
}

/// The two bars for a lanes box, given the scroll and its spans.
///
/// `lanes` is the box the lanes are drawn in (under the ruler, right
/// of the panel). The bars sit inside it along its bottom and right
/// edges, the corner between them left empty.
#[must_use]
pub fn bars(lanes: Rect, scroll: (f64, f64), spans: (f64, f64)) -> (Bar, Bar) {
    let horizontal = Rect::new(lanes.x0, lanes.y1 - THICK, lanes.x1 - THICK, lanes.y1);
    let vertical = Rect::new(lanes.x1 - THICK, lanes.y0, lanes.x1, lanes.y1 - THICK);
    (
        Bar::new(Axis::X, horizontal, scroll.0, spans.0, lanes.width() - THICK),
        Bar::new(Axis::Y, vertical, scroll.1, spans.1, lanes.height() - THICK),
    )
}

/// Draw both bars: a faint track, a thumb the panel can grab.
pub fn draw(painter: &mut impl PaintScene, palette: &Palette, bars: (Bar, Bar), held: Option<Axis>) {
    for bar in [bars.0, bars.1] {
        // The track a shade off the lanes, the thumb a shade off the
        // text: enough to find, not enough to compete with an item.
        painter.fill(Fill::NonZero, Affine::IDENTITY, palette.tcp_field.multiply_alpha(0.9), None, &bar.track);
        let ink: Color = if held == Some(bar.axis) { palette.text } else { palette.text_dim };
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            ink.multiply_alpha(0.85),
            None,
            &bar.thumb.inset(-1.5).to_rounded_rect(3.0),
        );
    }
}

/// How far in from an edge of the lanes a held drag starts to scroll.
pub const EDGE: f64 = 28.0;

/// The autoscroll for a pointer at `(x, y)` while something is being
/// dragged: how much the view should move this frame, per axis.
///
/// Nothing inside the margin; past it, faster the further out, so a
/// pointer parked at the edge crawls and one pushed against the rail
/// runs. Per frame, because the window redraws every frame while a
/// drag is held — a pointer that stops at the edge keeps scrolling.
#[must_use]
pub fn autoscroll(lanes: Rect, x: f64, y: f64) -> (f64, f64) {
    let push = |at: f64, low: f64, high: f64| {
        if at < low + EDGE {
            -((low + EDGE - at).clamp(0.0, EDGE * 2.0) * 0.4)
        } else if at > high - EDGE {
            (at - (high - EDGE)).clamp(0.0, EDGE * 2.0) * 0.4
        } else {
            0.0
        }
    };
    (push(x, lanes.x0, lanes.x1), push(y, lanes.y0, lanes.y1))
}

/// Where the view should scroll to follow a playhead at `play_x`
/// content pixels, if it should move at all.
///
/// A page turn, not a chase: the view stays put while the playhead
/// crosses it, and when the playhead leaves the right edge the view
/// jumps so the playhead is a tenth of the way in — REAPER's own
/// paging, the one that lets you read a screen of items while they
/// play rather than watching them slide.
#[must_use]
pub fn follow(scroll_x: f64, visible_w: f64, play_x: f64) -> Option<f64> {
    if visible_w <= 0.0 {
        return None;
    }
    let right = scroll_x + visible_w;
    if play_x > right - 2.0 || play_x < scroll_x {
        Some((play_x - visible_w * 0.1).max(0.0))
    } else {
        None
    }
}

const fn contains(rect: Rect, x: f64, y: f64) -> bool {
    x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lanes() -> Rect {
        Rect::new(343.0, 73.0, 2500.0, 1400.0)
    }

    /// The thumb is the view's share of the content, at the scroll's
    /// place along the track, and never too small to grab.
    #[test]
    fn the_thumb_is_the_views_share() {
        let (h, v) = bars(lanes(), (0.0, 0.0), (1000.0, 0.0));
        assert!((h.thumb.x0 - h.track.x0).abs() < f64::EPSILON, "at the start when unscrolled");
        assert!(h.thumb.width() < h.track.width());
        // No vertical span: the thumb is the whole track.
        assert!((v.thumb.height() - v.track.height()).abs() < f64::EPSILON);
        // A huge session still has a thumb.
        let (h, _) = bars(lanes(), (0.0, 0.0), (1.0e7, 0.0));
        assert!(h.thumb.width() >= THUMB_MIN);
        // Scrolled to the end, the thumb is at the end.
        let (h, _) = bars(lanes(), (1000.0, 0.0), (1000.0, 0.0));
        assert!((h.thumb.x1 - h.track.x1).abs() < 1e-6);
    }

    /// Dragging the thumb the whole track scrolls the whole span.
    #[test]
    fn a_whole_thumb_drag_is_the_whole_span() {
        let (h, _) = bars(lanes(), (0.0, 0.0), (5000.0, 0.0));
        let travel = h.track.width() - h.thumb.width();
        assert!((h.scroll_per_thumb(travel) - 5000.0).abs() < 1e-6);
    }

    /// A press is the thumb, a page either side of it, or a miss.
    #[test]
    fn a_press_is_read_against_the_thumb() {
        let (h, _) = bars(lanes(), (2500.0, 0.0), (5000.0, 0.0));
        let y = h.track.center().y;
        assert_eq!(h.press(h.thumb.center().x, y), Press::Thumb);
        assert_eq!(h.press(h.track.x0 + 1.0, y), Press::PageBack);
        assert_eq!(h.press(h.track.x1 - 1.0, y), Press::PageForward);
        assert_eq!(h.press(h.track.x0 + 1.0, y - 50.0), Press::Miss);
    }

    /// Inside the margin nothing moves; at the edge it crawls; past it
    /// it runs, and the sign says which way.
    #[test]
    fn the_edge_pushes_the_view() {
        let l = lanes();
        assert_eq!(autoscroll(l, l.center().x, l.center().y), (0.0, 0.0));
        let (dx, _) = autoscroll(l, l.x1 - 2.0, l.center().y);
        assert!(dx > 0.0);
        let (dx_far, _) = autoscroll(l, l.x1 + 20.0, l.center().y);
        assert!(dx_far > dx, "further out is faster");
        let (dx, dy) = autoscroll(l, l.x0 + 1.0, l.y0 + 1.0);
        assert!(dx < 0.0 && dy < 0.0);
    }

    /// The view stays until the playhead leaves it, then turns the page
    /// so the playhead sits near the left.
    #[test]
    fn follow_turns_the_page() {
        assert_eq!(follow(0.0, 1000.0, 500.0), None);
        let next = follow(0.0, 1000.0, 1001.0).expect("a page turn");
        assert!((next - 901.0).abs() < 1e-6);
        assert!(follow(2000.0, 1000.0, 100.0).is_some(), "behind the view turns back");
    }
}
