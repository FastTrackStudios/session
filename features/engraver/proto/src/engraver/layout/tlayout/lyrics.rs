//! Lyrics layout implementation.
//!
//! Handles layout of lyrics (text sung to notes), including:
//! - Syllable positioning under notes
//! - Verse stacking (multiple lyric lines)
//! - Hyphen connectors between syllables
//! - Melisma lines (extended notes)

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;

/// Syllable type indicating position within a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyllabicType {
    /// Single syllable word
    #[default]
    Single,
    /// Beginning of a multi-syllable word
    Begin,
    /// Middle of a multi-syllable word
    Middle,
    /// End of a multi-syllable word
    End,
}

impl SyllabicType {
    /// Whether this syllable should have a following dash.
    #[must_use]
    pub fn has_dash(&self) -> bool {
        matches!(self, Self::Begin | Self::Middle)
    }
}

/// Lyrics placement relative to staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LyricsPlacement {
    /// Below the staff (default for vocals)
    #[default]
    Below,
    /// Above the staff
    Above,
}

/// Lyrics layout parameters.
#[derive(Debug, Clone)]
pub struct LyricsParams {
    /// Unique identifier
    pub id: u64,
    /// The lyric text (syllable)
    pub text: String,
    /// Syllabic type (single, begin, middle, end)
    pub syllabic: SyllabicType,
    /// Verse number (0 = first verse)
    pub verse: u8,
    /// Placement (above/below staff)
    pub placement: LyricsPlacement,
    /// Whether this is a melisma (extended note)
    pub is_melisma: bool,
    /// X position of the associated note
    pub note_x: f64,
    /// Width of the associated note
    pub note_width: f64,
}

impl Default for LyricsParams {
    fn default() -> Self {
        Self {
            id: 0,
            text: String::new(),
            syllabic: SyllabicType::Single,
            verse: 0,
            placement: LyricsPlacement::Below,
            is_melisma: false,
            note_x: 0.0,
            note_width: 0.0,
        }
    }
}

/// Layout a single lyric syllable.
#[must_use]
pub fn layout_lyrics(params: &LyricsParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();

    // Text sizing
    let font_size = spatium * 1.8; // Lyrics are typically smaller than regular text
    let char_width = font_size * 0.5; // Approximate average character width
    let text_width = params.text.len() as f64 * char_width;
    let text_height = font_size * 1.2; // Line height

    // Calculate X position - center under notehead
    let center_x = params.note_x + params.note_width / 2.0;
    let x = center_x - text_width / 2.0;

    // Calculate Y position based on verse and placement
    let verse_spacing = text_height * 1.3; // Space between verses
    let base_offset = spatium * 3.0; // Distance from staff

    let y = match params.placement {
        LyricsPlacement::Below => {
            // Below staff: positive Y (down), verses stack downward
            base_offset + params.verse as f64 * verse_spacing
        }
        LyricsPlacement::Above => {
            // Above staff: negative Y (up), verses stack upward
            -base_offset - params.verse as f64 * verse_spacing - text_height
        }
    };

    // Draw the text
    let commands = vec![PaintCommand::text(
        params.text.clone(),
        "serif", // Lyrics typically use serif font
        font_size,
        Point::new(x, y + text_height * 0.8), // Baseline adjustment
        Color::BLACK,
    )];

    // Calculate bounding box
    let bbox = Rect::new(x, y, x + text_width, y + text_height);

    // Create shape for collision detection
    let shape = Shape::from_rect(bbox);

    // Create layout data
    let layout = LayoutData::new(Point::new(x, y), bbox, shape);

    // Create scene node with semantic ID
    let semantic_id = SemanticId::new(ElementType::Lyrics, params.id);
    let node = SceneNode::leaf(semantic_id, commands)
        .with_metadata("verse", params.verse.to_string())
        .with_metadata("syllabic", format!("{:?}", params.syllabic));

    (layout, node)
}

/// Layout a lyric dash (hyphen) connecting syllables.
#[must_use]
pub fn layout_lyrics_dash(
    start_x: f64,
    end_x: f64,
    y: f64,
    ctx: &LayoutContext,
) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let dash_width = spatium * 0.8;
    let dash_height = spatium * 0.1;

    // Calculate dash positions
    let total_width = end_x - start_x;
    let min_dash_spacing = spatium * 2.0;

    let mut commands = Vec::new();

    if total_width > min_dash_spacing {
        // Single centered dash
        let dash_x = start_x + (total_width - dash_width) / 2.0;
        let dash_y = y + spatium * 0.5; // Center vertically in text area

        commands.push(PaintCommand::line(
            Point::new(dash_x, dash_y),
            Point::new(dash_x + dash_width, dash_y),
            Color::BLACK,
            dash_height,
        ));
    }

    let bbox = Rect::new(start_x, y, end_x, y + spatium);
    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::new(start_x, y), bbox, shape);

    let node = SceneNode::anonymous_leaf(commands);

    (layout, node)
}

/// Layout a melisma line (extended note indicator).
#[must_use]
pub fn layout_melisma(
    start_x: f64,
    end_x: f64,
    y: f64,
    ctx: &LayoutContext,
) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let line_thickness = spatium * 0.08;

    // Melisma line is drawn from end of syllable to end of note
    let line_y = y + spatium * 0.8; // Just below text baseline

    let mut commands = Vec::new();

    if end_x > start_x + spatium * 0.5 {
        commands.push(PaintCommand::line(
            Point::new(start_x, line_y),
            Point::new(end_x, line_y),
            Color::BLACK,
            line_thickness,
        ));
    }

    let bbox = Rect::new(start_x, y, end_x, y + spatium);
    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::new(start_x, y), bbox, shape);

    let node = SceneNode::anonymous_leaf(commands);

    (layout, node)
}

/// Compute vertical positions for lyrics verses to avoid collisions.
///
/// This implements verse stacking similar to MuseScore's `computeVerticalPositions`.
pub fn compute_verse_positions(
    verse_count: u8,
    placement: LyricsPlacement,
    staff_height: f64,
    ctx: &LayoutContext,
) -> Vec<f64> {
    let spatium = ctx.spatium();
    let text_height = spatium * 1.8 * 1.2;
    let verse_spacing = text_height * 1.3;
    let base_offset = spatium * 3.0;

    (0..verse_count)
        .map(|verse| match placement {
            LyricsPlacement::Below => staff_height + base_offset + verse as f64 * verse_spacing,
            LyricsPlacement::Above => -base_offset - verse as f64 * verse_spacing - text_height,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engraver::style::MStyle;

    fn test_ctx() -> LayoutContext<'static> {
        use crate::engraver::layout::context::LayoutContextOwned;
        let owned = Box::leak(Box::new(LayoutContextOwned::new_minimal(MStyle::default())));
        owned.as_context()
    }

    #[test]
    fn test_layout_single_syllable() {
        let ctx = test_ctx();
        let params = LyricsParams {
            id: 1,
            text: "love".to_string(),
            syllabic: SyllabicType::Single,
            verse: 0,
            placement: LyricsPlacement::Below,
            note_x: 100.0,
            note_width: 10.0,
            ..Default::default()
        };

        let (layout, node) = layout_lyrics(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
        assert!(!node.commands.is_empty());
    }

    #[test]
    fn test_layout_multi_syllable() {
        let ctx = test_ctx();

        // First syllable
        let params1 = LyricsParams {
            id: 1,
            text: "beau".to_string(),
            syllabic: SyllabicType::Begin,
            verse: 0,
            note_x: 100.0,
            note_width: 10.0,
            ..Default::default()
        };
        let (layout1, _) = layout_lyrics(&params1, &ctx);

        // Second syllable
        let params2 = LyricsParams {
            id: 2,
            text: "ti".to_string(),
            syllabic: SyllabicType::Middle,
            verse: 0,
            note_x: 150.0,
            note_width: 10.0,
            ..Default::default()
        };
        let (layout2, _) = layout_lyrics(&params2, &ctx);

        // Third syllable
        let params3 = LyricsParams {
            id: 3,
            text: "ful".to_string(),
            syllabic: SyllabicType::End,
            verse: 0,
            note_x: 200.0,
            note_width: 10.0,
            ..Default::default()
        };
        let (layout3, _) = layout_lyrics(&params3, &ctx);

        // All should have valid bounding boxes
        assert!(!layout1.bbox.is_zero_area());
        assert!(!layout2.bbox.is_zero_area());
        assert!(!layout3.bbox.is_zero_area());

        // Syllables should be at increasing X positions
        assert!(layout2.position.x > layout1.position.x);
        assert!(layout3.position.x > layout2.position.x);
    }

    #[test]
    fn test_verse_stacking() {
        let ctx = test_ctx();

        // Verse 0
        let params_v0 = LyricsParams {
            id: 1,
            text: "First".to_string(),
            verse: 0,
            note_x: 100.0,
            note_width: 10.0,
            ..Default::default()
        };
        let (layout_v0, _) = layout_lyrics(&params_v0, &ctx);

        // Verse 1
        let params_v1 = LyricsParams {
            id: 2,
            text: "Second".to_string(),
            verse: 1,
            note_x: 100.0,
            note_width: 10.0,
            ..Default::default()
        };
        let (layout_v1, _) = layout_lyrics(&params_v1, &ctx);

        // Second verse should be below first (greater Y)
        assert!(layout_v1.position.y > layout_v0.position.y);
    }

    #[test]
    fn test_placement_above() {
        let ctx = test_ctx();

        let params_below = LyricsParams {
            id: 1,
            text: "below".to_string(),
            placement: LyricsPlacement::Below,
            note_x: 100.0,
            note_width: 10.0,
            ..Default::default()
        };
        let (layout_below, _) = layout_lyrics(&params_below, &ctx);

        let params_above = LyricsParams {
            id: 2,
            text: "above".to_string(),
            placement: LyricsPlacement::Above,
            note_x: 100.0,
            note_width: 10.0,
            ..Default::default()
        };
        let (layout_above, _) = layout_lyrics(&params_above, &ctx);

        // Above should have negative Y (or smaller), below should have positive Y
        assert!(layout_above.position.y < layout_below.position.y);
    }

    #[test]
    fn test_lyrics_dash() {
        let ctx = test_ctx();
        let (layout, node) = layout_lyrics_dash(100.0, 150.0, 20.0, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(!node.commands.is_empty());
    }

    #[test]
    fn test_melisma_line() {
        let ctx = test_ctx();
        let (layout, node) = layout_melisma(100.0, 200.0, 20.0, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(!node.commands.is_empty());
    }
}
