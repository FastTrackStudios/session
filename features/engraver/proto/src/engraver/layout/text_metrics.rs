//! Text font metrics for accurate text measurement.
//!
//! Provides actual glyph advance widths from font files using skrifa,
//! similar to MuseScore's FontMetrics class.

use std::sync::Arc;

use skrifa::MetadataProvider;
use skrifa::charmap::Charmap;
use skrifa::metrics::GlyphMetrics;
use skrifa::prelude::Size;
use skrifa::raw::FileRef;

/// Text font metrics provider.
///
/// Wraps a font file and provides methods to measure actual glyph widths.
/// This is similar to MuseScore's FontMetrics class which uses an IFontProvider.
#[derive(Clone)]
pub struct TextFontMetrics {
    /// Font data (Arc for cheap cloning)
    font_data: Arc<Vec<u8>>,
}

impl TextFontMetrics {
    /// Create new font metrics from font data.
    #[must_use]
    pub fn new(font_data: Arc<Vec<u8>>) -> Self {
        Self { font_data }
    }

    /// Get the horizontal advance width for a string at the given font size.
    ///
    /// This uses actual glyph metrics from the font file, not estimation.
    #[must_use]
    pub fn horizontal_advance(&self, text: &str, font_size: f64) -> f64 {
        let font_ref = match FileRef::new(&self.font_data) {
            Ok(FileRef::Font(font)) => font,
            _ => return self.estimate_width(text, font_size),
        };

        let charmap = font_ref.charmap();
        let size = Size::new(font_size as f32);
        let glyph_metrics = font_ref.glyph_metrics(size, skrifa::instance::LocationRef::default());

        text.chars()
            .map(|c| self.char_advance(c, &charmap, &glyph_metrics, font_size))
            .sum()
    }

    /// Get advance width for a single character.
    fn char_advance(
        &self,
        c: char,
        charmap: &Charmap,
        glyph_metrics: &GlyphMetrics,
        font_size: f64,
    ) -> f64 {
        if let Some(gid) = charmap.map(c)
            && let Some(advance) = glyph_metrics.advance_width(gid)
        {
            return advance as f64;
        }
        // Fallback to estimation if glyph not found
        self.estimate_char_width(c, font_size)
    }

    /// Estimate character width as fallback.
    fn estimate_char_width(&self, c: char, font_size: f64) -> f64 {
        let ratio = match c {
            'm' | 'w' | 'M' | 'W' => 0.9,
            'A'..='Z' => 0.7,
            'a'..='z' => 0.5,
            '0'..='9' => 0.55,
            '/' => 0.35,
            ' ' => 0.28,
            _ => 0.55,
        };
        font_size * ratio
    }

    /// Estimate text width as fallback when font can't be loaded.
    fn estimate_width(&self, text: &str, font_size: f64) -> f64 {
        text.chars()
            .map(|c| self.estimate_char_width(c, font_size))
            .sum()
    }

    /// Get the cap-height of the font at the given size.
    ///
    /// Cap-height is the height of capital letters, used as the basis
    /// for MuseScore's spacing calculations.
    #[must_use]
    pub fn cap_height(&self, font_size: f64) -> f64 {
        let font_ref = match FileRef::new(&self.font_data) {
            Ok(FileRef::Font(font)) => font,
            _ => return font_size * 0.7, // Typical ratio
        };

        let size = Size::new(font_size as f32);
        let metrics = font_ref.metrics(size, skrifa::instance::LocationRef::default());

        // Cap height from font metrics, or estimate from ascent
        metrics.cap_height.map_or(font_size * 0.7, |h| h as f64)
    }

    /// Get the x-height (height of lowercase letters) at the given size.
    #[must_use]
    pub fn x_height(&self, font_size: f64) -> f64 {
        let font_ref = match FileRef::new(&self.font_data) {
            Ok(FileRef::Font(font)) => font,
            _ => return font_size * 0.5, // Typical ratio
        };

        let size = Size::new(font_size as f32);
        let metrics = font_ref.metrics(size, skrifa::instance::LocationRef::default());

        metrics.x_height.map_or(font_size * 0.5, |h| h as f64)
    }

    /// Visual height (outline bounding-box height, ink top to ink bottom) of a
    /// single glyph at the given size. Returns `0.0` if the glyph has no
    /// outline (e.g. a space) or can't be loaded.
    ///
    /// Unlike [`cap_height`](Self::cap_height) (a single font-global value),
    /// this is per-glyph — needed because some glyphs (digits in the MuseJazz
    /// chord font) are drawn shorter than capitals, so matching their visual
    /// size requires their actual ink height.
    #[must_use]
    pub fn glyph_height(&self, c: char, font_size: f64) -> f64 {
        use skrifa::outline::{DrawSettings, OutlinePen};

        struct BoundsPen {
            y_min: f32,
            y_max: f32,
            drawn: bool,
        }
        impl BoundsPen {
            fn at(&mut self, y: f32) {
                self.y_min = self.y_min.min(y);
                self.y_max = self.y_max.max(y);
                self.drawn = true;
            }
        }
        impl OutlinePen for BoundsPen {
            fn move_to(&mut self, _x: f32, y: f32) {
                self.at(y);
            }
            fn line_to(&mut self, _x: f32, y: f32) {
                self.at(y);
            }
            fn quad_to(&mut self, _cx: f32, cy: f32, _x: f32, y: f32) {
                self.at(cy);
                self.at(y);
            }
            fn curve_to(&mut self, _a: f32, b: f32, _c: f32, d: f32, _x: f32, y: f32) {
                self.at(b);
                self.at(d);
                self.at(y);
            }
            fn close(&mut self) {}
        }

        let Ok(FileRef::Font(font_ref)) = FileRef::new(&self.font_data) else {
            return 0.0;
        };
        let Some(gid) = font_ref.charmap().map(c) else {
            return 0.0;
        };
        let outlines = font_ref.outline_glyphs();
        let Some(glyph) = outlines.get(gid) else {
            return 0.0;
        };
        let mut pen = BoundsPen {
            y_min: f32::MAX,
            y_max: f32::MIN,
            drawn: false,
        };
        let settings = DrawSettings::unhinted(
            Size::new(font_size as f32),
            skrifa::instance::LocationRef::default(),
        );
        if glyph.draw(settings, &mut pen).is_err() || !pen.drawn {
            return 0.0;
        }
        (pen.y_max - pen.y_min) as f64
    }
}

impl std::fmt::Debug for TextFontMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextFontMetrics")
            .field("font_data_len", &self.font_data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_width_fallback() {
        // Empty font data will trigger estimation fallback
        let metrics = TextFontMetrics::new(Arc::new(vec![]));

        let width = metrics.horizontal_advance("Test", 12.0);
        assert!(width > 0.0);
    }
}
