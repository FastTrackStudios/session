//! The arrangement as ONE node: ruler, track panel and lanes, painted
//! rather than built.
//!
//! # Why this one surface is not a DOM
//!
//! Everything else in the window is better as components — a button
//! wants `:hover`, focus, a name a screen reader can say, and a layout
//! somebody can change without recompiling. The arrangement wants none
//! of that and pays dearly for all of it.
//!
//! Measured on the golden session at 5120x1440, the same picture drawn
//! both ways, gated pixel-for-pixel against each other by
//! `tests/component_lanes.rs`:
//!
//! | gesture | painted | as a component tree |
//! |---|---|---|
//! | scroll | 1.06 ms | 15.5 ms |
//! | zoom | 0.71 ms | 21.8 ms |
//! | the whole session on screen | 3.77 ms | — |
//!
//! Fifteen to thirty times, and it is not the GPU: that is 0.3–1.0 ms
//! either way. It is ten thousand nodes going through Stylo's cascade,
//! Taffy's layout, damage propagation and a per-node paint walk, every
//! frame, to produce a picture that a recorded command list replays in
//! one. No amount of tuning closes a gap of that shape — a month of it
//! took the component tree from 124 ms to 15, and 15 is still not 4.17.
//!
//! So the arrangement becomes a [`Widget`]: one element in the DOM, laid
//! out and positioned by CSS like any other, whose contents are drawn by
//! the renderer this window already had. The toolbars, the transport and
//! the rails stay components, because for them the DOM is the point.
//!
//! # What this costs, honestly
//!
//! Hit testing, keyboard focus and accessibility inside this rectangle
//! are ours to write. The first two we already wrote — the wheel is
//! handled at the winit level because Blitz dispatches no wheel event,
//! and `arrange_edit` has done item hit testing from the start. The
//! third is the real debt: a screen reader sees one element here. The
//! answer is that every piece of session state is reachable through the
//! CLI and the RPC surface, so the window is not the only way in — but
//! that is an argument, not an implementation, and it should be written
//! down as one.
//!
//! # Why it still runs in a browser
//!
//! [`Widget::paint`] returns an `anyrender::Scene` — a list of
//! commands, not a wgpu call. Whatever can replay that list can draw
//! this: the CPU backend, the WebGL-class one, and a canvas in a browser
//! tab. The escape hatch that takes a `Device` and `Queue` in
//! `can_create_surfaces` is the one that would tie us to native, and
//! this does not use it.

use std::cell::RefCell;
use std::rc::Rc;

use anyrender::{RenderContext, Scene};
use blitz_dom::node::{ComputedStyles, Widget};

use vello::kurbo::Affine;

use crate::arrangement::{Arrangement, Palette, TCP_WIDTH, Viewport};
use crate::profile::Counts;
use crate::ruler::{self, Bars};

/// The finest grid division the ruler will draw.
const FINEST: f64 = 1.0 / 16.0;

/// Where the view is, shared between the window and the widget.
///
/// A plain cell rather than a signal. The paint happens inside Blitz's
/// own traversal, which is not the Dioxus runtime — reading a `Signal`
/// there is a panic waiting for the first frame that takes a different
/// path. What the widget needs is four numbers, and four numbers do not
/// need a reactive graph.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub zoom_x: f64,
    pub zoom_y: f64,
}

/// A handle on that, for the window to write and the widget to read.
pub type Shared = Rc<RefCell<View>>;

/// How much the last frame drew, for the benchmark to report.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drawn {
    pub replayed: u64,
    pub submitted: u64,
}

/// The arrangement, as a widget.
pub struct ArrangementWidget {
    scene: Arrangement,
    palette: Palette,
    font: crate::text::Font,
    bars: Bars,
    grid: adaptive_grid::Adaptive,
    /// Pixels per second before the zoom — the base the view scales.
    pps: f64,
    view: Shared,
    drawn: Rc<RefCell<Drawn>>,
}

impl ArrangementWidget {
    /// Build the recording once, from the session.
    ///
    /// Once, and that is the whole point: the commands for a lane do not
    /// change when the view moves over them, so a pan replays what is on
    /// screen and records nothing.
    #[must_use]
    pub fn new(
        scene: Arrangement,
        palette: Palette,
        font: crate::text::Font,
        bpm: f64,
        pps: f64,
        view: Shared,
    ) -> Self {
        Self {
            scene,
            palette,
            font,
            bars: Bars::at(bpm),
            grid: adaptive_grid::Adaptive::default(),
            pps,
            view,
            drawn: Rc::new(RefCell::new(Drawn::default())),
        }
    }

    /// What the last frame replayed and submitted.
    #[must_use]
    pub fn drawn(&self) -> Rc<RefCell<Drawn>> {
        Rc::clone(&self.drawn)
    }
}

impl Widget for ArrangementWidget {
    fn paint(
        &mut self,
        _render_ctx: &mut dyn RenderContext,
        _styles: &ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> Scene {
        let at = *self.view.borrow();
        let view = Viewport {
            scroll_x: at.scroll_x,
            scroll_y: at.scroll_y,
            pps: self.pps * at.zoom_x,
            zoom_y: at.zoom_y,
            width: f64::from(width),
            height: f64::from(height),
        };

        let mut out = Scene::new();
        // The same five calls, in the same order, as the painted window
        // — including the ruler AFTER the lanes, because the lane
        // backgrounds are opaque and would paint the grid straight out
        // of the frame.
        let lanes = self.scene.replay_lanes(
            &mut out,
            view,
            Affine::translate((TCP_WIDTH - view.scroll_x, -view.scroll_y))
                * Affine::scale_non_uniform(view.pps, view.zoom_y),
        );
        let panel = self.scene.replay_panel(
            &mut out,
            view,
            Affine::translate((0.0, -view.scroll_y)) * Affine::scale_non_uniform(1.0, view.zoom_y),
        );
        ruler::grid(
            &mut out,
            &self.palette,
            view,
            self.bars,
            &self.grid,
            FINEST,
            (0.0, 0.0),
        );
        ruler::ruler(
            &mut out,
            &self.palette,
            &self.font,
            view,
            self.scene.tempo(),
            (0.0, 0.0),
        );
        ruler::tempo(
            &mut out,
            &self.palette,
            &self.font,
            view,
            (0.0, 0.0),
            self.scene.tempo(),
        );

        let total = |a: Counts, b: Counts| Drawn {
            replayed: a.replayed.saturating_add(b.replayed),
            submitted: a.submitted.saturating_add(b.submitted),
        };
        *self.drawn.borrow_mut() = total(lanes, panel);
        out
    }
}
