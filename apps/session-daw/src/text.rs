//! Text for the recorded scene.
//!
//! Vello draws GLYPHS, not strings: it wants glyph ids with positions,
//! which means the mapping from characters to ids and the advance
//! between them has to happen here. `skrifa` provides both from the font
//! file — the same font the DOM track panel asks for by name.
//!
//! # What this deliberately is not
//!
//! It is not a text shaper. There is no bidi, no script itemisation, no
//! ligature or kerning support, and a character the font has no glyph
//! for is dropped rather than substituted from elsewhere. Track names,
//! record inputs and index numbers are short, left-to-right, and drawn
//! into a fixed-width field, which is exactly the case a shaper is not
//! needed for.
//!
//! When something here needs real shaping — a user's track named in
//! Arabic, say — the answer is parley, not more code in this file. It is
//! written so that swap is a change of one type.

use eyre::{eyre, Result};
use skrifa::instance::Size;
use skrifa::metrics::GlyphMetrics;
use skrifa::{FontRef, MetadataProvider};
use vello::peniko::{Blob, FontData};

/// A loaded font, with the two tables laying text out needs.
pub struct Font {
    data: FontData,
    /// Retained so glyph lookup does not re-parse the file per string.
    inner: FontRef<'static>,
}

/// One positioned glyph, in the same shape `anyrender` records.
pub type Glyph = anyrender::Glyph;

impl Font {
    /// Load the embedded sans face.
    ///
    /// The bytes are compiled in, so a window cannot come up with a
    /// track panel that has no text in it because a file was missing at
    /// runtime — which is what a fontconfig lookup allows on a box that
    /// is not this one.
    ///
    /// # Errors
    ///
    /// If the embedded face does not parse. That cannot depend on the
    /// machine or the run — it would mean the build shipped a corrupt
    /// asset — but it is returned rather than panicked because the
    /// caller is opening a window and can say so.
    pub fn embedded() -> Result<Self> {
        /// `DejaVu Sans`: the fallback the track panel's CSS already
        /// names, so the canvas and the DOM ask for the same face.
        static BYTES: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

        let inner = FontRef::new(BYTES).map_err(|e| eyre!("the embedded font is corrupt: {e}"))?;
        Ok(Self {
            data: FontData::new(Blob::new(std::sync::Arc::new(BYTES)), 0),
            inner,
        })
    }

    #[must_use]
    pub const fn data(&self) -> &FontData {
        &self.data
    }

    /// Lay `text` out at `size`, starting from the origin.
    ///
    /// Positions are relative to the text's baseline-left, so the caller
    /// places the run with a transform rather than doing arithmetic on
    /// every glyph.
    #[must_use]
    pub fn layout(&self, text: &str, size: f32) -> Vec<Glyph> {
        let charmap = self.inner.charmap();
        let metrics = GlyphMetrics::new(&self.inner, Size::new(size), skrifa::instance::LocationRef::default());
        let mut x = 0.0_f32;
        let mut glyphs = Vec::with_capacity(text.len());
        for ch in text.chars() {
            // A character the face cannot draw is skipped rather than
            // rendered as .notdef: a row of empty boxes reads as a
            // rendering bug, where a missing glyph reads as a missing
            // glyph.
            let Some(id) = charmap.map(ch) else { continue };
            glyphs.push(Glyph {
                id: id.to_u32(),
                x,
                y: 0.0,
            });
            x += metrics.advance_width(id).unwrap_or_default();
        }
        glyphs
    }

    /// How wide `text` would be at `size`, without building the run.
    ///
    /// Used to centre and to right-align, and to decide when a name has
    /// to be cut short.
    #[must_use]
    pub fn width(&self, text: &str, size: f32) -> f64 {
        let charmap = self.inner.charmap();
        let metrics = GlyphMetrics::new(&self.inner, Size::new(size), skrifa::instance::LocationRef::default());
        f64::from(
            text.chars()
                .filter_map(|ch| charmap.map(ch))
                .filter_map(|id| metrics.advance_width(id))
                .sum::<f32>(),
        )
    }

    /// The longest prefix of `text` that fits `max` wide.
    ///
    /// REAPER truncates rather than scrolling or shrinking, so a long
    /// name simply stops. An ellipsis is added when anything was cut,
    /// which is the only way to tell "Strings" from "Strings (Section
    /// A)" at this width.
    #[must_use]
    pub fn elide(&self, text: &str, size: f32, max: f64) -> String {
        if self.width(text, size) <= max {
            return text.to_owned();
        }
        let ellipsis = self.width("…", size);
        let mut out = String::new();
        let mut used = 0.0;
        for ch in text.chars() {
            let w = self.width(ch.encode_utf8(&mut [0_u8; 4]), size);
            if used + w + ellipsis > max {
                break;
            }
            out.push(ch);
            used += w;
        }
        out.push('…');
        out
    }
}
