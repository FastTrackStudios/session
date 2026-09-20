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
use vello::kurbo::{Affine, BezPath, Rect};
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
        //
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
#[derive(Clone)]
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
        let c = |col: daw_ui::theming::Color| Color::from_rgba8(col.r, col.g, col.b, col.a);
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
    /// What vertical zoom [`Self::panel`] was cut at.
    ///
    /// Read by the window to know whether the cut it is about to replay
    /// is still the right one. Not a `Cell` the replay checks itself,
    /// because re-cutting needs the rows and the theme and the replay
    /// has neither — the decision belongs to whoever is drawing.
    pub panel_zoom: f64,
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
    /// Where the tempo or the signature changes.
    tempo: Vec<daw_ui::studio::project::TempoChange>,
    /// Every item's title, by row, in seconds — drawn per frame in
    /// pixel space over the lanes, because recorded text would stretch
    /// with the zoom.
    titles: Vec<Title>,
    /// Every item, by row, in seconds, with its fades — what a hit
    /// test asks and what the fade handles are drawn from.
    items: Vec<ItemBox>,
    /// The song's shape, for the ruler's lanes.
    sections: Vec<daw_ui::studio::project::Section>,
    markers: Vec<daw_ui::studio::project::Marker>,
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
        let bottom = self
            .offsets
            .get(row.saturating_add(1))
            .copied()
            .unwrap_or(top);
        bottom - top
    }

    /// A row's top and height, for laying its controls out.
    ///
    /// The overlay and the hit test need this for the same reason the
    /// mixer's `strip_box` exists: the panel is recorded, so anything
    /// live has to be told where the recorded thing is.
    #[must_use]
    pub fn row_box(&self, row: usize) -> Option<(f64, f64)> {
        let top = *self.offsets.get(row)?;
        let bottom = *self.offsets.get(row.checked_add(1)?)?;
        Some((top, (bottom - top - DIVIDER).max(0.5)))
    }

    /// A row's band ON SCREEN: where [`Self::row_box`] puts it, after
    /// the vertical zoom.
    ///
    /// The difference matters to everything drawn OVER the panel
    /// rather than recorded into it. The recording is replayed under a
    /// transform that already carries `zoom_y`, so session coordinates
    /// are all it needs. The live controls — the knob, the arm, the
    /// name, mute and solo — are drawn fresh every frame under a plain
    /// translate, because they must not stretch when the rows get
    /// taller. That leaves them to do the placing themselves, and not
    /// doing it is why they walked off their own rows the moment
    /// anybody zoomed.
    ///
    /// The height is scaled too, and deliberately: it is what decides
    /// which tier of controls a row shows, and the answer has to be
    /// about the row on screen rather than the row in the session.
    /// What does NOT scale is the art inside the band — see
    /// `crate::art::squashed`, which clamps at 1.
    #[must_use]
    pub fn row_band(&self, row: usize, view: Viewport) -> Option<(f64, f64)> {
        let (top, height) = self.row_box(row)?;
        let zoom = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        Some((top * zoom, height * zoom))
    }

    /// Which row is at a content y, if any.
    ///
    /// `content_y` is in SESSION units — divide a screen offset by the
    /// vertical zoom before asking, or use [`Self::row_at_screen`],
    /// which does it for you.
    ///
    /// The same binary search `visible_rows` uses, over the same
    /// cumulative offsets — so a hit and a draw cannot disagree about
    /// which row a pixel belongs to unless the offsets themselves are
    /// wrong.
    #[must_use]
    pub fn row_at(&self, content_y: f64) -> Option<usize> {
        if content_y < 0.0 || self.rows == 0 {
            return None;
        }
        let row = self
            .offsets
            .partition_point(|&y| y <= content_y)
            .checked_sub(1)?;
        (row < self.rows).then_some(row)
    }

    /// Which row is under a y measured in SCREEN pixels from the top
    /// of the lanes.
    ///
    /// The pair to [`Self::row_band`], and the reason both exist: a
    /// hit arrives in screen pixels and the offsets are in session
    /// units, so something has to divide — and for a while nothing
    /// did, which meant that at any zoom but 1 the pointer reported a
    /// row it was not over.
    #[must_use]
    pub fn row_at_screen(&self, screen_y: f64, view: Viewport) -> Option<usize> {
        let zoom = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        self.row_at(screen_y / zoom)
    }

    /// The panel row and control under a point, in the SCREEN
    /// coordinates the lanes start from.
    ///
    /// `x` is measured from the left of the panel and `y` from the top
    /// of the lanes, which is the frame every caller already has to put
    /// the panel in. Lives here rather than in a window because the
    /// answer has to come from the same geometry the row was DRAWN
    /// from, and two windows working it out separately is two chances
    /// to disagree about which button the pointer is on.
    #[must_use]
    pub fn row_spot_at(
        &self,
        view: Viewport,
        rows: &[(daw_proto::Track, u32)],
        x: f64,
        y: f64,
    ) -> Option<(usize, crate::row::Control)> {
        if x < 0.0 || x >= TCP_WIDTH {
            return None;
        }
        let index = self.row_at_screen(y, view)?;
        let (top, height) = self.row_band(index, view)?;
        let (track, depth) = rows.get(index)?;
        let row = crate::row::Row::new(
            top,
            height,
            i32::try_from(*depth).unwrap_or(0),
            track.is_folder,
        );
        // In the band's own frame, which is what `Row` measures from.
        Some((index, row.control_at(x, y)?))
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
        let first = self
            .offsets
            .partition_point(|&y| y <= top)
            .saturating_sub(1);
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

/// The track panel, recorded at a vertical zoom.
///
/// Its own function, and re-runnable, because the panel is the one
/// surface that must NOT be scaled into place. The lanes can be: a
/// waveform stretched twice as tall is a waveform twice as tall, which
/// is what zooming a lane means. A track panel stretched twice as tall
/// is a name plate with the lettering pulled out of shape and a knob
/// turned into an ellipse — every one of which was on screen the
/// moment anybody zoomed.
///
/// So the row art is cut fresh at the heights it will be seen at, and
/// replayed under a translate. What does not scale with it is
/// [`DIVIDER`]: a hairline between rows is a hairline at every zoom,
/// and multiplying it was the other half of the same bug.
fn record_panel(
    palette: &Palette,
    font: &crate::text::Font,
    rows: &[(daw_proto::Track, u32)],
    layout: crate::layout::Layout,
    zoom: f64,
) -> Panels {
    let mut panel = Scene::new();
    let mut bar = Scene::new();
    let mut spans = Vec::with_capacity(rows.len());
    let mut bar_spans = Vec::with_capacity(rows.len());
    let mut lineage: Vec<Color> = Vec::new();
    let mut y = 0.0_f64;
    for (track, depth) in rows {
        let level = usize::try_from(*depth).unwrap_or(0);
        lineage.truncate(level);
        let ancestors = lineage.clone();
        lineage.push(crate::tcp::folder_band(palette, track));

        let from = command_index(&panel);
        let bar_from = command_index(&bar);
        let h = layout.height_of(track.height) * zoom;
        let body = (h - DIVIDER).max(0.5);

        crate::tcp::draw_row(
            &mut panel,
            palette,
            font,
            track,
            i32::try_from(*depth).unwrap_or(0),
            y,
            body,
            &ancestors,
        );
        panel.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.divider,
            None,
            &Rect::new(0.0, y + body, TCP_WIDTH, y + h),
        );

        // The same row as a band, for when it is too short on screen to
        // be worth more. Its tint and its gutter and nothing else.
        bar.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            crate::tcp::row_tint(palette, track),
            None,
            &Rect::new(0.0, y, TCP_WIDTH, y + body),
        );
        bar.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.divider,
            None,
            &Rect::new(0.0, y + body, TCP_WIDTH, y + h),
        );

        spans.push(from..command_index(&panel));
        bar_spans.push(bar_from..command_index(&bar));
        y += h;
    }
    Panels {
        panel,
        bar,
        spans,
        bar_spans,
        zoom,
    }
}

/// One cut of the panel, at one zoom.
struct Panels {
    panel: Scene,
    bar: Scene,
    spans: Vec<std::ops::Range<u32>>,
    bar_spans: Vec<std::ops::Range<u32>>,
    zoom: f64,
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
        previews: &crate::midi::Previews,
    ) -> Self {
        let mut lanes = Scene::new();
        let mut index = Index::default();
        let mut titles = Vec::with_capacity(project.item_count);
        let mut boxes = Vec::with_capacity(project.item_count);
        let mut offsets = Vec::with_capacity(rows.len().saturating_add(1));
        let mut y = 0.0_f64;
        for (row, (track, depth)) in rows.iter().enumerate() {
            // A folder closes simply by the next row being shallower,
            // so the truncate IS the close.
            // Command indices are `u32`: two per row plus one per item,
            // so four billion of them is a project nothing could open.
            // Saturating rather than wrapping, because a wrapped index
            // would cull the wrong rows rather than fail.
            let lanes_from = command_index(&lanes);
            offsets.push(y);
            // The track's own height, or the user's default, floored at
            // something far below REAPER's minimum — see `layout`. The
            // divider lives INSIDE the row, so a 24 track is 24 tall.
            let h = layout.height_of(track.height);
            let body = (h - DIVIDER).max(0.5);
            let stripe = if row % 2 == 0 {
                palette.row_a
            } else {
                palette.row_b
            };

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

            // The panel row is NOT recorded here — see `record_panel`,
            // which cuts it fresh at whatever height the zoom asks for
            // rather than letting a transform stretch it.

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
            let track_index = usize::try_from(track.index).unwrap_or(0);
            for item in project.lane(&track.guid) {
                let x0 = item.position.as_seconds();
                let x1 = x0 + item.length.as_seconds().max(0.001);
                let color = item.color.map_or(track_color, |rgb| {
                    rgb24(rgb, if item.muted { 0x66 } else { 0xff })
                });
                // The body, dimmed, and the waveform over it in the
                // full colour: the item is read by its waveform, and a
                // solid block of colour was a waveform you could not
                // see through.
                lanes.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color.multiply_alpha(0.42),
                    None,
                    &Rect::new(x0, y + inset, x1, y + body - inset),
                );
                index.x.push((x0, x1));
                let top = y + inset;
                let bottom = y + body - inset;
                // What the item CONTAINS: notes if it holds MIDI, the
                // envelope otherwise. An item drawn from a waveform it
                // does not have is why a chord track looked like a
                // shaker.
                //
                // Notes only once they have been read. Until then the
                // block is drawn plain rather than filled with a fake
                // shape, because a wrong picture that later corrects
                // itself is worse than an honest empty one.
                match previews.get(&item.guid) {
                    Some(notes) => {
                        if let Some(roll) = midi_preview(&notes, x0, x1, top, bottom) {
                            lanes.fill(Fill::NonZero, Affine::IDENTITY, color, None, &roll);
                            index.x.push((x0, x1));
                        }
                    }
                    None if !project.is_midi(&item.guid) => {
                        if let Some(wave) = waveform(track_index, x0, x1, top, bottom) {
                            lanes.fill(Fill::NonZero, Affine::IDENTITY, color, None, &wave);
                            index.x.push((x0, x1));
                        }
                    }
                    None => {}
                }
                // The fades, as the part of the item they take away:
                // the region over the gain curve, darkened, from each
                // end. Recorded in seconds like the item, so the zoom
                // stretches them with it.
                let fades = Fades::of(item);
                if let Some(path) = fades.path(x0, x1, top, bottom, 1.0) {
                    lanes.fill(Fill::NonZero, Affine::IDENTITY, FADE_SHADE, None, &path);
                    index.x.push((x0, x1));
                }
                boxes.push(ItemBox {
                    row,
                    guid: item.guid.clone(),
                    x0,
                    x1,
                    fades,
                });
                if let Some(name) = project.title(item) {
                    titles.push(Title {
                        row,
                        x0,
                        x1,
                        name: name.to_owned(),
                    });
                }
            }

            index.lanes.push(lanes_from..command_index(&lanes));
            y += h;
        }
        // The bottom of the last row, so every row has a `..end`.
        offsets.push(y);

        debug_assert_eq!(
            index.x.len(),
            lanes.commands.len(),
            "one x extent per lane command"
        );

        // The panel, cut at zoom 1 to start with. `repanel` re-cuts it
        // whenever the vertical zoom moves.
        let cut = record_panel(palette, font, rows.as_slice(), layout, 1.0);
        index.panel = cut.spans;
        index.panel_bar = cut.bar_spans;

        Self {
            lanes,
            panel: cut.panel,
            panel_bar: cut.bar,
            panel_zoom: cut.zoom,
            index,
            offsets,
            rows: rows.len(),
            length_secs: project.length_secs,
            bpm: project.bpm,
            tempo: project.tempo.clone(),
            item_count: project.item_count,
            titles,
            items: boxes,
            sections: project.sections.clone(),
            markers: project.markers.clone(),
        }
    }

    /// The item under a point in the lanes, and which part of it.
    ///
    /// `x` is the content x in pixels from the lanes' left edge (the
    /// TCP's right), `content_y` the content y the row was found at.
    /// The parts are REAPER's: the fade handles in the top corners,
    /// the edges, and the body — checked in that order, because a
    /// handle sits on an edge and an edge sits on the body.
    #[must_use]
    pub fn item_at(
        &self,
        view: Viewport,
        row: usize,
        x: f64,
        content_y: f64,
    ) -> Option<(usize, ItemZone)> {
        let (top, height) = self.row_box(row)?;
        let scale = view.pps;
        // Last drawn is on top, so the last match wins.
        let mut found = None;
        for (index, item) in self.items.iter().enumerate() {
            if item.row != row {
                continue;
            }
            let x0 = item.x0.mul_add(scale, -view.scroll_x);
            let x1 = item.x1.mul_add(scale, -view.scroll_x);
            if x < x0 - GRAB || x > x1 + GRAB {
                continue;
            }
            let in_handle_band = content_y - top <= HANDLE_BAND * view.zoom_y.max(0.1) + 2.0;
            let fade_in_x = item.fades.fade_in.mul_add(scale, x0);
            let fade_out_x = x1 - item.fades.fade_out * scale;
            let zone = if in_handle_band && (x - fade_in_x).abs() <= GRAB {
                ItemZone::FadeIn
            } else if in_handle_band && (x - fade_out_x).abs() <= GRAB {
                ItemZone::FadeOut
            } else if (x - x0).abs() <= EDGE {
                ItemZone::LeftEdge
            } else if (x - x1).abs() <= EDGE {
                ItemZone::RightEdge
            } else {
                ItemZone::Body
            };
            let _ = height;
            found = Some((index, zone));
        }
        found
    }

    /// An item's box, by the index `item_at` gave.
    #[must_use]
    pub fn item(&self, index: usize) -> Option<&ItemBox> {
        self.items.get(index)
    }

    /// Every item's box, in the order they were recorded.
    ///
    /// For the operations that are about a REGION rather than about a
    /// thing the pointer is on — a razor area asks "what is inside this
    /// rectangle", which no amount of hit testing answers.
    ///
    /// Named for the boxes and not for the items because `items()`
    /// already answers how many the scene draws, which is a different
    /// question with the same noun in it.
    #[must_use]
    pub fn item_boxes(&self) -> &[ItemBox] {
        &self.items
    }

    /// Every other edge sitting at `at` on the same row.
    ///
    /// Two items butted together share a boundary: one ends where the
    /// next begins. Grabbing it and moving only one of them opens a gap
    /// or an overlap, which is never what was meant — what the hand is
    /// on is the SEAM, and a seam moves as one thing.
    ///
    /// Answered in seconds with a tolerance, because the times came
    /// from a file and two edges written to be equal are equal to
    /// within a rounding. Returns each neighbour's guid and which of
    /// its own edges is the one that touches.
    #[must_use]
    pub fn edges_at(&self, row: usize, at: f64, except: &str) -> Vec<(String, ItemZone)> {
        /// A millisecond. Below a millisecond two edges are the same
        /// edge however they were written down, and above it they are
        /// two edges somebody meant to put near each other.
        const SAME: f64 = 0.001;
        self.items
            .iter()
            .filter(|item| item.row == row && item.guid != except)
            .filter_map(|item| {
                if (item.x0 - at).abs() <= SAME {
                    Some((item.guid.clone(), ItemZone::LeftEdge))
                } else if (item.x1 - at).abs() <= SAME {
                    Some((item.guid.clone(), ItemZone::RightEdge))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Every region edge and marker sitting at `at`, except the one
    /// being dragged.
    ///
    /// The ruler's [`Self::edges_at`], and the same reason for
    /// existing: two regions that meet share a boundary, and a marker
    /// written on that boundary is part of it. Dragging one and leaving
    /// the others behind opens a gap between sections that were meant
    /// to be contiguous — a song with a hole between the verse and the
    /// chorus.
    ///
    /// The same millisecond, too. Below it two marks are the same
    /// moment however they were written down; above it they are two
    /// moments somebody meant to put near each other.
    ///
    /// `except` is what the hand has hold of. A region is excluded
    /// whole rather than by the edge pressed: a zero-length region has
    /// both its bounds at one time, and dragging its end must not drag
    /// its own start along.
    #[must_use]
    pub fn marks_at(
        &self,
        at: f64,
        except: Option<crate::ruler::On>,
    ) -> Vec<crate::ruler::MarkEdge> {
        use crate::ruler::{MarkEdge, On};
        /// A millisecond — see [`Self::edges_at`].
        const SAME: f64 = 0.001;
        let (skip_region, skip_marker) = match except {
            Some(On::Region { id, .. }) => (Some(id), None),
            Some(On::Marker { id }) => (None, Some(id)),
            _ => (None, None),
        };
        let mut found = Vec::new();
        for section in &self.sections {
            if skip_region == Some(section.id) {
                continue;
            }
            if (section.start - at).abs() <= SAME {
                found.push(MarkEdge::RegionStart {
                    id: section.id,
                    end: section.end,
                });
            }
            if (section.end - at).abs() <= SAME {
                found.push(MarkEdge::RegionEnd {
                    id: section.id,
                    start: section.start,
                });
            }
        }
        for marker in &self.markers {
            if skip_marker != Some(marker.idx) && (marker.at - at).abs() <= SAME {
                found.push(MarkEdge::Marker { id: marker.idx });
            }
        }
        found
    }

    /// An item's box, by its guid.
    #[must_use]
    pub fn item_by_guid(&self, guid: &str) -> Option<&ItemBox> {
        self.items.iter().find(|b| b.guid == guid)
    }

    /// The regions, for the ruler's lanes.
    #[must_use]
    pub fn sections(&self) -> &[daw_ui::studio::project::Section] {
        &self.sections
    }

    /// The markers, for the ruler's lanes.
    #[must_use]
    pub fn markers(&self) -> &[daw_ui::studio::project::Marker] {
        &self.markers
    }

    /// Where the tempo and the signature change, for the ruler's strip
    /// and for the bar grid that has to count through them.
    #[must_use]
    pub fn tempo(&self) -> &[daw_ui::studio::project::TempoChange] {
        &self.tempo
    }

    /// The item titles on the rows a viewport shows, with where each
    /// sits — for the per-frame pass that writes them in pixel space.
    pub fn titles_in(&self, view: Viewport) -> impl Iterator<Item = (&Title, f64, f64)> + '_ {
        let rows = self.visible_rows(view);
        self.titles.iter().filter_map(move |title| {
            if !rows.contains(&title.row) {
                return None;
            }
            let (top, height) = self.row_box(title.row)?;
            Some((title, top, height))
        })
    }
}

/// How far from a fade handle or an item edge a hit still takes it.
const GRAB: f64 = 6.0;

/// How far from an item's end a hit is the edge rather than the body.
const EDGE: f64 = 5.0;

/// How deep the band along an item's top is where the fade handles
/// live, in pixels at zoom 1.
const HANDLE_BAND: f64 = 10.0;

/// The shade a fade takes off the item: what is faded out is darker.
const FADE_SHADE: Color = Color::from_rgba8(0x00, 0x00, 0x00, 0x5c);

/// Which part of an item a point is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemZone {
    Body,
    LeftEdge,
    RightEdge,
    /// The fade-in's handle: the top corner where the fade ends.
    FadeIn,
    /// The fade-out's handle: the top corner where the fade starts.
    FadeOut,
}

/// An item as the hit test and the handles see it.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemBox {
    pub row: usize,
    pub guid: String,
    /// Its span, in seconds.
    pub x0: f64,
    pub x1: f64,
    pub fades: Fades,
}

/// An item's two fades: how long, and what shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fades {
    pub fade_in: f64,
    pub fade_out: f64,
    pub in_shape: daw_proto::item::FadeShape,
    pub out_shape: daw_proto::item::FadeShape,
}

impl Fades {
    #[must_use]
    pub fn of(item: &daw_proto::Item) -> Self {
        Self {
            fade_in: item.fade_in_length.as_seconds().max(0.0),
            fade_out: item.fade_out_length.as_seconds().max(0.0),
            in_shape: item.fade_in_shape,
            out_shape: item.fade_out_shape,
        }
    }

    /// The gain a fade shape has reached at `t` of its length, 0..1.
    ///
    /// The plugin's own seven shapes: linear; fast start (the gain is
    /// up quickly) and fast end (it is up late), each with a steeper
    /// cousin; and the slow-start-and-end S, with its steeper cousin.
    #[must_use]
    pub fn gain(shape: daw_proto::item::FadeShape, t: f64) -> f64 {
        use daw_proto::item::FadeShape as S;
        let t = t.clamp(0.0, 1.0);
        match shape {
            S::Linear => t,
            S::FastStart => 1.0 - (1.0 - t).powi(2),
            S::FastStartSteep => 1.0 - (1.0 - t).powi(3),
            S::FastEnd => t.powi(2),
            S::FastEndSteep => t.powi(3),
            S::SlowStartEnd => t * t * (3.0 - 2.0 * t),
            S::SlowStartEndSteep => {
                let s = t * t * (3.0 - 2.0 * t);
                s * s * (3.0 - 2.0 * s)
            }
        }
    }

    /// Both fades as one path: the region over each gain curve, which
    /// is the part of the item the fade takes away.
    ///
    /// `x0..x1` and the fades are in the same unit — seconds for the
    /// recording, pixels for the live overlay — and `scale` turns that
    /// unit into the x axis; `top..bottom` are pixels either way.
    /// `None` when there is no fade to draw.
    #[must_use]
    pub fn path(self, x0: f64, x1: f64, top: f64, bottom: f64, scale: f64) -> Option<BezPath> {
        const STEPS: usize = 12;
        let height = bottom - top;
        if height <= 0.0 || (self.fade_in <= 0.0 && self.fade_out <= 0.0) {
            return None;
        }
        let span = (x1 - x0).max(0.0);
        let fade_in = self.fade_in.min(span);
        let fade_out = self.fade_out.min(span);
        let at = |x: f64| x * scale;
        let y_of = |gain: f64| gain.mul_add(-height, bottom);
        let mut path = BezPath::new();
        if fade_in > 0.0 {
            path.move_to((at(x0), top));
            for k in 0..=STEPS {
                let t = crate::num::coord(k) / crate::num::coord(STEPS);
                let x = fade_in.mul_add(t, x0);
                path.line_to((at(x), y_of(Self::gain(self.in_shape, t))));
            }
            path.close_path();
        }
        if fade_out > 0.0 {
            path.move_to((at(x1), top));
            for k in 0..=STEPS {
                let t = crate::num::coord(k) / crate::num::coord(STEPS);
                let x = x1 - fade_out * t;
                path.line_to((at(x), y_of(Self::gain(self.out_shape, t))));
            }
            path.close_path();
        }
        Some(path)
    }
}

/// The fade handles and the fade being dragged, in pixel space over
/// the lanes: the per-frame pass.
///
/// A handle is drawn on the hovered item — a small square at the top
/// corner where each fade ends — so a fade you can grab is a fade you
/// can see the grab for, and nothing is drawn on the others: twenty
/// thousand handles would be a texture. A fade in flight is drawn
/// whole over its recorded self, so the drag is seen before the
/// recording catches up.
pub fn fade_overlay(
    painter: &mut impl PaintScene,
    palette: &Palette,
    scene: &Arrangement,
    view: Viewport,
    origin: (f64, f64),
    hovered: Option<usize>,
    in_flight: Option<(usize, Fades)>,
) {
    let (ox, oy) = origin;
    let x_of = |t: f64| t.mul_add(view.pps, ox);
    if let Some((index, fades)) = in_flight
        && let Some(item) = scene.item(index)
        && let Some((top, height)) = scene.row_box(item.row)
    {
        let inset = (height * 0.05).clamp(0.0, 2.0);
        let (top, bottom) = (
            top.mul_add(view.zoom_y, oy) + inset,
            (top + height).mul_add(view.zoom_y, oy) - inset,
        );
        let px = Fades {
            fade_in: fades.fade_in * view.pps,
            fade_out: fades.fade_out * view.pps,
            ..fades
        };
        if let Some(path) = px.path(x_of(item.x0), x_of(item.x1), top, bottom, 1.0) {
            painter.fill(Fill::NonZero, Affine::IDENTITY, FADE_SHADE, None, &path);
            painter.stroke(
                &vello::kurbo::Stroke::new(1.2),
                Affine::IDENTITY,
                palette.text,
                None,
                &path,
            );
        }
    }
    let Some(index) = hovered.or(in_flight.map(|(i, _)| i)) else {
        return;
    };
    let Some(item) = scene.item(index) else {
        return;
    };
    let Some((top, _)) = scene.row_box(item.row) else {
        return;
    };
    let fades = in_flight.map_or(item.fades, |(_, f)| f);
    let top = top.mul_add(view.zoom_y, oy) + 2.0;
    for x in [
        x_of(item.x0 + fades.fade_in),
        x_of(item.x1 - fades.fade_out),
    ] {
        let r = Rect::new(x - 3.0, top, x + 3.0, top + 6.0);
        painter.fill(Fill::NonZero, Affine::IDENTITY, palette.text, None, &r);
    }
}

/// The selection and the ghosts, in pixel space over the lanes.
///
/// A selected item wears an outline; an item being moved or trimmed
/// shows where it will land as an outline at the new place, over the
/// recorded item where it still is — the recording catches up on the
/// release.
/// The razor areas, over the lanes.
///
/// Drawn as a filled rectangle with a hard edge down each side,
/// because the EDGES are the operative part: an area slices there, and
/// a soft-edged wash would be a marquee — a thing that selects what it
/// covers rather than cutting where it stops. The fill is the accent
/// at low alpha so the waveform under it stays readable; you are
/// choosing a piece of that waveform and need to see which piece.
///
/// `in_flight` is the area being drawn right now, which is not in the
/// set yet — a drag that added to the set every frame would merge with
/// itself and grow in one direction only.
pub fn razor_overlay(
    painter: &mut impl PaintScene,
    palette: &Palette,
    scene: &Arrangement,
    view: Viewport,
    origin: (f64, f64),
    set: &razor::RazorSet,
    in_flight: Option<razor::RazorArea>,
) {
    let (ox, oy) = origin;
    let x_of = |t: f64| t.mul_add(view.pps, ox);
    let visible = scene.visible_rows(view);
    for area in set.areas.iter().copied().chain(in_flight) {
        if area.is_empty() {
            continue;
        }
        // Row by row rather than one rectangle from the first row's top
        // to the last row's bottom: rows are not all the same height,
        // and a rectangle drawn between two of them would cover the
        // ones between at whatever height the maths happened to give.
        let (lo, hi) = (area.row_lo.max(0), area.row_hi.max(0));
        for row in lo..=hi {
            let Ok(row) = usize::try_from(row) else {
                continue;
            };
            if !visible.contains(&row) {
                continue;
            }
            let Some((top, height)) = scene.row_box(row) else {
                continue;
            };
            let r = Rect::new(
                x_of(area.t0),
                top.mul_add(view.zoom_y, oy),
                x_of(area.t1),
                (top + height).mul_add(view.zoom_y, oy),
            );
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.accent.multiply_alpha(0.18),
                None,
                &r,
            );
            for edge in [r.x0, r.x1] {
                painter.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    palette.accent,
                    None,
                    &Rect::new(edge - 0.5, r.y0, edge + 0.5, r.y1),
                );
            }
        }
    }
}

pub fn selection_overlay(
    painter: &mut impl PaintScene,
    palette: &Palette,
    scene: &Arrangement,
    view: Viewport,
    origin: (f64, f64),
    selected: &std::collections::HashSet<String>,
    ghost: Option<(usize, f64, f64)>,
) {
    let (ox, oy) = origin;
    let x_of = |t: f64| t.mul_add(view.pps, ox);
    let rows = scene.visible_rows(view);
    for item in scene
        .items
        .iter()
        .filter(|i| rows.contains(&i.row) && selected.contains(&i.guid))
    {
        let Some((top, height)) = scene.row_box(item.row) else {
            continue;
        };
        let inset = (height * 0.05).clamp(0.0, 2.0);
        let r = Rect::new(
            x_of(item.x0),
            top.mul_add(view.zoom_y, oy) + inset,
            x_of(item.x1),
            (top + height).mul_add(view.zoom_y, oy) - inset,
        );
        painter.stroke(
            &vello::kurbo::Stroke::new(1.5),
            Affine::IDENTITY,
            palette.text,
            None,
            &r.inset(-0.75),
        );
    }
    if let Some((index, x0, x1)) = ghost
        && let Some(item) = scene.item(index)
        && let Some((top, height)) = scene.row_box(item.row)
    {
        let inset = (height * 0.05).clamp(0.0, 2.0);
        let r = Rect::new(
            x_of(x0),
            top.mul_add(view.zoom_y, oy) + inset,
            x_of(x1),
            (top + height).mul_add(view.zoom_y, oy) - inset,
        );
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.text.multiply_alpha(0.12),
            None,
            &r,
        );
        painter.stroke(
            &vello::kurbo::Stroke::new(1.0),
            Affine::IDENTITY,
            palette.text,
            None,
            &r,
        );
    }
}

/// An item's name and where it sits: the row, and its span in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Title {
    pub row: usize,
    pub x0: f64,
    pub x1: f64,
    pub name: String,
}

/// An item's notes as one path: a block per note, stacked by pitch.
///
/// A piano roll squeezed into a lane. The pitch range is the item's
/// own rather than the full 0..127, so a bass part fills its block
/// instead of hugging the floor of a scale it never plays in — what a
/// preview is for is the SHAPE of the part, and a shape pressed into
/// the bottom eighth of the block has none.
///
/// One path for every note, because a lane of a hundred notes is a
/// hundred fills otherwise, and the index has to carry an extent per
/// command.
fn midi_preview(
    notes: &[crate::midi::Note],
    x0: f64,
    x1: f64,
    top: f64,
    bottom: f64,
) -> Option<BezPath> {
    if notes.is_empty() || bottom - top < 2.0 {
        return None;
    }
    let (low, high) = notes.iter().fold((u8::MAX, u8::MIN), |(lo, hi), note| {
        (lo.min(note.pitch), hi.max(note.pitch))
    });
    // A part on one pitch has no range to spread over, so it is drawn
    // down the middle rather than divided by zero.
    let span = f64::from(high.saturating_sub(low)).max(1.0);
    let width = x1 - x0;
    let height = bottom - top;
    // Tall enough to see, short enough that neighbouring pitches do not
    // merge into a block.
    let note_h = (height / span.min(24.0)).clamp(1.0, height / 3.0);

    let mut path = BezPath::new();
    for note in notes {
        let at = x0 + width * f64::from(note.at);
        // Every note gets a width, however short: a preview of a
        // sixteenth-note part at this zoom is otherwise nothing at all.
        let len = (width * f64::from(note.len)).max(width * 0.004);
        let from_top = f64::from(high.saturating_sub(note.pitch)) / span;
        let y = (height - note_h).mul_add(from_top, top);
        // Written out rather than built from a Rect: one path holding
        // every note keeps this to a single fill, and the index carries
        // one extent per command.
        let (x_from, x_to) = (at, (at + len).min(x1));
        path.move_to((x_from, y));
        path.line_to((x_to, y));
        path.line_to((x_to, y + note_h));
        path.line_to((x_from, y + note_h));
        path.close_path();
    }
    Some(path)
}

/// How many points a second a recorded waveform has.
///
/// Recorded once in seconds, so the zoom stretches it: at a hundred
/// pixels a second twelve points is a facet every eight pixels, which
/// is the coarsest a waveform can be before it reads as a polygon —
/// and at the opening zoom it is finer than the pixels.
const WAVE_POINTS_PER_SECOND: f64 = 12.0;

/// How many readings each point holds the peak of.
///
/// A waveform display is a peak display: each column is the loudest
/// the audio got across it, not a sample from it. Sampled, a hit that
/// fell between two points was a bead where a transient should be.
const WAVE_HOLD: usize = 4;

/// An item's waveform as one closed path: the envelope forward along
/// the top, back along the bottom, mirrored about the lane's middle.
///
/// From the simulation until the engine streams peaks — see
/// `simulate::waveform` — and `None` for a lane too short to show one.
fn waveform(track: usize, x0: f64, x1: f64, top: f64, bottom: f64) -> Option<BezPath> {
    let half = (bottom - top) / 2.0;
    if half < 1.5 {
        return None;
    }
    let mid = (top + bottom) / 2.0;
    let span = (x1 - x0).max(0.0);
    let count = crate::num::index((span * WAVE_POINTS_PER_SECOND).ceil()).max(2);
    let at = |i: usize| x0 + span * crate::num::coord(i) / crate::num::coord(count);
    // A hair of amplitude at silence, so a quiet item still has a
    // line down its middle and reads as audio rather than as a gap.
    // Each point holds the peak over the stretch it stands for.
    let step = 1.0 / WAVE_POINTS_PER_SECOND / crate::num::coord(WAVE_HOLD);
    let amp = |t: f64| {
        (0..WAVE_HOLD)
            .map(|k| crate::simulate::waveform(track, crate::num::coord(k).mul_add(-step, t)))
            .fold(0.0_f64, f64::max)
            .mul_add(half - 1.0, 0.6)
    };
    let mut path = BezPath::new();
    path.move_to((x0, mid - amp(x0)));
    for i in 1..=count {
        let x = at(i).min(x1);
        path.line_to((x, mid - amp(x)));
    }
    for i in (0..=count).rev() {
        let x = at(i).min(x1);
        path.line_to((x, mid + amp(x)));
    }
    path.close_path();
    Some(path)
}

/// The item titles, in pixel space, over the lanes: the per-frame pass.
///
/// `origin` is where the lanes' (0 s, row 0) lands on the surface —
/// the same translation the lanes are replayed under, so a title sits
/// on its item at every scroll and zoom. Written only where there is
/// room to read one: a row shorter than a line, or an item narrower
/// than a few characters, gets none.
pub fn titles(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &crate::text::Font,
    scene: &Arrangement,
    view: Viewport,
    origin: (f64, f64),
) {
    const SIZE: f32 = 8.0;
    const PAD: f64 = 3.0;
    for (title, top, height) in scene.titles_in(view) {
        let row_h = height * view.zoom_y;
        if row_h < 12.0 {
            continue;
        }
        let left = title.x0.mul_add(view.pps, origin.0);
        let right = title.x1.mul_add(view.pps, origin.0);
        let room = right - left - PAD * 2.0;
        if room < 12.0 {
            continue;
        }
        // Cut the name to what fits, so a title never runs off its item
        // onto the next one.
        let mut name: &str = &title.name;
        while !name.is_empty() && font.width(name, SIZE) > room {
            let mut end = name.len().saturating_sub(1);
            while end > 0 && !name.is_char_boundary(end) {
                end = end.saturating_sub(1);
            }
            name = name.get(..end).unwrap_or("");
        }
        if name.is_empty() {
            continue;
        }
        let y = top.mul_add(view.zoom_y, origin.1) + f64::from(SIZE) + 2.0;
        crate::tcp::glyphs(painter, font, palette.text, name, left + PAD, y, SIZE);
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

    /// Cut the panel again, for a new vertical zoom.
    ///
    /// Cheap enough to do while a zoom is in flight — it is forty rows
    /// of vector art, not the whole session — and the only way to zoom
    /// a panel without deforming it. See [`record_panel`].
    ///
    /// A no-op when the zoom has not moved, so the caller can ask every
    /// frame and pay only on the frames that changed.
    pub fn repanel(
        &mut self,
        palette: &Palette,
        font: &crate::text::Font,
        rows: &[(daw_proto::Track, u32)],
        layout: crate::layout::Layout,
        zoom: f64,
    ) -> bool {
        let zoom = if zoom > 0.0 { zoom } else { 1.0 };
        if (zoom - self.panel_zoom).abs() < f64::EPSILON {
            return false;
        }
        let cut = record_panel(palette, font, rows, layout, zoom);
        self.panel = cut.panel;
        self.panel_bar = cut.bar;
        self.index.panel = cut.spans;
        self.index.panel_bar = cut.bar_spans;
        self.panel_zoom = cut.zoom;
        true
    }

    /// Forget which zoom the panel was cut at, so the next
    /// [`Self::repanel`] re-cuts whatever the zoom is doing.
    ///
    /// For the things a cut BAKES IN that are not the zoom — the track
    /// name above all. A name is recorded chrome, not a live value, so
    /// renaming a track changes nothing on screen until the row it is
    /// written into is drawn again.
    pub fn forget_panel(&mut self) {
        // NaN, because every comparison against it is false — which is
        // exactly "this cut matches nothing".
        self.panel_zoom = f64::NAN;
    }

    /// The same for the track panel, which scrolls vertically only and
    /// therefore needs no horizontal test.
    ///
    /// `transform` must be a TRANSLATE. The cut being replayed was made
    /// at `panel_zoom` and already carries the vertical zoom in its own
    /// geometry, so scaling it here would apply the zoom twice — and
    /// deform it, which is the thing [`Self::repanel`] exists to stop.
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
pub fn replay_all(painter: &mut impl PaintScene, scene: &Scene, transform: Affine) -> Counts {
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
        // Clipping. The rack scrolls, which means it draws past its own
        // box in both directions — above into the rails, below over the
        // strip's own controls — and a clip is the only thing that
        // stops it. Dropped, as these were, a scrolled rack silently
        // painted over its neighbours.
        //
        // Balanced by construction: whoever pushes one pops it inside
        // the same scene, and the overlay submits a scene whole. The
        // RECORDED scenes are replayed in per-strip ranges and must
        // therefore never carry a layer, since a range could cut
        // between a push and its pop.
        RenderCommand::PushClipLayer(clip) => {
            painter.push_clip_layer(compose(transform, clip.transform), &clip.clip);
            true
        }
        RenderCommand::PopLayer => {
            painter.pop_layer();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod chrome_tests {
    /// The fader cap's ring is drawn in `chrome.hardware`, and a ring
    /// painted in a colour that is itself translucent is a ring you can
    /// see the meter through — which is the one thing the cap's window
    /// exists to be, and the one thing its ring must not be.
    #[test]
    fn the_hardware_the_cap_is_moulded_from_is_opaque() {
        let chrome = super::chrome(&daw_ui::theming::Theme::dark());
        assert_eq!(chrome.hardware.a, 255, "the cap's body is translucent");
        assert_eq!(
            chrome.hardware_edge.a, 255,
            "the cap's border is translucent"
        );
        // And it has to be TELLABLE from the strip it sits on. An
        // opaque ring the same grey as the background reads as no ring
        // at all, which looks exactly like a transparent one.
        // It is the strip's own grey, deliberately: these controls are
        // moulded out of the panel rather than sitting on it, and the
        // bevel and border are what give them their edges.
        //
        // Chasing a "transparent" cap once led here, and this was the
        // wrong suspect — the cap was solid all along. What made it
        // look see-through was the level being painted over the whole
        // cap instead of only the pane cut in it. See `CAP_PANE_Y0`.
        assert_eq!(chrome.hardware, chrome.surface_raised);
    }
}
