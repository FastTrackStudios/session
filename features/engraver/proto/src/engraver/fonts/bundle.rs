//! Chart Font Bundle
//!
//! Provides a single source of truth for all fonts needed by the chart layout
//! and rendering pipeline. Both the REAPER extension and web app should use this
//! bundle to guarantee identical font configuration.

use std::sync::Arc;

use crate::engraver::layout::chart::ChartLayoutEngine;
#[cfg(feature = "wgpu")]
use crate::engraver::renderer::scene_renderer::VelloSceneRenderer;
use crate::engraver::style::MStyle;

use super::SMuFLFont;

// Embedded fonts — single source of truth for the entire workspace
static LELAND_FONT_BYTES: &[u8] = include_bytes!("../../../fonts/Leland.otf");
static LELAND_METADATA_BYTES: &[u8] = include_bytes!("../../../fonts/leland_metadata.json");
static LELAND_TEXT_FONT_BYTES: &[u8] = include_bytes!("../../../fonts/LelandText.otf");
static BRAVURA_FONT_BYTES: &[u8] = include_bytes!("../../../fonts/Bravura.otf");
static FREESANS_FONT_BYTES: &[u8] = include_bytes!("../../../fonts/FreeSans.ttf");
static MUSEJAZZ_FONT_BYTES: &[u8] = include_bytes!("../../../fonts/MuseJazz.otf");
static MUSEJAZZ_TEXT_FONT_BYTES: &[u8] = include_bytes!("../../../fonts/MuseJazzText.otf");
static CHICAGO_FLF_FONT_BYTES: &[u8] = include_bytes!("../../../fonts/ChicagoFLF.ttf");

/// All fonts needed for chart layout and rendering, bundled together.
///
/// This ensures both the REAPER extension and web app use exactly the same
/// fonts with the same configuration, making it impossible for them to diverge.
///
/// # Font roles
/// - **Text font** (`MuseJazz Text`): chord-symbol letters + numerals
///   (the jazz-style text font that pairs with MuseJazz the music font).
/// - **Symbol font** (`Leland`): SMuFL music notation font for noteheads,
///   rests, accidentals, and other music symbols. (MuseScore 4 default.)
/// - **Aux font** (`ChicagoFLF`): titles, headings, and other "regular
///   document text". A free Chicago revival by Robin Casady (public
///   domain) — matches the look of Tom Brooks-era Mac/Finale charts.
/// - **MuseJazz** (the music font): kept available for places that want
///   SMuFL-style jazz glyphs (cued in via PUA codepoints).
/// - **Leland Text**: kept available as a chord-symbol alternate and as a
///   fallback for SMuFL-text in case a chart references it directly.
pub struct ChartFontBundle {
    smufl_font: SMuFLFont<'static>,
    /// MuseJazz Text — chord symbol measurement + letter rendering.
    text_font_data: Arc<Vec<u8>>,
    /// MuseJazz — companion music font; chord-symbol PUA glyphs.
    musejazz_font_data: Arc<Vec<u8>>,
    /// Leland — SMuFL music notation symbols
    symbol_font_data: Arc<Vec<u8>>,
    /// ChicagoFLF — default document text font (titles / headings).
    aux_font_data: Arc<Vec<u8>>,
    /// Leland Text — alternate text font kept available by name.
    leland_text_font_data: Arc<Vec<u8>>,
    /// Bravura — kept available as a fallback / alternate SMuFL font
    bravura_font_data: Arc<Vec<u8>>,
    /// FreeSans — kept as a generic sans-serif fallback
    freesans_font_data: Arc<Vec<u8>>,
}

impl ChartFontBundle {
    /// Create a new font bundle with all embedded fonts loaded.
    ///
    /// # Errors
    /// Returns an error if the embedded Bravura font or metadata cannot be parsed.
    pub fn new() -> Result<Self, String> {
        let smufl_font = SMuFLFont::from_reader(
            LELAND_FONT_BYTES,
            std::io::Cursor::new(LELAND_METADATA_BYTES),
        )
        .map_err(|e| format!("Failed to load Leland font: {e}"))?;

        Ok(Self {
            smufl_font,
            text_font_data: Arc::new(MUSEJAZZ_TEXT_FONT_BYTES.to_vec()),
            musejazz_font_data: Arc::new(MUSEJAZZ_FONT_BYTES.to_vec()),
            symbol_font_data: Arc::new(LELAND_FONT_BYTES.to_vec()),
            aux_font_data: Arc::new(CHICAGO_FLF_FONT_BYTES.to_vec()),
            leland_text_font_data: Arc::new(LELAND_TEXT_FONT_BYTES.to_vec()),
            bravura_font_data: Arc::new(BRAVURA_FONT_BYTES.to_vec()),
            freesans_font_data: Arc::new(FREESANS_FONT_BYTES.to_vec()),
        })
    }

    /// Get the loaded SMuFL font (Bravura).
    #[must_use]
    pub fn smufl_font(&self) -> &SMuFLFont<'static> {
        &self.smufl_font
    }

    /// Get the text/chord font data (MuseJazzText).
    #[must_use]
    pub fn text_font_data(&self) -> &Arc<Vec<u8>> {
        &self.text_font_data
    }

    /// Get the symbol font data (Bravura).
    #[must_use]
    pub fn symbol_font_data(&self) -> &Arc<Vec<u8>> {
        &self.symbol_font_data
    }

    /// Get the auxiliary font data (Leland Text) for titles, section notes, etc.
    #[must_use]
    pub fn aux_font_data(&self) -> &Arc<Vec<u8>> {
        &self.aux_font_data
    }

    /// Get MuseJazz font data (the music-symbol companion to MuseJazz Text).
    #[must_use]
    pub fn musejazz_font_data(&self) -> &Arc<Vec<u8>> {
        &self.musejazz_font_data
    }

    /// Get Leland Text font data.
    #[must_use]
    pub fn leland_text_font_data(&self) -> &Arc<Vec<u8>> {
        &self.leland_text_font_data
    }

    /// Get ChicagoFLF (the default document text font) — same bytes as
    /// `aux_font_data` but named after its purpose.
    #[must_use]
    pub fn chicago_font_data(&self) -> &Arc<Vec<u8>> {
        &self.aux_font_data
    }

    /// Get Bravura font data (alternate SMuFL font kept for compatibility/fallback).
    #[must_use]
    pub fn bravura_font_data(&self) -> &Arc<Vec<u8>> {
        &self.bravura_font_data
    }

    /// Get FreeSans font data (generic sans-serif fallback).
    #[must_use]
    pub fn freesans_font_data(&self) -> &Arc<Vec<u8>> {
        &self.freesans_font_data
    }

    /// Create a correctly-wired `ChartLayoutEngine`.
    ///
    /// Uses MuseJazz for text metrics (chord width measurement) and Bravura for
    /// symbol font, matching the web app's configuration exactly.
    #[must_use]
    pub fn create_layout_engine(&self, style: &'static MStyle) -> ChartLayoutEngine {
        ChartLayoutEngine::new(
            style,
            self.text_font_data.clone(),   // MuseJazz — chord measurement
            self.symbol_font_data.clone(), // Bravura — symbols
        )
    }

    /// Configure a `VelloSceneRenderer` with all required named fonts.
    ///
    /// Registers the SMuFL font, text font, and all named font aliases so that
    /// `PaintCommand::Text` references resolve correctly. This is the canonical
    /// font configuration — both REAPER and web should use this method.
    ///
    /// Only available with the `wgpu` (GPU renderer) feature.
    #[cfg(feature = "wgpu")]
    #[must_use]
    pub fn configure_renderer<'a>(
        &'a self,
        renderer: VelloSceneRenderer<'a>,
    ) -> VelloSceneRenderer<'a> {
        renderer
            .with_font(&self.smufl_font)
            // ChicagoFLF as the default text fallback — matches the Mac/Finale
            // chart aesthetic that titles/headings expect.
            .with_text_font_arc(self.aux_font_data.clone())
            // Chord-symbol jazz text font.
            .with_named_font_arc("MuseJazz Text", self.text_font_data.clone())
            .with_named_font_arc("MuseJazzText", self.text_font_data.clone())
            // MuseJazz (the music font) — distinct from MuseJazz Text.
            .with_named_font_arc("MuseJazz", self.musejazz_font_data.clone())
            // Leland (SMuFL) + Leland Text (text companion).
            .with_named_font_arc("Leland", self.symbol_font_data.clone())
            .with_named_font_arc("Leland Text", self.leland_text_font_data.clone())
            .with_named_font_arc("LelandText", self.leland_text_font_data.clone())
            // Style defaults reference "Edwin"; alias to Leland Text until/unless Edwin ships.
            .with_named_font_arc("Edwin", self.leland_text_font_data.clone())
            // Chicago — document text.
            .with_named_font_arc("Chicago", self.aux_font_data.clone())
            .with_named_font_arc("ChicagoFLF", self.aux_font_data.clone())
            .with_named_font_arc("Bravura", self.bravura_font_data.clone())
            .with_named_font_arc("FreeSans", self.freesans_font_data.clone())
            .with_named_font_arc("section-note", self.aux_font_data.clone())
            .with_named_font_arc("title-bold", self.aux_font_data.clone())
            .with_named_font_arc("part-name-bold", self.aux_font_data.clone())
    }
}
