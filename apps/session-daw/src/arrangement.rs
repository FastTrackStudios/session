//! The arrangement as a recorded scene.
//!
//! Built once, in CONTENT space — seconds along the timeline, row
//! indices down it — and replayed every frame with a transform that
//! carries the scroll and the zoom. `anyrender::Scene` records the
//! commands; `PaintScene::append_scene` replays them into whatever
//! backend is drawing, which on the desktop is Vello on the GPU.
//!
//! That split is the whole design. The scene is rebuilt when the DATA
//! changes — a project opening, a folder collapsing — and never on a
//! scroll, a zoom or a playhead tick. Those are a different `Affine`,
//! not a different scene.
//!
//! # Why one scene for the panel AND the lanes
//!
//! Because they must not be able to disagree. The WebView version drew
//! the lanes on a canvas while the track panel stayed in the DOM, and
//! they visibly tore apart while scrolling: native scroll moves DOM
//! content on the compositor, a canvas redraws on the main thread, and
//! nothing reconciles the two. Recorded together and replayed under one
//! transform, being out of sync is not a bug that can happen.

use anyrender::recording::RenderCommand;
use anyrender::{Paint, PaintScene, Scene};
use vello::kurbo::{Affine, Rect, RoundedRect};
use vello::peniko::{Color, Fill};

use daw_ui::studio::{ProjectRef, RowsRef};

use crate::profile::Counts;

/// A row and its one-pixel divider — the pitch the panel and the lanes
/// share, and the only thing keeping them level.
pub const ROW_PITCH: f64 = 71.0;
/// The panel's width, from the measured REAPER geometry.
pub const TCP_WIDTH: f64 = 343.0;

/// Colours resolved once, so the draw loop never parses a hex string.
pub struct Palette {
    pub surface: Color,
    pub row_a: Color,
    pub row_b: Color,
    pub divider: Color,
    pub grid: Color,
    pub item_edge: Color,
    pub text: Color,
}

impl Palette {
    pub fn from_theme(theme: &daw_ui::theming::Theme) -> Self {
        let c = |col: daw_ui::theming::Color| {
            Color::from_rgba8(col.r, col.g, col.b, col.a)
        };
        Self {
            surface: c(theme.arrange.bg),
            row_a: c(theme.arrange.row_bg[0]),
            row_b: c(theme.arrange.row_bg[1]),
            divider: c(theme.arrange.row_divider[0]),
            grid: c(theme.arrange.grid_measure),
            item_edge: c(theme.arrange.item_edge),
            text: c(theme.tokens.text),
        }
    }
}

/// Everything drawn, recorded in content space.
pub struct Arrangement {
    /// The lanes and their items, at x = seconds * 1.0. The zoom is
    /// applied by the replay transform, so this is recorded at one
    /// pixel per second and scaled at draw time.
    pub lanes: Scene,
    /// The track panel. Recorded separately because it does NOT move
    /// horizontally — it is replayed under a transform that carries only
    /// the vertical scroll.
    pub panel: Scene,
    pub rows: usize,
    pub length_secs: f64,
    /// How many items were recorded, for reports that want to say what
    /// was actually drawn.
    pub item_count: usize,
    /// Where each row's commands live, so a replay can skip the rows
    /// that are not on screen. See [`Index`].
    pub index: Index,
}

/// Which commands belong to which row, and how wide each one is.
///
/// The scene is recorded once and replayed under a transform, which is
/// what makes scrolling free of a rebuild — but it also means a naive
/// replay walks the WHOLE session every frame. At 5120x1440 a 71px row
/// pitch puts about twenty rows on screen; with two thousand tracks that
/// is 1% of the scene, and the profiler measured the other 99% costing
/// 70% of the frame.
///
/// Rows are recorded consecutively, so a row's commands are a contiguous
/// range and the index is two `Vec`s of offsets rather than a tree. The
/// x extents are kept per command so a row that IS on screen still skips
/// the items scrolled off the sides of it — the case that matters when
/// zoomed in on bar 400 of 500.
#[derive(Default)]
pub struct Index {
    /// `lanes` command range per row.
    lanes: Vec<std::ops::Range<u32>>,
    /// `panel` command range per row.
    panel: Vec<std::ops::Range<u32>>,
    /// Content-space x extent of every `lanes` command, indexed the same
    /// way as `lanes.commands`. In seconds, like the recording.
    x: Vec<(f32, f32)>,
}

/// What part of the session a frame can actually see.
///
/// Held in the same content space the scene is recorded in — seconds
/// across, row pitch down — so the visibility test is the inverse of the
/// replay transform and nothing here has to know about `Affine`.
#[derive(Clone, Copy)]
pub struct Viewport {
    pub scroll_x: f64,
    pub scroll_y: f64,
    /// Pixels per second, zoom included.
    pub pps: f64,
    pub zoom_y: f64,
    pub width: f64,
    pub height: f64,
}

impl Viewport {
    /// The rows that intersect the viewport, clamped to `rows`.
    ///
    /// One row of bleed on each side: a row scrolled half off the top
    /// still paints its visible half, and dropping it would tear the
    /// edge of the screen during exactly the fast scroll this is for.
    #[must_use]
    pub fn rows(self, rows: usize) -> std::ops::Range<usize> {
        let pitch = (ROW_PITCH * self.zoom_y).max(0.001);
        let first = ((self.scroll_y / pitch).floor() as isize - 1).max(0) as usize;
        let last = (((self.scroll_y + self.height) / pitch).ceil() as isize + 1).max(0) as usize;
        first.min(rows)..last.min(rows)
    }

    /// The span of the session visible across, in seconds, with a screen
    /// of bleed either side for the same reason.
    #[must_use]
    pub fn secs(self) -> (f64, f64) {
        let pps = self.pps.max(1e-9);
        let left = (self.scroll_x - TCP_WIDTH) / pps;
        let right = (self.scroll_x + self.width) / pps;
        (left - 1.0, right + 1.0)
    }
}

impl Arrangement {
    pub fn build(palette: &Palette, project: &ProjectRef, rows: &RowsRef) -> Self {
        let mut lanes = Scene::new();
        let mut panel = Scene::new();
        let mut index = Index::default();

        for (row, (track, depth)) in rows.iter().enumerate() {
            let lanes_from = lanes.commands.len() as u32;
            let panel_from = panel.commands.len() as u32;
            let y = row as f64 * ROW_PITCH;
            let stripe = if row % 2 == 0 { palette.row_a } else { palette.row_b };

            // The lane's background and the divider under it. Recorded
            // absurdly wide so a replay at any zoom still covers the
            // viewport; the backend clips.
            lanes.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                stripe,
                None,
                &Rect::new(0.0, y, project.length_secs.max(1.0), y + ROW_PITCH - 1.0),
            );
            index.x.push((f32::MIN, f32::MAX));
            lanes.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.divider,
                None,
                &Rect::new(0.0, y + ROW_PITCH - 1.0, project.length_secs.max(1.0), y + ROW_PITCH),
            );
            index.x.push((f32::MIN, f32::MAX));

            // The panel row: its stripe, the track's colour chip, and an
            // indent for folder depth. The REAPER-matched controls are
            // the next piece of work — see the module docs.
            panel.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                stripe,
                None,
                &Rect::new(0.0, y, TCP_WIDTH, y + ROW_PITCH - 1.0),
            );
            panel.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.divider,
                None,
                &Rect::new(0.0, y + ROW_PITCH - 1.0, TCP_WIDTH, y + ROW_PITCH),
            );
            let chip_x = 6.0 + f64::from(*depth) * 10.0;
            let chip = track
                .color
                .map_or(palette.text, |rgb| {
                    Color::from_rgba8(
                        ((rgb >> 16) & 0xff) as u8,
                        ((rgb >> 8) & 0xff) as u8,
                        (rgb & 0xff) as u8,
                        0xff,
                    )
                });
            panel.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                chip,
                None,
                &RoundedRect::new(chip_x, y + 8.0, chip_x + 4.0, y + ROW_PITCH - 9.0, 2.0),
            );

            // The items on this lane.
            for item in project.lane(&track.guid) {
                let x0 = item.position.as_seconds();
                let x1 = x0 + item.length.as_seconds().max(0.001);
                let color = item.color.map_or(chip, |rgb| {
                    Color::from_rgba8(
                        ((rgb >> 16) & 0xff) as u8,
                        ((rgb >> 8) & 0xff) as u8,
                        (rgb & 0xff) as u8,
                        if item.muted { 0x66 } else { 0xff },
                    )
                });
                lanes.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Rect::new(x0, y + 2.0, x1, y + ROW_PITCH - 3.0),
                );
                index.x.push((x0 as f32, x1 as f32));
            }

            index.lanes.push(lanes_from..lanes.commands.len() as u32);
            index.panel.push(panel_from..panel.commands.len() as u32);
        }

        debug_assert_eq!(index.x.len(), lanes.commands.len(), "one x extent per lane command");

        Self {
            lanes,
            panel,
            index,
            rows: rows.len(),
            length_secs: project.length_secs,
            item_count: project.item_count,
        }
    }
}

impl Arrangement {
    /// How many items this scene draws.
    pub fn items(&self) -> usize {
        self.item_count
    }

    /// Replay what `view` can see, under `transform`.
    ///
    /// This is THE replay: the window and the bench both call it, so a
    /// benchmark cannot measure work the app does not do, and an
    /// optimization cannot land in one and miss the other. Returns what
    /// it walked and what it submitted, which is the pair that says
    /// whether the culling is working.
    pub fn replay_lanes(
        &self,
        painter: &mut impl PaintScene,
        view: Viewport,
        transform: Affine,
    ) -> Counts {
        let (left, right) = view.secs();
        let (left, right) = (left as f32, right as f32);
        let mut counts = Counts::default();
        for row in view.rows(self.index.lanes.len()) {
            let span = self.index.lanes[row].clone();
            for i in span.start as usize..span.end as usize {
                counts.replayed += 1;
                let (x0, x1) = self.index.x[i];
                if x1 < left || x0 > right {
                    continue;
                }
                if submit(painter, &self.lanes.commands[i], transform) {
                    counts.submitted += 1;
                }
            }
        }
        counts
    }

    /// The same for the track panel, which scrolls vertically only and
    /// therefore needs no horizontal test.
    pub fn replay_panel(
        &self,
        painter: &mut impl PaintScene,
        view: Viewport,
        transform: Affine,
    ) -> Counts {
        let mut counts = Counts::default();
        for row in view.rows(self.index.panel.len()) {
            let span = self.index.panel[row].clone();
            for i in span.start as usize..span.end as usize {
                counts.replayed += 1;
                if submit(painter, &self.panel.commands[i], transform) {
                    counts.submitted += 1;
                }
            }
        }
        counts
    }
}

/// Replay EVERYTHING, ignoring the viewport.
///
/// The control for [`Arrangement::replay_lanes`]. Culling is only ever a
/// win if the frame it produces is identical to the frame without it, so
/// `just daw-verify` renders both and compares the pixels — a claim
/// about what is off screen is exactly the kind that is easy to argue
/// and easy to get subtly wrong at the edges.
pub fn replay_all(
    painter: &mut impl PaintScene,
    scene: &Scene,
    transform: Affine,
) -> Counts {
    let mut counts = Counts::default();
    for cmd in &scene.commands {
        counts.replayed += 1;
        if submit(painter, cmd, transform) {
            counts.submitted += 1;
        }
    }
    counts
}

/// Push one recorded command through `transform`.
///
/// Only solid fills are recorded today, so anything else is skipped
/// rather than silently mis-drawn; when strokes and glyphs arrive they
/// get their own arms here.
fn submit(painter: &mut impl PaintScene, cmd: &RenderCommand, transform: Affine) -> bool {
    if let RenderCommand::Fill(fill) = cmd
        && let Paint::Solid(color) = fill.brush
    {
        painter.fill(
            fill.fill,
            transform * fill.transform,
            Paint::Solid(color),
            fill.brush_transform,
            &fill.shape,
        );
        return true;
    }
    false
}
