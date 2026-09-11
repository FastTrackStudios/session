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
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

use daw_ui::studio::{ProjectRef, RowsRef};

use crate::profile::Counts;

/// A row and its one-pixel divider — the pitch the panel and the lanes
/// share, and the only thing keeping them level.
pub const ROW_PITCH: f64 = 71.0;
/// The panel's width, from the measured REAPER geometry.
pub const TCP_WIDTH: f64 = 343.0;

/// The resolved theme, as the art crate's control palette.
///
/// Started from the art's own defaults and overridden rather than built
/// field by field: `Chrome` carries relationships that were measured
/// together — the hardware face, its edge and its mark — and a palette
/// assembled from scratch would silently lose the ones this theme has
/// nothing to say about.
fn chrome(theme: &daw_ui::theming::Theme) -> daw_theme::Chrome {
    let c = |col: daw_ui::theming::Color| daw_theme::Color {
        r: col.r,
        g: col.g,
        b: col.b,
        a: col.a,
    };
    daw_theme::Chrome {
        surface: c(theme.tokens.surface),
        surface_raised: c(theme.tokens.surface_raised),
        surface_sunken: c(theme.tokens.surface_sunken),
        border: c(theme.tokens.border),
        text: c(theme.tokens.text),
        text_dim: c(theme.tokens.text_dim),
        text_faint: c(theme.tokens.text_faint),
        accent: c(theme.tokens.accent),
        // A control's face, its edge and the ink on it. The buttons read
        // as part of the panel when these come off the same ladder the
        // panel does, and as borrowed art when they do not.
        hardware: c(theme.tokens.surface_raised),
        hardware_edge: c(theme.tokens.border),
        hardware_mark: c(theme.tokens.text_dim),
        ..daw_theme::Theme::default().chrome
    }
}

/// Colours resolved once, so the draw loop never parses a hex string.
pub struct Palette {
    pub surface: Color,
    pub row_a: Color,
    pub row_b: Color,
    pub divider: Color,
    pub grid: Color,
    /// The sub-bar lines, dimmer than the bar lines they sit between.
    pub grid_beat: Color,
    pub ruler_bg: Color,
    pub ruler_fg: Color,
    pub item_edge: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub accent: Color,
    pub mute: Color,
    pub solo: Color,
    pub rec: Color,
    pub meter_warn: Color,
    pub meter_danger: Color,
    /// The track panel's own surfaces. Named `tcp_*` because they come
    /// from the theme's TCP context, which a REAPER theme colours
    /// separately from the arrange view — a panel drawn in arrange
    /// colours is the giveaway that a theme was only half applied.
    pub tcp_tint: Color,
    pub tcp_gutter: Color,
    pub tcp_column: Color,
    pub tcp_rule: Color,
    pub tcp_field: Color,
    pub tcp_button: Color,
    pub tcp_combo: Color,
    pub tcp_meter_well: Color,
    /// How strongly a track's colour tints its row, from the theme.
    pub track_tint: f32,
    /// The palette the ported control art is drawn against.
    ///
    /// `daw_theme_art`'s components reach for `Theme::default()`, which
    /// is right for the one theme they were drawn for and wrong here:
    /// this window opens whatever REAPER theme the user has, and taking
    /// the default put the track panel's buttons in a grey the rest of
    /// the window had moved away from. So the drawings take a palette,
    /// and this is it.
    pub chrome: daw_theme::Chrome,
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
            grid_beat: c(theme.arrange.grid_beat),
            ruler_bg: c(theme.arrange.ruler_bg),
            ruler_fg: c(theme.arrange.ruler_fg),
            item_edge: c(theme.arrange.item_edge),
            text_dim: c(theme.tokens.text_dim),
            text_faint: c(theme.tokens.text_faint),
            accent: c(theme.tokens.accent),
            mute: c(theme.tokens.mute),
            solo: c(theme.tokens.solo),
            rec: c(theme.tokens.rec),
            meter_warn: c(theme.tokens.meter_warn),
            meter_danger: c(theme.tokens.meter_danger),
            tcp_tint: c(theme.tokens.surface_raised),
            tcp_gutter: c(theme.tokens.surface),
            tcp_column: c(theme.tokens.surface_sunken),
            tcp_rule: c(theme.tokens.border),
            tcp_field: c(theme.tokens.surface_sunken),
            tcp_button: c(theme.tokens.surface),
            tcp_combo: c(theme.tokens.surface_sunken),
            tcp_meter_well: c(theme.tokens.surface_sunken),
            track_tint: theme.metrics.track_tint,
            chrome: chrome(theme),
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
    /// The project tempo, for the ruler's bar lines.
    pub bpm: f64,
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
    pub fn build(
        palette: &Palette,
        font: &crate::text::Font,
        project: &ProjectRef,
        rows: &RowsRef,
    ) -> Self {
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

            // The panel row — the whole REAPER-matched control panel, at
            // the geometry the DOM row uses. See `crate::tcp`.
            crate::tcp::draw_row(&mut panel, palette, font, track, i32::try_from(*depth).unwrap_or(0), y);
            // The divider under it, matching the lane's.
            panel.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.divider,
                None,
                &Rect::new(0.0, y + ROW_PITCH - 1.0, TCP_WIDTH, y + ROW_PITCH),
            );

            // The items on this lane. An item with no colour of its own
            // takes its TRACK's, which is what makes a session read by
            // section when it is zoomed out far enough that names are
            // gone — the same rule the panel's row tint follows.
            let track_color = crate::tcp::track_color(palette, track);
            for item in project.lane(&track.guid) {
                let x0 = item.position.as_seconds();
                let x1 = x0 + item.length.as_seconds().max(0.001);
                let color = item.color.map_or(track_color, |rgb| {
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
            bpm: project.bpm,
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
    match cmd {
        RenderCommand::Fill(fill) => {
            let Paint::Solid(color) = fill.brush else {
                return false;
            };
            painter.fill(
                fill.fill,
                transform * fill.transform,
                Paint::Solid(color),
                fill.brush_transform,
                &fill.shape,
            );
            true
        }
        RenderCommand::Stroke(stroke) => {
            let Paint::Solid(color) = stroke.brush else {
                return false;
            };
            painter.stroke(
                &stroke.style,
                transform * stroke.transform,
                Paint::Solid(color),
                stroke.brush_transform,
                &stroke.shape,
            );
            true
        }
        // Text. Dropping this arm is not a silent degradation of
        // quality, it is a track panel with no names in it, so it is
        // handled here rather than defaulted.
        RenderCommand::GlyphRun(run) => {
            let Paint::Solid(color) = run.brush else {
                return false;
            };
            painter.draw_glyphs(
                &run.font_data,
                run.font_size,
                run.hint,
                &run.normalized_coords,
                run.embolden,
                &run.style,
                Paint::Solid(color),
                run.brush_alpha,
                transform * run.transform,
                run.glyph_transform,
                run.glyphs.iter().copied(),
            );
            true
        }
        _ => false,
    }
}
