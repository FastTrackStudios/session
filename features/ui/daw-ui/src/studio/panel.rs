//! The track control panel, as components.
//!
//! One row per track: its colour, the folders it sits inside, its
//! number, its name. Laid out at the geometry measured off REAPER, the
//! same numbers the painted panel uses, so the two columns line up row
//! for row against the lanes beside them.
//!
//! # What is here and what is not
//!
//! The row's GROUND — tint, gutter, the folders' bands, the rail, the
//! rules, the number, the name field and the name. All of it rectangles
//! and text, which is what the measurements say to build things out of.
//!
//! Not the controls: the record arm, the volume, the pan, the routing,
//! the FX pill, mute and solo and the meter. Every one of those shows a
//! VALUE that changes while the window is open, and several of them are
//! genuinely round — a ring, an arc, a pointer — which is the one shape
//! a rectangle cannot be talked into. They are a separate layer and a
//! separate problem.
//!
//! # Density
//!
//! A row is whatever height the project says, and what fits changes with
//! it. Below [`BAND_BELOW`] there is nothing to draw but the band: a
//! glyph is not legible and a control is not hittable, and a session
//! zoomed out to fit is being read for its colours anyway. That is not a
//! degradation to apologise for — it is what the tier is for, and it is
//! why a two-thousand-track session can be shown at once at all.

use crate::prelude::*;

use super::lanes::{Colors, DIVIDER, FONT, Offsets, Rows, View, ink_on};
use super::{ProjectRef, RowsRef};

/// How wide the panel is.
pub const ROW_W: f64 = 343.0;

/// Where the row's tint ends and REAPER's meter gutter begins.
pub const TINT_W: f64 = 296.0;

/// The one-pixel rule between the left column and the row.
pub const RAIL_W: f64 = 20.0;

/// Where the name field starts, and how wide it is.
pub const NAME_FIELD_X: f64 = 33.0;
pub const NAME_FIELD_W: f64 = 136.0;

/// Where the name itself starts — past the record arm on the field.
pub const NAME_X: f64 = 58.0;

/// How far a folder's children are indented per level.
///
/// REAPER indents the row's CONTENT, not the row, so the tint still
/// reaches the panel's edge and only the field and the number move.
pub const INDENT: f64 = 10.0;

/// Past this, an indent would push the name field into the volume knob.
pub const MAX_INDENT: f64 = 60.0;

/// Below this height a row is a band and nothing else.
pub const BAND_BELOW: f64 = 14.0;

/// The height the control band is authored at.
pub const AUTHORED: f64 = 24.0;

/// Where the control band sits in a full-height row.
pub const ROW_ONE: f64 = 6.0;

/// And the height above which a row shows everything REAPER's does.
pub const FULL_ABOVE: f64 = 58.0;

/// What fits in a row this tall.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Density {
    /// Everything REAPER's row has.
    Full,
    /// The control row flattened into whatever height there is.
    ///
    /// Controls are SQUASHED here, not dropped. A track at fourteen
    /// pixels still has everything a track at seventy has, just flatter,
    /// and every one stays in the same column: a panel where controls
    /// appear and vanish as tracks resize cannot be read down, and
    /// hitting one would depend on how tall its track happened to be.
    Compact,
    /// A coloured band.
    Bar,
}

impl Density {
    /// What fits in `height`.
    #[must_use]
    pub fn at(height: f64) -> Self {
        if height >= FULL_ABOVE {
            Self::Full
        } else if height >= BAND_BELOW {
            Self::Compact
        } else {
            Self::Bar
        }
    }
}

/// How big a name is written at this row height.
///
/// The type shrinks with the row rather than being squashed with it: a
/// flattened glyph is unreadable where a smaller one is merely small.
#[must_use]
pub fn name_size(height: f64) -> f64 {
    (height * 0.48).clamp(6.5, 11.5)
}

/// A track's own colour, mixed into the panel's grey.
///
/// REAPER tints the whole row rather than showing a colour chip, which
/// is what makes a session readable by section at a glance. The strength
/// is the theme's, not a number chosen here.
#[must_use]
pub fn row_tint(colors: &Colors, track: &daw_proto::Track) -> String {
    track.color.map_or_else(
        || colors.tcp_tint.clone(),
        |rgb| mix(&colors.tcp_tint_rgb, rgb, colors.track_tint),
    )
}

/// The colour a folder writes down the left edge of its children.
///
/// The track's own colour, not [`row_tint`]'s. That one mixes a few per
/// cent of the colour into the panel's grey — right for a strip body,
/// where the colour is a hint behind controls you are reading — and
/// hopeless for a ten-pixel band whose ENTIRE job is to be identifiable
/// at a glance across half a screen.
///
/// Still short of the raw colour: pulled toward the panel so a column of
/// bands reads as part of the panel rather than as a stripe of paint
/// down it.
#[must_use]
pub fn folder_band(colors: &Colors, track: &daw_proto::Track) -> String {
    /// How far toward the track's own colour the band goes.
    const STRENGTH: f32 = 0.62;
    track.color.map_or_else(
        || colors.tcp_gutter.clone(),
        |rgb| mix(&colors.tcp_tint_rgb, rgb, STRENGTH),
    )
}

/// Blend a packed `0xRRGGBB` into a base colour.
fn mix(base: &(u8, u8, u8), rgb: u32, t: f32) -> String {
    let t = t.clamp(0.0, 1.0);
    let at = |shift: u32| u8::try_from((rgb >> shift) & 0xff).unwrap_or(0);
    let blend = |a: u8, b: u8| {
        let (a, b) = (f32::from(a), f32::from(b));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions,
            reason = "a blend of two channels, which stays inside 0..255"
        )]
        let out = (b - a).mul_add(t, a).clamp(0.0, 255.0) as u8;
        out
    };
    format!(
        "rgb({}, {}, {})",
        blend(base.0, at(16)),
        blend(base.1, at(8)),
        blend(base.2, at(0))
    )
}

/// The panel: a row per visible track.
#[component]
pub fn Panel(
    project: ProjectRef,
    rows: RowsRef,
    view: View,
    colors: Colors,
    #[props(default)] sizing: Rows,
) -> Element {
    let _ = project;
    let offsets = use_memo({
        let rows = rows.clone();
        move || Offsets::of(&rows, sizing)
    });
    let offsets = offsets();
    let visible = offsets.visible(view);

    // The colour of the folder open at each depth, so a row can draw the
    // folders it sits inside down its own left edge. Walked from the top
    // of the visible range rather than from the top of the session: a
    // folder's lineage is whatever is still open above it, and the rows
    // above the screen are what say so.
    let mut lineage: Vec<String> = Vec::new();
    for (track, depth) in rows.iter().take(visible.start) {
        lineage.truncate(usize::try_from(*depth).unwrap_or(0));
        lineage.push(folder_band(&colors, track));
    }

    rsx! {
        div {
            style: "position:relative; width:{ROW_W}px; height:{view.height}px; \
                    overflow:hidden; background:{colors.tcp_gutter}; font-family:{FONT};",
            "data-testid": "studio-panel",
            // The panel's own right edge — the boundary with the arrange
            // view. One rule down the whole column rather than a
            // fragment of one per row: it is the PANEL's edge, and every
            // row was drawing the same pixel.
            div {
                style: "position:absolute; left:{ROW_W - 2.0}px; top:0; width:1px; \
                        bottom:0; background:{colors.rule}; z-index:1;",
            }
            for row in visible {
                if let (Some((top, height)), Some((track, depth))) =
                    (offsets.row(row), rows.get(row))
                {
                    {
                        let level = usize::try_from(*depth).unwrap_or(0);
                        lineage.truncate(level);
                        let ancestors = lineage.clone();
                        lineage.push(folder_band(&colors, track));
                        rsx! {
                            Row {
                                key: "{track.guid}",
                                track: track.clone(),
                                depth: level,
                                ancestors,
                                top: top.mul_add(view.zoom_y, -view.scroll_y),
                                height: height * view.zoom_y,
                                colors: colors.clone(),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One row of the panel.
#[component]
fn Row(
    track: daw_proto::Track,
    depth: usize,
    ancestors: Vec<String>,
    top: f64,
    height: f64,
    colors: Colors,
) -> Element {
    let body = (height - DIVIDER).max(0.5);
    // The tier is decided by the ROW, not by the row less its divider:
    // the divider is inside the height, and a row that is a band at 14
    // has to be a band at 14 in both renderers or the two panels
    // disagree about which tracks show controls.
    let density = Density::at(height);
    let tint = row_tint(&colors, &track);
    let band = folder_band(&colors, &track);
    let indent = (f64::from(u32::try_from(depth).unwrap_or(0)) * INDENT).min(MAX_INDENT);

    // The band tier is the row's tint and its divider and nothing else.
    // Five rectangles a row is nothing at seventy pixels and everything
    // at two, and a session zoomed to fit has every row on screen at
    // once — culling cannot help when nothing is off screen.
    if density == Density::Bar {
        return rsx! {
            div {
                style: "position:absolute; left:0; top:{top}px; width:{TINT_W}px; \
                        height:{height}px; box-sizing:border-box; background:{tint}; \
                        border-bottom:{DIVIDER}px solid {colors.divider};",
            }
        };
    }

    let mark_h = mark_height(&track, body);
    let digits = (track.index + 1).to_string().len();
    // The name sits on the field's middle, which CSS says with a line
    // box the height of the field.
    let field_h = if density == Density::Full {
        AUTHORED
    } else {
        // The band is the row less a pixel top and bottom, so the
        // controls are not flush against the dividers.
        (height - 2.0).max(1.0)
    };
    let name_size = name_size(field_h);
    let shows_number = body - mark_h >= 11.0;
    // A rail with nothing above the number can carry it itself.
    let numbered_here = shows_number && !track.is_folder;
    let numbered = if numbered_here {
        format!(
            "line-height:{body}px; text-align:center; font-size:{}px; color:{};",
            number_size(digits),
            ink_on_band(&band)
        )
    } else {
        String::new()
    };
    let field_top = if density == Density::Full {
        ROW_ONE
    } else {
        1.0
    };
    let name_w = (NAME_FIELD_X + NAME_FIELD_W - NAME_X - indent).max(0.0);

    rsx! {
        div {
            style: "position:absolute; left:0; top:{top}px; width:{ROW_W}px; \
                    height:{height}px; box-sizing:border-box; background:{tint}; \
                    border-bottom:{DIVIDER}px solid {colors.divider};",
            "data-track": "{track.guid}",

            // The meter gutter, over the tint's right end: one row with a
            // gutter at the end of it, not two panels side by side.
            div {
                style: "position:absolute; left:{TINT_W}px; top:0; right:0; bottom:0; \
                        background:{colors.tcp_gutter};",
            }

            // The left column, and the folders this row sits inside.
            // Each ancestor paints one step of the indent in its own
            // colour, and because every child of a folder paints it at
            // the same x the steps join top to bottom into one unbroken
            // line — the folder's own left edge running down past
            // everything inside it.
            div {
                style: "position:absolute; left:0; top:0; width:{indent + RAIL_W}px; \
                        bottom:0; background:{colors.tcp_column};",
            }
            for (level, tint) in ancestors.iter().enumerate() {
                {
                    let left = f64::from(u32::try_from(level).unwrap_or(0)) * INDENT;
                    if left >= MAX_INDENT {
                        return rsx! {};
                    }
                    let wide = (left + INDENT).min(MAX_INDENT) - left;
                    rsx! {
                        div {
                            key: "{level}",
                            style: "position:absolute; left:{left}px; top:0; \
                                    width:{wide}px; bottom:0; background:{tint};",
                        }
                    }
                }
            }
            // The rail carries the row's OWN colour, at the same
            // strength — so the whole left edge is the track and its
            // lineage, unbroken from the panel's edge to the row's
            // content, and a folder's stripe starts on the folder's own
            // row rather than on its first child.
            div {
                style: "position:absolute; left:{indent}px; top:0; width:{RAIL_W}px; \
                        bottom:0; background:{band}; \
                        border-right:1px solid {colors.rule}; \
                        box-sizing:border-box; {numbered}",
                // On a row with no folder mark the rail IS the number's
                // box, so the number is its text. A folder's rail has
                // the mark at its top and the number under it, which
                // needs a box of its own — see below.
                if numbered_here {
                    "{track.index + 1}"
                }
            }

            // The folder mark, at the TOP of the rail.
            //
            // REAPER puts it at the bottom; at the top it lines up with
            // the controls beside it, which is what makes a folder
            // readable while scanning a collapsed session rather than
            // something you find by looking down.
            //
            // Two rectangles, because that is what the mark is: a body
            // and a tab. The art draws it as one six-point polygon and
            // the polygon has no curve in it, so this is the same shape
            // rather than an impression of it.
            if track.is_folder {
                {
                    let scale = mark_scale(body);
                    let left = indent + (RAIL_W - MARK_W * scale) / 2.0;
                    let top = 2.0 * scale;
                    rsx! {
                        div {
                            style: "position:absolute; left:{left}px; top:{top}px; \
                                    width:{TAB_W * scale}px; height:{TAB_H * scale}px; \
                                    background:{colors.text_dim};",
                        }
                        div {
                            style: "position:absolute; left:{left}px; \
                                    top:{top + TAB_H * scale}px; \
                                    width:{MARK_W * scale}px; \
                                    height:{(MARK_H - TAB_H) * scale}px; \
                                    background:{colors.text_dim};",
                        }
                    }
                }
            }

            // The track number, under whatever the rail already holds.
            // It gets the space that is left, and gives way entirely
            // when a folder's mark has taken the rail — the mark is the
            // fact worth keeping when only one of the two fits.
            if shows_number && !numbered_here {
                div {
                    style: "position:absolute; left:{indent}px; top:{mark_h}px; \
                            width:{RAIL_W}px; height:{body - mark_h}px; \
                            line-height:{body - mark_h}px; text-align:center; \
                            font-size:{number_size(digits)}px; \
                            color:{ink_on_band(&band)}; overflow:hidden;",
                    "{track.index + 1}"
                }
            }

            // The name field, with the name ON it: a pill on its left
            // end and square on its right, because the record arm sits
            // on the field rather than beside it — and the name is the
            // field's own text, padded past where the arm goes, rather
            // than a second box laid over the first.
            //
            // Clipped rather than wrapped or shrunk; REAPER truncates
            // here too.
            div {
                style: "position:absolute; left:{NAME_FIELD_X + indent}px; \
                        top:{field_top}px; width:{(NAME_FIELD_W - indent).max(0.0)}px; \
                        height:{field_h}px; box-sizing:border-box; \
                        padding-left:{NAME_X - NAME_FIELD_X}px; \
                        background:{colors.tcp_field}; \
                        border-radius:{field_h / 2.0}px 0 0 {field_h / 2.0}px; \
                        line-height:{field_h}px; font-size:{name_size}px; \
                        color:{name_ink(&colors, &track)}; white-space:nowrap; \
                        overflow:hidden;",
                "{track.name}"
            }

        }
    }
}

/// How big the track number is written.
///
/// Sized to the RAIL'S WIDTH, not to the row's height: the rail is what
/// it has to fit inside, and a two-digit number at a fixed size ran past
/// the rule closing it — one track's number crossing into the body of
/// its own row. A three-digit session is not unusual and this is where
/// it shows.
///
/// The advance is the face's own digit width. Digits are tabular in
/// DejaVu, so one number fits them all, which is what lets this be
/// arithmetic instead of a text measurement the component cannot do.
#[must_use]
pub fn number_size(digits: usize) -> f64 {
    /// How wide a digit is, as a fraction of the type size.
    const ADVANCE: f64 = 0.636;
    /// The room the rail gives, less the rule closing it.
    const ROOM: f64 = RAIL_W - 3.0;
    let digits = f64::from(u32::try_from(digits.max(1)).unwrap_or(1));
    (ROOM / (digits * ADVANCE)).clamp(6.0, 11.0)
}

/// How tall the folder mark is, including the gap under it.
///
/// Nought for a track that is not a folder — which is the whole of the
/// rail then going to the number.
#[must_use]
pub fn mark_height(track: &daw_proto::Track, body: f64) -> f64 {
    if track.is_folder {
        (MARK_H + 2.0) * mark_scale(body)
    } else {
        0.0
    }
}

/// How much the mark is scaled down on a short row.
fn mark_scale(body: f64) -> f64 {
    (body / 18.0).clamp(0.4, 1.0)
}

/// The folder mark's authored size.
const MARK_W: f64 = 9.0;
const MARK_H: f64 = 7.0;
/// And its tab, which is the part that makes it a folder rather than a
/// box: four across and two down, out of nine by seven.
const TAB_W: f64 = 4.0;
const TAB_H: f64 = 2.0;

/// Ink that reads on a folder band.
fn ink_on_band(band: &str) -> String {
    // The band is `rgb(r, g, b)` because `mix` made it, or a token
    // colour otherwise — either way the luminance decides.
    band.trim_start_matches("rgb(")
        .trim_start_matches("rgba(")
        .trim_end_matches(')')
        .split(',')
        .take(3)
        .map(|part| part.trim().parse::<f32>().unwrap_or(0.0))
        .collect::<Vec<_>>()
        .split_first()
        .map_or_else(
            || "rgba(232, 232, 234, 1.000)".to_owned(),
            |(r, rest)| {
                let (g, b) = (
                    rest.first().copied().unwrap_or(0.0),
                    rest.get(1).copied().unwrap_or(0.0),
                );
                ink_on(crate::theming::Color {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::as_conversions,
                        reason = "a channel that was parsed out of a channel"
                    )]
                    r: r.clamp(0.0, 255.0) as u8,
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::as_conversions,
                        reason = "a channel that was parsed out of a channel"
                    )]
                    g: g.clamp(0.0, 255.0) as u8,
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::as_conversions,
                        reason = "a channel that was parsed out of a channel"
                    )]
                    b: b.clamp(0.0, 255.0) as u8,
                    a: 255,
                })
            },
        )
}

/// A selected track's name is brighter than the rest.
fn name_ink(colors: &Colors, track: &daw_proto::Track) -> String {
    if track.selected {
        colors.text.clone()
    } else {
        colors.text_dim.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{BAND_BELOW, Density, FULL_ABOVE, MARK_H, RAIL_W, name_size, number_size};

    /// What fits in a row is decided by its height and nothing else.
    #[test]
    fn a_row_shows_what_fits_in_it() {
        assert_eq!(Density::at(70.0), Density::Full);
        assert_eq!(Density::at(FULL_ABOVE), Density::Full);
        assert_eq!(Density::at(FULL_ABOVE - 0.1), Density::Compact);
        assert_eq!(Density::at(32.0), Density::Compact);
        assert_eq!(Density::at(BAND_BELOW), Density::Compact);
        assert_eq!(Density::at(BAND_BELOW - 0.1), Density::Bar);
        assert_eq!(Density::at(1.0), Density::Bar);
    }

    /// The type shrinks with the row, between a floor and a ceiling —
    /// it is never squashed and never grows past what the field holds.
    #[test]
    fn a_name_shrinks_with_its_row_but_only_so_far() {
        assert!(
            (name_size(24.0) - 11.5).abs() < 1e-9,
            "it grew past the field"
        );
        assert!((name_size(200.0) - 11.5).abs() < 1e-9);
        assert!(
            (name_size(1.0) - 6.5).abs() < 1e-9,
            "it shrank past legible"
        );
        assert!(
            name_size(20.0) < name_size(24.0),
            "it did not shrink at all"
        );
    }

    /// The number is sized to the rail, so a long one fits inside it.
    ///
    /// The case that matters is three digits: a session with two hundred
    /// tracks has them, and at a fixed size the third one crossed the
    /// rule closing the rail.
    #[test]
    fn a_number_shrinks_to_fit_its_rail() {
        const ADVANCE: f64 = 0.636;
        for digits in 1..=4 {
            let size = number_size(digits);
            let width = f64::from(u32::try_from(digits).unwrap()) * ADVANCE * size;
            assert!(
                width <= RAIL_W - 3.0 + 1e-9 || size <= 6.0,
                "{digits} digits at {size}px is {width} wide in a {RAIL_W} rail"
            );
        }
        assert!((number_size(1) - 11.0).abs() < 1e-9, "one digit was shrunk");
        assert!(number_size(3) < 11.0, "three digits were not shrunk");
        assert!(number_size(9) >= 6.0, "it shrank past legible");
    }

    /// The mark scales with the row and takes the rail's top.
    #[test]
    fn a_folder_mark_shrinks_with_its_row() {
        let tall = super::mark_scale(70.0);
        let short = super::mark_scale(14.0);
        assert!((tall - 1.0).abs() < 1e-9, "it grew past its authored size");
        assert!(
            short < tall && short >= 0.4,
            "it shrank past visible: {short}"
        );
        assert!(MARK_H > 0.0);
    }
}
