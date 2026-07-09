//! MuseScore-compatible style system for music notation.
//!
//! This module provides a comprehensive style property system modeled after
//! MuseScore's MStyle class. It supports ~80 properties focused on lead sheet
//! rendering including page layout, spacing, chord symbols, and text styles.

// region:    --- Modules

mod styledef;

pub use styledef::{StyleValue, default_value};

// endregion: --- Modules

// region:    --- Sid

use serde::{Deserialize, Serialize};

/// Style ID enumeration (subset of MuseScore's 2130+ properties).
/// Focused on lead sheet rendering needs.
///
/// Property values are stored in an array indexed by Sid, allowing O(1)
/// access to any style property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum Sid {
    // ========================================================================
    // Page Layout (10 properties)
    // ========================================================================
    /// Page width in inches (US Letter default: 8.5")
    PageWidth = 0,
    /// Page height in inches (US Letter default: 11.0")
    PageHeight,
    /// Printable width in inches (page width minus margins)
    PagePrintableWidth,
    /// Left margin for even pages in inches
    PageEvenLeftMargin,
    /// Top margin for even pages in inches
    PageEvenTopMargin,
    /// Bottom margin for even pages in inches
    PageEvenBottomMargin,
    /// Left margin for odd pages in inches
    PageOddLeftMargin,
    /// Top margin for odd pages in inches
    PageOddTopMargin,
    /// Bottom margin for odd pages in inches
    PageOddBottomMargin,
    /// Whether to use two-sided page layout
    PageTwosided,

    // ========================================================================
    // Staff & System Spacing (12 properties)
    // ========================================================================
    /// Base musical unit in points (default: 5pt = 1.75mm)
    Spatium,
    /// Distance between staves within a system in spatiums (default: 6.5 sp)
    StaffDistance,
    /// Top margin area in spatiums (default: 7.0 sp)
    StaffUpperBorder,
    /// Bottom margin area in spatiums (default: 7.0 sp)
    StaffLowerBorder,
    /// Minimum distance between systems in spatiums (default: 8.5 sp)
    MinSystemDistance,
    /// Maximum distance between systems in spatiums (default: 15.0 sp)
    MaxSystemDistance,
    /// Header/footer padding in spatiums (default: 1.0 sp)
    StaffHeaderFooterPadding,
    /// Whether to spread systems to fill page vertically
    EnableVerticalSpread,
    /// System spread multiplier (default: 2.5)
    SpreadSystem,
    /// Minimum spread amount in spatiums (default: 8.5 sp)
    MinSystemSpread,
    /// Maximum spread amount in spatiums
    MaxSystemSpread,
    /// Whether to align first/last systems to margins
    AlignSystemToMargin,

    // ========================================================================
    // Horizontal Spacing (10 properties)
    // ========================================================================
    /// Distance from barline to first note in spatiums (default: 1.5 sp)
    BarNoteDistance,
    /// Distance from last note to barline in spatiums (default: 1.0 sp)
    NoteBarDistance,
    /// Minimum distance between notes in spatiums (default: 0.5 sp)
    MinNoteDistance,
    /// Measure spacing ratio (default: 1.5)
    MeasureSpacing,
    /// Barline thickness in spatiums (default: 0.16 sp)
    BarWidth,
    /// Double barline thickness in spatiums (default: 0.16 sp)
    DoubleBarWidth,
    /// Gap between double barlines in spatiums (default: 0.46 sp)
    DoubleBarDistance,
    /// Final barline thickness in spatiums (default: 0.5 sp)
    EndBarWidth,
    /// Final barline gap in spatiums (default: 0.65 sp)
    EndBarDistance,
    /// Clef left margin in spatiums (default: 0.64 sp)
    ClefLeftMargin,

    // ========================================================================
    // Chord Symbols (15 properties)
    // ========================================================================
    /// Chord symbol placement (above/below staff)
    HarmonyPlacement,
    /// Y offset when placed above staff in spatiums (default: -2.5 sp)
    HarmonyPosAbove,
    /// Y offset when placed below staff in spatiums (default: 3.5 sp)
    HarmonyPosBelow,
    /// Minimum horizontal distance between chord symbols in spatiums
    MinHarmonyDistance,
    /// Maximum shift towards barline in spatiums (default: 0.8 sp)
    MaxHarmonyBarDistance,
    /// Chord symbol font family
    ChordSymbolAFontFace,
    /// Chord symbol font size in points (default: 12pt)
    ChordSymbolAFontSize,
    /// Chord symbol font style (normal/bold/italic)
    ChordSymbolAFontStyle,
    /// Chord symbol color
    ChordSymbolAColor,
    /// Extension scale factor (default: 1.0)
    ChordExtensionMag,
    /// Modifier scale factor (default: 1.0)
    ChordModifierMag,
    /// Stacked/superscript modifier scale (default: 0.75)
    ChordStackedModifierMag,
    /// Bass note scale factor (default: 1.0)
    ChordBassNoteScale,
    /// Chord style (standard/jazz)
    ChordStyle,
    /// Horizontal alignment for chord symbols
    HarmonyAlign,

    // ========================================================================
    // Section Headers & Rehearsal Marks (8 properties)
    // ========================================================================
    /// Rehearsal mark placement (above/below)
    RehearsalMarkPlacement,
    /// Rehearsal mark Y offset when above
    RehearsalMarkPosAbove,
    /// Rehearsal mark font family
    RehearsalMarkFontFace,
    /// Rehearsal mark font size in points
    RehearsalMarkFontSize,
    /// Rehearsal mark font style
    RehearsalMarkFontStyle,
    /// Rehearsal mark frame type (none/square/circle)
    RehearsalMarkFrameType,
    /// Rehearsal mark frame padding
    RehearsalMarkFramePadding,
    /// Rehearsal mark frame line width
    RehearsalMarkFrameWidth,

    // ========================================================================
    // Headers & Footers (12 properties)
    // ========================================================================
    /// Whether to show page header
    ShowHeader,
    /// Whether to show header on first page
    HeaderFirstPage,
    /// Whether to use different headers for odd/even pages
    HeaderOddEven,
    /// Left-aligned text for odd page headers
    OddHeaderL,
    /// Center-aligned text for odd page headers
    OddHeaderC,
    /// Right-aligned text for odd page headers
    OddHeaderR,
    /// Whether to show page footer
    ShowFooter,
    /// Whether to show footer on first page
    FooterFirstPage,
    /// Whether to use different footers for odd/even pages
    FooterOddEven,
    /// Left-aligned text for odd page footers
    OddFooterL,
    /// Center-aligned text for odd page footers
    OddFooterC,
    /// Right-aligned text for odd page footers
    OddFooterR,

    // ========================================================================
    // Title/Composer Text (12 properties)
    // ========================================================================
    /// Title font family
    TitleFontFace,
    /// Title font size in points (default: 24pt)
    TitleFontSize,
    /// Title font style
    TitleFontStyle,
    /// Title horizontal alignment
    TitleAlign,
    /// Subtitle font family
    SubtitleFontFace,
    /// Subtitle font size in points (default: 14pt)
    SubtitleFontSize,
    /// Composer font family
    ComposerFontFace,
    /// Composer font size in points (default: 12pt)
    ComposerFontSize,
    /// Composer horizontal alignment
    ComposerAlign,
    /// Lyricist font family
    LyricistFontFace,
    /// Lyricist font size in points (default: 12pt)
    LyricistFontSize,
    /// Lyricist horizontal alignment
    LyricistAlign,

    /// Sentinel value for array sizing - must be last
    #[doc(hidden)]
    Styles,
}

impl Sid {
    /// Total number of style properties
    pub const COUNT: usize = Self::Styles as usize;

    /// All Sid variants in order (for safe iteration without unsafe transmute)
    pub const ALL: [Sid; Self::COUNT] = [
        // Page Layout
        Self::PageWidth,
        Self::PageHeight,
        Self::PagePrintableWidth,
        Self::PageEvenLeftMargin,
        Self::PageEvenTopMargin,
        Self::PageEvenBottomMargin,
        Self::PageOddLeftMargin,
        Self::PageOddTopMargin,
        Self::PageOddBottomMargin,
        Self::PageTwosided,
        // Staff & System Spacing
        Self::Spatium,
        Self::StaffDistance,
        Self::StaffUpperBorder,
        Self::StaffLowerBorder,
        Self::MinSystemDistance,
        Self::MaxSystemDistance,
        Self::StaffHeaderFooterPadding,
        Self::EnableVerticalSpread,
        Self::SpreadSystem,
        Self::MinSystemSpread,
        Self::MaxSystemSpread,
        Self::AlignSystemToMargin,
        // Horizontal Spacing
        Self::BarNoteDistance,
        Self::NoteBarDistance,
        Self::MinNoteDistance,
        Self::MeasureSpacing,
        Self::BarWidth,
        Self::DoubleBarWidth,
        Self::DoubleBarDistance,
        Self::EndBarWidth,
        Self::EndBarDistance,
        Self::ClefLeftMargin,
        // Chord Symbols
        Self::HarmonyPlacement,
        Self::HarmonyPosAbove,
        Self::HarmonyPosBelow,
        Self::MinHarmonyDistance,
        Self::MaxHarmonyBarDistance,
        Self::ChordSymbolAFontFace,
        Self::ChordSymbolAFontSize,
        Self::ChordSymbolAFontStyle,
        Self::ChordSymbolAColor,
        Self::ChordExtensionMag,
        Self::ChordModifierMag,
        Self::ChordStackedModifierMag,
        Self::ChordBassNoteScale,
        Self::ChordStyle,
        Self::HarmonyAlign,
        // Section Headers & Rehearsal Marks
        Self::RehearsalMarkPlacement,
        Self::RehearsalMarkPosAbove,
        Self::RehearsalMarkFontFace,
        Self::RehearsalMarkFontSize,
        Self::RehearsalMarkFontStyle,
        Self::RehearsalMarkFrameType,
        Self::RehearsalMarkFramePadding,
        Self::RehearsalMarkFrameWidth,
        // Headers & Footers
        Self::ShowHeader,
        Self::HeaderFirstPage,
        Self::HeaderOddEven,
        Self::OddHeaderL,
        Self::OddHeaderC,
        Self::OddHeaderR,
        Self::ShowFooter,
        Self::FooterFirstPage,
        Self::FooterOddEven,
        Self::OddFooterL,
        Self::OddFooterC,
        Self::OddFooterR,
        // Title/Composer Text
        Self::TitleFontFace,
        Self::TitleFontSize,
        Self::TitleFontStyle,
        Self::TitleAlign,
        Self::SubtitleFontFace,
        Self::SubtitleFontSize,
        Self::ComposerFontFace,
        Self::ComposerFontSize,
        Self::ComposerAlign,
        Self::LyricistFontFace,
        Self::LyricistFontSize,
        Self::LyricistAlign,
    ];

    /// Get the XML name for this style property (for serialization)
    #[must_use]
    pub fn xml_name(&self) -> &'static str {
        match self {
            Self::PageWidth => "pageWidth",
            Self::PageHeight => "pageHeight",
            Self::PagePrintableWidth => "pagePrintableWidth",
            Self::PageEvenLeftMargin => "pageEvenLeftMargin",
            Self::PageEvenTopMargin => "pageEvenTopMargin",
            Self::PageEvenBottomMargin => "pageEvenBottomMargin",
            Self::PageOddLeftMargin => "pageOddLeftMargin",
            Self::PageOddTopMargin => "pageOddTopMargin",
            Self::PageOddBottomMargin => "pageOddBottomMargin",
            Self::PageTwosided => "pageTwosided",
            Self::Spatium => "spatium",
            Self::StaffDistance => "staffDistance",
            Self::StaffUpperBorder => "staffUpperBorder",
            Self::StaffLowerBorder => "staffLowerBorder",
            Self::MinSystemDistance => "minSystemDistance",
            Self::MaxSystemDistance => "maxSystemDistance",
            Self::StaffHeaderFooterPadding => "staffHeaderFooterPadding",
            Self::EnableVerticalSpread => "enableVerticalSpread",
            Self::SpreadSystem => "spreadSystem",
            Self::MinSystemSpread => "minSystemSpread",
            Self::MaxSystemSpread => "maxSystemSpread",
            Self::AlignSystemToMargin => "alignSystemToMargin",
            Self::BarNoteDistance => "barNoteDistance",
            Self::NoteBarDistance => "noteBarDistance",
            Self::MinNoteDistance => "minNoteDistance",
            Self::MeasureSpacing => "measureSpacing",
            Self::BarWidth => "barWidth",
            Self::DoubleBarWidth => "doubleBarWidth",
            Self::DoubleBarDistance => "doubleBarDistance",
            Self::EndBarWidth => "endBarWidth",
            Self::EndBarDistance => "endBarDistance",
            Self::ClefLeftMargin => "clefLeftMargin",
            Self::HarmonyPlacement => "harmonyPlacement",
            Self::HarmonyPosAbove => "harmonyPosAbove",
            Self::HarmonyPosBelow => "harmonyPosBelow",
            Self::MinHarmonyDistance => "minHarmonyDistance",
            Self::MaxHarmonyBarDistance => "maxHarmonyBarDistance",
            Self::ChordSymbolAFontFace => "chordSymbolAFontFace",
            Self::ChordSymbolAFontSize => "chordSymbolAFontSize",
            Self::ChordSymbolAFontStyle => "chordSymbolAFontStyle",
            Self::ChordSymbolAColor => "chordSymbolAColor",
            Self::ChordExtensionMag => "chordExtensionMag",
            Self::ChordModifierMag => "chordModifierMag",
            Self::ChordStackedModifierMag => "chordStackedModifierMag",
            Self::ChordBassNoteScale => "chordBassNoteScale",
            Self::ChordStyle => "chordStyle",
            Self::HarmonyAlign => "harmonyAlign",
            Self::RehearsalMarkPlacement => "rehearsalMarkPlacement",
            Self::RehearsalMarkPosAbove => "rehearsalMarkPosAbove",
            Self::RehearsalMarkFontFace => "rehearsalMarkFontFace",
            Self::RehearsalMarkFontSize => "rehearsalMarkFontSize",
            Self::RehearsalMarkFontStyle => "rehearsalMarkFontStyle",
            Self::RehearsalMarkFrameType => "rehearsalMarkFrameType",
            Self::RehearsalMarkFramePadding => "rehearsalMarkFramePadding",
            Self::RehearsalMarkFrameWidth => "rehearsalMarkFrameWidth",
            Self::ShowHeader => "showHeader",
            Self::HeaderFirstPage => "headerFirstPage",
            Self::HeaderOddEven => "headerOddEven",
            Self::OddHeaderL => "oddHeaderL",
            Self::OddHeaderC => "oddHeaderC",
            Self::OddHeaderR => "oddHeaderR",
            Self::ShowFooter => "showFooter",
            Self::FooterFirstPage => "footerFirstPage",
            Self::FooterOddEven => "footerOddEven",
            Self::OddFooterL => "oddFooterL",
            Self::OddFooterC => "oddFooterC",
            Self::OddFooterR => "oddFooterR",
            Self::TitleFontFace => "titleFontFace",
            Self::TitleFontSize => "titleFontSize",
            Self::TitleFontStyle => "titleFontStyle",
            Self::TitleAlign => "titleAlign",
            Self::SubtitleFontFace => "subtitleFontFace",
            Self::SubtitleFontSize => "subtitleFontSize",
            Self::ComposerFontFace => "composerFontFace",
            Self::ComposerFontSize => "composerFontSize",
            Self::ComposerAlign => "composerAlign",
            Self::LyricistFontFace => "lyricistFontFace",
            Self::LyricistFontSize => "lyricistFontSize",
            Self::LyricistAlign => "lyricistAlign",
            Self::Styles => "STYLES",
        }
    }
}

// endregion: --- Sid

// region:    --- Supporting Types

/// Placement for elements relative to staff
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Placement {
    #[default]
    Above,
    Below,
}

/// Horizontal alignment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Font style flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl FontStyle {
    pub const NORMAL: Self = Self {
        bold: false,
        italic: false,
        underline: false,
    };
    pub const BOLD: Self = Self {
        bold: true,
        italic: false,
        underline: false,
    };
    pub const ITALIC: Self = Self {
        bold: false,
        italic: true,
        underline: false,
    };
    pub const BOLD_ITALIC: Self = Self {
        bold: true,
        italic: true,
        underline: false,
    };
}

/// Chord style (standard notation vs jazz)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChordStyleType {
    #[default]
    Standard,
    Jazz,
}

/// Frame type for text elements
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FrameType {
    #[default]
    None,
    Square,
    Circle,
}

/// RGBA color
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

impl Color {
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
}

// endregion: --- Supporting Types

// region:    --- MStyle

/// Main style container (equivalent to MuseScore's MStyle).
///
/// Stores all style properties and provides typed accessors.
/// Properties are stored in an array indexed by Sid for O(1) access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MStyle {
    values: Vec<StyleValue>,
}

impl Default for MStyle {
    fn default() -> Self {
        Self::new()
    }
}

impl MStyle {
    /// Create a new style with all default values.
    #[must_use]
    pub fn new() -> Self {
        let values = Sid::ALL.iter().map(|sid| default_value(*sid)).collect();
        Self { values }
    }

    /// Create a lead sheet style preset.
    #[must_use]
    pub fn lead_sheet() -> Self {
        let mut style = Self::new();
        // Lead sheet specific overrides
        style.set(Sid::EnableVerticalSpread, StyleValue::Bool(true));
        style.set(
            Sid::ChordStyle,
            StyleValue::ChordStyle(ChordStyleType::Standard),
        );
        style
    }

    /// Create a jazz lead sheet style preset.
    #[must_use]
    pub fn jazz_lead_sheet() -> Self {
        let mut style = Self::new();
        style.set(Sid::EnableVerticalSpread, StyleValue::Bool(true));
        style.set(
            Sid::ChordStyle,
            StyleValue::ChordStyle(ChordStyleType::Jazz),
        );
        // Jazz style uses slightly tighter spacing
        style.set(Sid::MinHarmonyDistance, StyleValue::Spatium(0.4));
        style
    }

    /// Get a style value by ID.
    #[must_use]
    pub fn get(&self, sid: Sid) -> &StyleValue {
        &self.values[sid as usize]
    }

    /// Set a style value by ID.
    pub fn set(&mut self, sid: Sid, value: StyleValue) {
        self.values[sid as usize] = value;
    }

    /// Check if a value equals the default.
    #[must_use]
    pub fn is_default(&self, sid: Sid) -> bool {
        self.get(sid) == &default_value(sid)
    }

    // ========================================================================
    // Typed accessors
    // ========================================================================

    /// Get a spatium value (in staff-space units).
    #[must_use]
    pub fn spatium(&self, sid: Sid) -> f32 {
        match self.get(sid) {
            StyleValue::Spatium(v) => *v,
            StyleValue::Real(v) => *v,
            _ => 0.0,
        }
    }

    /// Get a real (float) value.
    #[must_use]
    pub fn real(&self, sid: Sid) -> f32 {
        match self.get(sid) {
            StyleValue::Real(v) | StyleValue::Spatium(v) => *v,
            _ => 0.0,
        }
    }

    /// Get a boolean value.
    #[must_use]
    pub fn bool(&self, sid: Sid) -> bool {
        match self.get(sid) {
            StyleValue::Bool(v) => *v,
            _ => false,
        }
    }

    /// Get a string value.
    #[must_use]
    pub fn string(&self, sid: Sid) -> &str {
        match self.get(sid) {
            StyleValue::String(v) => v.as_str(),
            _ => "",
        }
    }

    /// Get a color value.
    #[must_use]
    pub fn color(&self, sid: Sid) -> Color {
        match self.get(sid) {
            StyleValue::Color(v) => *v,
            _ => Color::BLACK,
        }
    }

    /// Get a placement value.
    #[must_use]
    pub fn placement(&self, sid: Sid) -> Placement {
        match self.get(sid) {
            StyleValue::Placement(v) => *v,
            _ => Placement::Above,
        }
    }

    /// Get an alignment value.
    #[must_use]
    pub fn align(&self, sid: Sid) -> HAlign {
        match self.get(sid) {
            StyleValue::Align(v) => *v,
            _ => HAlign::Center,
        }
    }

    /// Get a font style value.
    #[must_use]
    pub fn font_style(&self, sid: Sid) -> FontStyle {
        match self.get(sid) {
            StyleValue::FontStyle(v) => *v,
            _ => FontStyle::NORMAL,
        }
    }

    /// Get the base spatium value in points.
    #[must_use]
    pub fn base_spatium(&self) -> f32 {
        self.real(Sid::Spatium)
    }

    // ========================================================================
    // Page layout helpers
    // ========================================================================

    /// Get margins for a specific page (respects two-sided setting).
    /// Page numbers are 1-indexed (page 1 is first page).
    #[must_use]
    pub fn margins_for_page(&self, page_num: usize) -> super::model::Margins {
        let is_odd = page_num % 2 == 1;
        let two_sided = self.bool(Sid::PageTwosided);

        // Convert inches to points (72 points per inch)
        const INCH_TO_PT: f32 = 72.0;

        if !two_sided || is_odd {
            super::model::Margins {
                top: self.real(Sid::PageOddTopMargin) * INCH_TO_PT,
                bottom: self.real(Sid::PageOddBottomMargin) * INCH_TO_PT,
                left: self.real(Sid::PageOddLeftMargin) * INCH_TO_PT,
                right: self.calculated_right_margin(true) * INCH_TO_PT,
            }
        } else {
            super::model::Margins {
                top: self.real(Sid::PageEvenTopMargin) * INCH_TO_PT,
                bottom: self.real(Sid::PageEvenBottomMargin) * INCH_TO_PT,
                left: self.real(Sid::PageEvenLeftMargin) * INCH_TO_PT,
                right: self.calculated_right_margin(false) * INCH_TO_PT,
            }
        }
    }

    /// Calculate right margin from page width, printable width, and left margin.
    fn calculated_right_margin(&self, is_odd: bool) -> f32 {
        let page_width = self.real(Sid::PageWidth);
        let printable_width = self.real(Sid::PagePrintableWidth);
        let left_margin = if is_odd {
            self.real(Sid::PageOddLeftMargin)
        } else {
            self.real(Sid::PageEvenLeftMargin)
        };
        page_width - printable_width - left_margin
    }

    /// Get page dimensions in points.
    #[must_use]
    pub fn page_dimensions_pt(&self) -> (f32, f32) {
        const INCH_TO_PT: f32 = 72.0;
        (
            self.real(Sid::PageWidth) * INCH_TO_PT,
            self.real(Sid::PageHeight) * INCH_TO_PT,
        )
    }
}

// endregion: --- MStyle

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sid_count() {
        // Verify the count matches actual enum variants
        assert_eq!(Sid::COUNT, 79);
    }

    #[test]
    fn test_default_style() {
        let style = MStyle::new();

        // Check some default values (US Letter: 8.5")
        assert!((style.real(Sid::PageWidth) - 8.5).abs() < 0.01);
        assert!((style.spatium(Sid::BarNoteDistance) - 1.5).abs() < 0.01);
        assert!(style.bool(Sid::PageTwosided));
    }

    #[test]
    fn test_lead_sheet_preset() {
        let style = MStyle::lead_sheet();
        assert!(style.bool(Sid::EnableVerticalSpread));
    }

    #[test]
    fn test_margins_for_page() {
        let style = MStyle::new();

        let margins_p1 = style.margins_for_page(1);
        let margins_p2 = style.margins_for_page(2);

        // Both should have same margins by default
        assert!((margins_p1.top - margins_p2.top).abs() < 0.1);
    }

    #[test]
    fn test_set_and_get() {
        let mut style = MStyle::new();

        style.set(Sid::BarNoteDistance, StyleValue::Spatium(2.0));
        assert!((style.spatium(Sid::BarNoteDistance) - 2.0).abs() < 0.01);

        style.set(Sid::EnableVerticalSpread, StyleValue::Bool(false));
        assert!(!style.bool(Sid::EnableVerticalSpread));
    }
}

// endregion: --- Tests
