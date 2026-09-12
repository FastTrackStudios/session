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
//! # One scene for the panel as well as the lanes
//!
//! Because they must not be able to disagree. The web-view version drew
//! the lanes on a canvas while the track panel stayed in the DOM, and
//! they visibly tore apart while scrolling: native scroll moves DOM
//! content on the compositor, a canvas redraws on the main thread, and
//! nothing reconciles the two. Recorded together and replayed under one
//! transform, being out of sync is not a bug that can happen.

use anyrender::recording::RenderCommand;
use anyrender::{PaintScene, Scene};
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

use daw_ui::studio::{ProjectRef, RowsRef};

use crate::profile::Counts;

/// The divider under every row.
///
/// Part of the row's height rather than added to it, so a track set to
/// 24 in REAPER occupies 24 here too. Adding it would make every row a
/// pixel taller than the project says, and two thousand of those is a
/// hundred and forty pixels of drift down the session.
pub const DIVIDER: f64 = 1.0;
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
        // The art prints on this and rings with it — the record arm's
        // unlit ring is #a6a6a6 in the source, and a control FACE, not a
        // label. Mapped to `text_dim` alone it came out dark enough to
        // vanish against its own housing, which is why the fader's grip
        // needed hand-brightening at the call site.
        hardware_mark: c(theme.tokens.text_dim).mix(c(theme.tokens.text), 0.5),
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
    pub meter_safe: Color,
    pub meter_warn: Color,
    /// Pan's own colour — yellow, so it is not mistaken for volume's
    /// blue in the column beside it.
    pub pan: Color,
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
    #[must_use]
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
            meter_safe: c(theme.tokens.meter_safe),
            meter_warn: c(theme.tokens.meter_warn),
            pan: c(theme.tokens.route_send),
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
    /// The same panel as bands: two rectangles a row, no controls.
    ///
    /// The detail a row deserves depends on how tall it is ON SCREEN,
    /// and that is the zoom — which the recorded scene cannot know,
    /// because not depending on the zoom is the whole reason it can be
    /// recorded once. So both are recorded and the replay picks per row.
    ///
    /// Without this, a session zoomed out to fit drew two thousand
    /// complete track panels into a strip a fifth of a pixel tall each:
    /// seventy-five thousand commands to produce a column of coloured
    /// lines, and the one view that most needs to be fast was the
    /// slowest thing the window did.
    pub panel_bar: Scene,
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
    /// The top of every row, plus the bottom of the last — so row `i`
    /// occupies `offsets[i]..offsets[i + 1]` and there are `rows + 1`
    /// entries.
    ///
    /// Cumulative rather than a list of heights, because what a scroll
    /// actually asks is "which row is at this y", and a running sum
    /// answers it with a binary search where heights would need a walk.
    offsets: Vec<f64>,
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
    /// `panel_bar` command range per row.
    panel_bar: Vec<std::ops::Range<u32>>,
    /// Content-space x extent of every `lanes` command, indexed the same
    /// way as `lanes.commands`. In seconds, like the recording.
    x: Vec<(f64, f64)>,
}

impl Arrangement {
    /// How tall the whole session is, in content pixels.
    #[must_use]
    pub fn content_height(&self) -> f64 {
        self.offsets.last().copied().unwrap_or(0.0)
    }

    /// How tall row `i` is, in content pixels.
    #[must_use]
    fn row_height(&self, row: usize) -> f64 {
        let top = self.offsets.get(row).copied().unwrap_or(0.0);
        let bottom = self.offsets.get(row.saturating_add(1)).copied().unwrap_or(top);
        bottom - top
    }

    /// The rows that intersect `view`, clamped to what exists.
    ///
    /// A binary search over [`Arrangement::offsets`] rather than a
    /// division by a row pitch: there is no pitch any more. Rows are
    /// whatever height the project says, so "which row is at this y" is
    /// a lookup, not arithmetic.
    ///
    /// One row of bleed on each side: a row scrolled half off the top
    /// still paints its visible half, and dropping it would tear the
    /// edge of the screen during exactly the fast scroll this is for.
    #[must_use]
    pub fn visible_rows(&self, view: Viewport) -> std::ops::Range<usize> {
        let rows = self.index.lanes.len();
        if rows == 0 {
            return 0..0;
        }
        let zoom = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        let top = view.scroll_y / zoom;
        let bottom = (view.scroll_y + view.height) / zoom;
        // `partition_point` gives the first row whose TOP is past the
        // edge; the row before it is the one the edge falls inside.
        let first = self.offsets.partition_point(|&y| y <= top).saturating_sub(1);
        let last = self.offsets.partition_point(|&y| y < bottom);
        first.min(rows)..last.min(rows)
    }
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
    /// Record the whole project, once.
    #[must_use]
    pub fn build(
        palette: &Palette,
        font: &crate::text::Font,
        project: &ProjectRef,
        rows: &RowsRef,
        layout: crate::layout::Layout,
    ) -> Self {
        let mut lanes = Scene::new();
        let mut panel = Scene::new();
        let mut panel_bar = Scene::new();
        let mut index = Index::default();
        let mut offsets = Vec::with_capacity(rows.len().saturating_add(1));
        let mut y = 0.0_f64;
        for (row, (track, depth)) in rows.iter().enumerate() {
            // Command indices are `u32`: two per row plus one per item,
            // so four billion of them is a project nothing could open.
            // Saturating rather than wrapping, because a wrapped index
            // would cull the wrong rows rather than fail.
            let lanes_from = command_index(&lanes);
            let panel_from = command_index(&panel);
            let bar_from = command_index(&panel_bar);
            offsets.push(y);
            // The track's own height, or the user's default, floored at
            // something far below REAPER's minimum — see `layout`. The
            // divider lives INSIDE the row, so a 24 track is 24 tall.
            let h = layout.height_of(track.height);
            let body = (h - DIVIDER).max(0.5);
            let stripe = if row % 2 == 0 { palette.row_a } else { palette.row_b };

            // The lane's background and the divider under it. Recorded
            // absurdly wide so a replay at any zoom still covers the
            // viewport; the backend clips.
            lanes.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                stripe,
                None,
                &Rect::new(0.0, y, project.length_secs.max(1.0), y + body),
            );
            index.x.push((f64::MIN, f64::MAX));
            lanes.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.divider,
                None,
                &Rect::new(0.0, y + body, project.length_secs.max(1.0), y + h),
            );
            index.x.push((f64::MIN, f64::MAX));

            // The panel row — the whole REAPER-matched control panel, at
            // the geometry the DOM row uses. See `crate::tcp`.
            crate::tcp::draw_row(
                &mut panel,
                palette,
                font,
                track,
                i32::try_from(*depth).unwrap_or(0),
                y,
                body,
            );
            // The same row as a band, for when it is too short on
            // screen to be worth more. Its tint and its gutter and
            // nothing else — see `Arrangement::panel_bar`.
            panel_bar.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                crate::tcp::row_tint(palette, track),
                None,
                &Rect::new(0.0, y, TCP_WIDTH, y + body),
            );
            panel_bar.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.divider,
                None,
                &Rect::new(0.0, y + body, TCP_WIDTH, y + h),
            );

            // The divider under it, matching the lane's.
            panel.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.divider,
                None,
                &Rect::new(0.0, y + body, TCP_WIDTH, y + h),
            );

            // The items on this lane. An item with no colour of its own
            // takes its TRACK's, which is what makes a session read by
            // section when it is zoomed out far enough that names are
            // gone — the same rule the panel's row tint follows.
            let track_color = crate::tcp::track_color(palette, track);
            // Items sit inside their lane, but a two-pixel inset on a
            // three-pixel row leaves nothing to see. Scaled down as the
            // row shrinks, so a collapsed session still shows its items
            // as bands rather than as empty lanes.
            let inset = (body * 0.05).clamp(0.0, 2.0);
            for item in project.lane(&track.guid) {
                let x0 = item.position.as_seconds();
                let x1 = x0 + item.length.as_seconds().max(0.001);
                let color = item.color.map_or(track_color, |rgb| {
                    rgb24(rgb, if item.muted { 0x66 } else { 0xff })
                });
                lanes.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Rect::new(x0, y + inset, x1, y + body - inset),
                );
                index.x.push((x0, x1));
            }

            index.lanes.push(lanes_from..command_index(&lanes));
            index.panel.push(panel_from..command_index(&panel));
            index.panel_bar.push(bar_from..command_index(&panel_bar));
            y += h;
        }
        // The bottom of the last row, so every row has a `..end`.
        offsets.push(y);

        debug_assert_eq!(index.x.len(), lanes.commands.len(), "one x extent per lane command");

        Self {
            lanes,
            panel,
            panel_bar,
            index,
            offsets,
            rows: rows.len(),
            length_secs: project.length_secs,
            bpm: project.bpm,
            item_count: project.item_count,
        }
    }
}

impl Arrangement {
    /// How many items this scene draws.
    #[must_use]
    pub const fn items(&self) -> usize {
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
        let mut counts = Counts::default();
        for row in self.visible_rows(view) {
            let Some(span) = self.index.lanes.get(row) else {
                continue;
            };
            for (i, cmd) in commands(&self.lanes, span) {
                counts.replayed = counts.replayed.saturating_add(1);
                // Off the sides of a row that IS on screen — the case
                // that matters when zoomed in on bar 400 of 500.
                match self.index.x.get(i) {
                    Some(&(x0, x1)) if x1 < left || x0 > right => continue,
                    _ => {}
                }
                if submit_command(painter, cmd, transform) {
                    counts.submitted = counts.submitted.saturating_add(1);
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
        let zoom = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        for row in self.visible_rows(view) {
            // How tall this row lands ON SCREEN, which is what decides
            // whether its controls are worth drawing. A row at its full
            // height under a zoom that shrinks it to a fifth of a pixel
            // is a band, whatever the project says about it.
            let on_screen = self.row_height(row) * zoom;
            let (scene, index) = if on_screen >= crate::tcp::BAND_BELOW {
                (&self.panel, &self.index.panel)
            } else {
                (&self.panel_bar, &self.index.panel_bar)
            };
            let Some(span) = index.get(row) else {
                continue;
            };
            for (_, cmd) in commands(scene, span) {
                counts.replayed = counts.replayed.saturating_add(1);
                if submit_command(painter, cmd, transform) {
                    counts.submitted = counts.submitted.saturating_add(1);
                }
            }
        }
        counts
    }
}

/// One row's commands, with their indices.
///
/// The indices are what the x-extent table is keyed by, so they come out
/// alongside rather than being recomputed. Bounds are the slice's, which
/// is what makes an index recorded against a scene that has since been
/// rebuilt yield nothing instead of panicking.
fn commands<'a>(
    scene: &'a Scene,
    span: &std::ops::Range<u32>,
) -> impl Iterator<Item = (usize, &'a RenderCommand)> {
    let start = usize::try_from(span.start).unwrap_or(usize::MAX);
    let end = usize::try_from(span.end).unwrap_or(usize::MAX);
    scene
        .commands
        .get(start..end.min(scene.commands.len()))
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(move |(offset, cmd)| (start.saturating_add(offset), cmd))
}

/// How many commands a scene holds, as an index.
///
/// Saturating: a wrapped index would quietly cull the wrong rows, where
/// a saturated one draws too much. Four billion commands is a project
/// that could not be opened in the first place.
fn command_index(scene: &Scene) -> u32 {
    u32::try_from(scene.commands.len()).unwrap_or(u32::MAX)
}

/// A packed `0xRRGGBB` with an alpha.
fn rgb24(rgb: u32, alpha: u8) -> Color {
    let byte = |shift: u32| u8::try_from((rgb >> shift) & 0xff).unwrap_or(0);
    Color::from_rgba8(byte(16), byte(8), byte(0), alpha)
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
        counts.replayed = counts.replayed.saturating_add(1);
        if submit_command(painter, cmd, transform) {
            counts.submitted = counts.submitted.saturating_add(1);
        }
    }
    counts
}

impl Arrangement {
    /// Every row of the panel, at the detail the viewport's ZOOM asks
    /// for — but with no culling.
    ///
    /// The control for [`Arrangement::replay_panel`]. It has to make the
    /// same level-of-detail choice, because that choice is not culling:
    /// substituting a band for a row a fifth of a pixel tall changes the
    /// picture on purpose, while skipping a row that is off screen must
    /// not change it at all. Comparing against a control that drew full
    /// panels everywhere would be testing the two together and failing
    /// on the one that is behaving.
    pub fn replay_all_panel(
        &self,
        painter: &mut impl PaintScene,
        view: Viewport,
        transform: Affine,
    ) -> Counts {
        let zoom = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        let mut counts = Counts::default();
        for row in 0..self.index.panel.len() {
            let on_screen = self.row_height(row) * zoom;
            let (scene, index) = if on_screen >= crate::tcp::BAND_BELOW {
                (&self.panel, &self.index.panel)
            } else {
                (&self.panel_bar, &self.index.panel_bar)
            };
            let Some(span) = index.get(row) else {
                continue;
            };
            for (_, cmd) in commands(scene, span) {
                counts.replayed = counts.replayed.saturating_add(1);
                if submit_command(painter, cmd, transform) {
                    counts.submitted = counts.submitted.saturating_add(1);
                }
            }
        }
        counts
    }
}

/// `outer` applied after `inner`.
///
/// Named rather than written as `*` because multiplying two affines is
/// composition, not arithmetic: there is nothing here to overflow, and
/// the lint that flags the operator is right to flag it everywhere else.
fn compose(outer: Affine, inner: Affine) -> Affine {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "affine composition; the operator is matrix multiplication"
    )]
    let composed = outer * inner;
    composed
}

/// Push one recorded command through `transform`.
///
/// The brush is forwarded WHOLE rather than unwrapped.
///
/// This used to match `Paint::Solid` and return false for anything else,
/// which silently dropped every gradient that reached a recorded scene:
/// the fader cap's moulding, the volume knob's drop shadow and body, the
/// record arm's ring. They were not mis-drawn, they were not drawn at
/// all — whatever had been painted underneath showed through instead,
/// which is why the result looked merely flat rather than broken. A
/// direct `painter.fill` with the same gradient worked, which is what
/// finally separated "gradients do not work" from "this replay discards
/// them".
pub fn submit_command(
    painter: &mut impl PaintScene,
    cmd: &RenderCommand,
    transform: Affine,
) -> bool {
    match cmd {
        RenderCommand::Fill(fill) => {
            painter.fill(
                fill.fill,
                compose(transform, fill.transform),
                &fill.brush,
                fill.brush_transform,
                &fill.shape,
            );
            true
        }
        RenderCommand::Stroke(stroke) => {
            painter.stroke(
                &stroke.style,
                compose(transform, stroke.transform),
                &stroke.brush,
                stroke.brush_transform,
                &stroke.shape,
            );
            true
        }
        // Text. Dropping this arm is not a silent degradation of
        // quality, it is a track panel with no names in it, so it is
        // handled here rather than defaulted.
        RenderCommand::GlyphRun(run) => {
            painter.draw_glyphs(
                &run.font_data,
                run.font_size,
                run.hint,
                &run.normalized_coords,
                run.embolden,
                &run.style,
                &run.brush,
                run.brush_alpha,
                compose(transform, run.transform),
                run.glyph_transform,
                run.glyphs.iter().copied(),
            );
            true
        }
        _ => false,
    }
}
