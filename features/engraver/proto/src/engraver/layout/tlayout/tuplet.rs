//! Tuplet layout module.
//!
//! Implements layout for tuplets (triplets, quintuplets, etc.) following
//! MuseScore's tuplet layout algorithm.

use kurbo::{BezPath, Point, Rect};
use peniko::Color;

use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::paint::{FontStyle, FontWeight, TextAnchor};
use crate::engraver::scene::{PaintCommand, SceneNode};

use super::chord::StemDirection;

/// Tuplet bracket visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TupletBracketType {
    /// Automatically determine bracket visibility
    #[default]
    Auto,
    /// Always show bracket
    ShowBracket,
    /// Never show bracket
    NoBracket,
}

/// Tuplet number display type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TupletNumberType {
    /// Show just the number (e.g., "3")
    #[default]
    ShowNumber,
    /// Show the ratio (e.g., "3:2")
    ShowRatio,
    /// No number
    NoText,
}

/// Configuration for tuplet layout.
#[derive(Debug, Clone)]
pub struct TupletConfig {
    /// Maximum slope for the tuplet bracket
    pub max_slope: f64,
    /// Distance from bracket to notehead
    pub head_distance: f64,
    /// Distance from bracket to stem tip
    pub stem_distance: f64,
    /// Bracket hook height
    pub bracket_hook_height: f64,
    /// Bracket line width
    pub bracket_width: f64,
    /// Font size for tuplet number (in spatiums)
    pub number_size: f64,
    /// Number type
    pub number_type: TupletNumberType,
    /// Bracket type
    pub bracket_type: TupletBracketType,
}

impl Default for TupletConfig {
    fn default() -> Self {
        Self {
            max_slope: 0.5,
            head_distance: 0.5,
            stem_distance: 0.25,
            bracket_hook_height: 1.0,
            bracket_width: 0.08,
            number_size: 1.1,
            number_type: TupletNumberType::ShowNumber,
            bracket_type: TupletBracketType::Auto,
        }
    }
}

/// Information about a note in a tuplet.
#[derive(Debug, Clone)]
pub struct TupletNote {
    /// X position of the note
    pub x: f64,
    /// Y position of the notehead center
    pub y_head: f64,
    /// Y position of the stem tip (if stemmed)
    pub y_stem_tip: Option<f64>,
    /// Stem direction
    pub stem_direction: StemDirection,
    /// Is this a rest?
    pub is_rest: bool,
}

/// Tuplet ratio (e.g., 3:2 for triplet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TupletRatio {
    /// Numerator (how many notes in the tuplet)
    pub numerator: u8,
    /// Denominator (how many notes they replace)
    pub denominator: u8,
}

impl TupletRatio {
    // Const versions for use in const Duration constructors
    /// Triplet ratio (3:2) - 3 notes in the space of 2
    pub const TRIPLET: Self = Self {
        numerator: 3,
        denominator: 2,
    };
    /// Quintuplet ratio (5:4) - 5 notes in the space of 4
    pub const QUINTUPLET: Self = Self {
        numerator: 5,
        denominator: 4,
    };
    /// Sextuplet ratio (6:4) - 6 notes in the space of 4
    pub const SEXTUPLET: Self = Self {
        numerator: 6,
        denominator: 4,
    };
    /// Septuplet ratio (7:8) - 7 notes in the space of 8 (matches MuseScore)
    pub const SEPTUPLET: Self = Self {
        numerator: 7,
        denominator: 8,
    };

    /// Create a new tuplet ratio.
    pub const fn new(numerator: u8, denominator: u8) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Create a triplet (3:2).
    pub const fn triplet() -> Self {
        Self::TRIPLET
    }

    /// Create a quintuplet (5:4).
    pub const fn quintuplet() -> Self {
        Self::QUINTUPLET
    }

    /// Create a sextuplet (6:4).
    pub const fn sextuplet() -> Self {
        Self::SEXTUPLET
    }

    /// Create a septuplet (7:8).
    pub const fn septuplet() -> Self {
        Self::SEPTUPLET
    }

    /// Get the tick multiplier for this ratio as (numerator, denominator).
    /// A triplet (3:2) plays 3 notes in the time of 2, so each note is 2/3 duration.
    /// Returns (denominator, numerator) for multiplication: ticks * num / denom.
    pub const fn tick_multiplier(&self) -> (i32, i32) {
        (self.denominator as i32, self.numerator as i32)
    }

    /// Format as display text.
    pub fn to_display_string(&self, number_type: TupletNumberType) -> String {
        match number_type {
            TupletNumberType::ShowNumber => format!("{}", self.numerator),
            TupletNumberType::ShowRatio => format!("{}:{}", self.numerator, self.denominator),
            TupletNumberType::NoText => String::new(),
        }
    }
}

/// Result of tuplet layout.
#[derive(Debug, Clone)]
pub struct TupletLayout {
    /// Paint commands for the tuplet bracket and number
    pub commands: Vec<PaintCommand>,
    /// Bounding box of the entire tuplet
    pub bbox: Rect,
    /// Scene node for the tuplet
    pub scene: SceneNode,
}

/// Layout a tuplet bracket and number.
///
/// # Arguments
/// * `notes` - Notes in the tuplet (must have at least 2)
/// * `ratio` - The tuplet ratio (e.g., 3:2 for triplet)
/// * `spatium` - Staff space height
/// * `config` - Tuplet configuration
/// * `max_x` - Optional maximum X position (measure boundary) to clamp bracket endpoints
///
/// # Returns
/// A `TupletLayout` containing paint commands and bounds.
pub fn layout_tuplet(
    notes: &[TupletNote],
    ratio: TupletRatio,
    id: u64,
    spatium: f64,
    config: &TupletConfig,
    max_x: Option<f64>,
) -> TupletLayout {
    if notes.len() < 2 {
        return TupletLayout {
            commands: Vec::new(),
            bbox: Rect::ZERO,
            scene: SceneNode::group(SemanticId::new(ElementType::Tuplet, id)),
        };
    }

    let mut commands = Vec::new();

    // Determine bracket direction (up or down)
    let is_up = determine_tuplet_direction(notes);

    // Get first and last notes (safe due to len >= 2 check above)
    let (Some(_first), Some(last)) = (notes.first(), notes.last()) else {
        return TupletLayout {
            commands: Vec::new(),
            bbox: Rect::ZERO,
            scene: SceneNode::group(SemanticId::new(ElementType::Tuplet, id)),
        };
    };

    // Calculate bracket endpoints
    let (p1, p2) = calculate_bracket_endpoints(notes, is_up, spatium, config);

    // Apply slope constraint
    let (p1, p2) = apply_slope_constraint(p1, p2, config.max_slope);

    // Check for collisions with inner notes and adjust
    let (p1, p2) = avoid_collisions(notes, p1, p2, is_up, spatium, config);

    // Apply measure boundary constraint (MuseScore-style clamping)
    // The bracket's right endpoint should not extend past the measure boundary,
    // but it must still reach at least to the last note's position
    let p2 = if let Some(boundary) = max_x {
        let padding = 0.6 * spatium; // Standard tuplet bracket padding from barline
        let max_endpoint = boundary - padding;

        // Minimum endpoint is just past the last note (small extension for visual clarity)
        let min_endpoint = last.x + spatium * 0.5;

        // Clamp: prefer staying within boundary, but never go before last note
        let clamped_x = p2.x.min(max_endpoint).max(min_endpoint);

        if p2.x != clamped_x {
            tracing::debug!(
                "[tuplet-boundary] Adjusting bracket p2.x from {:.1} to {:.1} (boundary={:.1}, last_note={:.1})",
                p2.x,
                clamped_x,
                boundary,
                last.x
            );
        }
        Point::new(clamped_x, p2.y)
    } else {
        p2
    };

    // Determine if bracket should be shown
    let show_bracket = should_show_bracket(notes, config.bracket_type);

    // Calculate bracket hook height (sign depends on direction)
    let hook_height = if is_up {
        config.bracket_hook_height * spatium
    } else {
        -config.bracket_hook_height * spatium
    };

    // Calculate number position (center of bracket)
    let number_x = (p1.x + p2.x) / 2.0;
    let number_y = (p1.y + p2.y) / 2.0;

    // Get number text
    let number_text = ratio.to_display_string(config.number_type);
    let number_width = number_text.len() as f64 * config.number_size * spatium * 0.6;
    let _number_height = config.number_size * spatium;

    // Gap in bracket for number
    let gap_start = number_x - number_width / 2.0 - spatium * 0.2;
    let gap_end = number_x + number_width / 2.0 + spatium * 0.2;

    // Draw bracket if needed
    if show_bracket {
        let bracket_y1 = p1.y;
        let bracket_y2 = p2.y;

        // Left hook
        let mut left_hook = BezPath::new();
        left_hook.move_to(Point::new(p1.x, bracket_y1 + hook_height));
        left_hook.line_to(Point::new(p1.x, bracket_y1));

        // Left horizontal portion (up to gap)
        if gap_start > p1.x {
            left_hook.line_to(Point::new(gap_start, interpolate_y(p1, p2, gap_start)));
        }

        commands.push(PaintCommand::stroked_path(
            left_hook,
            Color::BLACK,
            config.bracket_width * spatium,
        ));

        // Right horizontal portion (from gap) and right hook
        let mut right_hook = BezPath::new();
        if gap_end < p2.x {
            right_hook.move_to(Point::new(gap_end, interpolate_y(p1, p2, gap_end)));
            right_hook.line_to(Point::new(p2.x, bracket_y2));
        } else {
            right_hook.move_to(Point::new(p2.x, bracket_y2));
        }
        right_hook.line_to(Point::new(p2.x, bracket_y2 + hook_height));

        commands.push(PaintCommand::stroked_path(
            right_hook,
            Color::BLACK,
            config.bracket_width * spatium,
        ));
    }

    // Draw number if needed
    if !number_text.is_empty() {
        // For now, we'll create a text command
        // The actual font rendering will be handled by the renderer
        commands.push(PaintCommand::Text {
            text: number_text,
            font_family: "serif".to_string(),
            font_size: config.number_size * spatium,
            position: Point::new(number_x, number_y),
            color: Color::BLACK,
            anchor: TextAnchor::Middle,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        });
    }

    // Calculate bounding box
    let min_x = p1.x.min(p2.x);
    let max_x = p1.x.max(p2.x);
    let min_y =
        p1.y.min(p2.y)
            .min(p1.y + hook_height)
            .min(p2.y + hook_height);
    let max_y =
        p1.y.max(p2.y)
            .max(p1.y + hook_height)
            .max(p2.y + hook_height);
    let bbox = Rect::new(min_x, min_y, max_x, max_y);

    // Create scene node
    let mut scene = SceneNode::group(SemanticId::new(ElementType::Tuplet, id));
    scene.commands = commands.clone();
    scene.bounds = bbox;

    TupletLayout {
        commands,
        bbox,
        scene,
    }
}

/// Determine the direction of the tuplet bracket.
fn determine_tuplet_direction(notes: &[TupletNote]) -> bool {
    // Count stems going up vs down
    let mut up_count = 0i32;
    let mut down_count = 0i32;

    for note in notes {
        if note.is_rest {
            continue;
        }
        match note.stem_direction {
            StemDirection::Up => up_count += 1,
            StemDirection::Down => down_count += 1,
            StemDirection::Auto => {
                // Use stem tip position to determine direction
                if let Some(stem_tip) = note.y_stem_tip {
                    if stem_tip < note.y_head {
                        up_count += 1;
                    } else {
                        down_count += 1;
                    }
                }
            }
        }
    }

    // Default to up if tie
    up_count >= down_count
}

/// Calculate the bracket endpoints.
/// Follows MuseScore's algorithm: when either endpoint is a rest,
/// align both bracket points vertically to keep the bracket horizontal.
fn calculate_bracket_endpoints(
    notes: &[TupletNote],
    is_up: bool,
    spatium: f64,
    config: &TupletConfig,
) -> (Point, Point) {
    let (Some(first), Some(last)) = (notes.first(), notes.last()) else {
        return (Point::ZERO, Point::ZERO);
    };

    // Calculate vertical offset based on stem/head
    let head_offset = config.head_distance * spatium;
    let stem_offset = config.stem_distance * spatium;

    // Helper to calculate Y position for a single note
    let calc_y = |note: &TupletNote, is_up: bool| -> f64 {
        if is_up {
            if let Some(stem_tip) = note.y_stem_tip {
                if note.stem_direction == StemDirection::Up
                    || (note.stem_direction == StemDirection::Auto && stem_tip < note.y_head)
                {
                    stem_tip - stem_offset
                } else {
                    note.y_head - head_offset
                }
            } else {
                note.y_head - head_offset
            }
        } else if let Some(stem_tip) = note.y_stem_tip {
            if note.stem_direction == StemDirection::Down
                || (note.stem_direction == StemDirection::Auto && stem_tip > note.y_head)
            {
                stem_tip + stem_offset
            } else {
                note.y_head + head_offset
            }
        } else {
            note.y_head + head_offset
        }
    };

    let y1 = calc_y(first, is_up);
    let y2 = calc_y(last, is_up);

    // MuseScore special case: when either endpoint is a rest,
    // align both bracket points to the same Y to keep it horizontal.
    // Also apply this when all notes are at the same staff position (rhythmic notation).
    let (final_y1, final_y2) = if first.is_rest || last.is_rest {
        // Find the most extreme Y from non-rest notes, or use average if all rests
        let non_rest_ys: Vec<f64> = notes
            .iter()
            .filter(|n| !n.is_rest)
            .map(|n| calc_y(n, is_up))
            .collect();

        let unified_y = if non_rest_ys.is_empty() {
            // All rests - use average position
            (y1 + y2) / 2.0
        } else if is_up {
            // For upward bracket, use the minimum (highest on screen) Y
            non_rest_ys.iter().copied().fold(f64::INFINITY, f64::min)
        } else {
            // For downward bracket, use the maximum (lowest on screen) Y
            non_rest_ys
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
        };
        (unified_y, unified_y)
    } else {
        // No rests at endpoints - check if all notes are at approximately the same Y
        // This handles rhythmic notation where all slashes are on the same line
        let y_variance = (y2 - y1).abs();
        if y_variance < spatium * 0.5 {
            // Notes are close enough - use horizontal bracket
            let avg_y = (y1 + y2) / 2.0;
            (avg_y, avg_y)
        } else {
            (y1, y2)
        }
    };

    // Add some horizontal padding
    let x1 = first.x - spatium * 0.25;
    let x2 = last.x + spatium * 1.25; // Account for notehead width

    (Point::new(x1, final_y1), Point::new(x2, final_y2))
}

/// Apply slope constraint to bracket endpoints.
fn apply_slope_constraint(p1: Point, p2: Point, max_slope: f64) -> (Point, Point) {
    let run = p2.x - p1.x;
    if run.abs() < 0.001 {
        return (p1, p2);
    }

    let slope = (p2.y - p1.y) / run;

    if slope.abs() <= max_slope {
        return (p1, p2);
    }

    // Clamp slope
    let clamped_slope = slope.clamp(-max_slope, max_slope);
    let new_y2 = p1.y + clamped_slope * run;

    (p1, Point::new(p2.x, new_y2))
}

/// Avoid collisions with all notes in the tuplet.
/// Checks stem tips and noteheads to ensure the bracket doesn't overlap.
fn avoid_collisions(
    notes: &[TupletNote],
    mut p1: Point,
    mut p2: Point,
    is_up: bool,
    spatium: f64,
    _config: &TupletConfig,
) -> (Point, Point) {
    if notes.len() < 2 {
        return (p1, p2);
    }

    let run = p2.x - p1.x;
    if run.abs() < 0.001 {
        return (p1, p2);
    }

    // Use a generous margin for clearance (includes space for the number)
    let margin = spatium * 1.5;

    // Check ALL notes for collision (not just inner ones)
    for note in notes.iter() {
        // Skip rests - they don't have stems to collide with
        if note.is_rest {
            continue;
        }

        // Get the relevant Y position (stem tip or head)
        let note_y = if is_up {
            if let Some(stem_tip) = note.y_stem_tip {
                if note.stem_direction == StemDirection::Up
                    || (note.stem_direction == StemDirection::Auto && stem_tip < note.y_head)
                {
                    stem_tip
                } else {
                    note.y_head
                }
            } else {
                note.y_head
            }
        } else if let Some(stem_tip) = note.y_stem_tip {
            if note.stem_direction == StemDirection::Down
                || (note.stem_direction == StemDirection::Auto && stem_tip > note.y_head)
            {
                stem_tip
            } else {
                note.y_head
            }
        } else {
            note.y_head
        };

        // Calculate bracket Y at this X position
        let bracket_y = interpolate_y(p1, p2, note.x);

        // Check for collision and shift bracket if needed
        if is_up {
            if bracket_y > note_y - margin {
                let offset = bracket_y - (note_y - margin);
                p1.y -= offset;
                p2.y -= offset;
            }
        } else if bracket_y < note_y + margin {
            let offset = (note_y + margin) - bracket_y;
            p1.y += offset;
            p2.y += offset;
        }
    }

    (p1, p2)
}

/// Interpolate Y position along the bracket line.
fn interpolate_y(p1: Point, p2: Point, x: f64) -> f64 {
    let run = p2.x - p1.x;
    if run.abs() < 0.001 {
        return (p1.y + p2.y) / 2.0;
    }
    let t = (x - p1.x) / run;
    p1.y + t * (p2.y - p1.y)
}

/// Determine if bracket should be shown.
fn should_show_bracket(_notes: &[TupletNote], bracket_type: TupletBracketType) -> bool {
    match bracket_type {
        TupletBracketType::ShowBracket => true,
        TupletBracketType::NoBracket => false,
        TupletBracketType::Auto => {
            // Auto: show bracket if notes are not all beamed together
            // For now, we'll default to showing the bracket
            // A more sophisticated check would look at beam groupings
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tuplet_ratio_triplet() {
        let ratio = TupletRatio::triplet();
        assert_eq!(ratio.numerator, 3);
        assert_eq!(ratio.denominator, 2);
    }

    #[test]
    fn test_tuplet_ratio_display() {
        let ratio = TupletRatio::triplet();
        assert_eq!(ratio.to_display_string(TupletNumberType::ShowNumber), "3");
        assert_eq!(ratio.to_display_string(TupletNumberType::ShowRatio), "3:2");
        assert_eq!(ratio.to_display_string(TupletNumberType::NoText), "");
    }

    #[test]
    fn test_tuplet_direction_all_up() {
        let notes = vec![
            TupletNote {
                x: 0.0,
                y_head: 0.0,
                y_stem_tip: Some(-10.0),
                stem_direction: StemDirection::Up,
                is_rest: false,
            },
            TupletNote {
                x: 20.0,
                y_head: 0.0,
                y_stem_tip: Some(-10.0),
                stem_direction: StemDirection::Up,
                is_rest: false,
            },
            TupletNote {
                x: 40.0,
                y_head: 0.0,
                y_stem_tip: Some(-10.0),
                stem_direction: StemDirection::Up,
                is_rest: false,
            },
        ];

        assert!(determine_tuplet_direction(&notes));
    }

    #[test]
    fn test_tuplet_direction_all_down() {
        let notes = vec![
            TupletNote {
                x: 0.0,
                y_head: 0.0,
                y_stem_tip: Some(10.0),
                stem_direction: StemDirection::Down,
                is_rest: false,
            },
            TupletNote {
                x: 20.0,
                y_head: 0.0,
                y_stem_tip: Some(10.0),
                stem_direction: StemDirection::Down,
                is_rest: false,
            },
            TupletNote {
                x: 40.0,
                y_head: 0.0,
                y_stem_tip: Some(10.0),
                stem_direction: StemDirection::Down,
                is_rest: false,
            },
        ];

        assert!(!determine_tuplet_direction(&notes));
    }

    #[test]
    fn test_layout_tuplet_basic() {
        let notes = vec![
            TupletNote {
                x: 0.0,
                y_head: 0.0,
                y_stem_tip: Some(-15.0),
                stem_direction: StemDirection::Up,
                is_rest: false,
            },
            TupletNote {
                x: 20.0,
                y_head: -2.5,
                y_stem_tip: Some(-17.5),
                stem_direction: StemDirection::Up,
                is_rest: false,
            },
            TupletNote {
                x: 40.0,
                y_head: -5.0,
                y_stem_tip: Some(-20.0),
                stem_direction: StemDirection::Up,
                is_rest: false,
            },
        ];

        let config = TupletConfig::default();
        let result = layout_tuplet(&notes, TupletRatio::triplet(), 1, 5.0, &config, None);

        // Should have commands for bracket and number
        assert!(!result.commands.is_empty());
        // Bounding box should be non-zero
        assert!(result.bbox.width() > 0.0);
        assert!(result.bbox.height() > 0.0);
    }

    #[test]
    fn test_slope_constraint() {
        let p1 = Point::new(0.0, 0.0);
        let p2 = Point::new(100.0, 100.0); // Slope of 1.0

        let (new_p1, new_p2) = apply_slope_constraint(p1, p2, 0.5);

        let new_slope = (new_p2.y - new_p1.y) / (new_p2.x - new_p1.x);
        assert!(new_slope.abs() <= 0.5 + 0.001);
    }

    #[test]
    fn test_interpolate_y() {
        let p1 = Point::new(0.0, 0.0);
        let p2 = Point::new(100.0, 50.0);

        assert!((interpolate_y(p1, p2, 0.0) - 0.0).abs() < 0.001);
        assert!((interpolate_y(p1, p2, 50.0) - 25.0).abs() < 0.001);
        assert!((interpolate_y(p1, p2, 100.0) - 50.0).abs() < 0.001);
    }
}
