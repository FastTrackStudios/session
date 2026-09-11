//! Where the window's colours come from.
//!
//! Not from hexes written into the stylesheet. The stylesheet declares
//! *variables*; this module fills them from [`crate::theming::Theme`],
//! which is the same token model the rest of daw-ui draws from and the
//! target of [`crate::theming::reaper_import`] — so pointing the window
//! at a real `.ReaperTheme` directory re-colours it whole, with no
//! change here or in the components.
//!
//! The mapping is REAPER's own vocabulary, because that is what
//! [`ArrangeTheme`] already is: `col_arrangebg` behind the lanes,
//! `col_tr1_bg`/`col_tr2_bg` alternating per track, `col_gridlines2` for
//! the measure lines, `col_tl_bg`/`col_tl_fg` for the ruler,
//! `playcursor_color` for the playhead, `marker_lane_*`/`region_lane_*`
//! for the two ruler lanes. A window that invents its own greys beside a
//! theme system is two themes, and they diverge the first time either
//! one is touched.
//!
//! One string, set on the root element. Every rule in [`super::css`]
//! reads through these variables, so a theme change is a single
//! attribute write — and, since the variables are also what `calc()`
//! positions from, it costs one style pass rather than a re-render.

use crate::theming::{Color, Theme};

/// How far apart two colours must be, in luminance, to be read as text
/// on a surface. Not a WCAG ratio — a cheap monotonic stand-in, applied
/// only to rescue an unusable pairing, never to restyle a working one.
const MIN_CONTRAST: f32 = 0.28;

/// `ink`, or `ink` pushed far enough from `bg` to be legible on it.
///
/// A palette key is only meaningful in the context REAPER uses it in,
/// and this window puts some of them somewhere REAPER does not. The
/// REAPER 7 default is the case in point: its `col_main_text` is
/// `rgb(30,34,34)` — nearly black — because REAPER draws that text over
/// light theme *images*, not over the mid-grey `col_main_bg2` this
/// window paints. Taken literally it makes every label in the transport
/// and the track panel dark-on-dark.
///
/// So the theme's colour is used as given whenever it works, and only
/// corrected when it cannot be read at all — by pushing it away from the
/// surface, which keeps the theme's hue and only moves its lightness.
fn readable_on(ink: Color, bg: Color) -> Color {
    let (ink_l, bg_l) = (ink.luminance(), bg.luminance());
    if (ink_l - bg_l).abs() >= MIN_CONTRAST {
        return ink;
    }
    // Away from the surface, not toward an arbitrary white: a dark
    // surface lightens its ink and a light one darkens it. The amount is
    // SOLVED for, not guessed — `lighten(t)` mixes `t` of the way to
    // white, so it moves luminance by `t * (1 - l)`, not by `t`. Passing
    // the raw deficit is what made the first attempt at this worse than
    // no correction: a near-black ink on a mid-grey surface moved a
    // fraction of the way it needed to and landed *on* the surface,
    // erasing every track name in the panel.
    if bg_l < 0.5 {
        let target = (bg_l + MIN_CONTRAST).min(1.0);
        let t = ((target - ink_l) / (1.0 - ink_l).max(f32::EPSILON)).clamp(0.0, 1.0);
        ink.lighten(t)
    } else {
        let target = (bg_l - MIN_CONTRAST).max(0.0);
        let t = (1.0 - target / ink_l.max(f32::EPSILON)).clamp(0.0, 1.0);
        ink.darken(t)
    }
}

/// The custom-property block for the studio root.
///
/// Written as an inline `style`, which is what gives it precedence over
/// the `:root`-level fallbacks the stylesheet declares for the case
/// where no theme has been provided at all.
pub fn css_variables(theme: &Theme) -> String {
    let t = &theme.tokens;
    let a = &theme.arrange;
    // Inks are corrected against the surface they actually land on, not
    // against the one the theme's author had in mind — see
    // [`readable_on`].
    let ink = readable_on(t.text, t.surface_raised);
    let ink_dim = readable_on(t.text_dim, t.surface);
    let ink_faint = readable_on(t.text_faint, t.surface);
    let ruler_fg = readable_on(a.ruler_fg, a.ruler_bg);
    let item_label = readable_on(a.item_label, a.item_bg[0]);
    // REAPER alternates lane backgrounds per track index; the TCP rows
    // beside them have to alternate on the same parity or the two
    // columns stripe against each other.
    format!(
        "--ink: {ink}; \
         --ink-dim: {ink_dim}; \
         --ink-faint: {ink_faint}; \
         --surface: {surface}; \
         --surface-raised: {raised}; \
         --surface-sunken: {sunken}; \
         --rule: {rule}; \
         --accent: {accent}; \
         --solo: {solo}; \
         --mute: {mute}; \
         --arrange-bg: {arrange_bg}; \
         --row-bg-a: {row_a}; \
         --row-bg-b: {row_b}; \
         --row-divider: {row_div}; \
         --grid-measure: {grid_measure}; \
         --grid-beat: {grid_beat}; \
         --ruler-bg: {ruler_bg}; \
         --ruler-fg: {ruler_fg}; \
         --ruler-fg2: {ruler_fg2}; \
         --marker-lane-bg: {marker_bg}; \
         --marker-lane-text: {marker_text}; \
         --marker-fill: {marker_fill}; \
         --marker-edge: {marker_edge}; \
         --region-lane-bg: {region_bg}; \
         --region-lane-text: {region_text}; \
         --region-fill: {region_fill}; \
         --item-fallback: {item_fallback}; \
         --item-edge: {item_edge}; \
         --item-label: {item_label}; \
         --play-cursor: {play_cursor}; \
         --edit-cursor: {edit_cursor}; \
         --radius: {radius}px;",
        ink = ink.css(),
        ink_dim = ink_dim.css(),
        ink_faint = ink_faint.css(),
        surface = t.surface.css(),
        raised = t.surface_raised.css(),
        sunken = t.surface_sunken.css(),
        rule = t.border.css(),
        accent = t.accent.css(),
        solo = t.solo.css(),
        mute = t.mute.css(),
        arrange_bg = a.bg.css(),
        row_a = a.row_bg[0].css(),
        row_b = a.row_bg[1].css(),
        row_div = a.row_divider[0].css(),
        grid_measure = a.grid_measure.css(),
        grid_beat = a.grid_beat.css(),
        ruler_bg = a.ruler_bg.css(),
        ruler_fg = ruler_fg.css(),
        ruler_fg2 = a.ruler_fg2.css(),
        marker_bg = a.marker_lane_bg.css(),
        marker_text = a.marker_lane_text.css(),
        marker_fill = a.marker.css(),
        marker_edge = a.marker_edge.css(),
        region_bg = a.region_lane_bg.css(),
        region_text = a.region_lane_text.css(),
        region_fill = a.region.css(),
        item_fallback = a.item_bg[0].css(),
        item_edge = a.item_edge.css(),
        item_label = item_label.css(),
        play_cursor = a.play_cursor.css(),
        edit_cursor = a.edit_cursor.css(),
        radius = theme.metrics.radius,
    )
}

/// A track's own colour, or the theme's neutral — never an invented grey.
///
/// REAPER stores a track colour as `0xRRGGBB` and leaves it unset for a
/// track nobody has coloured; `Tokens::neutral_track` is what the theme
/// says those should look like.
pub fn track_color(theme: &Theme, color: Option<u32>) -> String {
    color.map_or_else(
        || theme.tokens.neutral_track.css(),
        |c| format!("#{c:06x}"),
    )
}
