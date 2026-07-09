//! Style property definitions and default values.
//!
//! This module contains the StyleValue enum and default values for all style
//! properties. Default values are based on MuseScore's styledef.cpp.

use super::{ChordStyleType, Color, FontStyle, FrameType, HAlign, Placement, Sid};
use serde::{Deserialize, Serialize};

/// Style property value with type information.
///
/// Each variant corresponds to a different property type in MuseScore:
/// - Spatium: Values that scale with staff space (musical unit)
/// - Real: Plain floating point values
/// - Bool: Boolean flags
/// - String: Text values
/// - Color: RGBA colors
/// - Point: X, Y offset pairs
/// - Various enum types for specific properties
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StyleValue {
    /// Value in staff-space units (spatium)
    Spatium(f32),
    /// Plain floating point value
    Real(f32),
    /// Boolean value
    Bool(bool),
    /// Integer value
    Int(i32),
    /// String value
    String(String),
    /// RGBA color
    Color(Color),
    /// X, Y offset point
    Point(f32, f32),
    /// Placement (above/below)
    Placement(Placement),
    /// Horizontal alignment
    Align(HAlign),
    /// Font style (bold/italic/underline)
    FontStyle(FontStyle),
    /// Chord notation style
    ChordStyle(ChordStyleType),
    /// Frame type for text elements
    FrameType(FrameType),
}

/// Get the default value for a style property.
///
/// Default values are based on MuseScore's styledef.cpp and represent
/// standard engraving conventions for lead sheet rendering.
#[must_use]
pub fn default_value(sid: Sid) -> StyleValue {
    match sid {
        // ====================================================================
        // Page Layout - US Letter default with 15mm margins
        // ====================================================================
        Sid::PageWidth => StyleValue::Real(8.5), // US Letter width in inches
        Sid::PageHeight => StyleValue::Real(11.0), // US Letter height in inches
        Sid::PagePrintableWidth => StyleValue::Real(7.32), // Width minus margins
        Sid::PageEvenLeftMargin => StyleValue::Real(0.59), // 15mm in inches
        Sid::PageEvenTopMargin => StyleValue::Real(0.59),
        Sid::PageEvenBottomMargin => StyleValue::Real(0.59),
        Sid::PageOddLeftMargin => StyleValue::Real(0.59),
        Sid::PageOddTopMargin => StyleValue::Real(0.59),
        Sid::PageOddBottomMargin => StyleValue::Real(0.59),
        Sid::PageTwosided => StyleValue::Bool(true),

        // ====================================================================
        // Staff & System Spacing
        // ====================================================================
        Sid::Spatium => StyleValue::Real(5.0), // 5pt = 1.75mm (MuseScore default)
        Sid::StaffDistance => StyleValue::Spatium(6.5),
        Sid::StaffUpperBorder => StyleValue::Spatium(7.0),
        Sid::StaffLowerBorder => StyleValue::Spatium(7.0),
        Sid::MinSystemDistance => StyleValue::Spatium(8.5),
        Sid::MaxSystemDistance => StyleValue::Spatium(15.0),
        Sid::StaffHeaderFooterPadding => StyleValue::Spatium(1.0),
        Sid::EnableVerticalSpread => StyleValue::Bool(false),
        Sid::SpreadSystem => StyleValue::Real(2.5),
        Sid::MinSystemSpread => StyleValue::Spatium(8.5),
        Sid::MaxSystemSpread => StyleValue::Spatium(32.0),
        Sid::AlignSystemToMargin => StyleValue::Bool(true),

        // ====================================================================
        // Horizontal Spacing
        // ====================================================================
        Sid::BarNoteDistance => StyleValue::Spatium(1.5), // MuseScore default: 1.5
        Sid::NoteBarDistance => StyleValue::Spatium(1.0), // MuseScore default: 1.0
        Sid::MinNoteDistance => StyleValue::Spatium(0.5),
        Sid::MeasureSpacing => StyleValue::Real(1.5), // Horizontal stretch ratio
        Sid::BarWidth => StyleValue::Spatium(0.16),
        Sid::DoubleBarWidth => StyleValue::Spatium(0.16),
        Sid::DoubleBarDistance => StyleValue::Spatium(0.46),
        Sid::EndBarWidth => StyleValue::Spatium(0.5),
        Sid::EndBarDistance => StyleValue::Spatium(0.65),
        Sid::ClefLeftMargin => StyleValue::Spatium(0.64),

        // ====================================================================
        // Chord Symbols
        // ====================================================================
        Sid::HarmonyPlacement => StyleValue::Placement(Placement::Above),
        Sid::HarmonyPosAbove => StyleValue::Spatium(-2.5), // Negative = above staff
        Sid::HarmonyPosBelow => StyleValue::Spatium(3.5),
        Sid::MinHarmonyDistance => StyleValue::Spatium(0.5),
        Sid::MaxHarmonyBarDistance => StyleValue::Spatium(0.8),
        Sid::ChordSymbolAFontFace => StyleValue::String("MuseJazz".to_string()),
        Sid::ChordSymbolAFontSize => StyleValue::Real(12.0),
        Sid::ChordSymbolAFontStyle => StyleValue::FontStyle(FontStyle::NORMAL),
        Sid::ChordSymbolAColor => StyleValue::Color(Color::BLACK),
        Sid::ChordExtensionMag => StyleValue::Real(1.0),
        Sid::ChordModifierMag => StyleValue::Real(1.0),
        Sid::ChordStackedModifierMag => StyleValue::Real(0.75), // MuseScore default for superscripts
        Sid::ChordBassNoteScale => StyleValue::Real(1.0),
        Sid::ChordStyle => StyleValue::ChordStyle(ChordStyleType::Standard),
        Sid::HarmonyAlign => StyleValue::Align(HAlign::Left),

        // ====================================================================
        // Section Headers & Rehearsal Marks
        // ====================================================================
        Sid::RehearsalMarkPlacement => StyleValue::Placement(Placement::Above),
        Sid::RehearsalMarkPosAbove => StyleValue::Spatium(-3.0),
        Sid::RehearsalMarkFontFace => StyleValue::String("Leland Text".to_string()),
        Sid::RehearsalMarkFontSize => StyleValue::Real(14.0),
        Sid::RehearsalMarkFontStyle => StyleValue::FontStyle(FontStyle::BOLD),
        Sid::RehearsalMarkFrameType => StyleValue::FrameType(FrameType::Square),
        Sid::RehearsalMarkFramePadding => StyleValue::Spatium(0.5),
        Sid::RehearsalMarkFrameWidth => StyleValue::Spatium(0.16),

        // ====================================================================
        // Headers & Footers
        // ====================================================================
        Sid::ShowHeader => StyleValue::Bool(true),
        Sid::HeaderFirstPage => StyleValue::Bool(true),
        Sid::HeaderOddEven => StyleValue::Bool(false),
        Sid::OddHeaderL => StyleValue::String(String::new()),
        Sid::OddHeaderC => StyleValue::String(String::new()),
        Sid::OddHeaderR => StyleValue::String(String::new()),
        Sid::ShowFooter => StyleValue::Bool(true),
        Sid::FooterFirstPage => StyleValue::Bool(true),
        Sid::FooterOddEven => StyleValue::Bool(false),
        Sid::OddFooterL => StyleValue::String(String::new()),
        Sid::OddFooterC => StyleValue::String("$p".to_string()), // Page number
        Sid::OddFooterR => StyleValue::String(String::new()),

        // ====================================================================
        // Title/Composer Text - MuseScore default sizes
        // ====================================================================
        Sid::TitleFontFace => StyleValue::String("Chicago".to_string()),
        Sid::TitleFontSize => StyleValue::Real(24.0),
        Sid::TitleFontStyle => StyleValue::FontStyle(FontStyle::NORMAL),
        Sid::TitleAlign => StyleValue::Align(HAlign::Center),
        Sid::SubtitleFontFace => StyleValue::String("Chicago".to_string()),
        Sid::SubtitleFontSize => StyleValue::Real(14.0),
        Sid::ComposerFontFace => StyleValue::String("Chicago".to_string()),
        Sid::ComposerFontSize => StyleValue::Real(12.0),
        Sid::ComposerAlign => StyleValue::Align(HAlign::Right),
        Sid::LyricistFontFace => StyleValue::String("Chicago".to_string()),
        Sid::LyricistFontSize => StyleValue::Real(12.0),
        Sid::LyricistAlign => StyleValue::Align(HAlign::Left),

        // Sentinel - should never be accessed
        Sid::Styles => StyleValue::Int(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_defaults_defined() {
        // Ensure every Sid has a default value (no panic)
        for sid in Sid::ALL {
            let _ = default_value(sid);
        }
    }

    #[test]
    fn test_spatium_values() {
        // MuseScore defaults for key spacing values
        assert_eq!(
            default_value(Sid::BarNoteDistance),
            StyleValue::Spatium(1.5)
        );
        assert_eq!(
            default_value(Sid::NoteBarDistance),
            StyleValue::Spatium(1.0)
        );
        assert_eq!(
            default_value(Sid::MinSystemDistance),
            StyleValue::Spatium(8.5)
        );
    }

    #[test]
    fn test_page_defaults() {
        // US Letter page size
        if let StyleValue::Real(w) = default_value(Sid::PageWidth) {
            assert!((w - 8.5).abs() < 0.01);
        } else {
            panic!("PageWidth should be Real");
        }
    }

    #[test]
    fn test_chord_symbol_defaults() {
        assert_eq!(
            default_value(Sid::ChordStackedModifierMag),
            StyleValue::Real(0.75)
        );
    }
}
