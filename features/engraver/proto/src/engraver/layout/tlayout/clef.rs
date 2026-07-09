//! Clef layout implementation.
//!
//! Handles layout of clef symbols at the beginning of staves and mid-measure.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;

/// SMuFL codepoints for clefs.
pub mod glyphs {
    /// G clef (treble)
    pub const CLEF_G: char = '\u{E050}';
    /// G clef change (smaller)
    pub const CLEF_G_CHANGE: char = '\u{E07A}';
    /// F clef (bass)
    pub const CLEF_F: char = '\u{E062}';
    /// F clef change (smaller)
    pub const CLEF_F_CHANGE: char = '\u{E07C}';
    /// C clef (alto/tenor)
    pub const CLEF_C: char = '\u{E05C}';
    /// C clef change (smaller)
    pub const CLEF_C_CHANGE: char = '\u{E07B}';
    /// Percussion clef
    pub const CLEF_PERCUSSION: char = '\u{E069}';
    /// Tab clef
    pub const CLEF_TAB: char = '\u{E06D}';
    /// 8va above
    pub const OTTAVA_ALTA: char = '\u{E510}';
    /// 8vb below
    pub const OTTAVA_BASSA: char = '\u{E511}';
}

/// Clef type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClefType {
    /// Treble clef (G clef on line 2)
    Treble,
    /// Bass clef (F clef on line 4)
    Bass,
    /// Alto clef (C clef on line 3)
    Alto,
    /// Tenor clef (C clef on line 4)
    Tenor,
    /// Percussion clef
    Percussion,
    /// Tablature clef
    Tab,
}

impl ClefType {
    /// Get the SMuFL glyph for this clef type.
    #[must_use]
    pub const fn glyph(&self, is_change: bool) -> char {
        match (self, is_change) {
            (Self::Treble, false) => glyphs::CLEF_G,
            (Self::Treble, true) => glyphs::CLEF_G_CHANGE,
            (Self::Bass, false) => glyphs::CLEF_F,
            (Self::Bass, true) => glyphs::CLEF_F_CHANGE,
            (Self::Alto | Self::Tenor, false) => glyphs::CLEF_C,
            (Self::Alto | Self::Tenor, true) => glyphs::CLEF_C_CHANGE,
            (Self::Percussion, _) => glyphs::CLEF_PERCUSSION,
            (Self::Tab, _) => glyphs::CLEF_TAB,
        }
    }

    /// Get the staff line where the clef is positioned.
    /// Line 0 = middle line, positive = up.
    #[must_use]
    pub const fn line(&self) -> i32 {
        match self {
            Self::Treble => -2, // G on second line from bottom
            Self::Bass => 2,    // F on fourth line from bottom
            Self::Alto => 0,    // C on middle line
            Self::Tenor => 2,   // C on fourth line
            Self::Percussion => 0,
            Self::Tab => 0,
        }
    }

    /// Get the width of this clef in spatiums.
    #[must_use]
    pub const fn width(&self) -> f64 {
        match self {
            Self::Treble => 2.7,
            Self::Bass => 2.4,
            Self::Alto | Self::Tenor => 2.2,
            Self::Percussion => 1.8,
            Self::Tab => 2.0,
        }
    }

    /// Get the height of this clef in spatiums.
    #[must_use]
    pub const fn height(&self) -> f64 {
        match self {
            Self::Treble => 6.5,
            Self::Bass => 3.5,
            Self::Alto | Self::Tenor => 4.0,
            Self::Percussion => 2.0,
            Self::Tab => 4.0,
        }
    }
}

/// Octave transposition for clefs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClefOctave {
    /// No transposition
    #[default]
    None,
    /// One octave up (8va)
    OctaveUp,
    /// One octave down (8vb)
    OctaveDown,
    /// Two octaves up (15ma)
    TwoOctavesUp,
    /// Two octaves down (15mb)
    TwoOctavesDown,
}

/// Clef layout parameters.
#[derive(Debug, Clone)]
pub struct ClefParams {
    /// Unique identifier
    pub id: u64,
    /// Clef type
    pub clef_type: ClefType,
    /// Octave transposition
    pub octave: ClefOctave,
    /// Whether this is a mid-measure clef change (smaller size)
    pub is_change: bool,
}

impl Default for ClefParams {
    fn default() -> Self {
        Self {
            id: 0,
            clef_type: ClefType::Treble,
            octave: ClefOctave::None,
            is_change: false,
        }
    }
}

/// Layout a clef symbol.
#[must_use]
pub fn layout_clef(params: &ClefParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let scale = if params.is_change { 0.75 } else { 1.0 };

    let clef_width = params.clef_type.width() * spatium * scale;
    let clef_height = params.clef_type.height() * spatium * scale;

    // Position clef on its line
    let y = -params.clef_type.line() as f64 * spatium / 2.0;

    let mut commands = Vec::new();

    // Draw clef glyph
    commands.push(PaintCommand::glyph(
        params.clef_type.glyph(params.is_change),
        Point::new(0.0, y),
        spatium * scale,
        Color::BLACK,
    ));

    // Draw octave indicator if present
    match params.octave {
        ClefOctave::OctaveUp | ClefOctave::TwoOctavesUp => {
            let text = if params.octave == ClefOctave::TwoOctavesUp {
                "15"
            } else {
                "8"
            };
            commands.push(PaintCommand::text(
                text,
                "Times New Roman",
                spatium * 0.8 * scale,
                Point::new(clef_width / 2.0, y - clef_height / 2.0 - spatium * 0.5),
                Color::BLACK,
            ));
        }
        ClefOctave::OctaveDown | ClefOctave::TwoOctavesDown => {
            let text = if params.octave == ClefOctave::TwoOctavesDown {
                "15"
            } else {
                "8"
            };
            commands.push(PaintCommand::text(
                text,
                "Times New Roman",
                spatium * 0.8 * scale,
                Point::new(clef_width / 2.0, y + clef_height / 2.0 + spatium * 0.8),
                Color::BLACK,
            ));
        }
        ClefOctave::None => {}
    }

    let bbox = Rect::new(
        0.0,
        y - clef_height / 2.0,
        clef_width,
        y + clef_height / 2.0,
    );
    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::ZERO, bbox, shape);

    let semantic_id = SemanticId::new(ElementType::Clef, params.id);
    let node = SceneNode::leaf(semantic_id, commands)
        .with_metadata("clef-type", format!("{:?}", params.clef_type));

    (layout, node)
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
    fn test_layout_treble_clef() {
        let ctx = test_ctx();
        let params = ClefParams {
            id: 1,
            clef_type: ClefType::Treble,
            ..Default::default()
        };

        let (layout, node) = layout_clef(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
    }

    #[test]
    fn test_layout_bass_clef() {
        let ctx = test_ctx();
        let params = ClefParams {
            id: 2,
            clef_type: ClefType::Bass,
            ..Default::default()
        };

        let (layout, _node) = layout_clef(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
    }

    #[test]
    fn test_layout_clef_change() {
        let ctx = test_ctx();
        let params = ClefParams {
            id: 3,
            clef_type: ClefType::Treble,
            is_change: true,
            ..Default::default()
        };

        let (layout_full, _) = layout_clef(
            &ClefParams {
                is_change: false,
                ..params.clone()
            },
            &ctx,
        );
        let (layout_change, _) = layout_clef(&params, &ctx);

        // Clef change should be smaller
        assert!(layout_change.bbox.width() < layout_full.bbox.width());
    }

    #[test]
    fn test_layout_clef_with_octave() {
        let ctx = test_ctx();
        let params = ClefParams {
            id: 4,
            clef_type: ClefType::Treble,
            octave: ClefOctave::OctaveUp,
            ..Default::default()
        };

        let (layout, node) = layout_clef(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        // Should have clef + "8" text = 2 commands
        assert_eq!(node.commands.len(), 2);
    }

    #[test]
    fn test_clef_lines() {
        assert_eq!(ClefType::Treble.line(), -2);
        assert_eq!(ClefType::Bass.line(), 2);
        assert_eq!(ClefType::Alto.line(), 0);
    }
}
