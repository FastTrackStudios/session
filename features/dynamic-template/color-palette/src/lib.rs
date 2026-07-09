//! Unified color palette library.
//!
//! This crate provides:
//! - A unified [`Color`] type with conversions between formats
//! - A comprehensive Tailwind-style [`palette`] with 22 color families
//!
//! # Quick Start
//!
//! ```rust
//! use color_palette::{Color, palette};
//!
//! // Create colors in various ways
//! let blue = Color::hex(0x3B82F6);
//! let red = Color::rgb(239, 68, 68);
//! let green = Color::from_hex_str("#22C55E").unwrap();
//!
//! // Use the Tailwind palette
//! let sky_blue = palette::sky::S500;
//! let dark_purple = palette::purple::S800;
//!
//! // Color manipulation
//! let lighter = blue.lighten(0.2);
//! let darker = blue.darken(0.2);
//! let mixed = blue.mix(red, 0.5);
//! ```
//!
//! # Color Formats
//!
//! The [`Color`] type can be converted to/from:
//! - Hex integers: `0xRRGGBB`
//! - Hex strings: `"#rrggbb"` or `"#rgb"`
//! - RGB tuples: `(r, g, b)`
//! - CSS strings: `"rgb(r, g, b)"` or `"rgba(r, g, b, a)"`
//! - REAPER native format (platform-specific)
//!
//! # Palette Organization
//!
//! The palette is organized into color families, each with 11 shades:
//! - **50**: Lightest
//! - **100-400**: Light shades
//! - **500**: Base/default shade
//! - **600-900**: Dark shades
//! - **950**: Darkest
//!
//! Color families include: red, orange, amber, yellow, lime, green, emerald,
//! teal, cyan, sky, blue, indigo, violet, purple, fuchsia, pink, rose,
//! slate, gray, zinc, neutral, stone.

mod color;
pub mod palette;

pub use color::{Color, ColorParseError};
