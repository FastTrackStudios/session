//! Note layout implementation.
//!
//! Handles layout of individual noteheads, including position on staff,
//! accidentals, dots, and ledger lines.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;

/// SMuFL codepoints for noteheads.
pub mod glyphs {
    /// Whole note (semibreve)
    pub const NOTEHEAD_WHOLE: char = '\u{E0A2}';
    /// Half note (minim)
    pub const NOTEHEAD_HALF: char = '\u{E0A3}';
    /// Quarter note and shorter (crotchet)
    pub const NOTEHEAD_BLACK: char = '\u{E0A4}';
    /// Double whole note (breve)
    pub const NOTEHEAD_DOUBLE_WHOLE: char = '\u{E0A0}';

    // Slash noteheads (rhythmic notation)
    /// Slash notehead with vertical ends (filled, for quarter notes and shorter)
    pub const NOTEHEAD_SLASH_VERTICAL: char = '\u{E100}';
    /// Slash notehead with horizontal ends (filled, alternative)
    pub const NOTEHEAD_SLASH_HORIZONTAL: char = '\u{E101}';
    /// White slash whole note
    pub const NOTEHEAD_SLASH_WHITE_WHOLE: char = '\u{E102}';
    /// White slash half note
    pub const NOTEHEAD_SLASH_WHITE_HALF: char = '\u{E103}';
    /// White slash double whole note
    pub const NOTEHEAD_SLASH_WHITE_DOUBLE_WHOLE: char = '\u{E10A}';

    // Diamond noteheads (harmonics, also used for rhythmic notation half/whole)
    /// White diamond whole note
    pub const NOTEHEAD_DIAMOND_WHOLE: char = '\u{E0DB}';
    /// White diamond half note
    pub const NOTEHEAD_DIAMOND_HALF: char = '\u{E0DC}';
    /// Black diamond (filled, for quarter notes and shorter)
    pub const NOTEHEAD_DIAMOND_BLACK: char = '\u{E0DD}';
    /// Large diamond (wide variant)
    pub const NOTEHEAD_DIAMOND_WIDE: char = '\u{E0DE}';
    /// Slash diamond white (large white diamond for rhythmic notation)
    /// Base glyph - use ss08 stylistic set for oversized variant
    pub const NOTEHEAD_SLASH_DIAMOND_WHITE: char = '\u{E104}';

    // Accidentals
    /// Sharp
    pub const ACCIDENTAL_SHARP: char = '\u{E262}';
    /// Flat
    pub const ACCIDENTAL_FLAT: char = '\u{E260}';
    /// Natural
    pub const ACCIDENTAL_NATURAL: char = '\u{E261}';
    /// Double sharp
    pub const ACCIDENTAL_DOUBLE_SHARP: char = '\u{E263}';
    /// Double flat
    pub const ACCIDENTAL_DOUBLE_FLAT: char = '\u{E264}';

    // Augmentation dots
    /// Augmentation dot
    pub const AUGMENTATION_DOT: char = '\u{E1E7}';
}

/// Notehead type/style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoteHeadType {
    /// Standard notehead (black, half, whole)
    #[default]
    Normal,
    /// Slash notehead for rhythmic notation (jazz charts, lead sheets)
    Slash,
    /// X notehead (percussion, ghost notes)
    X,
    /// Diamond notehead (harmonics)
    Diamond,
    /// Triangle notehead
    Triangle,
}

/// Glyph style for long-duration slash noteheads (half / whole / double-whole).
///
/// Quarter-and-shorter slashes always use the filled horizontal slash glyph;
/// only longer notes have a real choice between two engraving conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlashLongStyle {
    /// Large white diamond (U+E104). Common in jazz lead sheets / Berklee style.
    #[default]
    Diamond,
    /// White slash glyphs (U+E102 / U+E103 / U+E10A).
    /// Closer to the filled-slash short-duration look.
    WhiteSlash,
}

impl NoteHeadType {
    /// Get the SMuFL glyph for this notehead type and duration, using the
    /// default `SlashLongStyle` for slash heads.
    #[must_use]
    pub const fn glyph(&self, duration: NoteDuration) -> char {
        self.glyph_with_slash_style(duration, SlashLongStyle::Diamond)
    }

    /// Get the SMuFL glyph for this notehead type and duration, with a chosen
    /// `SlashLongStyle` for half / whole / double-whole slash heads.
    #[must_use]
    pub const fn glyph_with_slash_style(
        &self,
        duration: NoteDuration,
        slash_long_style: SlashLongStyle,
    ) -> char {
        match self {
            Self::Normal => duration.notehead_glyph(),
            Self::Slash => match duration {
                NoteDuration::DoubleWhole => match slash_long_style {
                    SlashLongStyle::Diamond => glyphs::NOTEHEAD_SLASH_DIAMOND_WHITE,
                    SlashLongStyle::WhiteSlash => glyphs::NOTEHEAD_SLASH_WHITE_DOUBLE_WHOLE,
                },
                NoteDuration::Whole => match slash_long_style {
                    SlashLongStyle::Diamond => glyphs::NOTEHEAD_SLASH_DIAMOND_WHITE,
                    SlashLongStyle::WhiteSlash => glyphs::NOTEHEAD_SLASH_WHITE_WHOLE,
                },
                NoteDuration::Half => match slash_long_style {
                    SlashLongStyle::Diamond => glyphs::NOTEHEAD_SLASH_DIAMOND_WHITE,
                    SlashLongStyle::WhiteSlash => glyphs::NOTEHEAD_SLASH_WHITE_HALF,
                },
                // Quarter and shorter always use filled slash regardless of style.
                _ => glyphs::NOTEHEAD_SLASH_HORIZONTAL,
            },
            // Fallback to normal noteheads for unimplemented types
            Self::X | Self::Diamond | Self::Triangle => duration.notehead_glyph(),
        }
    }

    /// Get the width of this notehead type in spatiums.
    #[must_use]
    pub const fn width(&self) -> f64 {
        match self {
            Self::Normal => 1.18,
            Self::Slash => 1.5, // Slash noteheads are wider
            Self::X => 1.2,
            Self::Diamond => 1.3,
            Self::Triangle => 1.2,
        }
    }
}

/// Accidental type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accidental {
    None,
    Sharp,
    Flat,
    Natural,
    DoubleSharp,
    DoubleFlat,
}

impl Accidental {
    /// Get the SMuFL glyph for this accidental.
    #[must_use]
    pub const fn glyph(&self) -> Option<char> {
        match self {
            Self::None => None,
            Self::Sharp => Some(glyphs::ACCIDENTAL_SHARP),
            Self::Flat => Some(glyphs::ACCIDENTAL_FLAT),
            Self::Natural => Some(glyphs::ACCIDENTAL_NATURAL),
            Self::DoubleSharp => Some(glyphs::ACCIDENTAL_DOUBLE_SHARP),
            Self::DoubleFlat => Some(glyphs::ACCIDENTAL_DOUBLE_FLAT),
        }
    }

    /// Get the width of this accidental in spatiums.
    #[must_use]
    pub const fn width(&self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Sharp => 1.2,
            Self::Flat => 0.9,
            Self::Natural => 0.7,
            Self::DoubleSharp => 1.0,
            Self::DoubleFlat => 1.4,
        }
    }
}

/// Duration type for notehead selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteDuration {
    DoubleWhole,
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
}

impl NoteDuration {
    /// Get the SMuFL glyph for this duration's notehead.
    #[must_use]
    pub const fn notehead_glyph(&self) -> char {
        match self {
            Self::DoubleWhole => glyphs::NOTEHEAD_DOUBLE_WHOLE,
            Self::Whole => glyphs::NOTEHEAD_WHOLE,
            Self::Half => glyphs::NOTEHEAD_HALF,
            _ => glyphs::NOTEHEAD_BLACK,
        }
    }

    /// Check if this duration requires a stem.
    #[must_use]
    pub const fn has_stem(&self) -> bool {
        !matches!(self, Self::DoubleWhole | Self::Whole)
    }

    /// Get the number of flags (for eighth notes and shorter).
    #[must_use]
    pub const fn flag_count(&self) -> u8 {
        match self {
            Self::Eighth => 1,
            Self::Sixteenth => 2,
            Self::ThirtySecond => 3,
            Self::SixtyFourth => 4,
            _ => 0,
        }
    }
}

/// Note layout parameters.
#[derive(Debug, Clone)]
pub struct NoteParams {
    /// Unique identifier for this note
    pub id: u64,
    /// Duration type
    pub duration: NoteDuration,
    /// Notehead type/style
    pub head_type: NoteHeadType,
    /// Staff line position (0 = middle line, positive = up)
    pub line: i32,
    /// Accidental to display
    pub accidental: Accidental,
    /// Number of augmentation dots
    pub dots: u8,
    /// Whether this note is part of a chord with offset noteheads
    pub offset_x: f64,
    /// Whether to draw ledger lines
    pub ledger_lines: bool,
    /// Width (in spatiums) reserved for the chord's shared accidental column.
    ///
    /// When `Some(w)`, the accidental is right-aligned within a column of width `w`
    /// (so all noteheads in the chord land at the same X regardless of accidental
    /// width). When `None`, the accidental is drawn flush at x = 0 (single-note
    /// behavior).
    pub accidental_column_width: Option<f64>,
}

impl Default for NoteParams {
    fn default() -> Self {
        Self {
            id: 0,
            duration: NoteDuration::Quarter,
            head_type: NoteHeadType::Normal,
            line: 0,
            accidental: Accidental::None,
            dots: 0,
            offset_x: 0.0,
            ledger_lines: true,
            accidental_column_width: None,
        }
    }
}

/// Layout a single note.
///
/// # Returns
/// Tuple of (LayoutData, SceneNode) containing position/shape and visual representation.
#[must_use]
pub fn layout_note(params: &NoteParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let staff_line_distance = spatium; // Distance between staff lines

    // Calculate Y position from staff line
    // Line 0 = middle line (B4 in treble clef)
    // Positive lines go up, negative go down
    let y = -params.line as f64 * staff_line_distance / 2.0;

    // Reserved chord-wide accidental column width (in pixels), if any.
    // The notehead is the rhythmic anchor and remains at x = 0; accidentals
    // are placed to the left so noteheads, ledger lines, stems, and chord
    // symbols can share the same beat x.
    let column_px = params
        .accidental_column_width
        .map(|w| w * spatium)
        .unwrap_or(0.0);

    let mut commands = Vec::new();
    let mut left_extent: f64 = 0.0;
    let notehead_width = spatium * params.head_type.width();
    let mut right_extent = notehead_width;

    // Draw accidental if present
    if let Some(acc_glyph) = params.accidental.glyph() {
        let acc_width = params.accidental.width() * spatium;
        let gap = spatium * 0.15;
        let column_width = column_px.max(acc_width);
        let acc_x = -gap - column_width;
        left_extent = left_extent.max(-acc_x);

        commands.push(PaintCommand::glyph(
            acc_glyph,
            Point::new(acc_x, y),
            spatium,
            Color::BLACK,
        ));
    }

    // Draw notehead
    let notehead_x = params.offset_x;
    let notehead_glyph = params.head_type.glyph(params.duration);

    commands.push(PaintCommand::glyph(
        notehead_glyph,
        Point::new(notehead_x, y),
        spatium,
        Color::BLACK,
    ));

    // Draw ledger lines if needed
    if params.ledger_lines {
        let ledger_commands = draw_ledger_lines(params.line, notehead_x, notehead_width, spatium);
        commands.extend(ledger_commands);
    }

    // Draw augmentation dots
    // MuseScore uses dotNoteDistance = 0.5 spatiums (default)
    // And dotDotDistance = 0.5 spatiums between multiple dots
    const DOT_NOTE_DISTANCE: f64 = 0.5; // spatiums
    const DOT_DOT_DISTANCE: f64 = 0.5; // spatiums between adjacent dots
    const DOT_GLYPH_WIDTH: f64 = 0.35; // approximate width of the dot glyph

    if params.dots > 0 {
        let dot_x = notehead_x + notehead_width + spatium * DOT_NOTE_DISTANCE;
        let dot_y = if params.line % 2 == 0 {
            y - staff_line_distance / 4.0 // Move dot up if on a line
        } else {
            y
        };

        for i in 0..params.dots {
            commands.push(PaintCommand::glyph(
                glyphs::AUGMENTATION_DOT,
                Point::new(dot_x + i as f64 * spatium * DOT_DOT_DISTANCE, dot_y),
                spatium,
                Color::BLACK,
            ));
        }

        // Width calculation:
        // - DOT_NOTE_DISTANCE: gap from notehead to first dot
        // - (params.dots - 1) * DOT_DOT_DISTANCE: gaps between adjacent dots
        // - DOT_GLYPH_WIDTH: width of the last dot's glyph
        let dots_spacing = if params.dots > 1 {
            (params.dots - 1) as f64 * DOT_DOT_DISTANCE
        } else {
            0.0
        };
        right_extent += spatium * (DOT_NOTE_DISTANCE + dots_spacing + DOT_GLYPH_WIDTH);
    }

    // Calculate bounding box (relative to note position)
    let half_height = spatium * 0.5;
    let bbox = Rect::new(-left_extent, -half_height, right_extent, half_height);

    // Create shape for collision detection (in world coordinates)
    let world_bbox = Rect::new(-left_extent, y - half_height, right_extent, y + half_height);
    let shape = Shape::from_rect(world_bbox);

    // Create layout data with proper position
    let layout = LayoutData::new(Point::new(0.0, y), bbox, shape);

    // Create scene node with semantic ID
    let semantic_id = SemanticId::new(ElementType::Note, params.id);
    let node =
        SceneNode::leaf(semantic_id, commands).with_metadata("pitch-line", params.line.to_string());

    (layout, node)
}

/// Draw ledger lines for notes outside the staff.
fn draw_ledger_lines(
    line: i32,
    notehead_x: f64,
    notehead_width: f64,
    spatium: f64,
) -> Vec<PaintCommand> {
    let mut commands = Vec::new();
    let line_extension = spatium * 0.4; // How far ledger line extends past notehead
    let line_width = spatium * 0.16;
    let staff_line_distance = spatium;

    // Ledger lines above staff (line > 5, i.e., above top line)
    if line > 5 {
        let mut l = 6;
        while l <= line {
            if l % 2 == 0 {
                // Only draw on even lines (actual lines, not spaces)
                let ledger_y = -l as f64 * staff_line_distance / 2.0;
                commands.push(PaintCommand::line(
                    Point::new(notehead_x - line_extension, ledger_y),
                    Point::new(notehead_x + notehead_width + line_extension, ledger_y),
                    Color::BLACK,
                    line_width,
                ));
            }
            l += 1;
        }
    }

    // Ledger lines below staff (line < -5, i.e., below bottom line)
    if line < -5 {
        let mut l = -6;
        while l >= line {
            if l % 2 == 0 {
                let ledger_y = -l as f64 * staff_line_distance / 2.0;
                commands.push(PaintCommand::line(
                    Point::new(notehead_x - line_extension, ledger_y),
                    Point::new(notehead_x + notehead_width + line_extension, ledger_y),
                    Color::BLACK,
                    line_width,
                ));
            }
            l -= 1;
        }
    }

    commands
}

/// Calculate the shape for a note (for collision detection).
#[must_use]
pub fn note_shape(params: &NoteParams, ctx: &LayoutContext) -> Shape {
    let spatium = ctx.spatium();
    let y = -params.line as f64 * spatium / 2.0;
    let half_height = spatium * 0.5;

    let mut width = spatium * params.head_type.width(); // Notehead width based on type
    if params.accidental != Accidental::None {
        width += params.accidental.width() * spatium + spatium * 0.15;
    }
    if params.dots > 0 {
        // Match MuseScore's dotNoteDistance (0.5) + dotDotDistance (0.5)
        width += spatium * 0.5 + params.dots as f64 * spatium * 0.5;
    }

    Shape::from_rect(Rect::new(0.0, y - half_height, width, y + half_height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::layout::context::LayoutConfiguration;
    use crate::engraver::style::MStyle;

    fn test_ctx() -> LayoutContext<'static> {
        let config = LayoutConfiguration::default();
        let style = Box::leak(Box::new(MStyle::default()));
        LayoutContext::new_for_test(config, style)
    }

    #[test]
    fn test_layout_simple_note() {
        let ctx = test_ctx();
        let params = NoteParams {
            id: 1,
            duration: NoteDuration::Quarter,
            line: 0,
            ..Default::default()
        };

        let (layout, node) = layout_note(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
        assert!(!node.commands.is_empty());
    }

    #[test]
    fn test_layout_note_with_accidental() {
        let ctx = test_ctx();
        let params = NoteParams {
            id: 2,
            duration: NoteDuration::Quarter,
            line: 0,
            accidental: Accidental::Sharp,
            ..Default::default()
        };

        let (layout, node) = layout_note(&params, &ctx);

        // Should have at least 2 commands (accidental + notehead)
        assert!(node.commands.len() >= 2);
        // Bounding box should be wider with accidental
        assert!(layout.bbox.width() > ctx.spatium());
    }

    #[test]
    fn test_slash_long_style_diamond_vs_white_slash() {
        // Half-duration slash: diamond (default) vs white-slash glyph
        let diamond =
            NoteHeadType::Slash.glyph_with_slash_style(NoteDuration::Half, SlashLongStyle::Diamond);
        let white = NoteHeadType::Slash
            .glyph_with_slash_style(NoteDuration::Half, SlashLongStyle::WhiteSlash);
        assert_eq!(diamond, glyphs::NOTEHEAD_SLASH_DIAMOND_WHITE);
        assert_eq!(white, glyphs::NOTEHEAD_SLASH_WHITE_HALF);
        assert_ne!(diamond, white);

        // Quarter-duration: style doesn't matter, always filled slash
        let q_d = NoteHeadType::Slash
            .glyph_with_slash_style(NoteDuration::Quarter, SlashLongStyle::Diamond);
        let q_w = NoteHeadType::Slash
            .glyph_with_slash_style(NoteDuration::Quarter, SlashLongStyle::WhiteSlash);
        assert_eq!(q_d, glyphs::NOTEHEAD_SLASH_HORIZONTAL);
        assert_eq!(q_w, glyphs::NOTEHEAD_SLASH_HORIZONTAL);

        // Default `glyph()` preserves the legacy diamond behavior
        let default_half = NoteHeadType::Slash.glyph(NoteDuration::Half);
        assert_eq!(default_half, diamond);
    }

    #[test]
    fn test_accidental_column_aligns_noteheads() {
        // When two notes share an accidental column, the notehead Xs must match
        // regardless of whether each note carries an accidental or not.
        let ctx = test_ctx();
        let column = Accidental::Sharp.width(); // wider than no-accidental case

        let with_acc = layout_note(
            &NoteParams {
                id: 100,
                duration: NoteDuration::Quarter,
                line: 0,
                accidental: Accidental::Sharp,
                accidental_column_width: Some(column),
                ..Default::default()
            },
            &ctx,
        );
        let without_acc = layout_note(
            &NoteParams {
                id: 101,
                duration: NoteDuration::Quarter,
                line: 2,
                accidental: Accidental::None,
                accidental_column_width: Some(column),
                ..Default::default()
            },
            &ctx,
        );

        // The last command on each is the notehead glyph (dots/ledgers may follow
        // but neither test note has them). Find the notehead by glyph kind.
        fn notehead_x(node: &SceneNode) -> f64 {
            for cmd in &node.commands {
                if let PaintCommand::Glyph {
                    position,
                    codepoint,
                    ..
                } = cmd
                    && matches!(
                        *codepoint,
                        glyphs::NOTEHEAD_BLACK | glyphs::NOTEHEAD_HALF | glyphs::NOTEHEAD_WHOLE
                    )
                {
                    return position.x;
                }
            }
            panic!("no notehead glyph found in node");
        }

        let x_with = notehead_x(&with_acc.1);
        let x_without = notehead_x(&without_acc.1);
        assert!((x_with - 0.0).abs() < 1e-6);
        assert!(
            (x_with - x_without).abs() < 1e-6,
            "notehead Xs should match across accidental column: with={x_with}, without={x_without}"
        );
    }

    #[test]
    fn test_ledger_lines_center_on_notehead_anchor_with_accidental() {
        let ctx = test_ctx();
        let (_, node) = layout_note(
            &NoteParams {
                id: 102,
                duration: NoteDuration::Quarter,
                line: 6,
                accidental: Accidental::Sharp,
                ledger_lines: true,
                ..Default::default()
            },
            &ctx,
        );

        let mut notehead_x = None;
        let mut ledger_bounds = None;
        for cmd in &node.commands {
            match cmd {
                PaintCommand::Glyph {
                    position,
                    codepoint,
                    ..
                } if matches!(
                    *codepoint,
                    glyphs::NOTEHEAD_BLACK | glyphs::NOTEHEAD_HALF | glyphs::NOTEHEAD_WHOLE
                ) =>
                {
                    notehead_x = Some(position.x);
                }
                PaintCommand::Line { start, end, .. } => {
                    ledger_bounds = Some((start.x, end.x));
                }
                _ => {}
            }
        }

        let notehead_x = notehead_x.expect("notehead glyph");
        let (ledger_start, ledger_end) = ledger_bounds.expect("ledger line");
        assert!((notehead_x - 0.0).abs() < 1e-6);
        assert!(
            ledger_start < notehead_x && ledger_end > notehead_x,
            "ledger line must straddle the beat-aligned notehead x: notehead={notehead_x}, ledger=({ledger_start}, {ledger_end})"
        );
    }

    #[test]
    fn test_layout_note_with_dots() {
        let ctx = test_ctx();
        let params = NoteParams {
            id: 3,
            duration: NoteDuration::Half,
            line: 2,
            dots: 2,
            ..Default::default()
        };

        let (_layout, node) = layout_note(&params, &ctx);

        // Should have notehead + 2 dots = 3 commands minimum
        assert!(node.commands.len() >= 3);
    }

    #[test]
    fn test_notehead_glyphs() {
        assert_eq!(NoteDuration::Whole.notehead_glyph(), glyphs::NOTEHEAD_WHOLE);
        assert_eq!(NoteDuration::Half.notehead_glyph(), glyphs::NOTEHEAD_HALF);
        assert_eq!(
            NoteDuration::Quarter.notehead_glyph(),
            glyphs::NOTEHEAD_BLACK
        );
        assert_eq!(
            NoteDuration::Eighth.notehead_glyph(),
            glyphs::NOTEHEAD_BLACK
        );
    }

    #[test]
    fn test_stem_required() {
        assert!(!NoteDuration::Whole.has_stem());
        assert!(!NoteDuration::DoubleWhole.has_stem());
        assert!(NoteDuration::Half.has_stem());
        assert!(NoteDuration::Quarter.has_stem());
    }

    #[test]
    fn test_flag_count() {
        assert_eq!(NoteDuration::Quarter.flag_count(), 0);
        assert_eq!(NoteDuration::Eighth.flag_count(), 1);
        assert_eq!(NoteDuration::Sixteenth.flag_count(), 2);
        assert_eq!(NoteDuration::SixtyFourth.flag_count(), 4);
    }
}
