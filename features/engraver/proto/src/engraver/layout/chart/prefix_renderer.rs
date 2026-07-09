//! System prefix rendering (clef, key signature, and time signature).
//!
//! This module extracts the duplicated prefix rendering logic from
//! `layout_paginated` and `layout_continuous` into reusable functions.
//!
//! The standard order for system prefixes in music notation is:
//! **Clef → Key Signature → Time Signature**

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::tlayout::{
    ClefParams, ClefType, KeySigParams, KeySigType, TimeSigParams, TimeSigType, layout_clef,
    layout_keysig, layout_timesig,
};
use crate::engraver::scene::node::SceneNode;
use crate::key::Key;
use crate::primitives::Note; // Import trait for semitone() method
use kurbo::Affine;

/// Convert a Key to the number of fifths for key signature rendering.
///
/// Returns positive for sharps, negative for flats:
/// - G major = 1, D major = 2, ... C# major = 7
/// - F major = -1, Bb major = -2, ... Cb major = -7
/// - C major = 0
///
/// For minor keys, uses the relative major key signature.
#[must_use]
pub fn key_to_fifths(key: &Key) -> i8 {
    use crate::key::ScaleMode;
    use crate::primitives::Accidental;

    let root = key.root();
    let is_minor = key.mode == ScaleMode::aeolian();

    // Get the pitch class (0-11) of the root
    let semitone = root.semitone() as i8;

    // For minor keys, add 3 semitones to get relative major
    let effective_semitone = if is_minor {
        (semitone + 3) % 12
    } else {
        semitone
    };

    // Map semitones to fifths using circle of fifths
    // C=0, G=1, D=2, A=3, E=4, B=5, F#=6, C#=7
    // F=-1, Bb=-2, Eb=-3, Ab=-4, Db=-5, Gb=-6, Cb=-7
    let base_fifths = match effective_semitone {
        0 => 0,   // C
        1 => 7,   // C# / Db (prefer sharps for C#)
        2 => 2,   // D
        3 => -3,  // Eb / D# (prefer flats for Eb)
        4 => 4,   // E
        5 => -1,  // F
        6 => 6,   // F# / Gb (prefer sharps for F#)
        7 => 1,   // G
        8 => -4,  // Ab / G# (prefer flats for Ab)
        9 => 3,   // A
        10 => -2, // Bb / A# (prefer flats for Bb)
        11 => 5,  // B / Cb (prefer natural for B)
        _ => 0,
    };

    // Adjust for enharmonic spelling based on the root's accidental
    // If the key uses flats, prefer flat spellings
    // If the key uses sharps, prefer sharp spellings
    let adjustment = match root.accidental {
        Some(Accidental::Flat) | Some(Accidental::DoubleFlat) => {
            // Prefer flat versions for ambiguous keys
            match effective_semitone {
                1 => -12, // Db instead of C# (7 -> -5)
                6 => -12, // Gb instead of F# (6 -> -6)
                _ => 0,
            }
        }
        _ => 0,
    };

    base_fifths + adjustment
}

/// Context for rendering system prefix (clef, key signature, and time signature).
#[derive(Debug, Clone)]
pub struct PrefixRenderContext {
    /// Starting x position for prefix elements.
    pub x: f64,
    /// Y position of the staff.
    pub staff_y: f64,
    /// Spatium (staff space) in points.
    pub spatium: f64,
    /// Whether to render the clef.
    pub include_clef: bool,
    /// Which clef to render. Defaults to Treble.
    pub clef_type: ClefType,
    /// Whether to render the key signature.
    pub include_key_sig: bool,
    /// Whether to render the time signature.
    pub include_time_sig: bool,
    /// Key signature (number of sharps/flats: positive = sharps, negative = flats).
    pub key_signature: i8,
    /// Color for the key signature. `None` = black; a system whose first
    /// measure carries a key change passes red to highlight the change in
    /// place (instead of drawing a separate red indicator on top).
    pub key_sig_color: Option<peniko::Color>,
    /// Time signature (numerator, denominator).
    pub time_signature: (u8, u8),
    /// Width of the clef element.
    pub clef_width: f64,
    /// Width of the key signature element.
    pub key_sig_width: f64,
    /// Width of the time signature element.
    pub time_sig_width: f64,
    /// Page number (for metadata, optional in continuous mode).
    pub page_number: Option<u32>,
}

/// Result of prefix rendering.
#[derive(Debug)]
pub struct PrefixRenderResult {
    /// Rendered prefix nodes (clef and/or time signature).
    pub nodes: Vec<SceneNode>,
    /// Next ID counter value.
    pub next_id: u64,
    /// Total width consumed by prefix elements.
    pub total_width: f64,
}

/// Render system prefix (clef, key signature, and time signature).
///
/// The standard order is: Clef → Key Signature → Time Signature
///
/// Returns the rendered nodes and the total width consumed.
pub fn render_system_prefix(
    ctx: &PrefixRenderContext,
    mut id_counter: u64,
    layout_ctx: &LayoutContext<'_>,
) -> PrefixRenderResult {
    let mut nodes = Vec::new();
    let mut prefix_x = ctx.x;

    // Render clef if requested
    if ctx.include_clef {
        let clef_params = ClefParams {
            id: id_counter,
            clef_type: ctx.clef_type,
            ..Default::default()
        };
        id_counter += 1;

        let (_, mut clef_node) = layout_clef(&clef_params, layout_ctx);

        // Position clef on staff (middle line = 2 spatiums from top)
        clef_node.transform = Affine::translate((prefix_x, ctx.staff_y + 2.0 * ctx.spatium));

        // Add page metadata if available
        if let Some(page) = ctx.page_number {
            clef_node
                .metadata
                .insert("page".to_string(), page.to_string());
        }

        nodes.push(clef_node);
        prefix_x += ctx.clef_width;
    }

    // Render key signature if requested (between clef and time signature)
    if ctx.include_key_sig && ctx.key_signature != 0 {
        let key_params = KeySigParams {
            id: id_counter,
            key: KeySigType::Standard(ctx.key_signature),
            color: ctx.key_sig_color,
            ..Default::default()
        };
        id_counter += 1;

        let (_, mut key_node) = layout_keysig(&key_params, layout_ctx);

        // Position key signature on staff (middle line = 2 spatiums from top)
        key_node.transform = Affine::translate((prefix_x, ctx.staff_y + 2.0 * ctx.spatium));

        // Add page metadata if available
        if let Some(page) = ctx.page_number {
            key_node
                .metadata
                .insert("page".to_string(), page.to_string());
        }

        nodes.push(key_node);
        prefix_x += ctx.key_sig_width;
    }

    // Render time signature if requested
    if ctx.include_time_sig {
        let ts_params = TimeSigParams {
            id: id_counter,
            sig_type: TimeSigType::Numeric {
                numerator: ctx.time_signature.0,
                denominator: ctx.time_signature.1,
            },
            ..Default::default()
        };
        id_counter += 1;

        let (_, mut ts_node) = layout_timesig(&ts_params, layout_ctx);
        ts_node.transform = Affine::translate((prefix_x, ctx.staff_y + 2.0 * ctx.spatium));

        // Add page metadata if available
        if let Some(page) = ctx.page_number {
            ts_node
                .metadata
                .insert("page".to_string(), page.to_string());
        }

        nodes.push(ts_node);
        prefix_x += ctx.time_sig_width;
    }

    let total_width = prefix_x - ctx.x;

    PrefixRenderResult {
        nodes,
        next_id: id_counter,
        total_width,
    }
}

/// Calculate prefix width without rendering.
///
/// This is useful for layout calculations before rendering.
///
/// # Arguments
/// * `spatium` - Staff space in points
/// * `include_clef` - Whether clef will be rendered
/// * `include_key_sig` - Whether key signature will be rendered
/// * `key_signature` - Number of sharps (positive) or flats (negative)
/// * `include_time_sig` - Whether time signature will be rendered
///
/// # Returns
/// Tuple of (clef_width, key_sig_width, time_sig_width, total_width)
#[must_use]
pub fn calculate_prefix_width(
    spatium: f64,
    include_clef: bool,
    include_key_sig: bool,
    key_signature: i8,
    include_time_sig: bool,
) -> (f64, f64, f64, f64) {
    let clef_spacing = 0.5 * spatium;
    let key_sig_spacing = 0.5 * spatium;
    let time_sig_spacing = 0.8 * spatium;

    let clef_width = if include_clef {
        ClefType::Treble.width() * spatium + clef_spacing
    } else {
        0.0
    };

    // Key signature width depends on number of accidentals
    let key_sig_width = if include_key_sig && key_signature != 0 {
        let accidental_count = key_signature.unsigned_abs() as f64;
        let accidental_spacing = spatium * 0.9; // Match keysig.rs
        accidental_count * accidental_spacing + key_sig_spacing
    } else {
        0.0
    };

    let time_sig_width = if include_time_sig {
        2.0 * spatium + time_sig_spacing
    } else {
        0.0
    };

    let total_width = clef_width + key_sig_width + time_sig_width;

    (clef_width, key_sig_width, time_sig_width, total_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_prefix_width_all() {
        let spatium = 5.0;
        // Clef + 2 sharps + time sig
        let (clef_w, key_sig_w, time_sig_w, total) =
            calculate_prefix_width(spatium, true, true, 2, true);

        // Clef: ClefType::Treble.width() * 5.0 + 2.5 = ~8 spatiums * 5 + 2.5
        assert!(clef_w > 0.0);
        // Key sig: 2 accidentals * 4.5 + 2.5 = 11.5
        assert!(key_sig_w > 0.0);
        // Time sig: 2.0 * 5.0 + 4.0 = 14.0
        assert!((time_sig_w - 14.0).abs() < 0.01);
        assert!((total - (clef_w + key_sig_w + time_sig_w)).abs() < 0.01);
    }

    #[test]
    fn test_calculate_prefix_width_clef_only() {
        let spatium = 5.0;
        let (clef_w, key_sig_w, time_sig_w, total) =
            calculate_prefix_width(spatium, true, false, 0, false);

        assert!(clef_w > 0.0);
        assert_eq!(key_sig_w, 0.0);
        assert_eq!(time_sig_w, 0.0);
        assert_eq!(total, clef_w);
    }

    #[test]
    fn test_calculate_prefix_width_key_sig_sharps() {
        let spatium = 5.0;
        // 3 sharps (A major)
        let (clef_w, key_sig_w, time_sig_w, total) =
            calculate_prefix_width(spatium, false, true, 3, false);

        assert_eq!(clef_w, 0.0);
        assert!(key_sig_w > 0.0);
        assert_eq!(time_sig_w, 0.0);
        assert_eq!(total, key_sig_w);
    }

    #[test]
    fn test_calculate_prefix_width_key_sig_flats() {
        let spatium = 5.0;
        // 2 flats (Bb major)
        let (clef_w, key_sig_w, time_sig_w, total) =
            calculate_prefix_width(spatium, false, true, -2, false);

        assert_eq!(clef_w, 0.0);
        assert!(key_sig_w > 0.0); // Flats also have positive width
        assert_eq!(time_sig_w, 0.0);
        assert_eq!(total, key_sig_w);
    }

    #[test]
    fn test_calculate_prefix_width_c_major() {
        let spatium = 5.0;
        // C major (0 accidentals) - key sig should have no width
        let (clef_w, key_sig_w, time_sig_w, total) =
            calculate_prefix_width(spatium, false, true, 0, false);

        assert_eq!(clef_w, 0.0);
        assert_eq!(key_sig_w, 0.0); // No accidentals = no width
        assert_eq!(time_sig_w, 0.0);
        assert_eq!(total, 0.0);
    }

    #[test]
    fn test_calculate_prefix_width_none() {
        let spatium = 5.0;
        let (clef_w, key_sig_w, time_sig_w, total) =
            calculate_prefix_width(spatium, false, false, 0, false);

        assert_eq!(clef_w, 0.0);
        assert_eq!(key_sig_w, 0.0);
        assert_eq!(time_sig_w, 0.0);
        assert_eq!(total, 0.0);
    }
}
