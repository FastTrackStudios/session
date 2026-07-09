//! Unified color type with format conversions.
//!
//! This module provides a single `Color` type that can be constructed from
//! and converted to various color formats used throughout the codebase.

use facet::Facet;
use std::fmt;

/// A color represented as 24-bit RGB (0xRRGGBB).
///
/// This is the canonical color type used throughout the codebase.
/// It provides conversions to/from various formats:
/// - Hex integers: `0xRRGGBB`
/// - Hex strings: `"#rrggbb"` or `"#rgb"`
/// - RGB tuples: `(r, g, b)`
/// - CSS strings: `"rgb(r, g, b)"`
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
pub struct Color(u32);

impl Color {
    /// Black color (0x000000)
    pub const BLACK: Self = Self(0x000000);

    /// White color (0xFFFFFF)
    pub const WHITE: Self = Self(0xFFFFFF);

    /// Transparent (represented as black, but semantically different)
    pub const TRANSPARENT: Self = Self(0x000000);

    /// Create a color from RGB components.
    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    /// Create a color from a hex integer (0xRRGGBB).
    #[inline]
    pub const fn hex(value: u32) -> Self {
        Self(value & 0xFFFFFF)
    }

    /// Parse a color from a CSS hex string.
    ///
    /// Supports formats:
    /// - `"#RGB"` (shorthand)
    /// - `"#RRGGBB"`
    /// - `"RGB"` (without hash)
    /// - `"RRGGBB"` (without hash)
    pub fn from_hex_str(s: &str) -> Result<Self, ColorParseError> {
        let s = s.trim().trim_start_matches('#');

        match s.len() {
            3 => {
                // Shorthand: #RGB -> #RRGGBB
                let r = u8::from_str_radix(&s[0..1], 16)
                    .map_err(|_| ColorParseError::InvalidHex(s.to_string()))?;
                let g = u8::from_str_radix(&s[1..2], 16)
                    .map_err(|_| ColorParseError::InvalidHex(s.to_string()))?;
                let b = u8::from_str_radix(&s[2..3], 16)
                    .map_err(|_| ColorParseError::InvalidHex(s.to_string()))?;
                Ok(Self::rgb(r * 17, g * 17, b * 17))
            }
            6 => {
                let value = u32::from_str_radix(s, 16)
                    .map_err(|_| ColorParseError::InvalidHex(s.to_string()))?;
                Ok(Self::hex(value))
            }
            _ => Err(ColorParseError::InvalidLength(s.len())),
        }
    }

    /// Parse a color from a CSS color string.
    ///
    /// Supports formats:
    /// - `"#RRGGBB"` or `"#RGB"`
    /// - `"rgb(r, g, b)"`
    /// - `"rgba(r, g, b, a)"` (alpha is ignored)
    pub fn from_css(s: &str) -> Result<Self, ColorParseError> {
        let s = s.trim();

        if s.starts_with('#') || s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Self::from_hex_str(s);
        }

        if s.starts_with("rgb") {
            // Parse rgb(r, g, b) or rgba(r, g, b, a)
            let inner = s
                .trim_start_matches("rgba")
                .trim_start_matches("rgb")
                .trim_start_matches('(')
                .trim_end_matches(')');

            let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
            if parts.len() < 3 {
                return Err(ColorParseError::InvalidFormat(s.to_string()));
            }

            let r: u8 = parts[0]
                .parse()
                .map_err(|_| ColorParseError::InvalidFormat(s.to_string()))?;
            let g: u8 = parts[1]
                .parse()
                .map_err(|_| ColorParseError::InvalidFormat(s.to_string()))?;
            let b: u8 = parts[2]
                .parse()
                .map_err(|_| ColorParseError::InvalidFormat(s.to_string()))?;

            return Ok(Self::rgb(r, g, b));
        }

        Err(ColorParseError::InvalidFormat(s.to_string()))
    }

    /// Get the red component (0-255).
    #[inline]
    pub const fn r(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    /// Get the green component (0-255).
    #[inline]
    pub const fn g(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    /// Get the blue component (0-255).
    #[inline]
    pub const fn b(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// Convert to a hex integer (0xRRGGBB).
    #[inline]
    pub const fn to_hex(self) -> u32 {
        self.0
    }

    /// Convert to an RGB tuple.
    #[inline]
    pub const fn to_rgb(self) -> (u8, u8, u8) {
        (self.r(), self.g(), self.b())
    }

    /// Convert to an RGBA tuple with the given alpha.
    #[inline]
    pub const fn to_rgba(self, alpha: u8) -> (u8, u8, u8, u8) {
        (self.r(), self.g(), self.b(), alpha)
    }

    /// Convert to a CSS hex string (`#rrggbb`).
    pub fn to_hex_string(self) -> String {
        format!("#{:06x}", self.0)
    }

    /// Convert to a CSS rgb() string.
    pub fn to_css_rgb(self) -> String {
        format!("rgb({}, {}, {})", self.r(), self.g(), self.b())
    }

    /// Convert to a CSS rgba() string with the given alpha (0.0-1.0).
    pub fn to_css_rgba(self, alpha: f32) -> String {
        format!(
            "rgba({}, {}, {}, {:.3})",
            self.r(),
            self.g(),
            self.b(),
            alpha.clamp(0.0, 1.0)
        )
    }

    /// Convert to REAPER's native color format.
    ///
    /// REAPER uses platform-specific byte ordering:
    /// - Windows: 0x00BBGGRR (BGR)
    /// - macOS/Linux: 0x00RRGGBB (RGB)
    ///
    /// The high bit (0x01000000) indicates a custom color is set.
    #[cfg(target_os = "windows")]
    pub const fn to_reaper_native(self) -> i32 {
        // Windows: BGR format
        let bgr = ((self.b() as u32) << 16) | ((self.g() as u32) << 8) | (self.r() as u32);
        (bgr | 0x01000000) as i32
    }

    #[cfg(not(target_os = "windows"))]
    pub const fn to_reaper_native(self) -> i32 {
        // macOS/Linux: RGB format
        (self.0 | 0x01000000) as i32
    }

    /// Create from REAPER's native color format.
    #[cfg(target_os = "windows")]
    pub const fn from_reaper_native(value: i32) -> Self {
        // Windows: BGR format
        let value = (value as u32) & 0xFFFFFF;
        let r = (value & 0xFF) as u8;
        let g = ((value >> 8) & 0xFF) as u8;
        let b = ((value >> 16) & 0xFF) as u8;
        Self::rgb(r, g, b)
    }

    #[cfg(not(target_os = "windows"))]
    pub const fn from_reaper_native(value: i32) -> Self {
        // macOS/Linux: RGB format
        Self::hex((value as u32) & 0xFFFFFF)
    }

    /// Convert to HSL (Hue, Saturation, Lightness).
    ///
    /// Returns (h, s, l) where:
    /// - h is in degrees (0-360)
    /// - s is 0.0-1.0
    /// - l is 0.0-1.0
    pub fn to_hsl(self) -> (f32, f32, f32) {
        let r = self.r() as f32 / 255.0;
        let g = self.g() as f32 / 255.0;
        let b = self.b() as f32 / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;

        if (max - min).abs() < f32::EPSILON {
            return (0.0, 0.0, l);
        }

        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };

        let h = if (max - r).abs() < f32::EPSILON {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if (max - g).abs() < f32::EPSILON {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };

        (h * 60.0, s, l)
    }

    /// Create a color from HSL values.
    ///
    /// - h: hue in degrees (0-360)
    /// - s: saturation (0.0-1.0)
    /// - l: lightness (0.0-1.0)
    pub fn from_hsl(h: f32, s: f32, l: f32) -> Self {
        let s = s.clamp(0.0, 1.0);
        let l = l.clamp(0.0, 1.0);
        let h = h.rem_euclid(360.0);

        if s.abs() < f32::EPSILON {
            let v = (l * 255.0) as u8;
            return Self::rgb(v, v, v);
        }

        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;

        fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
            let t = t.rem_euclid(1.0);
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        }

        let h = h / 360.0;
        let r = (hue_to_rgb(p, q, h + 1.0 / 3.0) * 255.0) as u8;
        let g = (hue_to_rgb(p, q, h) * 255.0) as u8;
        let b = (hue_to_rgb(p, q, h - 1.0 / 3.0) * 255.0) as u8;

        Self::rgb(r, g, b)
    }

    /// Lighten the color by the given amount (0.0-1.0).
    pub fn lighten(self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl(h, s, (l + amount).clamp(0.0, 1.0))
    }

    /// Darken the color by the given amount (0.0-1.0).
    pub fn darken(self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl(h, s, (l - amount).clamp(0.0, 1.0))
    }

    /// Saturate the color by the given amount (0.0-1.0).
    pub fn saturate(self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl(h, (s + amount).clamp(0.0, 1.0), l)
    }

    /// Desaturate the color by the given amount (0.0-1.0).
    pub fn desaturate(self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl(h, (s - amount).clamp(0.0, 1.0), l)
    }

    /// Mix this color with another color.
    ///
    /// `ratio` is the amount of the other color (0.0 = all self, 1.0 = all other).
    pub fn mix(self, other: Self, ratio: f32) -> Self {
        let ratio = ratio.clamp(0.0, 1.0);
        let inv = 1.0 - ratio;

        let r = (self.r() as f32 * inv + other.r() as f32 * ratio) as u8;
        let g = (self.g() as f32 * inv + other.g() as f32 * ratio) as u8;
        let b = (self.b() as f32 * inv + other.b() as f32 * ratio) as u8;

        Self::rgb(r, g, b)
    }

    /// Generate a gradient between two colors.
    ///
    /// Returns `steps` colors from `start` to `end` (inclusive of both endpoints).
    pub fn gradient(start: Self, end: Self, steps: usize) -> Vec<Self> {
        if steps == 0 {
            return vec![];
        }
        if steps == 1 {
            return vec![start];
        }

        (0..steps)
            .map(|i| {
                let ratio = i as f32 / (steps - 1) as f32;
                start.mix(end, ratio)
            })
            .collect()
    }

    /// Adjust the lightness to a specific value (0.0-1.0).
    pub fn with_lightness(self, lightness: f32) -> Self {
        let (h, s, _) = self.to_hsl();
        Self::from_hsl(h, s, lightness.clamp(0.0, 1.0))
    }

    /// Get a shade of this color (darker variant).
    ///
    /// `shade` is 0-9 where:
    /// - 0 is the lightest (50 in Tailwind)
    /// - 4 is the base color (500 in Tailwind)
    /// - 9 is the darkest (950 in Tailwind)
    pub fn shade(self, shade: u8) -> Self {
        let shade = shade.min(9);
        // Map shade to lightness: 0 -> 0.95, 4 -> 0.5, 9 -> 0.05
        let lightness = 0.95 - (shade as f32 * 0.1);
        self.with_lightness(lightness)
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Color(#{:06x})", self.0)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.0)
    }
}

impl From<u32> for Color {
    fn from(value: u32) -> Self {
        Self::hex(value)
    }
}

impl From<(u8, u8, u8)> for Color {
    fn from((r, g, b): (u8, u8, u8)) -> Self {
        Self::rgb(r, g, b)
    }
}

impl From<Color> for u32 {
    fn from(color: Color) -> Self {
        color.0
    }
}

impl From<Color> for (u8, u8, u8) {
    fn from(color: Color) -> Self {
        color.to_rgb()
    }
}

/// Error type for color parsing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ColorParseError {
    #[error("Invalid hex color: {0}")]
    InvalidHex(String),

    #[error("Invalid color format: {0}")]
    InvalidFormat(String),

    #[error("Invalid hex length: expected 3 or 6, got {0}")]
    InvalidLength(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_construction() {
        let color = Color::rgb(255, 128, 64);
        assert_eq!(color.r(), 255);
        assert_eq!(color.g(), 128);
        assert_eq!(color.b(), 64);
        assert_eq!(color.to_hex(), 0xFF8040);
    }

    #[test]
    fn test_hex_construction() {
        let color = Color::hex(0x3B82F6);
        assert_eq!(color.r(), 0x3B);
        assert_eq!(color.g(), 0x82);
        assert_eq!(color.b(), 0xF6);
    }

    #[test]
    fn test_from_hex_str() {
        assert_eq!(
            Color::from_hex_str("#3B82F6").unwrap(),
            Color::hex(0x3B82F6)
        );
        assert_eq!(Color::from_hex_str("3B82F6").unwrap(), Color::hex(0x3B82F6));
        assert_eq!(Color::from_hex_str("#F00").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(Color::from_hex_str("F00").unwrap(), Color::rgb(255, 0, 0));
    }

    #[test]
    fn test_from_css() {
        assert_eq!(Color::from_css("#3B82F6").unwrap(), Color::hex(0x3B82F6));
        assert_eq!(
            Color::from_css("rgb(59, 130, 246)").unwrap(),
            Color::hex(0x3B82F6)
        );
        assert_eq!(
            Color::from_css("rgba(59, 130, 246, 0.5)").unwrap(),
            Color::hex(0x3B82F6)
        );
    }

    #[test]
    fn test_to_hex_string() {
        assert_eq!(Color::hex(0x3B82F6).to_hex_string(), "#3b82f6");
        assert_eq!(Color::BLACK.to_hex_string(), "#000000");
        assert_eq!(Color::WHITE.to_hex_string(), "#ffffff");
    }

    #[test]
    fn test_hsl_roundtrip() {
        let original = Color::hex(0x3B82F6);
        let (h, s, l) = original.to_hsl();
        let restored = Color::from_hsl(h, s, l);
        // Allow for small rounding errors
        assert!((original.r() as i16 - restored.r() as i16).abs() <= 1);
        assert!((original.g() as i16 - restored.g() as i16).abs() <= 1);
        assert!((original.b() as i16 - restored.b() as i16).abs() <= 1);
    }

    #[test]
    fn test_lighten_darken() {
        let color = Color::hex(0x3B82F6);
        let lighter = color.lighten(0.2);
        let darker = color.darken(0.2);

        let (_, _, l_orig) = color.to_hsl();
        let (_, _, l_light) = lighter.to_hsl();
        let (_, _, l_dark) = darker.to_hsl();

        assert!(l_light > l_orig);
        assert!(l_dark < l_orig);
    }

    #[test]
    fn test_mix() {
        let black = Color::BLACK;
        let white = Color::WHITE;

        let mid = black.mix(white, 0.5);
        assert_eq!(mid.r(), 127);
        assert_eq!(mid.g(), 127);
        assert_eq!(mid.b(), 127);
    }

    #[test]
    fn test_gradient() {
        let gradient = Color::gradient(Color::BLACK, Color::WHITE, 5);
        assert_eq!(gradient.len(), 5);
        assert_eq!(gradient[0], Color::BLACK);
        assert_eq!(gradient[4], Color::WHITE);
    }
}
