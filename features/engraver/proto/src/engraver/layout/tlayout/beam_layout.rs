//! Beam layout system ported from MuseScore.
//!
//! Handles beam rendering for groups of notes, including:
//! - Primary and secondary beams
//! - Beam angle calculation
//! - Beamlets for partial beams
//! - Stem length adjustment

use kurbo::{BezPath, Point, Rect};
use peniko::Color;
use tracing::debug;

use crate::engraver::scene::paint::PaintCommand;

use super::chord::StemDirection;
use super::note::{NoteDuration, NoteHeadType};

/// Configuration for beam layout.
#[derive(Debug, Clone)]
pub struct BeamLayoutConfig {
    /// Beam thickness in spatiums
    pub beam_thickness: f64,
    /// Distance between beam lines (for 16ths, etc.) in spatiums
    pub beam_spacing: f64,
    /// Minimum stem length in spatiums
    pub min_stem_length: f64,
    /// Maximum beam slope (rise per spatium of run)
    pub max_slope: f64,
    /// Beamlet length in spatiums
    pub beamlet_length: f64,
}

impl Default for BeamLayoutConfig {
    fn default() -> Self {
        Self {
            beam_thickness: 0.4,
            beam_spacing: 0.25,
            min_stem_length: 2.75, // Eighth beam minimum; denser beams grow via MIN_STEM_LENGTHS.
            max_slope: 0.5,
            beamlet_length: 1.2,
        }
    }
}

/// Slope constraint for beams (from MuseScore).
///
/// Determines whether a beam should be flat, have a small slope, or be unconstrained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlopeConstraint {
    /// Beam must be completely flat (slope = 0)
    Flat,
    /// Beam has a small slope (max 0.25 quarter spaces)
    SmallSlope,
    /// No constraint on beam slope
    NoConstraint,
}

/// Maximum slopes based on note interval (MuseScore's _maxSlopes table).
/// Index is the interval in half-steps (0 = unison, 1 = second, etc.)
const MAX_SLOPES: [i32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Minimum stem lengths in quarter spaces based on beam count.
/// Index is (beam_count - 1), so beams[0] is for 1 beam (eighth notes).
const MIN_STEM_LENGTHS: [i32; 8] = [11, 13, 15, 18, 21, 24, 27, 30];

/// Information about a note in a beam group.
#[derive(Debug, Clone)]
pub struct BeamNote {
    /// X position relative to beam group start
    pub x: f64,
    /// Primary staff line position (where the main notehead sits — drives
    /// stem attachment when there's only one head).
    pub line: i32,
    /// Top-most line in this chord's notehead stack (max line value across
    /// the primary head + any polyphony extras). Defaults to `line` when no
    /// extras are present. For down-stems, the stem attaches at the TOP of
    /// this line so it visually spans the full chord.
    pub top_line: i32,
    /// Bottom-most line — for up-stems, the stem attaches at the BOTTOM
    /// of this line and extends past the top to the beam.
    pub bottom_line: i32,
    /// Note duration (determines number of beams)
    pub duration: NoteDuration,
    /// Stem direction (should be consistent within beam)
    pub stem_direction: StemDirection,
    /// Notehead type (Normal, Slash, etc.) - affects stem attachment
    pub head_type: NoteHeadType,
}

impl BeamNote {
    /// Y position of notehead center (relative to middle staff line).
    /// Matches `layout_note`'s convention: positive line = up on staff = smaller Y.
    /// Screen coordinates (Y-down), so we negate to convert line to Y.
    pub fn y_center(&self, spatium: f64) -> f64 {
        -self.line as f64 * spatium / 2.0
    }

    /// Number of beam lines needed for this note.
    pub fn beam_count(&self) -> usize {
        self.duration.flag_count() as usize
    }
}

/// Result of beam layout.
#[derive(Debug, Clone)]
pub struct BeamLayout {
    /// Paint commands for the beam
    pub commands: Vec<PaintCommand>,
    /// Adjusted stem endpoints for each note (y position at stem tip)
    pub stem_tips: Vec<f64>,
    /// Bounding box of the entire beam
    pub bbox: Rect,
}

/// Determine the slope constraint based on note positions.
///
/// Implements MuseScore's `getSlopeConstraint` algorithm:
/// - If start and end notes are the same line, beam is flat
/// - If any middle note is more extreme than endpoints, beam is flat
/// - If a middle note equals the extreme endpoint, use small slope
fn get_slope_constraint(notes: &[BeamNote], stem_up: bool) -> SlopeConstraint {
    if notes.len() < 2 {
        return SlopeConstraint::NoConstraint;
    }

    let start_line = notes[0].line;
    // Safe: we checked notes.len() >= 2 above
    let end_line = notes.last().map_or(start_line, |n| n.line);

    // If start and end are the same, beam should be flat
    if start_line == end_line {
        return SlopeConstraint::Flat;
    }

    // For beams with only 2 notes, no additional constraints
    if notes.len() == 2 {
        return SlopeConstraint::NoConstraint;
    }

    // Check middle notes for constraint violations
    // Positive staff-line numbers are higher on the staff in this codebase.
    // The extreme endpoint depends on stem direction:
    // - Stem up: protect the highest note (largest line)
    // - Stem down: protect the lowest note (smallest line)
    let (higher_end, _lower_end) = if stem_up {
        (start_line.max(end_line), start_line.min(end_line))
    } else {
        (start_line.min(end_line), start_line.max(end_line))
    };

    // Get sorted line positions of middle notes
    let middle_lines: Vec<i32> = notes[1..notes.len() - 1].iter().map(|n| n.line).collect();

    for &middle_line in &middle_lines {
        // Check if middle note is more extreme than higher end
        let is_more_extreme = if stem_up {
            middle_line > higher_end // Higher on staff
        } else {
            middle_line < higher_end // Lower on staff
        };

        if is_more_extreme {
            return SlopeConstraint::Flat;
        }

        // Check if middle note equals higher end
        if middle_line == higher_end {
            // Check if it's adjacent to the endpoint
            let is_adjacent = (notes[1].line == higher_end && (start_line == higher_end))
                || (notes[notes.len() - 2].line == higher_end && (end_line == higher_end));

            if is_adjacent {
                return SlopeConstraint::SmallSlope;
            } else {
                return SlopeConstraint::Flat;
            }
        }
    }

    SlopeConstraint::NoConstraint
}

/// Get maximum slope based on beam width in spatiums.
///
/// Longer beams should have shallower maximum slopes.
fn get_max_slope(beam_width_spatiums: f64) -> i32 {
    if beam_width_spatiums < 3.0 {
        MAX_SLOPES[1] // 1
    } else if beam_width_spatiums < 5.0 {
        MAX_SLOPES[2] // 2
    } else if beam_width_spatiums < 7.5 {
        MAX_SLOPES[3] // 3
    } else if beam_width_spatiums < 10.0 {
        MAX_SLOPES[4] // 4
    } else if beam_width_spatiums < 15.0 {
        MAX_SLOPES[5] // 5
    } else if beam_width_spatiums < 20.0 {
        MAX_SLOPES[6] // 6
    } else {
        MAX_SLOPES[7] // 7
    }
}

/// Compute the desired slant in quarter spaces.
///
/// Takes into account:
/// - Note interval (distance between first and last notes)
/// - Beam width (wider beams have shallower slopes)
/// - Slope constraints (flat, small, or unconstrained)
fn compute_desired_slant(notes: &[BeamNote], stem_up: bool, spatium: f64) -> i32 {
    // Need at least 2 notes for a beam with slope
    let (Some(first), Some(last)) = (notes.first(), notes.last()) else {
        return 0;
    };
    if notes.len() < 2 {
        return 0;
    }

    let start_line = first.line;
    let end_line = last.line;

    // Same line = flat beam
    if start_line == end_line {
        return 0;
    }

    // Check slope constraint
    let constraint = get_slope_constraint(notes, stem_up);
    match constraint {
        SlopeConstraint::Flat => return 0,
        SlopeConstraint::SmallSlope => {
            // Small slope: just 1 quarter space
            return if end_line > start_line { 1 } else { -1 };
        }
        SlopeConstraint::NoConstraint => {}
    }

    // Calculate beam width in spatiums
    let beam_width = (last.x - first.x) / spatium;

    // Get max slope based on beam width
    let max_slope = get_max_slope(beam_width);

    // Calculate interval-based slope limit
    let interval = (end_line - start_line).unsigned_abs() as usize;
    let interval_max = MAX_SLOPES[interval.min(MAX_SLOPES.len() - 1)];

    // Use the smaller of the two limits
    let slope_limit = max_slope.min(interval_max);

    // Positive staff lines render higher on the staff (smaller screen Y), so
    // rising notes need a negative screen-space beam slant.
    let direction = if end_line > start_line { -1 } else { 1 };

    // Apply stem direction adjustment (MuseScore multiplies by up ? 1 : -1)
    // But we're working in screen coords where Y-down, so invert for stem-up
    slope_limit * direction
}

/// Get minimum stem length in quarter spaces for a given beam count.
fn get_min_stem_length_qs(beam_count: usize) -> i32 {
    if beam_count == 0 {
        return MIN_STEM_LENGTHS[0];
    }
    let idx = (beam_count - 1).min(MIN_STEM_LENGTHS.len() - 1);
    MIN_STEM_LENGTHS[idx]
}

/// Layout a beam group.
///
/// Returns paint commands for the beams and adjusted stem tip positions.
pub fn layout_beam(notes: &[BeamNote], spatium: f64, config: &BeamLayoutConfig) -> BeamLayout {
    if notes.len() < 2 {
        return BeamLayout {
            commands: Vec::new(),
            stem_tips: Vec::new(),
            bbox: Rect::ZERO,
        };
    }

    // Determine beam direction (all notes in a beam share direction)
    let stem_dir = determine_beam_direction(notes);

    // Calculate beam anchor positions (matching MuseScore's approach)
    // The beam line is defined between start and end anchor points
    // Safe: we checked notes.len() >= 2 above
    let first_note = &notes[0];
    let last_note = &notes[notes.len() - 1];

    // Anchor X positions use Start/End anchor types
    let start_anchor_x =
        chord_beam_anchor_x(first_note, stem_dir, ChordBeamAnchorType::Start, spatium);
    let end_anchor_x = chord_beam_anchor_x(last_note, stem_dir, ChordBeamAnchorType::End, spatium);

    // Calculate beam Y positions based on minimum stem length
    let (start_anchor_y, end_anchor_y) = calculate_beam_position(notes, stem_dir, spatium, config);

    // Calculate stem tips for each note
    let stem_tips = calculate_stem_tips(notes, stem_dir, start_anchor_y, end_anchor_y, spatium);

    // Generate beam commands
    let mut commands = Vec::new();
    let mut bbox = Rect::ZERO;

    // Draw stems from noteheads to beam using SMuFL anchor points
    let stem_width = spatium * STEM_WIDTH;

    // Small overlap to ensure stems fully intersect beam (prevents tiny gaps due to anti-aliasing)
    let stem_beam_overlap = spatium * 0.05;

    // Beam run for Y interpolation (using stem anchor X positions)
    let beam_run = end_anchor_x - start_anchor_x;

    for note in notes.iter() {
        let _note_y = note.y_center(spatium);

        // Calculate the actual stem X position (Middle anchor - center of stem)
        let stem_x = stem_x_for_note(note, stem_dir, spatium);
        let metrics = stem_attachment_metrics(note, stem_x, stem_width, spatium);
        debug!(
            target: "engraver_proto::engraver::layout::tlayout::stem",
            note_x = note.x,
            stem_x,
            stem_left = metrics.stem_left,
            stem_right = metrics.stem_right,
            notehead_left = metrics.notehead_left,
            notehead_right = metrics.notehead_right,
            attached = metrics.attached,
            line = note.line,
            top_line = note.top_line,
            bottom_line = note.bottom_line,
            ?stem_dir,
            head_type = ?note.head_type,
            duration = ?note.duration,
            "[beam-stem] notehead/stem alignment"
        );

        // Calculate beam edge Y at the STEM position by interpolating along the beam line
        // start_anchor_y/end_anchor_y represent the beam EDGE (not center):
        // - For DOWN stems: TOP edge of beam
        // - For UP stems: BOTTOM edge of beam
        let beam_edge_y = if beam_run.abs() < 0.001 {
            start_anchor_y
        } else {
            // Interpolate beam edge Y at the stem X position
            let t = (stem_x - start_anchor_x) / beam_run;
            start_anchor_y + t * (end_anchor_y - start_anchor_y)
        };

        // Stem tip connects directly to the beam edge
        // Add small overlap to extend stem slightly INTO the beam for clean visual connection
        let stem_tip_y = match stem_dir {
            StemDirection::Up | StemDirection::Auto => {
                // Beam is above, stem connects to bottom edge of beam
                // Subtract overlap to extend stem upward into the beam
                beam_edge_y - stem_beam_overlap
            }
            StemDirection::Down => {
                // Beam is below, stem connects to top edge of beam
                // Add overlap to extend stem downward into the beam
                beam_edge_y + stem_beam_overlap
            }
        };

        // Stem attaches at the farthest notehead from the beam so it
        // spans every head in a polyphony stack:
        //   - up-stem (beam above): attach at the BOTTOM-most line
        //   - down-stem (beam below): attach at the TOP-most line
        // For single-pitch beats, top_line == bottom_line == line, so
        // this collapses to the original behaviour.
        let attach_line = match stem_dir {
            StemDirection::Up | StemDirection::Auto => note.bottom_line,
            StemDirection::Down => note.top_line,
        };
        let attach_y = -(attach_line as f64) * spatium / 2.0;
        let stem_attach_y = attach_y + stem_y_offset(stem_dir, note.head_type, spatium);

        // Stem from notehead anchor to beam edge
        let stem_cmd = PaintCommand::line(
            Point::new(stem_x, stem_attach_y),
            Point::new(stem_x, stem_tip_y),
            Color::BLACK,
            stem_width,
        );
        if let Some(cmd_bbox) = stem_cmd.bounding_box() {
            if bbox.is_zero_area() {
                bbox = cmd_bbox;
            } else {
                bbox = bbox.union(cmd_bbox);
            }
        }
        commands.push(stem_cmd);
    }

    // Find the maximum beam level needed
    let max_beams = notes.iter().map(|n| n.beam_count()).max().unwrap_or(0);

    // Draw each beam level
    for level in 0..max_beams {
        let beam_commands = draw_beam_level(
            notes,
            level,
            start_anchor_y,
            end_anchor_y,
            stem_dir,
            spatium,
            config,
        );
        for cmd in &beam_commands {
            if let Some(cmd_bbox) = cmd.bounding_box() {
                if bbox.is_zero_area() {
                    bbox = cmd_bbox;
                } else {
                    bbox = bbox.union(cmd_bbox);
                }
            }
        }
        commands.extend(beam_commands);
    }

    BeamLayout {
        commands,
        stem_tips,
        bbox,
    }
}

/// Determine beam direction based on note positions.
fn determine_beam_direction(notes: &[BeamNote]) -> StemDirection {
    // If any note has explicit direction, use it
    for note in notes {
        if note.stem_direction != StemDirection::Auto {
            return note.stem_direction;
        }
    }

    // Slash noteheads default to stem down (rhythmic notation convention)
    if notes.iter().any(|n| n.head_type == NoteHeadType::Slash) {
        return StemDirection::Down;
    }

    // Calculate average line position
    let avg_line: f64 = notes.iter().map(|n| n.line as f64).sum::<f64>() / notes.len() as f64;

    // Standard rule: stem up if average is below middle line.
    // Notes exactly on the middle line (avg == 0) get stem Up by convention.
    // Must match StemDirection::resolve() in chord.rs for consistency.
    if avg_line > 0.0 {
        StemDirection::Down
    } else {
        StemDirection::Up
    }
}

/// Calculate the Y position of the primary beam at start and end anchor X positions.
///
/// This follows MuseScore's approach:
/// 1. Calculate base beam positions using minimum stem lengths
/// 2. Compute desired slant based on note interval and slope constraints
/// 3. Apply collision avoidance for inner notes
///
/// IMPORTANT: Y positions are calculated at BEAM ANCHOR X positions (Start/End anchors),
/// since the beam is drawn between those positions.
fn calculate_beam_position(
    notes: &[BeamNote],
    stem_dir: StemDirection,
    spatium: f64,
    config: &BeamLayoutConfig,
) -> (f64, f64) {
    // Safe: caller ensures notes is non-empty
    let first = &notes[0];
    let last = &notes[notes.len() - 1];

    // Use the stem-attachment anchor Y, not the notehead center, so stem-length math
    // is correct for any notehead type. Slash heads, for example, anchor up to 1sp
    // off-center, which would otherwise leave beams 1sp too close to the staff.
    let first_y = stem_anchor_y(first, stem_dir, spatium);
    let last_y = stem_anchor_y(last, stem_dir, spatium);

    // Get the beam anchor X positions (where the beam will be drawn)
    let first_anchor_x = chord_beam_anchor_x(first, stem_dir, ChordBeamAnchorType::Start, spatium);
    let last_anchor_x = chord_beam_anchor_x(last, stem_dir, ChordBeamAnchorType::End, spatium);

    // Calculate beam width for slope constraints
    let run = last_anchor_x - first_anchor_x;
    if run.abs() < 0.001 {
        // Notes are at same X position, return flat beam with minimum stem length
        let beam_y = match stem_dir {
            StemDirection::Up | StemDirection::Auto => first_y - config.min_stem_length * spatium,
            StemDirection::Down => first_y + config.min_stem_length * spatium,
        };
        return (beam_y, beam_y);
    }

    // Compute desired slant using slope constraints (from MuseScore algorithm)
    let stem_up = matches!(stem_dir, StemDirection::Up | StemDirection::Auto);
    let desired_slant_qs = compute_desired_slant(notes, stem_up, spatium);

    // Convert slant from quarter spaces to spatium-based Y offset
    // Quarter space = spatium / 4
    let slant_per_spatium = (desired_slant_qs as f64 * spatium / 4.0) / run;

    // Calculate base beam positions ensuring minimum stem length for endpoints
    // Using MuseScore's approach: dictator (extreme endpoint) and pointer (other endpoint)
    let max_beam_count = notes.iter().map(|n| n.beam_count()).max().unwrap_or(1);
    let min_stem_qs = get_min_stem_length_qs(max_beam_count);
    let min_stem_length = (min_stem_qs as f64 / 4.0) * spatium;

    // Use the larger of config min or beam-count-based min
    let effective_min_stem = min_stem_length.max(config.min_stem_length * spatium);

    let (first_base, last_base) = match stem_dir {
        StemDirection::Up | StemDirection::Auto => {
            (first_y - effective_min_stem, last_y - effective_min_stem)
        }
        StemDirection::Down => (first_y + effective_min_stem, last_y + effective_min_stem),
    };

    // Apply the constrained slope
    // Start from the endpoint with the more extreme beam position (dictator)
    let (mut start_y, mut end_y) = if desired_slant_qs == 0 {
        // Flat beam: use the more conservative position
        match stem_dir {
            StemDirection::Up | StemDirection::Auto => {
                let beam_y = first_base.min(last_base);
                (beam_y, beam_y)
            }
            StemDirection::Down => {
                let beam_y = first_base.max(last_base);
                (beam_y, beam_y)
            }
        }
    } else {
        // Sloped beam: apply calculated slant
        let slant_y = slant_per_spatium * run;
        (first_base, first_base + slant_y)
    };

    // Collision avoidance: ensure all inner notes have minimum stem length
    // This is MuseScore's offsetBeamToRemoveCollisions algorithm
    for note in notes.iter() {
        // Stem-anchor Y (not notehead center) so non-standard heads (slash, X) get
        // the same effective stem length as normal noteheads.
        let note_y = stem_anchor_y(note, stem_dir, spatium);
        let note_stem_x = chord_beam_anchor_x(note, stem_dir, ChordBeamAnchorType::Middle, spatium);
        let t = (note_stem_x - first_anchor_x) / run;
        let beam_at_note = start_y + t * (end_y - start_y);

        // Calculate minimum beam position for this note
        let note_beam_count = note.beam_count();
        let note_min_stem_qs = get_min_stem_length_qs(note_beam_count);
        let note_min_stem = (note_min_stem_qs as f64 / 4.0) * spatium;

        let required_beam_y = match stem_dir {
            StemDirection::Up | StemDirection::Auto => note_y - note_min_stem,
            StemDirection::Down => note_y + note_min_stem,
        };

        // Adjust beam if this note would have too short a stem
        match stem_dir {
            StemDirection::Up | StemDirection::Auto => {
                if beam_at_note > required_beam_y {
                    // Need to move beam up (smaller Y)
                    let offset = beam_at_note - required_beam_y;
                    start_y -= offset;
                    end_y -= offset;
                }
            }
            StemDirection::Down => {
                if beam_at_note < required_beam_y {
                    // Need to move beam down (larger Y)
                    let offset = required_beam_y - beam_at_note;
                    start_y += offset;
                    end_y += offset;
                }
            }
        }
    }

    (start_y, end_y)
}

/// Calculate stem tip Y position for each note along the beam.
/// Uses beam anchor X positions (Start/End) for the overall beam line,
/// and Middle anchor for each note's stem position during interpolation.
fn calculate_stem_tips(
    notes: &[BeamNote],
    stem_dir: StemDirection,
    start_y: f64,
    end_y: f64,
    spatium: f64,
) -> Vec<f64> {
    if notes.is_empty() {
        return Vec::new();
    }

    // Use beam anchor X positions (Start/End) for the beam line definition
    // This matches calculate_beam_position and draw_beam_level
    // Safe: we checked notes is non-empty above
    let first_anchor_x =
        chord_beam_anchor_x(&notes[0], stem_dir, ChordBeamAnchorType::Start, spatium);
    let last_anchor_x = chord_beam_anchor_x(
        &notes[notes.len() - 1],
        stem_dir,
        ChordBeamAnchorType::End,
        spatium,
    );
    let run = last_anchor_x - first_anchor_x;

    notes
        .iter()
        .map(|note| {
            if run.abs() < 0.001 {
                start_y
            } else {
                // Interpolate beam Y at this note's STEM position (Middle anchor)
                // The stem is drawn at the center, so we need the beam Y there
                let note_stem_x = stem_x_for_note(note, stem_dir, spatium);
                let t = (note_stem_x - first_anchor_x) / run;
                start_y + t * (end_y - start_y)
            }
        })
        .collect()
}

// ============================================================================
// SMuFL Anchor Points (from Bravura font metadata)
// ============================================================================
// These are the exact anchor points for stem attachment from the SMuFL spec.
// Coordinates are in staff spaces, relative to notehead origin.

/// SMuFL stemUpSE anchor for normal noteheads: attachment point for up-stems (South-East corner).
/// From Bravura metadata noteheadBlack: [1.18, 0.168]
const STEM_UP_SE_X: f64 = 1.18;
const STEM_UP_SE_Y: f64 = 0.168;

/// SMuFL stemDownNW anchor for normal noteheads: attachment point for down-stems (North-West corner).
/// From Bravura metadata noteheadBlack: [0.0, -0.168]
const STEM_DOWN_NW_X: f64 = 0.0;
const STEM_DOWN_NW_Y: f64 = -0.168;

/// SMuFL stemUpSE anchor for slash noteheads (noteheadSlashHorizontalEnds).
/// From Bravura metadata: [2.12, 1.0]
const SLASH_STEM_UP_SE_X: f64 = 2.12;
const SLASH_STEM_UP_SE_Y: f64 = 1.0;

/// SMuFL stemDownNW anchor for slash noteheads (noteheadSlashHorizontalEnds).
/// From Bravura metadata: [0.0, -1.0]
const SLASH_STEM_DOWN_NW_X: f64 = 0.0;
const SLASH_STEM_DOWN_NW_Y: f64 = -1.0;

/// Standard stem width in staff spaces (from MuseScore default).
const STEM_WIDTH: f64 = 0.12;

// ============================================================================
// Beam Anchor Types (matching MuseScore's ChordBeamAnchorType)
// ============================================================================

/// Beam anchor position type, matching MuseScore's ChordBeamAnchorType.
/// This determines how the stem width adjustment is applied for beam connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordBeamAnchorType {
    /// First note of beam group - stem edge towards beam start
    Start,
    /// Middle note of beam group - stem center
    Middle,
    /// Last note of beam group - stem edge towards beam end
    End,
}

// ============================================================================
// MuseScore-compatible Stem Position Functions
// ============================================================================

/// Calculate stem X position relative to notehead origin.
/// Matches MuseScore's `StemLayout::stemPosX(const Chord* item)`.
///
/// Uses SMuFL anchor points from font metadata:
/// - Normal noteheads: stemUpSE.x / stemDownNW.x
/// - Slash noteheads: noteheadSlashHorizontalEnds stemUpSE.x / stemDownNW.x
fn stem_pos_x(stem_dir: StemDirection, head_type: NoteHeadType) -> f64 {
    if head_type == NoteHeadType::Slash {
        // Slash noteheads use SMuFL anchors from noteheadSlashHorizontalEnds
        match stem_dir {
            StemDirection::Up | StemDirection::Auto => SLASH_STEM_UP_SE_X,
            StemDirection::Down => SLASH_STEM_DOWN_NW_X,
        }
    } else {
        // Normal noteheads use SMuFL anchor points
        match stem_dir {
            StemDirection::Up | StemDirection::Auto => STEM_UP_SE_X,
            StemDirection::Down => STEM_DOWN_NW_X,
        }
    }
}

/// Calculate beam anchor X position.
/// Matches MuseScore's `BeamTremoloLayout::chordBeamAnchorX`.
///
/// The key insight is that stem width matters for beam connections:
/// - Start anchor (first note): stem edge facing outward
/// - Middle anchor: stem center
/// - End anchor (last note): stem edge facing outward
fn chord_beam_anchor_x(
    note: &BeamNote,
    stem_dir: StemDirection,
    anchor_type: ChordBeamAnchorType,
    spatium: f64,
) -> f64 {
    let stem_x = note.x + stem_pos_x(stem_dir, note.head_type) * spatium;
    let stem_width = STEM_WIDTH * spatium;

    match anchor_type {
        ChordBeamAnchorType::Start => {
            match stem_dir {
                StemDirection::Up | StemDirection::Auto => {
                    // Up-stem start: subtract full stem width
                    // (stem extends to the left of the anchor point)
                    stem_x - stem_width
                }
                StemDirection::Down => {
                    // Down-stem start: no adjustment
                    stem_x
                }
            }
        }
        ChordBeamAnchorType::Middle => {
            match stem_dir {
                StemDirection::Up | StemDirection::Auto => {
                    // Up-stem middle: subtract half stem width (center)
                    stem_x - stem_width / 2.0
                }
                StemDirection::Down => {
                    // Down-stem middle: add half stem width (center)
                    stem_x + stem_width / 2.0
                }
            }
        }
        ChordBeamAnchorType::End => {
            match stem_dir {
                StemDirection::Up | StemDirection::Auto => {
                    // Up-stem end: no adjustment
                    stem_x
                }
                StemDirection::Down => {
                    // Down-stem end: add full stem width
                    stem_x + stem_width
                }
            }
        }
    }
}

/// Calculate the X position of the stem for a note (for drawing the stem line).
/// Uses the middle anchor type since stem drawing should use stem center.
fn stem_x_for_note(note: &BeamNote, stem_dir: StemDirection, spatium: f64) -> f64 {
    chord_beam_anchor_x(note, stem_dir, ChordBeamAnchorType::Middle, spatium)
}

#[derive(Debug, Clone, Copy)]
struct StemAttachmentMetrics {
    stem_left: f64,
    stem_right: f64,
    notehead_left: f64,
    notehead_right: f64,
    attached: bool,
}

fn stem_attachment_metrics(
    note: &BeamNote,
    stem_x: f64,
    stem_width: f64,
    spatium: f64,
) -> StemAttachmentMetrics {
    let stem_left = stem_x - stem_width / 2.0;
    let stem_right = stem_x + stem_width / 2.0;
    let notehead_left = note.x;
    let notehead_right = note.x + note.head_type.width() * spatium;
    let tolerance = spatium * 0.02;
    let attached =
        stem_right >= notehead_left - tolerance && stem_left <= notehead_right + tolerance;

    StemAttachmentMetrics {
        stem_left,
        stem_right,
        notehead_left,
        notehead_right,
        attached,
    }
}

/// Calculate the Y offset for stem attachment relative to notehead center.
/// Uses SMuFL anchor Y coordinates with Y-flip compensation.
///
/// SMuFL uses Y-up coordinates where positive Y is upward.
/// Our rendering uses Y-down (screen coordinates) where positive Y is downward.
/// The glyph renderer applies `Affine::scale_non_uniform(1.0, -1.0)` to flip Y.
/// Therefore, SMuFL Y coordinates must be negated for our coordinate system.
/// Y position of a note's stem attachment point in beam coordinates.
/// Equals the notehead center Y plus the stem anchor offset for the head type.
fn stem_anchor_y(note: &BeamNote, stem_dir: StemDirection, spatium: f64) -> f64 {
    note.y_center(spatium) + stem_y_offset(stem_dir, note.head_type, spatium)
}

fn stem_y_offset(stem_dir: StemDirection, head_type: NoteHeadType, spatium: f64) -> f64 {
    let (up_y, down_y) = if head_type == NoteHeadType::Slash {
        (SLASH_STEM_UP_SE_Y, SLASH_STEM_DOWN_NW_Y)
    } else {
        (STEM_UP_SE_Y, STEM_DOWN_NW_Y)
    };

    match stem_dir {
        StemDirection::Up | StemDirection::Auto => {
            // SMuFL stemUpSE.y is positive (below center in SMuFL Y-up)
            // After Y-flip, this becomes negative (above center in screen Y-down)
            // But we want stem to attach at SE corner which is below center in screen coords
            // So we negate to get positive (downward) offset
            -up_y * spatium
        }
        StemDirection::Down => {
            // SMuFL stemDownNW.y is negative (above center in SMuFL Y-up)
            // After Y-flip, this becomes positive (below center in screen Y-down)
            // But we want stem to attach at NW corner which is above center in screen coords
            // So we negate to get negative (upward) offset
            -down_y * spatium
        }
    }
}

/// Draw a single beam level (0 = primary beam, 1 = secondary for 16ths, etc.)
fn draw_beam_level(
    notes: &[BeamNote],
    level: usize,
    start_y: f64,
    end_y: f64,
    stem_dir: StemDirection,
    spatium: f64,
    config: &BeamLayoutConfig,
) -> Vec<PaintCommand> {
    let mut commands = Vec::new();

    // Offset for this beam level
    let level_offset = match stem_dir {
        StemDirection::Up | StemDirection::Auto => {
            (config.beam_thickness + config.beam_spacing) * spatium * level as f64
        }
        StemDirection::Down => {
            -(config.beam_thickness + config.beam_spacing) * spatium * level as f64
        }
    };

    // Find segments where this beam level applies
    let segments = find_beam_segments(notes, level);

    // Beam thickness (used for both main beams and beamlets)
    let beam_thickness = config.beam_thickness * spatium;

    // Calculate beam anchor X positions for first and last notes of the entire beam group
    // These define the beam line that all segments interpolate along
    // Following MuseScore: Start anchor for first note, End anchor for last note
    // Safe: caller ensures notes is non-empty
    let first_stem_x =
        chord_beam_anchor_x(&notes[0], stem_dir, ChordBeamAnchorType::Start, spatium);
    let last_stem_x = chord_beam_anchor_x(
        &notes[notes.len() - 1],
        stem_dir,
        ChordBeamAnchorType::End,
        spatium,
    );
    let beam_run = last_stem_x - first_stem_x;

    for (start_idx, end_idx) in segments {
        let segment_start = &notes[start_idx];
        let segment_end = &notes[end_idx];

        // Calculate beam anchor X positions for this segment's endpoints
        // Following MuseScore: Start anchor for first note, End anchor for last note
        // This places the beam at the outer edges of the stems
        let seg_start_x =
            chord_beam_anchor_x(segment_start, stem_dir, ChordBeamAnchorType::Start, spatium);
        let seg_end_x =
            chord_beam_anchor_x(segment_end, stem_dir, ChordBeamAnchorType::End, spatium);

        // Calculate beam Y at segment endpoints
        // Interpolate along the beam line defined from first_stem_x to last_stem_x
        // These Y values represent the beam EDGE where stems connect (not beam center):
        // - For DOWN stems: this is the TOP edge (stems connect from above)
        // - For UP stems: this is the BOTTOM edge (stems connect from below)
        let (seg_start_y, seg_end_y) = if beam_run.abs() < 0.001 {
            (start_y + level_offset, end_y + level_offset)
        } else {
            // Interpolate Y at the segment's X positions along the beam line
            let t1 = (seg_start_x - first_stem_x) / beam_run;
            let t2 = (seg_end_x - first_stem_x) / beam_run;
            let y1 = start_y + t1 * (end_y - start_y) + level_offset;
            let y2 = start_y + t2 * (end_y - start_y) + level_offset;
            (y1, y2)
        };

        // Draw beam as filled polygon
        // The beam is drawn from the stem connection edge in the direction of the stem
        // For DOWN stems: stems connect at top, beam extends downward
        // For UP stems: stems connect at bottom, beam extends upward
        let mut path = BezPath::new();
        match stem_dir {
            StemDirection::Down => {
                // TOP edge is at seg_start_y/seg_end_y (where stems connect)
                // Beam extends DOWNWARD (larger Y)
                path.move_to(Point::new(seg_start_x, seg_start_y)); // top-left
                path.line_to(Point::new(seg_end_x, seg_end_y)); // top-right
                path.line_to(Point::new(seg_end_x, seg_end_y + beam_thickness)); // bottom-right
                path.line_to(Point::new(seg_start_x, seg_start_y + beam_thickness));
                // bottom-left
            }
            StemDirection::Up | StemDirection::Auto => {
                // BOTTOM edge is at seg_start_y/seg_end_y (where stems connect)
                // Beam extends UPWARD (smaller Y)
                path.move_to(Point::new(seg_start_x, seg_start_y - beam_thickness)); // top-left
                path.line_to(Point::new(seg_end_x, seg_end_y - beam_thickness)); // top-right
                path.line_to(Point::new(seg_end_x, seg_end_y)); // bottom-right
                path.line_to(Point::new(seg_start_x, seg_start_y)); // bottom-left
            }
        }
        path.close_path();

        commands.push(PaintCommand::filled_path(path, Color::BLACK));
    }

    // Draw beamlets for isolated notes at this level
    let beamlets = find_beamlets(notes, level);
    for (note_idx, is_before) in beamlets {
        let note = &notes[note_idx];
        let note_stem_x = stem_x_for_note(note, stem_dir, spatium);

        // Calculate beam edge Y at the stem position using the beam line defined by stem anchors
        let note_beam_y = if beam_run.abs() < 0.001 {
            start_y + level_offset
        } else {
            // Interpolate along the beam line from first_stem_x to last_stem_x
            let t = (note_stem_x - first_stem_x) / beam_run;
            start_y + t * (end_y - start_y) + level_offset
        };

        let beamlet_len = config.beamlet_length * spatium;
        let (beamlet_start_x, beamlet_end_x) = if is_before {
            (note_stem_x - beamlet_len, note_stem_x)
        } else {
            (note_stem_x, note_stem_x + beamlet_len)
        };

        // Slope adjustment for beamlet endpoints using the beam line slope
        let slope_per_unit = if beam_run.abs() > 0.001 {
            (end_y - start_y) / beam_run
        } else {
            0.0
        };

        let beamlet_start_y = note_beam_y + slope_per_unit * (beamlet_start_x - note_stem_x);
        let beamlet_end_y = note_beam_y + slope_per_unit * (beamlet_end_x - note_stem_x);

        // Draw beamlet from edge position (matching main beam drawing)
        let mut path = BezPath::new();
        match stem_dir {
            StemDirection::Down => {
                // TOP edge at beamlet_y, extends downward
                path.move_to(Point::new(beamlet_start_x, beamlet_start_y));
                path.line_to(Point::new(beamlet_end_x, beamlet_end_y));
                path.line_to(Point::new(beamlet_end_x, beamlet_end_y + beam_thickness));
                path.line_to(Point::new(
                    beamlet_start_x,
                    beamlet_start_y + beam_thickness,
                ));
            }
            StemDirection::Up | StemDirection::Auto => {
                // BOTTOM edge at beamlet_y, extends upward
                path.move_to(Point::new(
                    beamlet_start_x,
                    beamlet_start_y - beam_thickness,
                ));
                path.line_to(Point::new(beamlet_end_x, beamlet_end_y - beam_thickness));
                path.line_to(Point::new(beamlet_end_x, beamlet_end_y));
                path.line_to(Point::new(beamlet_start_x, beamlet_start_y));
            }
        }
        path.close_path();

        commands.push(PaintCommand::filled_path(path, Color::BLACK));
    }

    commands
}

/// Find continuous segments where a beam level applies.
/// Returns (start_index, end_index) pairs.
fn find_beam_segments(notes: &[BeamNote], level: usize) -> Vec<(usize, usize)> {
    let mut segments = Vec::new();
    let mut segment_start: Option<usize> = None;

    for (i, note) in notes.iter().enumerate() {
        let has_beam_at_level = note.beam_count() > level;

        if has_beam_at_level {
            if segment_start.is_none() {
                segment_start = Some(i);
            }
        } else if let Some(start) = segment_start {
            // End of segment
            if i > start + 1 {
                // Only add if segment has at least 2 notes
                segments.push((start, i - 1));
            }
            segment_start = None;
        }
    }

    // Handle segment at end
    if let Some(start) = segment_start
        && notes.len() > start + 1
    {
        segments.push((start, notes.len() - 1));
    }

    // For primary beam (level 0), ensure we have at least one segment spanning all notes
    if level == 0 && segments.is_empty() && notes.len() >= 2 {
        segments.push((0, notes.len() - 1));
    }

    segments
}

/// Find notes that need beamlets at a given level.
/// Returns (note_index, is_before) pairs.
///
/// Beamlet direction rules (following standard engraving practice):
/// - If the note is the first in the beam group: beamlet points forward (right)
/// - If the note is the last in the beam group: beamlet points backward (left)
/// - Otherwise: beamlet points toward the adjacent note that has a beam at a lower level
fn find_beamlets(notes: &[BeamNote], level: usize) -> Vec<(usize, bool)> {
    let mut beamlets = Vec::new();

    for (i, note) in notes.iter().enumerate() {
        let has_beam = note.beam_count() > level;
        if !has_beam {
            continue;
        }

        // Check if neighbors have this beam level
        let prev_has = i > 0 && notes[i - 1].beam_count() > level;
        let next_has = i < notes.len() - 1 && notes[i + 1].beam_count() > level;

        // Need beamlet if isolated at this level
        if !prev_has && !next_has {
            // Determine beamlet direction:
            // - First note in group: point forward (is_before = false)
            // - Last note or any other position: point backward (is_before = true)
            // This follows the convention that beamlets point toward the adjacent
            // note they rhythmically connect with (e.g., sixteenth after dotted-eighth
            // points back to complete the rhythmic figure)
            let is_before = i > 0;
            beamlets.push((i, is_before));
        }
    }

    beamlets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_all_beam_stems_attached(notes: &[BeamNote], stem_dir: StemDirection, spatium: f64) {
        let stem_width = STEM_WIDTH * spatium;
        for note in notes {
            let stem_x = stem_x_for_note(note, stem_dir, spatium);
            let metrics = stem_attachment_metrics(note, stem_x, stem_width, spatium);
            assert!(
                metrics.attached,
                "beam stem detached from notehead: note={note:?}, stem_dir={stem_dir:?}, metrics={metrics:?}"
            );
        }
    }

    #[test]
    fn test_beam_stems_intersect_notehead_bounds() {
        let spatium = 10.0;
        let up_notes = vec![
            BeamNote {
                x: 0.0,
                line: -5,
                top_line: 6,
                bottom_line: -5,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 18.0,
                line: 6,
                top_line: 6,
                bottom_line: 6,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];
        assert_all_beam_stems_attached(&up_notes, StemDirection::Up, spatium);

        let down_notes = vec![
            BeamNote {
                x: 0.0,
                line: 6,
                top_line: 6,
                bottom_line: -5,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Down,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 18.0,
                line: -5,
                top_line: -5,
                bottom_line: -5,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Down,
                head_type: NoteHeadType::Normal,
            },
        ];
        assert_all_beam_stems_attached(&down_notes, StemDirection::Down, spatium);
    }

    #[test]
    fn test_beam_direction_auto() {
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: -2,
                top_line: -2,
                bottom_line: -2,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Auto,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 20.0,
                line: -4,
                top_line: -4,
                bottom_line: -4,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Auto,
                head_type: NoteHeadType::Normal,
            },
        ];

        let dir = determine_beam_direction(&notes);
        assert_eq!(dir, StemDirection::Up); // Notes above middle line
    }

    #[test]
    fn test_beam_two_eighths() {
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 25.0,
                line: -2,
                top_line: -2,
                bottom_line: -2,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let config = BeamLayoutConfig::default();
        let result = layout_beam(&notes, 5.0, &config);

        assert!(!result.commands.is_empty());
        assert_eq!(result.stem_tips.len(), 2);
    }

    #[test]
    fn test_beam_with_sixteenths() {
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Sixteenth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 15.0,
                line: -1,
                top_line: -1,
                bottom_line: -1,
                duration: NoteDuration::Sixteenth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 30.0,
                line: -2,
                top_line: -2,
                bottom_line: -2,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let config = BeamLayoutConfig::default();
        let result = layout_beam(&notes, 5.0, &config);

        // Should have primary beam and partial secondary beam
        assert!(!result.commands.is_empty());
    }

    #[test]
    fn test_slope_constraint_same_line_is_flat() {
        // Two notes on the same line should produce a flat beam
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 25.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let constraint = get_slope_constraint(&notes, true);
        assert_eq!(constraint, SlopeConstraint::Flat);
    }

    #[test]
    fn test_slope_constraint_middle_more_extreme_is_flat() {
        // Middle note higher than both endpoints (stem up) should be flat
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 15.0,
                line: 4,
                top_line: 4,
                bottom_line: 4, // Higher on staff (more extreme for stem-up)
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 30.0,
                line: 2,
                top_line: 2,
                bottom_line: 2,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let constraint = get_slope_constraint(&notes, true);
        assert_eq!(constraint, SlopeConstraint::Flat);
    }

    #[test]
    fn test_slope_constraint_two_notes_unconstrained() {
        // Two notes at different lines should be unconstrained
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 25.0,
                line: -2,
                top_line: -2,
                bottom_line: -2,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let constraint = get_slope_constraint(&notes, true);
        assert_eq!(constraint, SlopeConstraint::NoConstraint);
    }

    #[test]
    fn test_compute_desired_slant_flat() {
        // Same line should produce slant of 0
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 25.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let slant = compute_desired_slant(&notes, true, 5.0);
        assert_eq!(slant, 0);
    }

    #[test]
    fn test_compute_desired_slant_interval() {
        // Different lines should produce non-zero slant
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 25.0,
                line: 2,
                top_line: 2,
                bottom_line: 2, // Interval of 2
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let slant = compute_desired_slant(&notes, true, 5.0);
        // Should be negative (beam goes up as notes go up on staff)
        assert!(slant < 0);
    }

    #[test]
    fn rising_beam_group_uses_diagonal_beam() {
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 0,
                top_line: 0,
                bottom_line: 0,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 15.0,
                line: 1,
                top_line: 1,
                bottom_line: 1,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 30.0,
                line: 2,
                top_line: 2,
                bottom_line: 2,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Up,
                head_type: NoteHeadType::Normal,
            },
        ];

        let (start_y, end_y) =
            calculate_beam_position(&notes, StemDirection::Up, 5.0, &BeamLayoutConfig::default());

        assert!(
            end_y < start_y,
            "rising up-stem notes should produce an upward diagonal beam, got start_y={start_y}, end_y={end_y}"
        );
    }

    #[test]
    fn rising_down_stem_beam_group_uses_upward_diagonal_beam() {
        let notes = vec![
            BeamNote {
                x: 0.0,
                line: 6,
                top_line: 6,
                bottom_line: 6,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Down,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 15.0,
                line: 7,
                top_line: 7,
                bottom_line: 7,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Down,
                head_type: NoteHeadType::Normal,
            },
            BeamNote {
                x: 30.0,
                line: 8,
                top_line: 8,
                bottom_line: 8,
                duration: NoteDuration::Eighth,
                stem_direction: StemDirection::Down,
                head_type: NoteHeadType::Normal,
            },
        ];

        let (start_y, end_y) = calculate_beam_position(
            &notes,
            StemDirection::Down,
            5.0,
            &BeamLayoutConfig::default(),
        );

        assert!(
            end_y < start_y,
            "rising down-stem notes should still produce an upward diagonal beam, got start_y={start_y}, end_y={end_y}"
        );
    }

    #[test]
    fn test_get_max_slope_narrow_beam() {
        // Narrow beams should have smaller max slope
        assert_eq!(get_max_slope(2.0), 1);
        assert_eq!(get_max_slope(4.0), 2);
        assert_eq!(get_max_slope(6.0), 3);
    }

    #[test]
    fn test_get_min_stem_length_by_beam_count() {
        // More beams require longer stems
        assert_eq!(get_min_stem_length_qs(1), 11);
        assert_eq!(get_min_stem_length_qs(2), 13);
        assert_eq!(get_min_stem_length_qs(3), 15);
        assert_eq!(get_min_stem_length_qs(4), 18);
    }

    #[test]
    fn default_beam_config_uses_eighth_note_stem_minimum() {
        let config = BeamLayoutConfig::default();

        assert_eq!(config.min_stem_length, 2.75);
        assert_eq!(
            config.min_stem_length,
            f64::from(get_min_stem_length_qs(1)) / 4.0
        );
    }
}
