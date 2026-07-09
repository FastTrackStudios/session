//! Tailwind-style color palette with shades from 50 (lightest) to 950 (darkest).
//!
//! This provides a comprehensive color palette based on Tailwind CSS colors.
//! Each color family has 11 shades: 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950.
//!
//! # Usage
//!
//! ```rust
//! use color_palette::palette;
//!
//! let blue = palette::blue::S500;  // Base blue
//! let light_blue = palette::blue::S200;  // Light variant
//! let dark_blue = palette::blue::S800;  // Dark variant
//! ```

use crate::Color;

/// Red shades - warm, energetic colors
/// Commonly used for: Drums, errors, warnings
pub mod red {
    use super::Color;
    pub const S50: Color = Color::hex(0xFEF2F2);
    pub const S100: Color = Color::hex(0xFEE2E2);
    pub const S200: Color = Color::hex(0xFECACA);
    pub const S300: Color = Color::hex(0xFCA5A5);
    pub const S400: Color = Color::hex(0xF87171);
    pub const S500: Color = Color::hex(0xEF4444);
    pub const S600: Color = Color::hex(0xDC2626);
    pub const S700: Color = Color::hex(0xB91C1C);
    pub const S800: Color = Color::hex(0x991B1B);
    pub const S900: Color = Color::hex(0x7F1D1D);
    pub const S950: Color = Color::hex(0x450A0A);
}

/// Orange shades - energetic, warm colors
/// Commonly used for: Percussion, snare drums
pub mod orange {
    use super::Color;
    pub const S50: Color = Color::hex(0xFFF7ED);
    pub const S100: Color = Color::hex(0xFFEDD5);
    pub const S200: Color = Color::hex(0xFED7AA);
    pub const S300: Color = Color::hex(0xFDBA74);
    pub const S400: Color = Color::hex(0xFB923C);
    pub const S500: Color = Color::hex(0xF97316);
    pub const S600: Color = Color::hex(0xEA580C);
    pub const S700: Color = Color::hex(0xC2410C);
    pub const S800: Color = Color::hex(0x9A3412);
    pub const S900: Color = Color::hex(0x7C2D12);
    pub const S950: Color = Color::hex(0x431407);
}

/// Amber shades - warm gold/honey colors
/// Commonly used for: Bass, horns, acoustic guitars
pub mod amber {
    use super::Color;
    pub const S50: Color = Color::hex(0xFFFBEB);
    pub const S100: Color = Color::hex(0xFEF3C7);
    pub const S200: Color = Color::hex(0xFDE68A);
    pub const S300: Color = Color::hex(0xFCD34D);
    pub const S400: Color = Color::hex(0xFBBF24);
    pub const S500: Color = Color::hex(0xF59E0B);
    pub const S600: Color = Color::hex(0xD97706);
    pub const S700: Color = Color::hex(0xB45309);
    pub const S800: Color = Color::hex(0x92400E);
    pub const S900: Color = Color::hex(0x78350F);
    pub const S950: Color = Color::hex(0x451A03);
}

/// Yellow shades - bright, attention-grabbing colors
/// Commonly used for: Hi-hats, cymbals, warnings
pub mod yellow {
    use super::Color;
    pub const S50: Color = Color::hex(0xFEFCE8);
    pub const S100: Color = Color::hex(0xFEF9C3);
    pub const S200: Color = Color::hex(0xFEF08A);
    pub const S300: Color = Color::hex(0xFDE047);
    pub const S400: Color = Color::hex(0xFACC15);
    pub const S500: Color = Color::hex(0xEAB308);
    pub const S600: Color = Color::hex(0xCA8A04);
    pub const S700: Color = Color::hex(0xA16207);
    pub const S800: Color = Color::hex(0x854D0E);
    pub const S900: Color = Color::hex(0x713F12);
    pub const S950: Color = Color::hex(0x422006);
}

/// Lime shades - vibrant yellow-green
/// Commonly used for: Vamp sections
pub mod lime {
    use super::Color;
    pub const S50: Color = Color::hex(0xF7FEE7);
    pub const S100: Color = Color::hex(0xECFCCB);
    pub const S200: Color = Color::hex(0xD9F99D);
    pub const S300: Color = Color::hex(0xBEF264);
    pub const S400: Color = Color::hex(0xA3E635);
    pub const S500: Color = Color::hex(0x84CC16);
    pub const S600: Color = Color::hex(0x65A30D);
    pub const S700: Color = Color::hex(0x4D7C0F);
    pub const S800: Color = Color::hex(0x3F6212);
    pub const S900: Color = Color::hex(0x365314);
    pub const S950: Color = Color::hex(0x1A2E05);
}

/// Green shades - natural, balanced colors
/// Commonly used for: Keys, verse sections, success states
pub mod green {
    use super::Color;
    pub const S50: Color = Color::hex(0xF0FDF4);
    pub const S100: Color = Color::hex(0xDCFCE7);
    pub const S200: Color = Color::hex(0xBBF7D0);
    pub const S300: Color = Color::hex(0x86EFAC);
    pub const S400: Color = Color::hex(0x4ADE80);
    pub const S500: Color = Color::hex(0x22C55E);
    pub const S600: Color = Color::hex(0x16A34A);
    pub const S700: Color = Color::hex(0x15803D);
    pub const S800: Color = Color::hex(0x166534);
    pub const S900: Color = Color::hex(0x14532D);
    pub const S950: Color = Color::hex(0x052E16);
}

/// Emerald shades - rich blue-green
/// Commonly used for: Acoustic guitars, woodwinds
pub mod emerald {
    use super::Color;
    pub const S50: Color = Color::hex(0xECFDF5);
    pub const S100: Color = Color::hex(0xD1FAE5);
    pub const S200: Color = Color::hex(0xA7F3D0);
    pub const S300: Color = Color::hex(0x6EE7B7);
    pub const S400: Color = Color::hex(0x34D399);
    pub const S500: Color = Color::hex(0x10B981);
    pub const S600: Color = Color::hex(0x059669);
    pub const S700: Color = Color::hex(0x047857);
    pub const S800: Color = Color::hex(0x065F46);
    pub const S900: Color = Color::hex(0x064E3B);
    pub const S950: Color = Color::hex(0x022C22);
}

/// Teal shades - balanced blue-green
/// Commonly used for: SFX, instrumental sections
pub mod teal {
    use super::Color;
    pub const S50: Color = Color::hex(0xF0FDFA);
    pub const S100: Color = Color::hex(0xCCFBF1);
    pub const S200: Color = Color::hex(0x99F6E4);
    pub const S300: Color = Color::hex(0x5EEAD4);
    pub const S400: Color = Color::hex(0x2DD4BF);
    pub const S500: Color = Color::hex(0x14B8A6);
    pub const S600: Color = Color::hex(0x0D9488);
    pub const S700: Color = Color::hex(0x0F766E);
    pub const S800: Color = Color::hex(0x115E59);
    pub const S900: Color = Color::hex(0x134E4A);
    pub const S950: Color = Color::hex(0x042F2E);
}

/// Cyan shades - bright aqua colors
/// Commonly used for: Interlude sections
pub mod cyan {
    use super::Color;
    pub const S50: Color = Color::hex(0xECFEFF);
    pub const S100: Color = Color::hex(0xCFFAFE);
    pub const S200: Color = Color::hex(0xA5F3FC);
    pub const S300: Color = Color::hex(0x67E8F9);
    pub const S400: Color = Color::hex(0x22D3EE);
    pub const S500: Color = Color::hex(0x06B6D4);
    pub const S600: Color = Color::hex(0x0891B2);
    pub const S700: Color = Color::hex(0x0E7490);
    pub const S800: Color = Color::hex(0x155E75);
    pub const S900: Color = Color::hex(0x164E63);
    pub const S950: Color = Color::hex(0x083344);
}

/// Sky shades - light, airy blue
/// Commonly used for: Electric guitars
pub mod sky {
    use super::Color;
    pub const S50: Color = Color::hex(0xF0F9FF);
    pub const S100: Color = Color::hex(0xE0F2FE);
    pub const S200: Color = Color::hex(0xBAE6FD);
    pub const S300: Color = Color::hex(0x7DD3FC);
    pub const S400: Color = Color::hex(0x38BDF8);
    pub const S500: Color = Color::hex(0x0EA5E9);
    pub const S600: Color = Color::hex(0x0284C7);
    pub const S700: Color = Color::hex(0x0369A1);
    pub const S800: Color = Color::hex(0x075985);
    pub const S900: Color = Color::hex(0x0C4A6E);
    pub const S950: Color = Color::hex(0x082F49);
}

/// Blue shades - classic, professional blue
/// Commonly used for: Intro sections, default highlight color
pub mod blue {
    use super::Color;
    pub const S50: Color = Color::hex(0xEFF6FF);
    pub const S100: Color = Color::hex(0xDBEAFE);
    pub const S200: Color = Color::hex(0xBFDBFE);
    pub const S300: Color = Color::hex(0x93C5FD);
    pub const S400: Color = Color::hex(0x60A5FA);
    pub const S500: Color = Color::hex(0x3B82F6);
    pub const S600: Color = Color::hex(0x2563EB);
    pub const S700: Color = Color::hex(0x1D4ED8);
    pub const S800: Color = Color::hex(0x1E40AF);
    pub const S900: Color = Color::hex(0x1E3A8A);
    pub const S950: Color = Color::hex(0x172554);
}

/// Indigo shades - deep blue-violet
/// Commonly used for: Outro sections, synth bass
pub mod indigo {
    use super::Color;
    pub const S50: Color = Color::hex(0xEEF2FF);
    pub const S100: Color = Color::hex(0xE0E7FF);
    pub const S200: Color = Color::hex(0xC7D2FE);
    pub const S300: Color = Color::hex(0xA5B4FC);
    pub const S400: Color = Color::hex(0x818CF8);
    pub const S500: Color = Color::hex(0x6366F1);
    pub const S600: Color = Color::hex(0x4F46E5);
    pub const S700: Color = Color::hex(0x4338CA);
    pub const S800: Color = Color::hex(0x3730A3);
    pub const S900: Color = Color::hex(0x312E81);
    pub const S950: Color = Color::hex(0x1E1B4B);
}

/// Violet shades - rich purple-blue
/// Commonly used for: Synths, bridge sections
pub mod violet {
    use super::Color;
    pub const S50: Color = Color::hex(0xF5F3FF);
    pub const S100: Color = Color::hex(0xEDE9FE);
    pub const S200: Color = Color::hex(0xDDD6FE);
    pub const S300: Color = Color::hex(0xC4B5FD);
    pub const S400: Color = Color::hex(0xA78BFA);
    pub const S500: Color = Color::hex(0x8B5CF6);
    pub const S600: Color = Color::hex(0x7C3AED);
    pub const S700: Color = Color::hex(0x6D28D9);
    pub const S800: Color = Color::hex(0x5B21B6);
    pub const S900: Color = Color::hex(0x4C1D95);
    pub const S950: Color = Color::hex(0x2E1065);
}

/// Purple shades - classic purple
/// Commonly used for: Orchestra, bridge sections
pub mod purple {
    use super::Color;
    pub const S50: Color = Color::hex(0xFAF5FF);
    pub const S100: Color = Color::hex(0xF3E8FF);
    pub const S200: Color = Color::hex(0xE9D5FF);
    pub const S300: Color = Color::hex(0xD8B4FE);
    pub const S400: Color = Color::hex(0xC084FC);
    pub const S500: Color = Color::hex(0xA855F7);
    pub const S600: Color = Color::hex(0x9333EA);
    pub const S700: Color = Color::hex(0x7E22CE);
    pub const S800: Color = Color::hex(0x6B21A8);
    pub const S900: Color = Color::hex(0x581C87);
    pub const S950: Color = Color::hex(0x3B0764);
}

/// Fuchsia shades - vibrant magenta-purple
/// Commonly used for: Special effects, accents
pub mod fuchsia {
    use super::Color;
    pub const S50: Color = Color::hex(0xFDF4FF);
    pub const S100: Color = Color::hex(0xFAE8FF);
    pub const S200: Color = Color::hex(0xF5D0FE);
    pub const S300: Color = Color::hex(0xF0ABFC);
    pub const S400: Color = Color::hex(0xE879F9);
    pub const S500: Color = Color::hex(0xD946EF);
    pub const S600: Color = Color::hex(0xC026D3);
    pub const S700: Color = Color::hex(0xA21CAF);
    pub const S800: Color = Color::hex(0x86198F);
    pub const S900: Color = Color::hex(0x701A75);
    pub const S950: Color = Color::hex(0x4A044E);
}

/// Pink shades - soft, warm pink
/// Commonly used for: Vocals, count-in, breakdown sections
pub mod pink {
    use super::Color;
    pub const S50: Color = Color::hex(0xFDF2F8);
    pub const S100: Color = Color::hex(0xFCE7F3);
    pub const S200: Color = Color::hex(0xFBCFE8);
    pub const S300: Color = Color::hex(0xF9A8D4);
    pub const S400: Color = Color::hex(0xF472B6);
    pub const S500: Color = Color::hex(0xEC4899);
    pub const S600: Color = Color::hex(0xDB2777);
    pub const S700: Color = Color::hex(0xBE185D);
    pub const S800: Color = Color::hex(0x9D174D);
    pub const S900: Color = Color::hex(0x831843);
    pub const S950: Color = Color::hex(0x500724);
}

/// Rose shades - deep pink/red
/// Commonly used for: Chorus sections, strings
pub mod rose {
    use super::Color;
    pub const S50: Color = Color::hex(0xFFF1F2);
    pub const S100: Color = Color::hex(0xFFE4E6);
    pub const S200: Color = Color::hex(0xFECDD3);
    pub const S300: Color = Color::hex(0xFDA4AF);
    pub const S400: Color = Color::hex(0xFB7185);
    pub const S500: Color = Color::hex(0xF43F5E);
    pub const S600: Color = Color::hex(0xE11D48);
    pub const S700: Color = Color::hex(0xBE123C);
    pub const S800: Color = Color::hex(0x9F1239);
    pub const S900: Color = Color::hex(0x881337);
    pub const S950: Color = Color::hex(0x4C0519);
}

/// Slate shades - cool blue-gray
/// Commonly used for: Guide tracks, reference, neutral UI elements
pub mod slate {
    use super::Color;
    pub const S50: Color = Color::hex(0xF8FAFC);
    pub const S100: Color = Color::hex(0xF1F5F9);
    pub const S200: Color = Color::hex(0xE2E8F0);
    pub const S300: Color = Color::hex(0xCBD5E1);
    pub const S400: Color = Color::hex(0x94A3B8);
    pub const S500: Color = Color::hex(0x64748B);
    pub const S600: Color = Color::hex(0x475569);
    pub const S700: Color = Color::hex(0x334155);
    pub const S800: Color = Color::hex(0x1E293B);
    pub const S900: Color = Color::hex(0x0F172A);
    pub const S950: Color = Color::hex(0x020617);
}

/// Gray shades - true neutral gray
/// Commonly used for: End sections, disabled states
pub mod gray {
    use super::Color;
    pub const S50: Color = Color::hex(0xF9FAFB);
    pub const S100: Color = Color::hex(0xF3F4F6);
    pub const S200: Color = Color::hex(0xE5E7EB);
    pub const S300: Color = Color::hex(0xD1D5DB);
    pub const S400: Color = Color::hex(0x9CA3AF);
    pub const S500: Color = Color::hex(0x6B7280);
    pub const S600: Color = Color::hex(0x4B5563);
    pub const S700: Color = Color::hex(0x374151);
    pub const S800: Color = Color::hex(0x1F2937);
    pub const S900: Color = Color::hex(0x111827);
    pub const S950: Color = Color::hex(0x030712);
}

/// Zinc shades - cool gray with slight blue tint
/// Commonly used for: UI backgrounds, borders
pub mod zinc {
    use super::Color;
    pub const S50: Color = Color::hex(0xFAFAFA);
    pub const S100: Color = Color::hex(0xF4F4F5);
    pub const S200: Color = Color::hex(0xE4E4E7);
    pub const S300: Color = Color::hex(0xD4D4D8);
    pub const S400: Color = Color::hex(0xA1A1AA);
    pub const S500: Color = Color::hex(0x71717A);
    pub const S600: Color = Color::hex(0x52525B);
    pub const S700: Color = Color::hex(0x3F3F46);
    pub const S800: Color = Color::hex(0x27272A);
    pub const S900: Color = Color::hex(0x18181B);
    pub const S950: Color = Color::hex(0x09090B);
}

/// Neutral shades - pure neutral gray (same as gray but semantic name)
pub mod neutral {
    pub use super::gray::*;
}

/// Stone shades - warm gray with slight brown tint
/// Commonly used for: Harmonica, earth tones
pub mod stone {
    use super::Color;
    pub const S50: Color = Color::hex(0xFAFAF9);
    pub const S100: Color = Color::hex(0xF5F5F4);
    pub const S200: Color = Color::hex(0xE7E5E4);
    pub const S300: Color = Color::hex(0xD6D3D1);
    pub const S400: Color = Color::hex(0xA8A29E);
    pub const S500: Color = Color::hex(0x78716C);
    pub const S600: Color = Color::hex(0x57534E);
    pub const S700: Color = Color::hex(0x44403C);
    pub const S800: Color = Color::hex(0x292524);
    pub const S900: Color = Color::hex(0x1C1917);
    pub const S950: Color = Color::hex(0x0C0A09);
}

/// Get a color by family name and shade.
///
/// # Arguments
/// * `family` - Color family name (e.g., "blue", "red", "emerald")
/// * `shade` - Shade value (50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950)
///
/// # Returns
/// The color if found, or None if the family or shade is invalid.
pub fn get(family: &str, shade: u16) -> Option<Color> {
    let colors = match family.to_lowercase().as_str() {
        "red" => [
            red::S50,
            red::S100,
            red::S200,
            red::S300,
            red::S400,
            red::S500,
            red::S600,
            red::S700,
            red::S800,
            red::S900,
            red::S950,
        ],
        "orange" => [
            orange::S50,
            orange::S100,
            orange::S200,
            orange::S300,
            orange::S400,
            orange::S500,
            orange::S600,
            orange::S700,
            orange::S800,
            orange::S900,
            orange::S950,
        ],
        "amber" => [
            amber::S50,
            amber::S100,
            amber::S200,
            amber::S300,
            amber::S400,
            amber::S500,
            amber::S600,
            amber::S700,
            amber::S800,
            amber::S900,
            amber::S950,
        ],
        "yellow" => [
            yellow::S50,
            yellow::S100,
            yellow::S200,
            yellow::S300,
            yellow::S400,
            yellow::S500,
            yellow::S600,
            yellow::S700,
            yellow::S800,
            yellow::S900,
            yellow::S950,
        ],
        "lime" => [
            lime::S50,
            lime::S100,
            lime::S200,
            lime::S300,
            lime::S400,
            lime::S500,
            lime::S600,
            lime::S700,
            lime::S800,
            lime::S900,
            lime::S950,
        ],
        "green" => [
            green::S50,
            green::S100,
            green::S200,
            green::S300,
            green::S400,
            green::S500,
            green::S600,
            green::S700,
            green::S800,
            green::S900,
            green::S950,
        ],
        "emerald" => [
            emerald::S50,
            emerald::S100,
            emerald::S200,
            emerald::S300,
            emerald::S400,
            emerald::S500,
            emerald::S600,
            emerald::S700,
            emerald::S800,
            emerald::S900,
            emerald::S950,
        ],
        "teal" => [
            teal::S50,
            teal::S100,
            teal::S200,
            teal::S300,
            teal::S400,
            teal::S500,
            teal::S600,
            teal::S700,
            teal::S800,
            teal::S900,
            teal::S950,
        ],
        "cyan" => [
            cyan::S50,
            cyan::S100,
            cyan::S200,
            cyan::S300,
            cyan::S400,
            cyan::S500,
            cyan::S600,
            cyan::S700,
            cyan::S800,
            cyan::S900,
            cyan::S950,
        ],
        "sky" => [
            sky::S50,
            sky::S100,
            sky::S200,
            sky::S300,
            sky::S400,
            sky::S500,
            sky::S600,
            sky::S700,
            sky::S800,
            sky::S900,
            sky::S950,
        ],
        "blue" => [
            blue::S50,
            blue::S100,
            blue::S200,
            blue::S300,
            blue::S400,
            blue::S500,
            blue::S600,
            blue::S700,
            blue::S800,
            blue::S900,
            blue::S950,
        ],
        "indigo" => [
            indigo::S50,
            indigo::S100,
            indigo::S200,
            indigo::S300,
            indigo::S400,
            indigo::S500,
            indigo::S600,
            indigo::S700,
            indigo::S800,
            indigo::S900,
            indigo::S950,
        ],
        "violet" => [
            violet::S50,
            violet::S100,
            violet::S200,
            violet::S300,
            violet::S400,
            violet::S500,
            violet::S600,
            violet::S700,
            violet::S800,
            violet::S900,
            violet::S950,
        ],
        "purple" => [
            purple::S50,
            purple::S100,
            purple::S200,
            purple::S300,
            purple::S400,
            purple::S500,
            purple::S600,
            purple::S700,
            purple::S800,
            purple::S900,
            purple::S950,
        ],
        "fuchsia" => [
            fuchsia::S50,
            fuchsia::S100,
            fuchsia::S200,
            fuchsia::S300,
            fuchsia::S400,
            fuchsia::S500,
            fuchsia::S600,
            fuchsia::S700,
            fuchsia::S800,
            fuchsia::S900,
            fuchsia::S950,
        ],
        "pink" => [
            pink::S50,
            pink::S100,
            pink::S200,
            pink::S300,
            pink::S400,
            pink::S500,
            pink::S600,
            pink::S700,
            pink::S800,
            pink::S900,
            pink::S950,
        ],
        "rose" => [
            rose::S50,
            rose::S100,
            rose::S200,
            rose::S300,
            rose::S400,
            rose::S500,
            rose::S600,
            rose::S700,
            rose::S800,
            rose::S900,
            rose::S950,
        ],
        "slate" => [
            slate::S50,
            slate::S100,
            slate::S200,
            slate::S300,
            slate::S400,
            slate::S500,
            slate::S600,
            slate::S700,
            slate::S800,
            slate::S900,
            slate::S950,
        ],
        "gray" | "grey" | "neutral" => [
            gray::S50,
            gray::S100,
            gray::S200,
            gray::S300,
            gray::S400,
            gray::S500,
            gray::S600,
            gray::S700,
            gray::S800,
            gray::S900,
            gray::S950,
        ],
        "zinc" => [
            zinc::S50,
            zinc::S100,
            zinc::S200,
            zinc::S300,
            zinc::S400,
            zinc::S500,
            zinc::S600,
            zinc::S700,
            zinc::S800,
            zinc::S900,
            zinc::S950,
        ],
        "stone" => [
            stone::S50,
            stone::S100,
            stone::S200,
            stone::S300,
            stone::S400,
            stone::S500,
            stone::S600,
            stone::S700,
            stone::S800,
            stone::S900,
            stone::S950,
        ],
        _ => return None,
    };

    let index = match shade {
        50 => 0,
        100 => 1,
        200 => 2,
        300 => 3,
        400 => 4,
        500 => 5,
        600 => 6,
        700 => 7,
        800 => 8,
        900 => 9,
        950 => 10,
        _ => return None,
    };

    Some(colors[index])
}
