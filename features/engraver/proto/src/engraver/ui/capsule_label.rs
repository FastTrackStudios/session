//! Capsule Label - Rounded rectangle labels with text fitting.
//!
//! Provides reusable components for rendering labels inside rounded
//! capsule frames, commonly used for rehearsal marks, section labels, etc.
//!
//! # Text Fitting Modes
//!
//! - `AboveStaff`: Capsule wraps around text with padding (variable width)
//! - `FixedWidth`: Fixed capsule width, text scales to fit
//!
//! # Example
//!
//! ```ignore
//! use engraver::ui::{CapsuleLabelConfig, CapsuleLabelMode, ComputedCapsuleLabel};
//!
//! let config = CapsuleLabelConfig {
//!     mode: CapsuleLabelMode::FixedWidth {
//!         width: 100.0,
//!         height: 30.0,
//!         internal_padding_h: 2.0,
//!         internal_padding_v: 2.0,
//!     },
//!     font_size: 14.0,
//!     line_height: 18.0,
//! };
//!
//! // Measure text width first (e.g., using glyphon)
//! let text_width = 80.0;
//!
//! let label = ComputedCapsuleLabel::compute("VS 1", 50.0, 100.0, text_width, &config);
//! // Now use label.capsule_* and label.text_* for rendering
//! ```

/// Configuration for capsule label rendering.
#[derive(Debug, Clone)]
pub struct CapsuleLabelConfig {
    /// Positioning and sizing mode
    pub mode: CapsuleLabelMode,
    /// Base font size for text
    pub font_size: f32,
    /// Line height for text layout
    pub line_height: f32,
}

impl Default for CapsuleLabelConfig {
    fn default() -> Self {
        Self {
            mode: CapsuleLabelMode::AboveStaff {
                padding_h: 8.0,
                padding_v: 4.0,
            },
            font_size: 14.0,
            line_height: 18.0,
        }
    }
}

/// Positioning mode for capsule labels.
#[derive(Debug, Clone)]
pub enum CapsuleLabelMode {
    /// Capsule wraps around text with padding (variable width).
    /// Use for labels that float above staves.
    AboveStaff {
        /// Horizontal padding around text
        padding_h: f32,
        /// Vertical padding around text
        padding_v: f32,
    },
    /// Fixed capsule dimensions, text scales to fit.
    /// Use for labels in margins or fixed-size areas.
    FixedWidth {
        /// Fixed capsule width
        width: f32,
        /// Fixed capsule height
        height: f32,
        /// Internal horizontal padding (from capsule edge to text)
        internal_padding_h: f32,
        /// Internal vertical padding (from capsule edge to text)
        internal_padding_v: f32,
    },
}

/// Computed capsule label with all positions and dimensions.
#[derive(Debug, Clone)]
pub struct ComputedCapsuleLabel {
    /// The label text
    pub text: String,
    /// Capsule X position (top-left)
    pub capsule_x: f32,
    /// Capsule Y position (top-left)
    pub capsule_y: f32,
    /// Capsule width
    pub capsule_width: f32,
    /// Capsule height
    pub capsule_height: f32,
    /// Text X position (for rendering)
    pub text_x: f32,
    /// Text Y position (for rendering)
    pub text_y: f32,
    /// Scale factor for text (1.0 = normal, <1.0 = shrunk to fit)
    pub text_scale: f32,
    /// Corner radius for rounded rectangle (computed as height/4)
    pub corner_radius: f32,
}

impl ComputedCapsuleLabel {
    /// Compute capsule label dimensions based on text measurement.
    ///
    /// # Arguments
    ///
    /// * `text` - The label text
    /// * `base_x` - Base X position for the label
    /// * `base_y` - Base Y position for the label
    /// * `measured_text_width` - Measured width of the text (e.g., from glyphon)
    /// * `config` - Label configuration
    ///
    /// # Returns
    ///
    /// A computed label with all positions and dimensions filled in.
    #[must_use]
    pub fn compute(
        text: &str,
        base_x: f32,
        base_y: f32,
        measured_text_width: f32,
        config: &CapsuleLabelConfig,
    ) -> Self {
        let text_height = config.line_height;

        let (capsule_x, capsule_y, capsule_width, capsule_height, text_x, text_y, text_scale) =
            match &config.mode {
                CapsuleLabelMode::AboveStaff {
                    padding_h,
                    padding_v,
                } => {
                    // Text-first: capsule wraps around text with padding, no scaling needed
                    let tx = base_x;
                    let ty = base_y;
                    let cx = tx - padding_h;
                    let cy = ty - padding_v;
                    let cw = measured_text_width + padding_h * 2.0;
                    let ch = text_height + padding_v * 2.0;
                    (cx, cy, cw, ch, tx, ty, 1.0)
                }
                CapsuleLabelMode::FixedWidth {
                    width,
                    height,
                    internal_padding_h,
                    internal_padding_v,
                } => {
                    // Fixed capsule: scale text to fit if needed
                    let cx = base_x;
                    let cy = base_y;
                    let cw = *width;
                    let ch = *height;

                    // Calculate available space for text
                    let available_width = cw - internal_padding_h * 2.0;
                    let available_height = ch - internal_padding_v * 2.0;

                    // Calculate scale to fit (use minimum to maintain aspect ratio)
                    let scale_x = if measured_text_width > available_width {
                        available_width / measured_text_width
                    } else {
                        1.0
                    };
                    let scale_y = if text_height > available_height {
                        available_height / text_height
                    } else {
                        1.0
                    };
                    let scale = scale_x.min(scale_y);

                    // Scaled text dimensions for centering
                    let scaled_width = measured_text_width * scale;
                    let scaled_height = text_height * scale;

                    // Center scaled text within capsule
                    let tx = cx + (cw - scaled_width) / 2.0;
                    let ty = cy + (ch - scaled_height) / 2.0;

                    (cx, cy, cw, ch, tx, ty, scale)
                }
            };

        let corner_radius = capsule_height / 4.0;

        Self {
            text: text.to_string(),
            capsule_x,
            capsule_y,
            capsule_width,
            capsule_height,
            text_x,
            text_y,
            text_scale,
            corner_radius,
        }
    }

    /// Create a computed label for a left margin position.
    ///
    /// Convenience method for section labels in the left margin.
    #[must_use]
    pub fn in_left_margin(
        text: &str,
        margin_x: f32,
        staff_y: f32,
        margin_width: f32,
        staff_height: f32,
        measured_text_width: f32,
        config: &CapsuleLabelConfig,
    ) -> Self {
        // Calculate capsule position in margin
        let margin_padding_h = 2.0;
        let margin_padding_v = 3.0;

        let capsule_x = margin_x + margin_padding_h;
        let capsule_width = margin_width - (margin_padding_h * 2.0);
        let capsule_height = staff_height - (margin_padding_v * 2.0);
        let capsule_y = staff_y + margin_padding_v;

        // Now compute with fixed width mode
        let fixed_config = CapsuleLabelConfig {
            mode: CapsuleLabelMode::FixedWidth {
                width: capsule_width,
                height: capsule_height,
                internal_padding_h: 1.0,
                internal_padding_v: 1.0,
            },
            ..config.clone()
        };

        Self::compute(
            text,
            capsule_x,
            capsule_y,
            measured_text_width,
            &fixed_config,
        )
    }
}

/// Format a section label for display (like music_symbols example).
///
/// Uses the following format:
/// - Intro/Outro/Breakdown/Hits: Full name in uppercase ("INTRO", "OUTRO", "BREAKDOWN", "HITS")
/// - Other sections: Abbreviation + number ("VS 1", "CH 2", "BR 1")
///
/// # Arguments
///
/// * `section_type` - The type of section (e.g., "Intro", "Verse", "Chorus")
/// * `abbreviation` - The abbreviation for the section (e.g., "IN", "VS", "CH")
/// * `number` - Optional section number
///
/// # Returns
///
/// Formatted label string.
#[must_use]
pub fn format_rehearsal_label(
    section_type: &str,
    abbreviation: &str,
    number: Option<u32>,
) -> String {
    format_rehearsal_label_with_letter(section_type, abbreviation, number, None)
}

/// Format a rehearsal label with optional section letter for consecutive repeats.
///
/// When sections of the same type appear consecutively, they get lettered A, B, C, etc.
/// For example: "VS 1 A", "VS 1 B", "INT A", "INT B", etc.
/// The letter will be placed on its own line by the layout engine.
///
/// # Arguments
///
/// * `section_type` - Full section type name (e.g., "Verse", "Chorus")
/// * `abbreviation` - Short abbreviation (e.g., "VS", "CH")
/// * `number` - Optional section number
/// * `letter` - Optional section letter for consecutive repeats (A-Z)
///
/// # Returns
///
/// Formatted label string.
#[must_use]
pub fn format_rehearsal_label_with_letter(
    section_type: &str,
    abbreviation: &str,
    number: Option<u32>,
    letter: Option<char>,
) -> String {
    let section_lower = section_type.to_lowercase();

    // Intro, Outro, Breakdown, and Hits use full name uppercase without number
    if section_lower == "intro"
        || section_lower == "outro"
        || section_lower == "breakdown"
        || section_lower == "hits"
    {
        return match letter {
            Some(l) => format!("{} {}", section_type.to_uppercase(), l),
            None => section_type.to_uppercase(),
        };
    }

    // Other sections use abbreviation + number + optional letter
    let base = match number {
        Some(n) => format!("{} {}", abbreviation, n),
        None => abbreviation.to_string(),
    };

    match letter {
        Some(l) => format!("{} {}", base, l),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_rehearsal_label_intro() {
        assert_eq!(format_rehearsal_label("Intro", "IN", None), "INTRO");
        assert_eq!(format_rehearsal_label("Intro", "IN", Some(1)), "INTRO");
    }

    #[test]
    fn test_format_rehearsal_label_outro() {
        assert_eq!(format_rehearsal_label("Outro", "OUT", None), "OUTRO");
        assert_eq!(format_rehearsal_label("Outro", "OUT", Some(1)), "OUTRO");
    }

    #[test]
    fn test_format_rehearsal_label_verse() {
        assert_eq!(format_rehearsal_label("Verse", "VS", Some(1)), "VS 1");
        assert_eq!(format_rehearsal_label("Verse", "VS", Some(2)), "VS 2");
        assert_eq!(format_rehearsal_label("Verse", "VS", None), "VS");
    }

    #[test]
    fn test_format_rehearsal_label_chorus() {
        assert_eq!(format_rehearsal_label("Chorus", "CH", Some(1)), "CH 1");
        assert_eq!(format_rehearsal_label("Chorus", "CH", Some(2)), "CH 2");
    }

    #[test]
    fn test_format_rehearsal_label_breakdown() {
        assert_eq!(format_rehearsal_label("Breakdown", "BD", None), "BREAKDOWN");
        assert_eq!(
            format_rehearsal_label("Breakdown", "BD", Some(1)),
            "BREAKDOWN"
        );
    }

    #[test]
    fn test_format_rehearsal_label_hits() {
        assert_eq!(format_rehearsal_label("Hits", "HT", None), "HITS");
        assert_eq!(format_rehearsal_label("Hits", "HT", Some(1)), "HITS");
    }

    #[test]
    fn test_format_rehearsal_label_bridge() {
        assert_eq!(format_rehearsal_label("Bridge", "BR", Some(1)), "BR 1");
        assert_eq!(format_rehearsal_label("Bridge", "BR", None), "BR");
    }

    #[test]
    fn test_computed_label_above_staff() {
        let config = CapsuleLabelConfig {
            mode: CapsuleLabelMode::AboveStaff {
                padding_h: 8.0,
                padding_v: 4.0,
            },
            font_size: 14.0,
            line_height: 18.0,
        };

        let label = ComputedCapsuleLabel::compute("VS 1", 100.0, 50.0, 40.0, &config);

        // Capsule should be offset by padding
        assert_eq!(label.capsule_x, 92.0); // 100 - 8
        assert_eq!(label.capsule_y, 46.0); // 50 - 4
        assert_eq!(label.capsule_width, 56.0); // 40 + 8*2
        assert_eq!(label.capsule_height, 26.0); // 18 + 4*2
        assert_eq!(label.text_scale, 1.0);
    }

    #[test]
    fn test_computed_label_fixed_width_no_scaling() {
        let config = CapsuleLabelConfig {
            mode: CapsuleLabelMode::FixedWidth {
                width: 100.0,
                height: 30.0,
                internal_padding_h: 5.0,
                internal_padding_v: 2.0,
            },
            font_size: 14.0,
            line_height: 18.0,
        };

        // Text fits without scaling (40 < 100 - 10)
        let label = ComputedCapsuleLabel::compute("VS 1", 50.0, 100.0, 40.0, &config);

        assert_eq!(label.capsule_x, 50.0);
        assert_eq!(label.capsule_y, 100.0);
        assert_eq!(label.capsule_width, 100.0);
        assert_eq!(label.capsule_height, 30.0);
        assert_eq!(label.text_scale, 1.0);
    }

    #[test]
    fn test_computed_label_fixed_width_with_scaling() {
        let config = CapsuleLabelConfig {
            mode: CapsuleLabelMode::FixedWidth {
                width: 50.0,
                height: 30.0,
                internal_padding_h: 5.0,
                internal_padding_v: 2.0,
            },
            font_size: 14.0,
            line_height: 18.0,
        };

        // Text needs scaling (80 > 50 - 10 = 40)
        let label = ComputedCapsuleLabel::compute("CHORUS", 50.0, 100.0, 80.0, &config);

        assert_eq!(label.capsule_width, 50.0);
        assert!(label.text_scale < 1.0);
        assert!((label.text_scale - 0.5).abs() < 0.01); // 40/80 = 0.5
    }
}
