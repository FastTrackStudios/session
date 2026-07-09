//! Key signature layout implementation.
//!
//! Handles layout of key signatures with sharps or flats.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;
use super::note::glyphs::{ACCIDENTAL_FLAT, ACCIDENTAL_NATURAL, ACCIDENTAL_SHARP};

/// Key signature type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySigType {
    /// Number of sharps (positive) or flats (negative)
    /// Range: -7 (7 flats) to +7 (7 sharps)
    Standard(i8),
    /// Atonal / open key (no key signature)
    Open,
}

impl Default for KeySigType {
    fn default() -> Self {
        Self::Standard(0) // C major / A minor
    }
}

impl KeySigType {
    /// Get the number of accidentals.
    #[must_use]
    pub const fn accidental_count(&self) -> u8 {
        match self {
            Self::Standard(n) => n.unsigned_abs(),
            Self::Open => 0,
        }
    }

    /// Check if this key has sharps.
    #[must_use]
    pub const fn has_sharps(&self) -> bool {
        matches!(self, Self::Standard(n) if *n > 0)
    }

    /// Check if this key has flats.
    #[must_use]
    pub const fn has_flats(&self) -> bool {
        matches!(self, Self::Standard(n) if *n < 0)
    }
}

/// Clef-dependent position data for key signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClefContext {
    /// Treble clef
    #[default]
    Treble,
    /// Bass clef
    Bass,
    /// Alto clef
    Alto,
    /// Tenor clef
    Tenor,
}

impl ClefContext {
    /// Get the staff lines for sharps in order (F#, C#, G#, D#, A#, E#, B#).
    #[must_use]
    pub const fn sharp_positions(&self) -> [i32; 7] {
        match self {
            // Lines: 0 = middle line (B4 in treble), positive = up
            Self::Treble => [4, 1, 5, 2, -1, 3, 0], // F# C# G# D# A# E# B#
            Self::Bass => [2, -1, 3, 0, -3, 1, -2], // F# C# G# D# A# E# B#
            Self::Alto => [3, 0, 4, 1, -2, 2, -1],  // F# C# G# D# A# E# B#
            Self::Tenor => [1, -2, 2, -1, -4, 0, -3], // F# C# G# D# A# E# B#
        }
    }

    /// Get the staff lines for flats in order (Bb, Eb, Ab, Db, Gb, Cb, Fb).
    #[must_use]
    pub const fn flat_positions(&self) -> [i32; 7] {
        match self {
            Self::Treble => [0, 3, -1, 2, -2, 1, -3], // Bb Eb Ab Db Gb Cb Fb
            Self::Bass => [-2, 1, -3, 0, -4, -1, -5], // Bb Eb Ab Db Gb Cb Fb
            Self::Alto => [-1, 2, -2, 1, -3, 0, -4],  // Bb Eb Ab Db Gb Cb Fb
            Self::Tenor => [-3, 0, -4, -1, -5, -2, -6], // Bb Eb Ab Db Gb Cb Fb
        }
    }
}

/// Key signature layout parameters.
#[derive(Debug, Clone, Default)]
pub struct KeySigParams {
    /// Unique identifier
    pub id: u64,
    /// Key signature type
    pub key: KeySigType,
    /// Clef context for positioning
    pub clef: ClefContext,
    /// Whether this is a key change (show naturals for cancelled accidentals)
    pub show_naturals: bool,
    /// Previous key (for showing naturals on key change)
    pub prev_key: Option<KeySigType>,
    /// Optional glyph color override. `None` renders the default black; mid-chart
    /// key changes pass a red so they stand out from the prevailing key prefix.
    pub color: Option<Color>,
}

/// Layout a key signature.
#[must_use]
pub fn layout_keysig(params: &KeySigParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let accidental_spacing = spatium * 0.9; // Space between accidentals
    let color = params.color.unwrap_or(Color::BLACK);

    let mut commands = Vec::new();
    let mut x = 0.0;

    // First, show naturals if this is a key change
    if params.show_naturals
        && let Some(prev_key) = &params.prev_key
    {
        let naturals = calculate_naturals(prev_key, &params.key);
        let positions = if prev_key.has_sharps() {
            params.clef.sharp_positions()
        } else {
            params.clef.flat_positions()
        };

        for i in 0..naturals {
            let line = positions[i as usize];
            let y = -line as f64 * spatium / 2.0;

            commands.push(PaintCommand::glyph(
                ACCIDENTAL_NATURAL,
                Point::new(x, y),
                spatium,
                color,
            ));

            x += accidental_spacing * 0.8; // Naturals slightly closer
        }

        if naturals > 0 {
            x += spatium * 0.3; // Gap before new key
        }
    }

    // Now draw the new key signature
    match params.key {
        KeySigType::Standard(n) if n > 0 => {
            // Sharps
            let positions = params.clef.sharp_positions();
            for line in positions.iter().take(n as usize) {
                let y = -(*line) as f64 * spatium / 2.0;

                commands.push(PaintCommand::glyph(
                    ACCIDENTAL_SHARP,
                    Point::new(x, y),
                    spatium,
                    color,
                ));

                x += accidental_spacing;
            }
        }

        KeySigType::Standard(n) if n < 0 => {
            // Flats
            let positions = params.clef.flat_positions();
            for line in positions.iter().take((-n) as usize) {
                let y = -(*line) as f64 * spatium / 2.0;

                commands.push(PaintCommand::glyph(
                    ACCIDENTAL_FLAT,
                    Point::new(x, y),
                    spatium,
                    color,
                ));

                x += accidental_spacing;
            }
        }

        _ => {
            // C major or Open - no accidentals to draw
        }
    }

    // Calculate bounding box
    let width = if x > 0.0 { x } else { 0.0 };
    let height = spatium * 4.0; // Full staff height
    let bbox = Rect::new(0.0, -height / 2.0, width.max(0.1), height / 2.0);

    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::ZERO, bbox, shape);

    let semantic_id = SemanticId::new(ElementType::KeySignature, params.id);
    let key_name = key_name(&params.key);
    let node = SceneNode::leaf(semantic_id, commands).with_metadata("key", key_name);

    (layout, node)
}

/// Calculate how many naturals to show for a key change.
fn calculate_naturals(prev: &KeySigType, new: &KeySigType) -> u8 {
    match (prev, new) {
        (KeySigType::Standard(p), KeySigType::Standard(n)) => {
            if (*p > 0 && *n <= 0) || (*p < 0 && *n >= 0) {
                // Changing from sharps to flats or vice versa - natural all
                p.unsigned_abs()
            } else if p.unsigned_abs() > n.unsigned_abs() {
                // Fewer accidentals - natural the difference
                p.unsigned_abs() - n.unsigned_abs()
            } else {
                0
            }
        }
        (KeySigType::Standard(p), KeySigType::Open) => p.unsigned_abs(),
        _ => 0,
    }
}

/// Get key name string.
fn key_name(key: &KeySigType) -> String {
    match key {
        KeySigType::Open => "open".to_string(),
        KeySigType::Standard(0) => "C".to_string(),
        KeySigType::Standard(1) => "G".to_string(),
        KeySigType::Standard(2) => "D".to_string(),
        KeySigType::Standard(3) => "A".to_string(),
        KeySigType::Standard(4) => "E".to_string(),
        KeySigType::Standard(5) => "B".to_string(),
        KeySigType::Standard(6) => "F#".to_string(),
        KeySigType::Standard(7) => "C#".to_string(),
        KeySigType::Standard(-1) => "F".to_string(),
        KeySigType::Standard(-2) => "Bb".to_string(),
        KeySigType::Standard(-3) => "Eb".to_string(),
        KeySigType::Standard(-4) => "Ab".to_string(),
        KeySigType::Standard(-5) => "Db".to_string(),
        KeySigType::Standard(-6) => "Gb".to_string(),
        KeySigType::Standard(-7) => "Cb".to_string(),
        KeySigType::Standard(n) => format!("{} accidentals", n),
    }
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
    fn test_layout_c_major() {
        let ctx = test_ctx();
        let params = KeySigParams {
            id: 1,
            key: KeySigType::Standard(0),
            ..Default::default()
        };

        let (_layout, node) = layout_keysig(&params, &ctx);

        // C major has no accidentals
        assert!(node.commands.is_empty());
    }

    #[test]
    fn test_layout_g_major() {
        let ctx = test_ctx();
        let params = KeySigParams {
            id: 2,
            key: KeySigType::Standard(1),
            ..Default::default()
        };

        let (_layout, node) = layout_keysig(&params, &ctx);

        // G major has 1 sharp
        assert_eq!(node.commands.len(), 1);
    }

    #[test]
    fn test_layout_d_major() {
        let ctx = test_ctx();
        let params = KeySigParams {
            id: 3,
            key: KeySigType::Standard(2),
            ..Default::default()
        };

        let (_layout, node) = layout_keysig(&params, &ctx);

        // D major has 2 sharps
        assert_eq!(node.commands.len(), 2);
    }

    #[test]
    fn test_layout_f_major() {
        let ctx = test_ctx();
        let params = KeySigParams {
            id: 4,
            key: KeySigType::Standard(-1),
            ..Default::default()
        };

        let (_layout, node) = layout_keysig(&params, &ctx);

        // F major has 1 flat
        assert_eq!(node.commands.len(), 1);
    }

    #[test]
    fn test_layout_bb_major() {
        let ctx = test_ctx();
        let params = KeySigParams {
            id: 5,
            key: KeySigType::Standard(-2),
            ..Default::default()
        };

        let (_layout, node) = layout_keysig(&params, &ctx);

        // Bb major has 2 flats
        assert_eq!(node.commands.len(), 2);
    }

    #[test]
    fn test_layout_key_change_with_naturals() {
        let ctx = test_ctx();
        let params = KeySigParams {
            id: 6,
            key: KeySigType::Standard(0), // C major
            show_naturals: true,
            prev_key: Some(KeySigType::Standard(3)), // From A major (3 sharps)
            ..Default::default()
        };

        let (_layout, node) = layout_keysig(&params, &ctx);

        // Should show 3 naturals
        assert_eq!(node.commands.len(), 3);
    }

    #[test]
    fn test_key_sig_accidental_count() {
        assert_eq!(KeySigType::Standard(0).accidental_count(), 0);
        assert_eq!(KeySigType::Standard(3).accidental_count(), 3);
        assert_eq!(KeySigType::Standard(-4).accidental_count(), 4);
        assert_eq!(KeySigType::Open.accidental_count(), 0);
    }

    #[test]
    fn test_key_sig_has_sharps_flats() {
        assert!(KeySigType::Standard(3).has_sharps());
        assert!(!KeySigType::Standard(3).has_flats());
        assert!(!KeySigType::Standard(-2).has_sharps());
        assert!(KeySigType::Standard(-2).has_flats());
    }

    #[test]
    fn test_calculate_naturals() {
        // Going from 3 sharps to C major = 3 naturals
        assert_eq!(
            calculate_naturals(&KeySigType::Standard(3), &KeySigType::Standard(0)),
            3
        );

        // Going from 2 sharps to 4 sharps = 0 naturals
        assert_eq!(
            calculate_naturals(&KeySigType::Standard(2), &KeySigType::Standard(4)),
            0
        );

        // Going from 3 sharps to 2 flats = 3 naturals (all sharps cancelled)
        assert_eq!(
            calculate_naturals(&KeySigType::Standard(3), &KeySigType::Standard(-2)),
            3
        );
    }
}
