//! Chord layout implementation.
//!
//! Handles layout of chords (groups of notes), including stems, flags,
//! and proper notehead stacking for seconds.

use kurbo::{Point, Rect};
use peniko::Color;
use tracing::debug;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::SemanticId;
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;
use super::note::{Accidental, NoteDuration, NoteHeadType, NoteParams, layout_note};

/// SMuFL codepoints for stems and flags.
pub mod glyphs {
    /// Flag for eighth note (up stem)
    pub const FLAG_EIGHTH_UP: char = '\u{E240}';
    /// Flag for eighth note (down stem)
    pub const FLAG_EIGHTH_DOWN: char = '\u{E241}';
    /// Flag for sixteenth note (up stem)
    pub const FLAG_SIXTEENTH_UP: char = '\u{E242}';
    /// Flag for sixteenth note (down stem)
    pub const FLAG_SIXTEENTH_DOWN: char = '\u{E243}';
    /// Flag for 32nd note (up stem)
    pub const FLAG_32ND_UP: char = '\u{E244}';
    /// Flag for 32nd note (down stem)
    pub const FLAG_32ND_DOWN: char = '\u{E245}';
    /// Flag for 64th note (up stem)
    pub const FLAG_64TH_UP: char = '\u{E246}';
    /// Flag for 64th note (down stem)
    pub const FLAG_64TH_DOWN: char = '\u{E247}';
}

/// Stem direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemDirection {
    Up,
    Down,
    Auto,
}

impl StemDirection {
    /// Resolve auto direction based on note positions and head type.
    #[must_use]
    pub fn resolve(self, avg_line: f64) -> Self {
        match self {
            Self::Auto => {
                // Standard rule: stem down if average note is above middle line
                if avg_line > 0.0 { Self::Down } else { Self::Up }
            }
            _ => self,
        }
    }

    /// Resolve auto direction for a specific notehead type.
    /// Slash noteheads default to stem down (rhythmic notation convention).
    #[must_use]
    pub fn resolve_for_head_type(
        self,
        avg_line: f64,
        head_type: super::note::NoteHeadType,
    ) -> Self {
        match self {
            Self::Auto => {
                if head_type == super::note::NoteHeadType::Slash {
                    // Rhythmic slash notation: stems down by default
                    Self::Down
                } else {
                    // Standard rule: stem down if average note is above middle line
                    if avg_line > 0.0 { Self::Down } else { Self::Up }
                }
            }
            _ => self,
        }
    }
}

/// A single note within a chord.
#[derive(Debug, Clone)]
pub struct ChordNote {
    /// Staff line position
    pub line: i32,
    /// Accidental
    pub accidental: Accidental,
    /// Tie to next note
    pub tie: bool,
}

/// Chord layout parameters.
#[derive(Debug, Clone)]
pub struct ChordParams {
    /// Unique identifier
    pub id: u64,
    /// Duration type
    pub duration: NoteDuration,
    /// Notehead type (Normal, Slash, etc.)
    pub head_type: NoteHeadType,
    /// Notes in the chord (sorted by line)
    pub notes: Vec<ChordNote>,
    /// Stem direction
    pub stem_direction: StemDirection,
    /// Number of augmentation dots
    pub dots: u8,
    /// Whether chord is part of a beam (no flags)
    pub beamed: bool,
    /// Whether to hide the stem (stemless notation)
    pub stemless: bool,
}

impl Default for ChordParams {
    fn default() -> Self {
        Self {
            id: 0,
            duration: NoteDuration::Quarter,
            head_type: NoteHeadType::Normal,
            notes: vec![ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            }],
            stem_direction: StemDirection::Auto,
            dots: 0,
            beamed: false,
            stemless: false,
        }
    }
}

/// Layout a chord (group of notes with stem).
#[must_use]
pub fn layout_chord(params: &ChordParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();

    // Sort notes by line position
    let mut sorted_notes = params.notes.clone();
    sorted_notes.sort_by_key(|n| n.line);

    if sorted_notes.is_empty() {
        // Empty chord - return minimal layout
        let layout = LayoutData::new(Point::ZERO, Rect::ZERO, Shape::empty());
        let node = SceneNode::group(SemanticId::chord(params.id));
        return (layout, node);
    }

    // Calculate average line for stem direction
    let avg_line: f64 =
        sorted_notes.iter().map(|n| n.line as f64).sum::<f64>() / sorted_notes.len() as f64;

    let stem_dir = params
        .stem_direction
        .resolve_for_head_type(avg_line, params.head_type);

    // Create chord group node
    let mut chord_node = SceneNode::group(SemanticId::chord(params.id));
    let mut total_bbox = Rect::ZERO;

    // Calculate which noteheads need to be offset (for seconds)
    let notehead_offsets = calculate_notehead_offsets(&sorted_notes, stem_dir);

    // Shared accidental column for the chord. All accidentals are right-aligned
    // inside a column of `max_acc_width_sp` spatiums; every notehead then lands
    // at the same X regardless of which accidental (if any) it carries.
    let max_acc_width_sp = sorted_notes
        .iter()
        .filter(|n| n.accidental != Accidental::None)
        .map(|n| n.accidental.width())
        .fold(0.0f64, f64::max);
    let has_any_accidental = max_acc_width_sp > 0.0;
    let column_width_param = has_any_accidental.then_some(max_acc_width_sp);
    // Layout each note
    for (i, note) in sorted_notes.iter().enumerate() {
        let note_params = NoteParams {
            id: params.id * 1000 + i as u64,
            duration: params.duration,
            head_type: params.head_type,
            line: note.line,
            accidental: note.accidental,
            dots: if i == 0 { params.dots } else { 0 }, // Only first note gets dots
            offset_x: notehead_offsets[i],
            ledger_lines: i == 0 || i == sorted_notes.len() - 1, // Only top/bottom get ledgers
            accidental_column_width: column_width_param,
        };

        let (note_layout, note_node) = layout_note(&note_params, ctx);

        // Calculate world-space bbox (note_layout.bbox is relative to note_layout.position)
        let world_bbox = Rect::new(
            note_layout.bbox.x0 + note_layout.position.x,
            note_layout.bbox.y0 + note_layout.position.y,
            note_layout.bbox.x1 + note_layout.position.x,
            note_layout.bbox.y1 + note_layout.position.y,
        );

        // Expand total bounding box
        if total_bbox.is_zero_area() {
            total_bbox = world_bbox;
        } else {
            total_bbox = total_bbox.union(world_bbox);
        }

        chord_node.add_child(note_node);
    }

    // Add stem if required (and not stemless)
    if params.duration.has_stem() && !params.stemless {
        let stem_commands = draw_stem(
            &sorted_notes,
            stem_dir,
            &notehead_offsets,
            params.head_type,
            params.duration,
            spatium,
        );

        let stem_node = SceneNode::anonymous_leaf(stem_commands);
        chord_node.add_child(stem_node);

        // Add flags if not beamed
        if !params.beamed && params.duration.flag_count() > 0 {
            let flag_commands = draw_flags(
                &sorted_notes,
                stem_dir,
                &notehead_offsets,
                params.duration,
                params.head_type,
                spatium,
            );

            if !flag_commands.is_empty() {
                let flag_node = SceneNode::anonymous_leaf(flag_commands);
                chord_node.add_child(flag_node);
            }
        }
    }

    // Create collision shape
    let shape = Shape::from_rect(total_bbox);
    let layout = LayoutData::new(Point::ZERO, total_bbox, shape);

    (layout, chord_node)
}

/// Calculate notehead X offsets for chord voicing.
///
/// Implements MuseScore's layoutChords2 algorithm:
/// - Detects conflicts (notes within a second/unison)
/// - Alternates noteheads to opposite sides of stem when conflicts occur
/// - For up-stems: iterate bottom-to-top, conflicting notes go right
/// - For down-stems: iterate top-to-bottom, conflicting notes go left
fn calculate_notehead_offsets(notes: &[ChordNote], stem_dir: StemDirection) -> Vec<f64> {
    if notes.is_empty() {
        return Vec::new();
    }

    let mut offsets = vec![0.0; notes.len()];
    let notehead_width = 1.18; // In spatiums (from SMuFL noteheadBlack width)

    // Determine iteration direction based on stem direction
    // For up-stems: loop bottom-to-top (index 0 is bottom, already sorted)
    // For down-stems: loop top-to-bottom (reverse order)
    let is_up = matches!(stem_dir, StemDirection::Up | StemDirection::Auto);

    let (start_idx, end_idx, inc): (i32, i32, i32) = if is_up {
        (0, notes.len() as i32, 1)
    } else {
        (notes.len() as i32 - 1, -1, -1)
    };

    // Track state across iterations
    let mut prev_line: i32 = 1000; // Start high so first note won't conflict
    let mut is_left = is_up; // Notes start on stem side (left for up-stem, right for down-stem)

    let mut idx = start_idx;
    while idx != end_idx {
        let i = idx as usize;
        let line = notes[i].line;

        // Conflict exists if this note is within a second (adjacent line) or unison (same line)
        let conflict = (line - prev_line).abs() < 2;

        // Toggle side when there's a conflict or when we need to return to stem side
        if conflict {
            is_left = !is_left;
        }

        // Note needs to be offset (mirrored) if it's on the opposite side from stem
        // Up-stem default: notes on left (is_left=true means no offset)
        // Down-stem default: notes on right (is_left=false means no offset)
        let needs_mirror = if is_up { !is_left } else { is_left };

        if needs_mirror {
            // Offset direction depends on stem direction
            // Up-stem mirrored notes go to the right (positive offset)
            // Down-stem mirrored notes go to the left (negative offset)
            offsets[i] = if is_up {
                notehead_width
            } else {
                -notehead_width
            };
        }

        prev_line = line;
        idx += inc;
    }

    offsets
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

/// SMuFL stemUpSE anchor for diamond noteheads (noteheadSlashDiamondWhite).
/// From Bravura metadata: [2.0, 0.0] - stem connects at center of diamond
const DIAMOND_STEM_UP_SE_X: f64 = 2.0;
const DIAMOND_STEM_UP_SE_Y: f64 = 0.0;

/// SMuFL stemDownNW anchor for diamond noteheads (noteheadSlashDiamondWhite).
/// From Bravura metadata: [0.0, 0.0] - stem connects at center of diamond
const DIAMOND_STEM_DOWN_NW_X: f64 = 0.0;
const DIAMOND_STEM_DOWN_NW_Y: f64 = 0.0;

/// Standard stem width in staff spaces (from MuseScore default).
const STEM_WIDTH: f64 = 0.12;

#[derive(Debug, Clone, Copy)]
struct StemAttachmentMetrics {
    stem_left: f64,
    stem_right: f64,
    notehead_left: f64,
    notehead_right: f64,
    attached: bool,
}

fn stem_side_notehead_offset(stem_dir: StemDirection, offsets: &[f64]) -> f64 {
    match stem_dir {
        StemDirection::Up | StemDirection::Auto => offsets.first().copied(),
        StemDirection::Down => offsets.last().copied(),
    }
    .unwrap_or(0.0)
}

fn stem_attachment_metrics(
    stem_x: f64,
    stem_width: f64,
    notehead_x: f64,
    head_type: NoteHeadType,
    spatium: f64,
) -> StemAttachmentMetrics {
    let stem_left = stem_x - stem_width / 2.0;
    let stem_right = stem_x + stem_width / 2.0;
    let notehead_left = notehead_x;
    let notehead_right = notehead_x + head_type.width() * spatium;
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

/// Draw the stem for a chord.
/// Uses SMuFL anchor points for normal noteheads, proportional positioning for slash noteheads.
fn draw_stem(
    notes: &[ChordNote],
    stem_dir: StemDirection,
    offsets: &[f64],
    head_type: NoteHeadType,
    duration: NoteDuration,
    spatium: f64,
) -> Vec<PaintCommand> {
    // Early return if no notes (shouldn't happen but guard against panic)
    let (Some(top_note), Some(bottom_note)) = (notes.last(), notes.first()) else {
        return Vec::new();
    };

    let stem_width = STEM_WIDTH * spatium;
    let stem_length = spatium * 3.5;

    let top_y = -top_note.line as f64 * spatium / 2.0;
    let bottom_y = -bottom_note.line as f64 * spatium / 2.0;
    let stem_notehead_x_offset = stem_side_notehead_offset(stem_dir, offsets);

    // Determine if we're using diamond notehead (for half/whole in slash mode)
    let uses_diamond = head_type == NoteHeadType::Slash
        && matches!(
            duration,
            NoteDuration::Half | NoteDuration::Whole | NoteDuration::DoubleWhole
        );

    let (stem_x, stem_start_y, stem_end_y) = match stem_dir {
        StemDirection::Up | StemDirection::Auto => {
            // Use SMuFL anchors from Bravura font metadata
            let (anchor_x, anchor_y) = if uses_diamond {
                (DIAMOND_STEM_UP_SE_X, DIAMOND_STEM_UP_SE_Y)
            } else if head_type == NoteHeadType::Slash {
                (SLASH_STEM_UP_SE_X, SLASH_STEM_UP_SE_Y)
            } else {
                (STEM_UP_SE_X, STEM_UP_SE_Y)
            };
            let x = stem_notehead_x_offset + anchor_x * spatium - stem_width / 2.0;
            let y_offset = -anchor_y * spatium;
            let start = bottom_y + y_offset;
            let end = top_y - stem_length;
            (x, start, end)
        }
        StemDirection::Down => {
            // Use SMuFL anchors from Bravura font metadata
            let (anchor_x, anchor_y) = if uses_diamond {
                (DIAMOND_STEM_DOWN_NW_X, DIAMOND_STEM_DOWN_NW_Y)
            } else if head_type == NoteHeadType::Slash {
                (SLASH_STEM_DOWN_NW_X, SLASH_STEM_DOWN_NW_Y)
            } else {
                (STEM_DOWN_NW_X, STEM_DOWN_NW_Y)
            };
            let x = stem_notehead_x_offset + anchor_x * spatium + stem_width / 2.0;
            let y_offset = -anchor_y * spatium;
            let start = top_y + y_offset;
            let end = bottom_y + stem_length;
            (x, start, end)
        }
    };

    let metrics = stem_attachment_metrics(
        stem_x,
        stem_width,
        stem_notehead_x_offset,
        head_type,
        spatium,
    );
    debug!(
        target: "engraver_proto::engraver::layout::tlayout::stem",
        stem_x,
        stem_left = metrics.stem_left,
        stem_right = metrics.stem_right,
        notehead_left = metrics.notehead_left,
        notehead_right = metrics.notehead_right,
        attached = metrics.attached,
        ?stem_dir,
        ?head_type,
        ?duration,
        top_line = top_note.line,
        bottom_line = bottom_note.line,
        notehead_offset = stem_notehead_x_offset,
        "[chord-stem] notehead/stem alignment"
    );

    vec![PaintCommand::line(
        Point::new(stem_x, stem_start_y),
        Point::new(stem_x, stem_end_y),
        Color::BLACK,
        stem_width,
    )]
}

/// Draw flags for a chord (eighth notes and shorter).
/// Uses SMuFL anchor points for normal noteheads, proportional positioning for slash noteheads.
fn draw_flags(
    notes: &[ChordNote],
    stem_dir: StemDirection,
    offsets: &[f64],
    duration: NoteDuration,
    head_type: NoteHeadType,
    spatium: f64,
) -> Vec<PaintCommand> {
    let flag_count = duration.flag_count();
    if flag_count == 0 {
        return Vec::new();
    }

    // Early return if no notes (shouldn't happen but guard against panic)
    let (Some(top_note), Some(bottom_note)) = (notes.last(), notes.first()) else {
        return Vec::new();
    };

    let stem_length = spatium * 3.5;
    let stem_width = STEM_WIDTH * spatium;
    let stem_notehead_x_offset = stem_side_notehead_offset(stem_dir, offsets);

    // Use same anchor calculations as draw_stem for consistent flag positioning
    let (flag_x, flag_y, glyph) = match stem_dir {
        StemDirection::Up | StemDirection::Auto => {
            // Flag attaches at stem tip - use same X as stem
            let anchor_x = if head_type == NoteHeadType::Slash {
                SLASH_STEM_UP_SE_X
            } else {
                STEM_UP_SE_X
            };
            let x = stem_notehead_x_offset + anchor_x * spatium - stem_width / 2.0;
            let top_y = -top_note.line as f64 * spatium / 2.0;
            let y = top_y - stem_length;
            let g = match flag_count {
                1 => glyphs::FLAG_EIGHTH_UP,
                2 => glyphs::FLAG_SIXTEENTH_UP,
                3 => glyphs::FLAG_32ND_UP,
                _ => glyphs::FLAG_64TH_UP,
            };
            (x, y, g)
        }
        StemDirection::Down => {
            // Flag attaches at stem tip - use same X as stem
            let anchor_x = if head_type == NoteHeadType::Slash {
                SLASH_STEM_DOWN_NW_X
            } else {
                STEM_DOWN_NW_X
            };
            let x = stem_notehead_x_offset + anchor_x * spatium + stem_width / 2.0;
            let bottom_y = -bottom_note.line as f64 * spatium / 2.0;
            let y = bottom_y + stem_length;
            let g = match flag_count {
                1 => glyphs::FLAG_EIGHTH_DOWN,
                2 => glyphs::FLAG_SIXTEENTH_DOWN,
                3 => glyphs::FLAG_32ND_DOWN,
                _ => glyphs::FLAG_64TH_DOWN,
            };
            (x, y, g)
        }
    };

    vec![PaintCommand::glyph(
        glyph,
        Point::new(flag_x, flag_y),
        spatium,
        Color::BLACK,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::layout::context::LayoutConfiguration;
    use crate::engraver::style::MStyle;

    fn test_ctx() -> LayoutContext<'static> {
        let config = LayoutConfiguration::default();
        let style = Box::leak(Box::new(MStyle::default()));
        LayoutContext::new_for_test(config, style)
    }

    #[test]
    fn test_layout_single_note_chord() {
        let ctx = test_ctx();
        let params = ChordParams {
            id: 1,
            duration: NoteDuration::Quarter,
            notes: vec![ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            }],
            ..Default::default()
        };

        let (layout, node) = layout_chord(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
    }

    #[test]
    fn test_layout_two_note_chord() {
        let ctx = test_ctx();
        let params = ChordParams {
            id: 2,
            duration: NoteDuration::Quarter,
            notes: vec![
                ChordNote {
                    line: 0,
                    accidental: Accidental::None,
                    tie: false,
                },
                ChordNote {
                    line: 4,
                    accidental: Accidental::None,
                    tie: false,
                },
            ],
            ..Default::default()
        };

        let (layout, node) = layout_chord(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        // Should have multiple children (notes + stem)
        assert!(node.children.len() >= 2);
    }

    #[test]
    fn test_layout_chord_with_second() {
        let ctx = test_ctx();
        let params = ChordParams {
            id: 3,
            duration: NoteDuration::Quarter,
            notes: vec![
                ChordNote {
                    line: 0,
                    accidental: Accidental::None,
                    tie: false,
                },
                ChordNote {
                    line: 1,
                    accidental: Accidental::None,
                    tie: false,
                }, // Second
            ],
            ..Default::default()
        };

        let (layout, _node) = layout_chord(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
    }

    #[test]
    fn test_stem_direction_auto() {
        // Notes below middle line -> stem up
        assert_eq!(StemDirection::Auto.resolve(-2.0), StemDirection::Up);

        // Notes above middle line -> stem down
        assert_eq!(StemDirection::Auto.resolve(2.0), StemDirection::Down);

        // Notes at middle line -> stem up (convention)
        assert_eq!(StemDirection::Auto.resolve(0.0), StemDirection::Up);
    }

    #[test]
    fn test_layout_eighth_note_with_flag() {
        let ctx = test_ctx();
        let params = ChordParams {
            id: 4,
            duration: NoteDuration::Eighth,
            notes: vec![ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            }],
            beamed: false,
            ..Default::default()
        };

        let (layout, node) = layout_chord(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        // Should have note + stem + flag
        assert!(node.children.len() >= 2);
    }

    #[test]
    fn test_beamed_chord_no_flag() {
        let ctx = test_ctx();
        let params = ChordParams {
            id: 5,
            duration: NoteDuration::Eighth,
            notes: vec![ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            }],
            beamed: true, // Part of a beam group
            ..Default::default()
        };

        let (layout, _node) = layout_chord(&params, &ctx);

        // Beamed chords should not have flag children
        // Only note + stem
        assert!(!layout.bbox.is_zero_area());
    }

    #[test]
    fn test_notehead_offsets_no_conflict() {
        // Notes that are NOT adjacent should have no offsets
        let notes = vec![
            ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            },
            ChordNote {
                line: 2,
                accidental: Accidental::None,
                tie: false,
            }, // Third apart
            ChordNote {
                line: 4,
                accidental: Accidental::None,
                tie: false,
            }, // Third apart
        ];
        let offsets = calculate_notehead_offsets(&notes, StemDirection::Up);
        assert_eq!(offsets, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_notehead_offsets_second_up_stem() {
        // Two notes a second apart with up-stem
        // Lower note should be offset to the right
        let notes = vec![
            ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            },
            ChordNote {
                line: 1,
                accidental: Accidental::None,
                tie: false,
            }, // Second
        ];
        let offsets = calculate_notehead_offsets(&notes, StemDirection::Up);
        // First note at 0, second note offset right
        assert_eq!(offsets[0], 0.0);
        assert!(
            offsets[1] > 0.0,
            "Second note should be offset right for up-stem"
        );
    }

    #[test]
    fn test_notehead_offsets_second_down_stem() {
        // Two notes a second apart with down-stem
        // Higher note should be offset to the left
        let notes = vec![
            ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            },
            ChordNote {
                line: 1,
                accidental: Accidental::None,
                tie: false,
            }, // Second
        ];
        let offsets = calculate_notehead_offsets(&notes, StemDirection::Down);
        // For down-stem, we iterate top-to-bottom, first note (top, line 1) stays
        // Second note (bottom, line 0) goes left (negative offset)
        assert!(
            offsets[0] < 0.0,
            "Lower note should be offset left for down-stem"
        );
        assert_eq!(offsets[1], 0.0);
    }

    #[test]
    fn test_notehead_offsets_cluster() {
        // Three notes in a cluster (consecutive seconds) with up-stem
        let notes = vec![
            ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            },
            ChordNote {
                line: 1,
                accidental: Accidental::None,
                tie: false,
            },
            ChordNote {
                line: 2,
                accidental: Accidental::None,
                tie: false,
            },
        ];
        let offsets = calculate_notehead_offsets(&notes, StemDirection::Up);
        // For up-stem, alternating: 0, right, 0 or similar pattern
        // First note on left (0), second on right, third back on left (0)
        assert_eq!(offsets[0], 0.0);
        assert!(offsets[1] > 0.0);
        assert_eq!(offsets[2], 0.0);
    }

    #[test]
    fn test_notehead_offsets_unison() {
        // Two notes on the same line (unison) - treated as conflict
        let notes = vec![
            ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            },
            ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            }, // Unison
        ];
        let offsets = calculate_notehead_offsets(&notes, StemDirection::Up);
        // One should be offset
        assert!(
            offsets[0] != 0.0 || offsets[1] != 0.0,
            "Unison notes should have at least one offset"
        );
    }

    #[test]
    fn test_stem_attachment_ignores_accidental_column() {
        let spatium = 10.0;
        let notes = vec![ChordNote {
            line: 0,
            accidental: Accidental::Sharp,
            tie: false,
        }];
        let offsets = calculate_notehead_offsets(&notes, StemDirection::Up);
        let commands = draw_stem(
            &notes,
            StemDirection::Up,
            &offsets,
            NoteHeadType::Normal,
            NoteDuration::Quarter,
            spatium,
        );

        let PaintCommand::Line { start, width, .. } = commands[0] else {
            panic!("expected stem line");
        };
        let metrics =
            stem_attachment_metrics(start.x, width, offsets[0], NoteHeadType::Normal, spatium);
        assert!(
            metrics.attached,
            "accidental column must not detach stem from anchored notehead: {metrics:?}"
        );
    }

    #[test]
    fn test_stem_attachment_uses_stem_side_offset_for_seconds() {
        let spatium = 10.0;
        let notes = vec![
            ChordNote {
                line: 0,
                accidental: Accidental::None,
                tie: false,
            },
            ChordNote {
                line: 1,
                accidental: Accidental::None,
                tie: false,
            },
        ];
        let offsets = calculate_notehead_offsets(&notes, StemDirection::Down);
        let commands = draw_stem(
            &notes,
            StemDirection::Down,
            &offsets,
            NoteHeadType::Normal,
            NoteDuration::Quarter,
            spatium,
        );

        let PaintCommand::Line { start, width, .. } = commands[0] else {
            panic!("expected stem line");
        };
        let stem_side_offset = stem_side_notehead_offset(StemDirection::Down, &offsets);
        let metrics = stem_attachment_metrics(
            start.x,
            width,
            stem_side_offset,
            NoteHeadType::Normal,
            spatium,
        );
        assert!(
            metrics.attached,
            "stem must attach to the notehead on the stem side for seconds: {metrics:?}"
        );
    }
}
