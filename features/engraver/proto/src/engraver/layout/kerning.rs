//! Kerning types for horizontal spacing adjustments.
//!
//! Kerning controls how elements interact during collision detection.
//! Different kerning types allow or prevent overlapping of adjacent elements.

use serde::{Deserialize, Serialize};

/// Type of kerning behavior for spacing calculations.
///
/// Kerning determines how much elements can overlap during horizontal spacing.
/// Different element types have different kerning behaviors based on their
/// visual characteristics and musical semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KerningType {
    /// Normal kerning - elements can overlap up to their bounding boxes
    #[default]
    Kerning,

    /// No kerning - maintain minimum separation, no overlapping allowed
    NonKerning,

    /// Kern until the left edge of the following element
    KernUntilLeftEdge,

    /// Kern until the center of the following element
    KernUntilCenter,

    /// Kern until the right edge of the following element
    KernUntilRightEdge,

    /// Limited kerning for same-voice elements
    SameVoiceLimit,

    /// Allow collision (elements can fully overlap)
    AllowCollision,
}

impl KerningType {
    /// Check if this kerning type allows any overlap.
    #[must_use]
    pub const fn allows_kerning(&self) -> bool {
        !matches!(self, Self::NonKerning)
    }

    /// Check if this kerning type allows full collision.
    #[must_use]
    pub const fn allows_collision(&self) -> bool {
        matches!(self, Self::AllowCollision)
    }

    /// Check if kerning is limited for same-voice elements.
    #[must_use]
    pub const fn is_same_voice_limited(&self) -> bool {
        matches!(self, Self::SameVoiceLimit)
    }

    /// Get the effective kerning limit as a fraction of element width.
    ///
    /// Returns a value from 0.0 (left edge) to 1.0 (right edge), or None
    /// for standard kerning behavior.
    #[must_use]
    pub const fn kern_limit_fraction(&self) -> Option<f64> {
        match self {
            Self::KernUntilLeftEdge => Some(0.0),
            Self::KernUntilCenter => Some(0.5),
            Self::KernUntilRightEdge => Some(1.0),
            _ => None,
        }
    }
}

/// Minimum horizontal padding between elements.
///
/// These values are in spatiums (staff spaces) and are used as base
/// padding values before kerning adjustments.
#[derive(Debug, Clone)]
pub struct SpacingPadding {
    /// Default minimum padding between most elements
    pub default: f64,

    /// Padding between accidentals
    pub accidental: f64,

    /// Padding for ledger lines
    pub ledger_line: f64,

    /// Padding for noteheads
    pub notehead: f64,

    /// Padding for stems
    pub stem: f64,

    /// Padding for articulations
    pub articulation: f64,

    /// Padding for lyrics
    pub lyrics: f64,

    /// Padding for barlines
    pub barline: f64,

    /// Minimum distance between chords (noteheads)
    pub min_note_distance: f64,
}

impl Default for SpacingPadding {
    fn default() -> Self {
        Self {
            default: 0.35,
            accidental: 0.15,
            ledger_line: 0.0,
            notehead: 0.25,
            stem: 0.15,
            articulation: 0.15,
            lyrics: 0.5,
            barline: 0.4,
            min_note_distance: 0.35,
        }
    }
}

impl SpacingPadding {
    /// Create default padding values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert padding values to pixels given a spatium.
    #[must_use]
    pub fn to_pixels(&self, spatium: f64) -> SpacingPaddingPixels {
        SpacingPaddingPixels {
            default: self.default * spatium,
            accidental: self.accidental * spatium,
            ledger_line: self.ledger_line * spatium,
            notehead: self.notehead * spatium,
            stem: self.stem * spatium,
            articulation: self.articulation * spatium,
            lyrics: self.lyrics * spatium,
            barline: self.barline * spatium,
            min_note_distance: self.min_note_distance * spatium,
        }
    }
}

/// Spacing padding values in pixels.
#[derive(Debug, Clone)]
pub struct SpacingPaddingPixels {
    pub default: f64,
    pub accidental: f64,
    pub ledger_line: f64,
    pub notehead: f64,
    pub stem: f64,
    pub articulation: f64,
    pub lyrics: f64,
    pub barline: f64,
    pub min_note_distance: f64,
}

impl SpacingPaddingPixels {
    /// Get the padding for a given kerning type.
    #[must_use]
    pub fn for_kerning_type(&self, kerning: KerningType) -> f64 {
        match kerning {
            KerningType::AllowCollision => 0.0,
            KerningType::NonKerning => self.default,
            _ => self.default * 0.5, // Reduced padding when kerning is allowed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kerning_type_allows_kerning() {
        assert!(KerningType::Kerning.allows_kerning());
        assert!(!KerningType::NonKerning.allows_kerning());
        assert!(KerningType::AllowCollision.allows_kerning());
    }

    #[test]
    fn test_kerning_type_allows_collision() {
        assert!(!KerningType::Kerning.allows_collision());
        assert!(KerningType::AllowCollision.allows_collision());
    }

    #[test]
    fn test_kern_limit_fraction() {
        assert_eq!(
            KerningType::KernUntilLeftEdge.kern_limit_fraction(),
            Some(0.0)
        );
        assert_eq!(
            KerningType::KernUntilCenter.kern_limit_fraction(),
            Some(0.5)
        );
        assert_eq!(
            KerningType::KernUntilRightEdge.kern_limit_fraction(),
            Some(1.0)
        );
        assert_eq!(KerningType::Kerning.kern_limit_fraction(), None);
    }

    #[test]
    fn test_padding_to_pixels() {
        let padding = SpacingPadding::default();
        let pixels = padding.to_pixels(10.0);

        assert!((pixels.default - 3.5).abs() < 1e-10);
        assert!((pixels.accidental - 1.5).abs() < 1e-10);
    }
}
