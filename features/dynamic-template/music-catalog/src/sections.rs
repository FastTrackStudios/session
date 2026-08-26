//! Song section colors, abbreviations, and dynamic cue definitions.
//!
//! This is the canonical source for section colors used by both the
//! auto-color system (REAPER regions/markers) and the keyflow UI.

use color_palette::palette;
use color_palette::Color;

// ============================================================================
// Guide / Utility track colors
// ============================================================================

/// Near-white colors for utility tracks (click, guide, loop, count).
/// These read as "infrastructure, not music content."
pub mod utility {
    use super::*;

    pub const CLICK: Color = palette::slate::S100;
    pub const GUIDE_TRACK: Color = palette::slate::S100;
    pub const CUE: Color = palette::slate::S100;
}

// ============================================================================
// Song section colors
// ============================================================================

/// Canonical section colors — the single source of truth for what color
/// a Verse, Chorus, Bridge, etc. should be across all of FTS.
pub mod colors {
    use super::*;

    pub const INTRO: Color = palette::sky::S400;
    pub const VERSE: Color = palette::green::S400;
    pub const PRE_CHORUS: Color = palette::yellow::S400;
    pub const CHORUS: Color = palette::blue::S500;
    pub const POST_CHORUS: Color = palette::orange::S400;
    pub const BRIDGE: Color = palette::purple::S400;
    pub const BREAKDOWN: Color = palette::slate::S400;
    pub const INTERLUDE: Color = palette::teal::S400;
    pub const INSTRUMENTAL: Color = palette::emerald::S400;
    pub const SOLO: Color = palette::amber::S500;
    pub const OUTRO: Color = palette::amber::S400;
    pub const ENDING: Color = palette::slate::S600;
    pub const TAG: Color = palette::purple::S300;
    pub const VAMP: Color = palette::lime::S400;
    pub const TURNAROUND: Color = palette::slate::S300;
    pub const REFRAIN: Color = palette::blue::S500;
    pub const ACAPELLA: Color = palette::pink::S500;
    pub const RAP: Color = palette::violet::S500;
    pub const EXHORTATION: Color = palette::amber::S400;
}

// ============================================================================
// Dynamic cue colors
// ============================================================================

/// Colors for dynamic/performance cues (Build, All In, Softly, etc.)
pub mod cues {
    use super::*;

    pub const BUILD: Color = palette::amber::S500;
    pub const SLOWLY_BUILD: Color = palette::amber::S600;
    pub const SWELL: Color = palette::orange::S500;
    pub const ALL_IN: Color = palette::red::S500;
    pub const SOFTLY: Color = palette::sky::S300;
    pub const HOLD: Color = palette::slate::S300;
    pub const BREAK: Color = palette::slate::S600;
    pub const HITS: Color = palette::red::S600;
    pub const DRUMS_IN: Color = palette::red::S500;
    pub const BIG_ENDING: Color = palette::red::S700;
    pub const LAST_TIME: Color = palette::red::S800;
    pub const KEY_CHANGE_UP: Color = palette::yellow::S500;
    pub const KEY_CHANGE_DOWN: Color = palette::blue::S500;
    pub const AD_LIB: Color = palette::purple::S300;
    pub const WORSHIP_FREELY: Color = palette::purple::S300;
}

// ============================================================================
// UI color sets (bright/muted/text variants for section rendering)
// ============================================================================

/// Three-tone color set for UI rendering of sections.
/// - **bright**: progress bars, active elements (400/500 shade)
/// - **muted**: backgrounds, inactive states (200 shade)
/// - **text**: text on muted backgrounds (800 shade)
#[derive(Debug, Clone, Copy)]
pub struct SectionColorSet {
    pub bright: Color,
    pub muted: Color,
    pub text: Color,
}

impl SectionColorSet {
    pub const fn new(bright: Color, muted: Color, text: Color) -> Self {
        Self {
            bright,
            muted,
            text,
        }
    }

    /// Get bright color as CSS rgb() string
    #[must_use]
    pub fn bright_css(&self) -> String {
        self.bright.to_css_rgb()
    }

    /// Get muted color as CSS rgb() string
    #[must_use]
    pub fn muted_css(&self) -> String {
        self.muted.to_css_rgb()
    }

    /// Get text color as CSS rgb() string
    #[must_use]
    pub fn text_css(&self) -> String {
        self.text.to_css_rgb()
    }

    /// Get bright color as hex string (#RRGGBB)
    #[must_use]
    pub fn bright_hex(&self) -> String {
        self.bright.to_hex_string()
    }

    /// Get muted color as hex string (#RRGGBB)
    #[must_use]
    pub fn muted_hex(&self) -> String {
        self.muted.to_hex_string()
    }

    /// Get text color as hex string (#RRGGBB)
    #[must_use]
    pub fn text_hex(&self) -> String {
        self.text.to_hex_string()
    }
}

/// Pre-built palettes for UI section rendering, derived from the canonical colors.
pub mod ui_palettes {
    use super::SectionColorSet;
    use color_palette::palette;

    pub const INTRO: SectionColorSet =
        SectionColorSet::new(palette::sky::S400, palette::sky::S200, palette::sky::S800);
    pub const VERSE: SectionColorSet = SectionColorSet::new(
        palette::emerald::S400,
        palette::emerald::S200,
        palette::emerald::S800,
    );
    pub const CHORUS: SectionColorSet = SectionColorSet::new(
        palette::blue::S500,
        palette::blue::S200,
        palette::blue::S800,
    );
    pub const BRIDGE: SectionColorSet = SectionColorSet::new(
        palette::violet::S400,
        palette::violet::S200,
        palette::violet::S800,
    );
    pub const OUTRO: SectionColorSet = SectionColorSet::new(
        palette::amber::S400,
        palette::amber::S200,
        palette::amber::S800,
    );
    pub const INTERLUDE: SectionColorSet = SectionColorSet::new(
        palette::yellow::S400,
        palette::yellow::S200,
        palette::yellow::S800,
    );
    pub const SOLO: SectionColorSet = SectionColorSet::new(
        palette::rose::S400,
        palette::rose::S200,
        palette::rose::S800,
    );
    pub const INSTRUMENTAL: SectionColorSet = SectionColorSet::new(
        palette::emerald::S400,
        palette::emerald::S200,
        palette::emerald::S800,
    );
    pub const VAMP: SectionColorSet = SectionColorSet::new(
        palette::lime::S400,
        palette::lime::S200,
        palette::lime::S800,
    );
    pub const SLATE: SectionColorSet = SectionColorSet::new(
        palette::slate::S400,
        palette::slate::S200,
        palette::slate::S800,
    );
}
