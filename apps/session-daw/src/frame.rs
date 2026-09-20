//! One arrangement frame, painted the same way everywhere.
//!
//! The window, the benchmark and every screenshot draw the arrangement
//! through this and nothing else. It used to be the window's redraw
//! closure, and the bench's shots re-typed a subset of it — which is
//! how a shot went out with the lanes at one pixel a second under
//! titles at forty, and no track controls at all. A picture that is
//! not the window's picture proves nothing, so there is one painter.
//!
//! Everything a frame needs arrives in [`Arrange`]; what a headless
//! caller does not have (a pointer, a rename in progress, REAPER's
//! icons) has a default that draws nothing, rather than a code path
//! that draws differently.

use std::collections::HashSet;

use anyrender::PaintScene;
use vello::kurbo::{Affine, Rect};
use vello::peniko::Fill;

use crate::arrangement::{Arrangement, Fades, Palette, TCP_WIDTH, Viewport};
use crate::expression::Expression;
use crate::profile::Counts;
use crate::rails::{self, Frame};
use crate::ruler::{self, Bars, RULER_H};
use crate::text::Font;

/// The finest the grid ever gets — sixteenths, as a fraction of a
/// whole note. The zoom only ever coarsens away from it.
pub const FINEST: f64 = 1.0 / 16.0;

/// Everything one frame of the arrangement is drawn from.
pub struct Arrange<'a> {
    pub scene: &'a Arrangement,
    pub palette: &'a Palette,
    pub font: &'a Font,
    /// The window's frame — the rails, and the dock if one is open.
    pub frame: Frame,
    /// What the frame can see: scroll, zoom, and the panel's box.
    pub view: Viewport,
    pub bars: Bars,
    pub grid: &'a adaptive_grid::Adaptive,
    /// The rows the scene was recorded from, and every track.
    pub rows: &'a [(daw_proto::Track, u32)],
    pub tracks: &'a [daw_proto::Track],
    pub map: &'a crate::plan::Rows,
    /// The panel's pointer, for its live controls.
    pub panel: &'a crate::pointer::Pointer<crate::pointer::RowSpot>,
    /// A rename in progress on the panel.
    pub rename: Option<&'a crate::rename::Rename>,
    /// The rails' items and what the pointer is doing to them.
    pub profile: &'a rails::Profile,
    pub rail_at: (Option<rails::Action>, Option<rails::Action>),
    pub icons: &'a mut crate::icons::Icons,
    pub mode: session::modes::Mode,
    /// The playhead, in seconds, where it is NOW.
    pub play_at: f64,
    pub edit: crate::cursor::Edit,
    /// The item under the pointer, and a fade being dragged.
    pub hovered_item: Option<usize>,
    pub in_flight: Option<(usize, Fades)>,
    pub selected: &'a HashSet<String>,
    pub ghost: Option<(usize, f64, f64)>,
    pub scroll_bars: Option<(crate::scrollbar::Bar, crate::scrollbar::Bar)>,
    pub bar_held: Option<crate::scrollbar::Axis>,
    /// The editor in the dock, if one is docked.
    pub dock: Option<&'a mut Expression>,
    /// The zoom tool's Alt sweep, in window pixels, while one is drawn.
    pub zoom_box: Option<((f64, f64), (f64, f64))>,
    /// A refused edit still saying why. Drawn last, over everything,
    /// because it is a reply rather than part of the picture.
    pub notice: Option<&'a crate::notice::Notice>,
}

impl Arrange<'_> {
    /// Draw the frame. Returns what was walked and what was submitted.
    pub fn paint(self, painter: &mut impl PaintScene) -> Counts {
        let Self {
            scene,
            palette,
            font,
            frame,
            view,
            bars,
            grid,
            rows,
            tracks,
            map,
            panel,
            rename,
            profile,
            rail_at,
            icons,
            mode,
            play_at,
            edit,
            hovered_item,
            in_flight,
            selected,
            ghost,
            scroll_bars,
            bar_held,
            dock,
            zoom_box,
            notice,
        } = self;
        let (sx, sy, pps) = (view.scroll_x, view.scroll_y, view.pps);
        let zoom_y = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        let rail = (rails::SIDE, rails::TOP);
        let surface = palette.surface;
        let mut drawn = Counts::default();

        painter.reset();
        // The theme's surface, under everything.
        //
        // `VelloWindowRenderer` clears to WHITE, so any pixel the
        // arrangement does not cover is not merely undrawn, it is
        // bright white on a dark theme — during a fast scroll, at the
        // end of the session, or in the gap under the last row. One
        // rectangle makes the window's background the theme's instead
        // of the renderer's.
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            surface,
            None,
            &Rect::new(0.0, 0.0, frame.width, frame.height),
        );
        // The lanes: scrolled both ways, and scaled horizontally by the
        // zoom. Recorded at one pixel per second, so the scale IS the
        // zoom — no rebuild, no re-record.
        let lanes_at = (rail.0 + TCP_WIDTH - sx, rail.1 + RULER_H - sy);
        let a = scene.replay_lanes(
            painter,
            view,
            Affine::scale_non_uniform(pps, zoom_y).then_translate(lanes_at.into()),
        );
        items_over(
            painter,
            (palette, font, scene),
            view,
            lanes_at,
            (hovered_item, in_flight),
            (selected, ghost),
        );
        // The panel: the SAME vertical offset, which is the entire
        // point. It cannot drift from the lanes because there is
        // nothing to drift — one number moves both.
        let panel_at = Affine::translate((rail.0, rail.1 + RULER_H - sy));
        let b = scene.replay_panel(painter, view, panel_at);
        // After the lanes — their backgrounds are opaque — and the
        // ruler last of all, over everything scrolled under it.
        ruler::grid(painter, palette, view, bars, grid, FINEST, rail);
        // The panel's live values, over its recorded chrome.
        let live = crate::overlay::panel_controls(
            painter, palette, font, scene, rows, tracks, map, view, panel, panel_at,
        );
        chrome(
            painter,
            Chrome {
                scene,
                palette,
                font,
                frame,
                view,
                bars,
                rows,
                rename,
                edit,
                play_at,
                scroll_bars,
                bar_held,
                dock,
                zoom_box,
            },
            panel_at,
        );
        let c = live;
        rails::draw(
            painter,
            palette,
            font,
            icons,
            rail_at,
            frame,
            &profile.left,
            &profile.right,
            &profile.top,
        );
        rails::main_toolbar(painter, palette, font, icons, rail_at, mode);
        // Last of all: a refusal has to be legible over the rails and
        // an open rename alike, and it is the newest thing on screen.
        if let Some(notice) = notice
            && let Some(alpha) = notice.alpha()
        {
            crate::notice::paint(
                painter,
                palette,
                font,
                notice.why,
                crate::notice::area(font, scene, rows, notice, view),
                alpha,
                panel_at,
            );
        }
        drawn.replayed = a
            .replayed
            .saturating_add(b.replayed)
            .saturating_add(c.replayed);
        drawn.submitted = a
            .submitted
            .saturating_add(b.submitted)
            .saturating_add(c.submitted);
        drawn
    }
}

/// What is drawn over the lanes in pixel space: the items' titles,
/// the fade handles and a fade in flight, the selection and a ghost.
fn items_over(
    painter: &mut impl PaintScene,
    (palette, font, scene): (&Palette, &Font, &Arrangement),
    view: Viewport,
    lanes_at: (f64, f64),
    (hovered_item, in_flight): (Option<usize>, Option<(usize, Fades)>),
    (selected, ghost): (&HashSet<String>, Option<(usize, f64, f64)>),
) {
    // The items' titles, in pixel space over the lanes: text
    // recorded in seconds would stretch with the zoom.
    crate::arrangement::titles(painter, palette, font, scene, view, lanes_at);
    // The fade handles on the item under the pointer, and the fade
    // in flight over its recorded self.
    crate::arrangement::fade_overlay(
        painter,
        palette,
        scene,
        view,
        lanes_at,
        hovered_item,
        in_flight,
    );
    // The selection's outlines, and the ghost of an item being
    // moved or trimmed.
    crate::arrangement::selection_overlay(painter, palette, scene, view, lanes_at, selected, ghost);
}

/// The viewport for a frame at a scroll and zoom.
#[must_use]
pub fn viewport(frame: Frame, scroll: (f64, f64), pps: f64, zoom_y: f64) -> Viewport {
    Viewport {
        scroll_x: scroll.0,
        scroll_y: scroll.1,
        pps,
        zoom_y,
        width: frame.content_width(),
        height: frame.content_height(),
    }
}

/// The frame's chrome, over the lanes and the panel: a rename, the
/// ruler and its lanes, the cursors, the scrollbars and the dock.
struct Chrome<'a> {
    scene: &'a Arrangement,
    palette: &'a Palette,
    font: &'a Font,
    frame: Frame,
    view: Viewport,
    bars: Bars,
    rows: &'a [(daw_proto::Track, u32)],
    rename: Option<&'a crate::rename::Rename>,
    edit: crate::cursor::Edit,
    play_at: f64,
    scroll_bars: Option<(crate::scrollbar::Bar, crate::scrollbar::Bar)>,
    bar_held: Option<crate::scrollbar::Axis>,
    dock: Option<&'a mut Expression>,
    zoom_box: Option<((f64, f64), (f64, f64))>,
}

fn chrome(painter: &mut impl PaintScene, parts: Chrome<'_>, panel_at: Affine) {
    let Chrome {
        scene,
        palette,
        font,
        frame,
        view,
        bars,
        rows,
        rename,
        edit,
        play_at,
        scroll_bars,
        bar_held,
        dock,
        zoom_box,
    } = parts;
    let rail = (rails::SIDE, rails::TOP);
    let surface = palette.surface;
    // An open rename, over the name it replaces.
    if let Some(open) = rename.filter(|r| r.surface == crate::rename::Surface::Arrange)
        && let Some((top, height)) = scene.row_band(open.row, view)
    {
        let depth = rows
            .get(open.row)
            .map_or(0, |(_, d)| i32::try_from(*d).unwrap_or(0));
        let is_folder = rows.get(open.row).is_some_and(|(t, _)| t.is_folder);
        let row = crate::row::Row::new(top, height, depth, is_folder);
        if let Some(field) = row.rect(crate::row::Control::Name) {
            crate::rename::paint(painter, palette, font, open, field, panel_at);
        }
    }
    ruler::ruler(painter, palette, font, view, scene.tempo(), rail);
    ruler::tempo(painter, palette, font, view, rail, scene.tempo());
    ruler::lanes(
        painter,
        palette,
        font,
        view,
        rail,
        scene.sections(),
        scene.markers(),
    );
    // A mark's name being typed, over the lane it is in. After the
    // lanes so it is not painted under the band it renames, and before
    // the cursors, which belong over everything.
    if let Some(open) = rename.filter(|r| r.surface == crate::rename::Surface::Ruler) {
        use crate::rename::What;
        let at = match &open.what {
            What::Marker(id) => scene.markers().iter().find(|m| m.idx == *id).map(|m| m.at),
            What::Region(id) => scene
                .sections()
                .iter()
                .find(|s| s.id == *id)
                .map(|s| s.start),
            What::Track(_) => None,
        };
        if let Some(at) = at {
            let field = ruler::field(view, rail, open.row, at);
            crate::rename::paint(painter, palette, font, open, field, Affine::IDENTITY);
        }
    }
    let top = rail.1 + RULER_H;
    let bottom = rail.1 + view.height;
    ruler::lane_lines(
        painter,
        palette,
        view,
        rail,
        scene.sections(),
        scene.markers(),
        top,
        bottom,
    );
    // The cursors last, over the lanes and under nothing: a
    // playhead behind an item is a playhead you cannot follow.
    crate::cursor::paint_edit(painter, palette, &edit, view, rail, top, bottom);
    let x = play_at.mul_add(view.pps, rail.0 + TCP_WIDTH - view.scroll_x);
    crate::cursor::paint(
        painter,
        crate::cursor::Look::default(),
        x,
        top,
        bottom,
        rail.0 + TCP_WIDTH,
    );
    // The zoom tool's sweep: the box that will fill the lanes on
    // release.
    if let Some((a, b)) = zoom_box {
        let r = Rect::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.accent.multiply_alpha(0.15),
            None,
            &r,
        );
        painter.stroke(
            &vello::kurbo::Stroke::new(1.0),
            Affine::IDENTITY,
            palette.accent,
            None,
            &r,
        );
    }
    // The scrollbars, over the lanes and under the rails.
    if let Some(pair) = scroll_bars {
        crate::scrollbar::draw(painter, palette, pair, bar_held);
    }
    // The docked editor, under the arrangement: its own bars, roll
    // and strip, and a rule along its top edge that is also the
    // grip that resizes it.
    if let (Some(dock), Some(ex)) = (frame.dock_box(), dock) {
        ex.layout((dock.x0, dock.y0), (dock.width(), dock.height()));
        painter.fill(Fill::NonZero, Affine::IDENTITY, surface, None, &dock);
        ex.paint(painter);
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.tcp_rule,
            None,
            &Rect::new(dock.x0, dock.y0 - 1.0, dock.x1, dock.y0 + 1.0),
        );
    }
    // The rails over everything that scrolled under them, and the
    // mode selector in the corner the ruler leaves.
}
