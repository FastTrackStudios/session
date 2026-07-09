//! SMuFL font loading and glyph management.
//!
//! This module handles loading SMuFL-compliant fonts (like Bravura) and
//! provides access to glyph metrics and anchor points for precise positioning.
//!
//! Uses the `smufl` crate for proper metadata parsing.

// region:    --- Modules

pub mod bundle;
// Glyph tessellation (lyon) produces vertex buffers for the WGPU renderer;
// only needed with the `wgpu` feature. The SVG path uses outlines.
#[cfg(feature = "wgpu")]
pub mod tessellation;

// endregion: --- Modules

// region:    --- Re-exports

use kurbo::BezPath;
use skrifa::instance::Size;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::LocationRef;
use skrifa::{FontRef, MetadataProvider};
use std::io::BufReader;
use std::path::Path;

// Re-export key types from smufl crate
pub use bundle::ChartFontBundle;
pub use smufl::{Glyph, Metadata as SMuFLMetadata, StaffSpaces};

// Re-export tessellation utilities (GPU renderer only)
#[cfg(feature = "wgpu")]
pub use tessellation::{
    GlyphVertex, GlyphVertexConstructor, LyonPen, TessellatedGlyph, get_glyph_id, tessellate_glyph,
    tessellate_glyph_to_ndc,
};

// endregion: --- Re-exports

// region:    --- SMuFLFont

/// A loaded SMuFL font with its metadata.
pub struct SMuFLFont<'a> {
    /// The font data for rendering
    font: FontRef<'a>,
    /// SMuFL metadata (bounding boxes, anchors, engraving defaults)
    metadata: SMuFLMetadata,
}

/// Minimal valid TrueType font with required tables.
/// This is a minimal font that skrifa can parse, used for fallback/testing.
/// Contains: head, hhea, maxp, cmap, post, name tables
///
/// Public alias for tests (internal modules can use this for testing)
#[cfg(test)]
pub static EMPTY_FONT_DATA_FOR_TESTS: &[u8] = EMPTY_FONT_DATA;

#[rustfmt::skip]
static EMPTY_FONT_DATA: &[u8] = &[
    // Offset table: TrueType (0x00010000), 6 tables
    0x00, 0x01, 0x00, 0x00, // sfntVersion
    0x00, 0x06,             // numTables
    0x00, 0x40,             // searchRange
    0x00, 0x02,             // entrySelector
    0x00, 0x20,             // rangeShift
    // Table records (tag, checksum, offset, length)
    // cmap at 0x5C, length 0x14
    b'c', b'm', b'a', b'p', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5C, 0x00, 0x00, 0x00, 0x14,
    // head at 0x70, length 0x36
    b'h', b'e', b'a', b'd', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x36,
    // hhea at 0xA8, length 0x24
    b'h', b'h', b'e', b'a', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA8, 0x00, 0x00, 0x00, 0x24,
    // maxp at 0xCC, length 0x06
    b'm', b'a', b'x', b'p', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xCC, 0x00, 0x00, 0x00, 0x06,
    // name at 0xD4, length 0x1A
    b'n', b'a', b'm', b'e', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xD4, 0x00, 0x00, 0x00, 0x1A,
    // post at 0xF0, length 0x20
    b'p', b'o', b's', b't', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00, 0x20,
    // cmap table: format 0 (simple)
    0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0C,
    0x00, 0x04, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00,
    // head table
    0x00, 0x01, 0x00, 0x00, // version
    0x00, 0x01, 0x00, 0x00, // fontRevision
    0x00, 0x00, 0x00, 0x00, // checkSumAdjustment
    0x5F, 0x0F, 0x3C, 0xF5, // magicNumber
    0x00, 0x0B,             // flags
    0x04, 0x00,             // unitsPerEm (1024)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // created
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // modified
    0x00, 0x00,             // xMin
    0x00, 0x00,             // yMin
    0x04, 0x00,             // xMax
    0x04, 0x00,             // yMax
    0x00, 0x00,             // macStyle
    0x00, 0x08,             // lowestRecPPEM
    0x00, 0x02,             // fontDirectionHint
    0x00, 0x01,             // indexToLocFormat
    0x00, 0x00,             // glyphDataFormat
    0x00, 0x00,             // padding
    // hhea table
    0x00, 0x01, 0x00, 0x00, // version
    0x03, 0x00,             // ascender
    0xFF, 0x00,             // descender
    0x00, 0x00,             // lineGap
    0x04, 0x00,             // advanceWidthMax
    0x00, 0x00,             // minLeftSideBearing
    0x00, 0x00,             // minRightSideBearing
    0x04, 0x00,             // xMaxExtent
    0x00, 0x01,             // caretSlopeRise
    0x00, 0x00,             // caretSlopeRun
    0x00, 0x00,             // caretOffset
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
    0x00, 0x00,             // metricDataFormat
    0x00, 0x00,             // numberOfHMetrics
    // maxp table (simple, no glyphs)
    0x00, 0x00, 0x50, 0x00, // version 0.5
    0x00, 0x00,             // numGlyphs
    // name table (minimal)
    0x00, 0x00, 0x00, 0x01, 0x00, 0x0C, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x04, 0x00, 0x00, 0x00, 0x00, b'E', b'm', b'p', b't', b'y', 0x00,
    // post table
    0x00, 0x03, 0x00, 0x00, // version 3.0
    0x00, 0x00, 0x00, 0x00, // italicAngle
    0xFF, 0x00,             // underlinePosition
    0x00, 0x50,             // underlineThickness
    0x00, 0x00, 0x00, 0x00, // isFixedPitch
    0x00, 0x00, 0x00, 0x00, // minMemType42
    0x00, 0x00, 0x00, 0x00, // maxMemType42
    0x00, 0x00, 0x00, 0x00, // minMemType1
    0x00, 0x00, 0x00, 0x00, // maxMemType1
];

impl<'a> SMuFLFont<'a> {
    /// Load a SMuFL font from font data and metadata.
    ///
    /// # Arguments
    /// * `font_data` - Raw font file bytes (OTF/TTF)
    /// * `metadata` - Pre-loaded SMuFL metadata
    ///
    /// # Errors
    /// Returns an error if the font cannot be parsed.
    pub fn new(
        font_data: &'a [u8],
        metadata: SMuFLMetadata,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let font = FontRef::new(font_data)?;
        Ok(Self { font, metadata })
    }

    /// Try to create an empty SMuFL font for minimal contexts.
    ///
    /// This font has no glyphs and minimal metadata. Use for layout operations
    /// that don't need actual font data, such as positioning calculations.
    ///
    /// # Errors
    /// Returns an error if the embedded font data or metadata is invalid.
    /// This should never happen with valid builds.
    pub fn try_empty() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let font = FontRef::new(EMPTY_FONT_DATA)?;

        // Minimal valid SMuFL metadata JSON
        let metadata_json = r#"{
            "fontName": "EmptyTestFont",
            "engravingDefaults": {},
            "glyphAdvanceWidths": {},
            "glyphsWithAnchors": {},
            "glyphBBoxes": {}
        }"#;

        let metadata = SMuFLMetadata::from_reader(metadata_json.as_bytes())?;

        Ok(Self { font, metadata })
    }

    /// Create an empty SMuFL font for minimal contexts.
    ///
    /// This font has no glyphs and minimal metadata. Use for layout operations
    /// that don't need actual font data, such as positioning calculations.
    ///
    /// # Panics
    /// Panics if the embedded font data is invalid (should never happen).
    #[must_use]
    pub fn empty() -> Self {
        Self::try_empty().expect("Built-in empty font and metadata should be valid")
    }

    /// Load a SMuFL font from font data and metadata JSON reader.
    ///
    /// # Errors
    /// Returns an error if the font or metadata cannot be parsed.
    pub fn from_reader<R: std::io::Read>(
        font_data: &'a [u8],
        metadata_reader: R,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let font = FontRef::new(font_data)?;
        let reader = BufReader::new(metadata_reader);
        let metadata = SMuFLMetadata::from_reader(reader)?;
        Ok(Self { font, metadata })
    }

    /// Get the font reference for text shaping/rendering.
    #[must_use]
    pub fn font(&self) -> &FontRef<'a> {
        &self.font
    }

    /// Get the SMuFL metadata.
    #[must_use]
    pub fn metadata(&self) -> &SMuFLMetadata {
        &self.metadata
    }

    /// Get the advance width of a glyph in staff spaces.
    #[must_use]
    pub fn advance_width(&self, glyph: Glyph) -> Option<StaffSpaces> {
        self.metadata.advance_widths.get(glyph)
    }

    /// Get the bounding box of a glyph.
    #[must_use]
    pub fn bounding_box(&self, glyph: Glyph) -> Option<smufl::BoundingBox> {
        self.metadata.bounding_boxes.get(glyph)
    }

    /// Get anchor points for a glyph.
    #[must_use]
    pub fn anchors(&self, glyph: Glyph) -> Option<smufl::Anchors> {
        self.metadata.anchors.get(glyph)
    }

    /// Get the font's units per em.
    #[must_use]
    pub fn units_per_em(&self) -> u16 {
        use skrifa::raw::TableProvider;
        self.font
            .head()
            .ok()
            .map_or(1000, |head| head.units_per_em())
    }

    /// Get the staff line thickness from engraving defaults.
    #[must_use]
    pub fn staff_line_thickness(&self) -> Option<StaffSpaces> {
        self.metadata.engraving_defaults.staff_line_thickness
    }

    /// Get the stem thickness from engraving defaults.
    #[must_use]
    pub fn stem_thickness(&self) -> Option<StaffSpaces> {
        self.metadata.engraving_defaults.stem_thickness
    }

    /// Get the beam thickness from engraving defaults.
    #[must_use]
    pub fn beam_thickness(&self) -> Option<StaffSpaces> {
        self.metadata.engraving_defaults.beam_thickness
    }

    /// Get the BezPath for a glyph from the font.
    ///
    /// This returns the glyph outline at the specified size (in pixels/points),
    /// suitable for rendering with Vello or other vector graphics systems.
    ///
    /// # Arguments
    /// * `codepoint` - The Unicode codepoint of the glyph
    /// * `size` - The size to render the glyph at (in pixels/points)
    ///
    /// # Returns
    /// The glyph path, or `None` if the glyph doesn't exist in the font.
    #[must_use]
    pub fn get_glyph_path(&self, codepoint: char, size: f64) -> Option<BezPath> {
        // Look up glyph ID from codepoint
        let cmap = self.font.charmap();
        let glyph_id = cmap.map(codepoint)?;

        // Get outline glyphs
        let outline_glyphs = self.font.outline_glyphs();
        let outline = outline_glyphs.get(glyph_id)?;

        // Draw the glyph outline at the specified size
        let settings = DrawSettings::unhinted(Size::new(size as f32), LocationRef::default());

        let mut pen = BezPathPen::new();
        outline.draw(settings, &mut pen).ok()?;

        Some(pen.build())
    }
}

/// A pen that builds a kurbo BezPath from font glyph outlines.
///
/// Implements skrifa's `OutlinePen` trait to convert font outlines
/// into Vello-compatible BezPaths.
struct BezPathPen {
    path: BezPath,
}

impl BezPathPen {
    fn new() -> Self {
        Self {
            path: BezPath::new(),
        }
    }

    fn build(self) -> BezPath {
        self.path
    }
}

impl OutlinePen for BezPathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((x as f64, y as f64));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((x as f64, y as f64));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path
            .quad_to((cx0 as f64, cy0 as f64), (x as f64, y as f64));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(
            (cx0 as f64, cy0 as f64),
            (cx1 as f64, cy1 as f64),
            (x as f64, y as f64),
        );
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Load SMuFL metadata from a JSON file path.
///
/// # Errors
/// Returns an error if the file cannot be read or parsed.
pub fn load_metadata_from_path(
    path: impl AsRef<Path>,
) -> Result<SMuFLMetadata, Box<dyn std::error::Error + Send + Sync>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let metadata = SMuFLMetadata::from_reader(reader)?;
    Ok(metadata)
}

// endregion: --- SMuFLFont

// region:    --- Glyphs

/// Common SMuFL glyph constants for convenience.
/// These map to the `smufl::Glyph` enum variants.
pub mod glyphs {
    use super::Glyph;

    // Noteheads
    pub const NOTEHEAD_BLACK: Glyph = Glyph::NoteheadBlack;
    pub const NOTEHEAD_HALF: Glyph = Glyph::NoteheadHalf;
    pub const NOTEHEAD_WHOLE: Glyph = Glyph::NoteheadWhole;
    pub const NOTEHEAD_DOUBLE_WHOLE: Glyph = Glyph::NoteheadDoubleWhole;

    // Rests
    pub const REST_WHOLE: Glyph = Glyph::RestWhole;
    pub const REST_HALF: Glyph = Glyph::RestHalf;
    pub const REST_QUARTER: Glyph = Glyph::RestQuarter;
    pub const REST_8TH: Glyph = Glyph::Rest8th;
    pub const REST_16TH: Glyph = Glyph::Rest16th;
    pub const REST_32ND: Glyph = Glyph::Rest32nd;

    // Clefs
    pub const G_CLEF: Glyph = Glyph::GClef;
    pub const F_CLEF: Glyph = Glyph::FClef;
    pub const C_CLEF: Glyph = Glyph::CClef;

    // Accidentals
    pub const ACCIDENTAL_SHARP: Glyph = Glyph::AccidentalSharp;
    pub const ACCIDENTAL_FLAT: Glyph = Glyph::AccidentalFlat;
    pub const ACCIDENTAL_NATURAL: Glyph = Glyph::AccidentalNatural;
    pub const ACCIDENTAL_DOUBLE_SHARP: Glyph = Glyph::AccidentalDoubleSharp;
    pub const ACCIDENTAL_DOUBLE_FLAT: Glyph = Glyph::AccidentalDoubleFlat;

    // Flags
    pub const FLAG_8TH_UP: Glyph = Glyph::Flag8thUp;
    pub const FLAG_8TH_DOWN: Glyph = Glyph::Flag8thDown;
    pub const FLAG_16TH_UP: Glyph = Glyph::Flag16thUp;
    pub const FLAG_16TH_DOWN: Glyph = Glyph::Flag16thDown;

    // Time signatures
    pub const TIME_SIG_0: Glyph = Glyph::TimeSig0;
    pub const TIME_SIG_1: Glyph = Glyph::TimeSig1;
    pub const TIME_SIG_2: Glyph = Glyph::TimeSig2;
    pub const TIME_SIG_3: Glyph = Glyph::TimeSig3;
    pub const TIME_SIG_4: Glyph = Glyph::TimeSig4;
    pub const TIME_SIG_5: Glyph = Glyph::TimeSig5;
    pub const TIME_SIG_6: Glyph = Glyph::TimeSig6;
    pub const TIME_SIG_7: Glyph = Glyph::TimeSig7;
    pub const TIME_SIG_8: Glyph = Glyph::TimeSig8;
    pub const TIME_SIG_9: Glyph = Glyph::TimeSig9;
    pub const TIME_SIG_COMMON: Glyph = Glyph::TimeSigCommon;
    pub const TIME_SIG_CUT_COMMON: Glyph = Glyph::TimeSigCutCommon;

    // Dynamics
    pub const DYNAMIC_PIANO: Glyph = Glyph::DynamicPiano;
    pub const DYNAMIC_MEZZO: Glyph = Glyph::DynamicMezzo;
    pub const DYNAMIC_FORTE: Glyph = Glyph::DynamicForte;

    // Articulations
    pub const ARTIC_ACCENT_ABOVE: Glyph = Glyph::ArticAccentAbove;
    pub const ARTIC_STACCATO_ABOVE: Glyph = Glyph::ArticStaccatoAbove;
    pub const ARTIC_TENUTO_ABOVE: Glyph = Glyph::ArticTenutoAbove;

    // Fermatas
    pub const FERMATA_ABOVE: Glyph = Glyph::FermataAbove;
    pub const FERMATA_BELOW: Glyph = Glyph::FermataBelow;

    // Slash noteheads (for rhythmic notation / rhythm slashes)
    /// Slash notehead for quarter/eighth notes (filled)
    pub const NOTEHEAD_SLASH: Glyph = Glyph::NoteheadSlashHorizontalEnds;
    /// Slash notehead for half notes (open)
    pub const NOTEHEAD_SLASH_HALF: Glyph = Glyph::NoteheadSlashWhiteHalf;
    /// Slash notehead for whole notes (open)
    pub const NOTEHEAD_SLASH_WHOLE: Glyph = Glyph::NoteheadSlashWhiteWhole;
    /// Slash notehead for double whole notes
    pub const NOTEHEAD_SLASH_DOUBLE_WHOLE: Glyph = Glyph::NoteheadSlashWhiteDoubleWhole;
}

// endregion: --- Glyphs
