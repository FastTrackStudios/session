//! Page style configuration for music notation rendering.
//!
//! This module defines configurable page layout parameters based on
//! traditional engraving conventions (particularly LilyPond/MuseScore).

use serde::{Deserialize, Serialize};

/// Standard paper sizes for music notation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum PaperSize {
    /// A4: 210mm × 297mm (common in Europe)
    A4,
    /// Letter: 8.5" × 11" (common in US, default)
    #[default]
    Letter,
    /// Legal: 8.5" × 14"
    Legal,
    /// Tabloid/Ledger: 11" × 17"
    Tabloid,
    /// B4: 250mm × 353mm
    B4,
    /// Custom size with explicit dimensions in points (72pt = 1 inch)
    Custom { width_pt: f32, height_pt: f32 },
}

impl PaperSize {
    /// Get paper dimensions in points (72 points = 1 inch).
    #[must_use]
    pub fn dimensions_pt(&self) -> (f32, f32) {
        match self {
            Self::A4 => (595.28, 841.89),     // 210mm × 297mm
            Self::Letter => (612.0, 792.0),   // 8.5" × 11"
            Self::Legal => (612.0, 1008.0),   // 8.5" × 14"
            Self::Tabloid => (792.0, 1224.0), // 11" × 17"
            Self::B4 => (708.66, 1000.63),    // 250mm × 353mm
            Self::Custom {
                width_pt,
                height_pt,
            } => (*width_pt, *height_pt),
        }
    }

    /// Get paper dimensions in pixels at a given DPI.
    #[must_use]
    pub fn dimensions_px(&self, dpi: f32) -> (f32, f32) {
        let (w_pt, h_pt) = self.dimensions_pt();
        let scale = dpi / 72.0;
        (w_pt * scale, h_pt * scale)
    }
}

/// Margin configuration in points (72pt = 1 inch, ~2.83pt = 1mm).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Margins {
    /// Top margin in points
    pub top: f32,
    /// Bottom margin in points
    pub bottom: f32,
    /// Left margin in points
    pub left: f32,
    /// Right margin in points
    pub right: f32,
}

impl Default for Margins {
    /// Default margins based on MuseScore defaults: 15mm all around
    fn default() -> Self {
        Self {
            top: 42.5,    // 15mm
            bottom: 42.5, // 15mm
            left: 42.5,   // 15mm
            right: 42.5,  // 15mm
        }
    }
}

impl Margins {
    /// Create margins with all sides equal.
    #[must_use]
    pub fn all(value: f32) -> Self {
        Self {
            top: value,
            bottom: value,
            left: value,
            right: value,
        }
    }

    /// Create margins from millimeter values.
    #[must_use]
    pub fn from_mm(top: f32, bottom: f32, left: f32, right: f32) -> Self {
        const MM_TO_PT: f32 = 72.0 / 25.4; // ~2.835
        Self {
            top: top * MM_TO_PT,
            bottom: bottom * MM_TO_PT,
            left: left * MM_TO_PT,
            right: right * MM_TO_PT,
        }
    }

    /// Create margins from inch values.
    #[must_use]
    pub fn from_inches(top: f32, bottom: f32, left: f32, right: f32) -> Self {
        const INCH_TO_PT: f32 = 72.0;
        Self {
            top: top * INCH_TO_PT,
            bottom: bottom * INCH_TO_PT,
            left: left * INCH_TO_PT,
            right: right * INCH_TO_PT,
        }
    }
}
