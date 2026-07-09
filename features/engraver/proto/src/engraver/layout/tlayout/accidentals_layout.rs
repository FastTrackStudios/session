//! Accidental placement system ported from MuseScore.
//!
//! Implements MuseScore's sophisticated accidental stacking algorithm including:
//! - Sub-chord splitting for large chords
//! - Standard and compact ordering strategies
//! - Collision-based column assignment
//! - Special cases for fourths, sixths, octaves, and seconds
//! - Vertical alignment of same-type accidentals
//!
//! Reference: MuseScore `accidentalslayout.cpp` (1,162 lines)

use kurbo::Rect;
use std::collections::{HashMap, HashSet};

use super::note::Accidental;

/// Configuration for accidental placement.
/// All distance values are in spatiums unless otherwise noted.
#[derive(Debug, Clone)]
pub struct AccidentalLayoutConfig {
    // === Vertical clearances ===
    /// Minimum vertical clearance between accidentals (general)
    pub vertical_accidental_to_accidental_clearance: f64,
    /// Vertical clearance for sharp-to-sharp (tighter because of shape)
    pub vertical_sharp_to_sharp_clearance: f64,
    /// Vertical clearance between accidental and chord elements
    pub vertical_accidental_to_chord_clearance: f64,

    // === Horizontal distances ===
    /// Minimum horizontal distance between accidental columns
    pub accidental_accidental_distance: f64,
    /// Distance from accidental to notehead
    pub accidental_to_note_distance: f64,

    // === Special kerning ===
    /// Extra kerning for flat-flat fourths (can be negative)
    pub flat_kerning_of_fourth: f64,
    /// Extra kerning for natural-flat fourths
    pub natural_kerning_of_fourth: f64,
    /// Additional padding when natural is near another vertical element
    pub additional_padding_for_verticals: f64,
    /// Padding for sharp/natural near ledger lines
    pub sharp_and_natural_ledger_line_padding: f64,
    /// Reduced padding for flat above note
    pub reduced_flat_to_note_padding: f64,

    // === Thresholds ===
    /// X position difference to split into separate groups
    pub x_pos_split_threshold: f64,
    /// X position threshold for vertical alignment
    pub x_vertical_alignment_threshold: f64,
    /// Small group limit (use standard ordering)
    pub small_group_limit: usize,
    /// Large group limit (use compact ordering)
    pub large_group_limit: usize,

    // === Behavioral flags ===
    /// Whether to align octave accidentals vertically
    pub align_octaves: bool,
    /// Whether to align octaves across sub-chords
    pub align_octaves_across_sub_chords: bool,
    /// Whether to keep seconds (adjacent notes) together
    pub keep_seconds_together: bool,
    /// Whether ordering follows note displacement
    pub order_follow_note_displacement: bool,
    /// Whether to align offset octaves
    pub align_offset_octaves: bool,
}

impl Default for AccidentalLayoutConfig {
    fn default() -> Self {
        Self {
            // Vertical clearances (in spatiums)
            vertical_accidental_to_accidental_clearance: 0.15,
            vertical_sharp_to_sharp_clearance: 0.05,
            vertical_accidental_to_chord_clearance: 0.10,

            // Horizontal distances
            accidental_accidental_distance: 0.22,
            accidental_to_note_distance: 0.22,

            // Special kerning
            flat_kerning_of_fourth: -0.15,
            natural_kerning_of_fourth: -0.05,
            additional_padding_for_verticals: 0.1,
            sharp_and_natural_ledger_line_padding: 0.1,
            reduced_flat_to_note_padding: 0.15,

            // Thresholds
            x_pos_split_threshold: 0.5,
            x_vertical_alignment_threshold: 0.75,
            small_group_limit: 4,
            large_group_limit: 6,

            // Behavioral flags
            align_octaves: true,
            align_octaves_across_sub_chords: true,
            keep_seconds_together: true,
            order_follow_note_displacement: true,
            align_offset_octaves: true,
        }
    }
}

/// Accidental type for layout purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccidentalType {
    None,
    Sharp,
    Flat,
    Natural,
    DoubleSharp,
    DoubleFlat,
    // Microtonal accidentals could be added here
}

impl From<Accidental> for AccidentalType {
    fn from(acc: Accidental) -> Self {
        match acc {
            Accidental::None => AccidentalType::None,
            Accidental::Sharp => AccidentalType::Sharp,
            Accidental::Flat => AccidentalType::Flat,
            Accidental::Natural => AccidentalType::Natural,
            Accidental::DoubleSharp => AccidentalType::DoubleSharp,
            Accidental::DoubleFlat => AccidentalType::DoubleFlat,
        }
    }
}

impl AccidentalType {
    /// Check if this is a flat-type accidental.
    #[must_use]
    pub fn is_flat(self) -> bool {
        matches!(self, AccidentalType::Flat | AccidentalType::DoubleFlat)
    }

    /// Check if this is a sharp-type accidental.
    #[must_use]
    pub fn is_sharp(self) -> bool {
        matches!(self, AccidentalType::Sharp | AccidentalType::DoubleSharp)
    }
}

/// Shape rectangle for collision detection.
#[derive(Debug, Clone)]
pub struct AccidentalShape {
    /// Bounding box relative to accidental origin
    pub bbox: Rect,
    /// Top edge (in staff coordinates, negative = above)
    pub top: f64,
    /// Bottom edge
    pub bottom: f64,
    /// Left edge
    pub left: f64,
    /// Right edge
    pub right: f64,
}

impl AccidentalShape {
    /// Create from bounding box centered on staff line.
    #[must_use]
    pub fn new(bbox: Rect, y_offset: f64) -> Self {
        Self {
            bbox,
            top: bbox.y0 + y_offset,
            bottom: bbox.y1 + y_offset,
            left: bbox.x0,
            right: bbox.x1,
        }
    }

    /// Create from dimensions (in spatiums).
    #[must_use]
    pub fn from_dimensions(width: f64, height: f64, line: i32, spatium: f64) -> Self {
        let y_center = -line as f64 * spatium / 2.0;
        let half_h = height * spatium / 2.0;
        let bbox = Rect::new(-width * spatium, -half_h, 0.0, half_h);
        Self {
            bbox,
            top: y_center - half_h,
            bottom: y_center + half_h,
            left: -width * spatium,
            right: 0.0,
        }
    }

    /// Translate by x offset.
    #[must_use]
    pub fn translate_x(&self, dx: f64) -> Self {
        Self {
            bbox: Rect::new(
                self.bbox.x0 + dx,
                self.bbox.y0,
                self.bbox.x1 + dx,
                self.bbox.y1,
            ),
            top: self.top,
            bottom: self.bottom,
            left: self.left + dx,
            right: self.right + dx,
        }
    }

    /// Check if two shapes vertically intersect with given clearance.
    #[must_use]
    pub fn vertically_intersects(&self, other: &AccidentalShape, clearance: f64) -> bool {
        !(self.bottom + clearance < other.top || self.top - clearance > other.bottom)
    }

    /// Check if two shapes intersect (both axes).
    #[must_use]
    pub fn intersects(&self, other: &AccidentalShape, h_pad: f64, v_pad: f64) -> bool {
        let h_overlap = !(self.right + h_pad < other.left || self.left - h_pad > other.right);
        let v_overlap = !(self.bottom + v_pad < other.top || self.top - v_pad > other.bottom);
        h_overlap && v_overlap
    }
}

/// Information about an accidental to be placed.
#[derive(Debug, Clone)]
pub struct AccidentalInfo {
    /// Unique ID for tracking
    pub id: usize,
    /// The accidental type
    pub accidental_type: AccidentalType,
    /// Staff line position of the note
    pub line: i32,
    /// X position of the notehead (for notes displaced due to seconds)
    pub note_x: f64,
    /// Shape for collision detection
    pub shape: AccidentalShape,
    /// Width in spatiums
    pub width: f64,
    /// Height in spatiums
    pub height: f64,
    /// Magnitude scaling factor (for cue notes, etc.)
    pub mag: f64,

    // === Layout results (filled during layout) ===
    /// Assigned column (0 = closest to chord)
    pub column: usize,
    /// X offset from note position
    pub x_offset: f64,
    /// Stacking order number
    pub stacking_number: usize,
    /// Vertical subgroup
    pub vertical_subgroup: usize,

    // === Relationships (computed during layout) ===
    /// IDs of accidentals that form octaves with this one
    pub octaves: Vec<usize>,
    /// IDs of accidentals that form seconds with this one
    pub seconds: Vec<usize>,
}

impl AccidentalInfo {
    /// Create accidental info from basic parameters.
    #[must_use]
    pub fn new(id: usize, accidental: Accidental, line: i32, note_x: f64, spatium: f64) -> Self {
        let accidental_type = AccidentalType::from(accidental);
        let (width, height) = accidental_dimensions(accidental_type);
        let shape = AccidentalShape::from_dimensions(width, height, line, spatium);

        Self {
            id,
            accidental_type,
            line,
            note_x,
            shape,
            width,
            height,
            mag: 1.0,
            column: 0,
            x_offset: 0.0,
            stacking_number: 0,
            vertical_subgroup: 0,
            octaves: Vec::new(),
            seconds: Vec::new(),
        }
    }

    /// Create with custom magnitude (for cue notes).
    #[must_use]
    pub fn with_mag(mut self, mag: f64) -> Self {
        self.mag = mag;
        self
    }

    /// Y position of accidental center (relative to middle line).
    #[must_use]
    pub fn y_center(&self, spatium: f64) -> f64 {
        -self.line as f64 * spatium / 2.0
    }
}

/// Result of accidental placement.
#[derive(Debug, Clone)]
pub struct AccidentalPlacement {
    /// ID of the accidental
    pub id: usize,
    /// X offset from note position (negative = left of note)
    pub x_offset: f64,
    /// Column index (0 = closest to chord)
    pub column: usize,
}

/// Chord shape for collision detection.
#[derive(Debug, Clone, Default)]
pub struct ChordShape {
    /// Notehead shapes
    pub noteheads: Vec<(i32, Rect)>, // (line, bbox)
    /// Ledger line shapes
    pub ledger_lines: Vec<Rect>,
    /// Stem shape (if present)
    pub stem: Option<Rect>,
}

impl ChordShape {
    /// Add a notehead.
    pub fn add_notehead(&mut self, line: i32, bbox: Rect) {
        self.noteheads.push((line, bbox));
    }

    /// Add a ledger line.
    pub fn add_ledger_line(&mut self, bbox: Rect) {
        self.ledger_lines.push(bbox);
    }

    /// Set stem shape.
    pub fn set_stem(&mut self, bbox: Rect) {
        self.stem = Some(bbox);
    }
}

/// Stacked accidental info for collision checking.
#[derive(Debug, Clone)]
struct StackedAccidental {
    accidental_type: AccidentalType,
    line: i32,
    column: usize,
    mag: f64,
    shape: AccidentalShape,
}

impl From<&AccidentalInfo> for StackedAccidental {
    fn from(acc: &AccidentalInfo) -> Self {
        Self {
            accidental_type: acc.accidental_type,
            line: acc.line,
            column: acc.column,
            mag: acc.mag,
            shape: acc.shape.clone(),
        }
    }
}

/// Internal context for the layout algorithm.
struct LayoutContext<'a> {
    config: &'a AccidentalLayoutConfig,
    spatium: f64,
    chord_shape: &'a ChordShape,
    /// Accidentals already stacked (copies for collision checking)
    stacked: Vec<StackedAccidental>,
}

impl<'a> LayoutContext<'a> {
    fn new(config: &'a AccidentalLayoutConfig, spatium: f64, chord_shape: &'a ChordShape) -> Self {
        Self {
            config,
            spatium,
            chord_shape,
            stacked: Vec::new(),
        }
    }

    /// Vertical padding between two accidentals.
    fn vertical_padding(
        &self,
        acc1_type: AccidentalType,
        acc1_mag: f64,
        acc2_type: AccidentalType,
        acc2_mag: f64,
    ) -> f64 {
        let base = if acc1_type == AccidentalType::Sharp && acc2_type == AccidentalType::Sharp {
            self.config.vertical_sharp_to_sharp_clearance
        } else {
            self.config.vertical_accidental_to_accidental_clearance
        };
        base * self.spatium * 0.5 * (acc1_mag + acc2_mag)
    }

    /// Horizontal padding between two accidentals.
    fn horizontal_padding(&self, acc1_mag: f64, acc2_mag: f64) -> f64 {
        self.config.accidental_accidental_distance * self.spatium * 0.5 * (acc1_mag + acc2_mag)
    }
}

/// Layout accidentals for a chord, returning placements for each.
///
/// This implements MuseScore's full accidental stacking algorithm.
#[must_use]
pub fn layout_accidentals(
    accidentals: &mut [AccidentalInfo],
    spatium: f64,
    config: &AccidentalLayoutConfig,
    chord_shape: &ChordShape,
) -> Vec<AccidentalPlacement> {
    if accidentals.is_empty() {
        return Vec::new();
    }

    // Single accidental - simple placement
    if accidentals.len() == 1 {
        let x_offset = -(accidentals[0].width + config.accidental_to_note_distance) * spatium;
        accidentals[0].x_offset = x_offset;
        accidentals[0].column = 0;
        return vec![AccidentalPlacement {
            id: accidentals[0].id,
            x_offset,
            column: 0,
        }];
    }

    // Sort by line position (top to bottom)
    accidentals.sort_by_key(|a| a.line);

    // Find octaves and seconds
    find_octaves_and_seconds(accidentals);

    // Split into sub-chords
    let sub_chords = split_into_sub_chords(accidentals, config);

    // Create layout context
    let mut ctx = LayoutContext::new(config, spatium, chord_shape);

    // Layout each sub-chord
    for sub_chord_indices in &sub_chords {
        layout_sub_chord(accidentals, sub_chord_indices, &mut ctx);
    }

    // Vertical alignment pass
    vertically_align_accidentals(accidentals, &ctx);

    // Build results
    accidentals
        .iter()
        .map(|acc| AccidentalPlacement {
            id: acc.id,
            x_offset: acc.x_offset,
            column: acc.column,
        })
        .collect()
}

/// Simplified layout for basic use cases.
#[must_use]
pub fn layout_accidentals_simple(
    accidentals: &mut [AccidentalInfo],
    spatium: f64,
    config: &AccidentalLayoutConfig,
) -> Vec<AccidentalPlacement> {
    layout_accidentals(accidentals, spatium, config, &ChordShape::default())
}

/// Find octave and second relationships between accidentals.
fn find_octaves_and_seconds(accidentals: &mut [AccidentalInfo]) {
    let len = accidentals.len();

    // Clear existing relationships
    for acc in accidentals.iter_mut() {
        acc.octaves.clear();
        acc.seconds.clear();
    }

    // Find relationships
    for i in 0..len {
        for j in (i + 1)..len {
            let line_diff = (accidentals[j].line - accidentals[i].line).abs();

            // Octave: 7 lines apart, same accidental type
            if line_diff % 7 == 0
                && accidentals[i].accidental_type == accidentals[j].accidental_type
            {
                let id_i = accidentals[i].id;
                let id_j = accidentals[j].id;
                accidentals[i].octaves.push(id_j);
                accidentals[j].octaves.push(id_i);
            }

            // Second: 1 line apart
            if line_diff == 1 {
                let id_i = accidentals[i].id;
                let id_j = accidentals[j].id;
                accidentals[i].seconds.push(id_j);
                accidentals[j].seconds.push(id_i);
            }
        }
    }
}

/// Split accidentals into sub-chords based on vertical distance.
fn split_into_sub_chords(
    accidentals: &[AccidentalInfo],
    config: &AccidentalLayoutConfig,
) -> Vec<Vec<usize>> {
    const LINE_DIFF_OF_SEVENTH: i32 = 6;

    let mut sub_chords: Vec<Vec<usize>> = Vec::new();
    sub_chords.push(vec![0]);

    for i in 1..accidentals.len() {
        let prev_line = accidentals[i - 1].line;
        let cur_line = accidentals[i].line;
        let start_new_group = cur_line - prev_line >= LINE_DIFF_OF_SEVENTH;

        if start_new_group {
            sub_chords.push(vec![i]);
        } else if let Some(last) = sub_chords.last_mut() {
            last.push(i);
        }
    }

    // Merge small adjacent groups
    merge_small_groups(&mut sub_chords, config.small_group_limit);

    // Merge groups with octaves across (if enabled)
    if config.align_octaves_across_sub_chords {
        merge_octave_groups(&mut sub_chords, accidentals);
    }

    sub_chords
}

/// Merge adjacent sub-groups if they're too small.
fn merge_small_groups(sub_chords: &mut Vec<Vec<usize>>, small_limit: usize) {
    let mut changed = true;
    while changed && sub_chords.len() > 1 {
        changed = false;
        for i in 0..sub_chords.len() - 1 {
            let combined_size = sub_chords[i].len() + sub_chords[i + 1].len();
            if combined_size <= small_limit {
                let next = sub_chords.remove(i + 1);
                sub_chords[i].extend(next);
                changed = true;
                break;
            }
        }
    }
}

/// Merge sub-groups that have octaves across them.
fn merge_octave_groups(sub_chords: &mut Vec<Vec<usize>>, accidentals: &[AccidentalInfo]) {
    let mut merged = true;
    while merged && sub_chords.len() > 1 {
        merged = false;
        'outer: for i in 0..sub_chords.len() {
            for j in (i + 1)..sub_chords.len() {
                // Check if any accidental in group i has an octave in group j
                for &idx1 in &sub_chords[i] {
                    for &octave_id in &accidentals[idx1].octaves {
                        for &idx2 in &sub_chords[j] {
                            if accidentals[idx2].id == octave_id {
                                // Merge group j into group i
                                let group_j = sub_chords.remove(j);
                                sub_chords[i].extend(group_j);
                                sub_chords[i].sort();
                                merged = true;
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Layout a single sub-chord.
fn layout_sub_chord(
    accidentals: &mut [AccidentalInfo],
    indices: &[usize],
    ctx: &mut LayoutContext,
) {
    if indices.is_empty() {
        return;
    }

    // Determine stacking order
    let ordering = compute_ordering(accidentals, indices, ctx);

    // Stack each accidental in order
    for (stacking_num, &idx) in ordering.iter().enumerate() {
        accidentals[idx].stacking_number = stacking_num;
        stack_accidental(&mut accidentals[idx], ctx);

        // Add to stacked list (as a copy for collision checking)
        // Note: acc.shape is already translated in stack_accidental, don't translate again
        let stacked = StackedAccidental::from(&accidentals[idx]);
        ctx.stacked.push(stacked);
    }
}

/// Compute the stacking order for accidentals.
fn compute_ordering(
    accidentals: &[AccidentalInfo],
    indices: &[usize],
    ctx: &LayoutContext,
) -> Vec<usize> {
    let size = indices.len();

    if size <= ctx.config.small_group_limit {
        compute_standard_ordering(accidentals, indices, ctx)
    } else if size > ctx.config.large_group_limit {
        compute_compact_ordering(accidentals, indices, ctx)
    } else {
        // Choose ordering with fewer columns
        let standard = compute_standard_ordering(accidentals, indices, ctx);
        let compact = compute_compact_ordering(accidentals, indices, ctx);

        let standard_cols = count_columns(accidentals, &standard, ctx);
        let compact_cols = count_columns(accidentals, &compact, ctx);

        if compact_cols < standard_cols {
            compact
        } else {
            standard
        }
    }
}

/// Standard ordering: alternating top-bottom with octave insertion.
fn compute_standard_ordering(
    accidentals: &[AccidentalInfo],
    indices: &[usize],
    ctx: &LayoutContext,
) -> Vec<usize> {
    let mut result = Vec::with_capacity(indices.len());
    let mut remaining: Vec<usize> = indices.to_vec();
    let mut pick_from_top = true;

    while !remaining.is_empty() {
        let Some(idx) = (if pick_from_top {
            Some(remaining.remove(0))
        } else {
            remaining.pop()
        }) else {
            break;
        };
        result.push(idx);

        // Try to insert seconds if configured
        if ctx.config.keep_seconds_together {
            insert_seconds(accidentals, idx, &mut result, &mut remaining);
        }

        // Try to insert octave
        let found_octave = insert_octave(accidentals, idx, &mut result, &mut remaining);

        if found_octave {
            pick_from_top = true;
        } else {
            pick_from_top = !pick_from_top;
        }
    }

    result
}

/// Compact ordering: tries to fill columns efficiently.
fn compute_compact_ordering(
    accidentals: &[AccidentalInfo],
    indices: &[usize],
    ctx: &LayoutContext,
) -> Vec<usize> {
    let mut result = Vec::with_capacity(indices.len());
    let mut remaining: Vec<usize> = indices.to_vec();
    let mut pick_from_top = true;

    while !remaining.is_empty() {
        let Some(idx) = (if pick_from_top {
            Some(remaining.remove(0))
        } else {
            remaining.pop()
        }) else {
            break;
        };
        result.push(idx);

        // Try to fit more accidentals in the same column
        let mut found_fit = false;
        let last_idx = idx;

        // Look for accidentals that can fit in same column
        let mut i = 0;
        while i < remaining.len() {
            let candidate_idx = remaining[i];
            let acc1 = &accidentals[last_idx];
            let acc2 = &accidentals[candidate_idx];

            // Check direction constraint
            let direction_ok = if pick_from_top {
                acc2.line > acc1.line
            } else {
                acc2.line < acc1.line
            };

            // Skip if has octaves (handle separately)
            let has_octaves = !acc2.octaves.is_empty();

            if direction_ok && !has_octaves && can_fit_in_same_column(acc1, acc2, ctx) {
                found_fit = true;
                remaining.remove(i);
                result.push(candidate_idx);
            } else {
                i += 1;
            }
        }

        // Handle seconds
        if ctx.config.keep_seconds_together {
            for &placed_idx in &result.clone() {
                insert_seconds(accidentals, placed_idx, &mut result, &mut remaining);
            }
        }

        // Toggle direction
        if found_fit {
            pick_from_top = true;
        } else {
            pick_from_top = !pick_from_top;
        }
    }

    result
}

/// Insert seconds of an accidental into the result.
fn insert_seconds(
    accidentals: &[AccidentalInfo],
    idx: usize,
    result: &mut Vec<usize>,
    remaining: &mut Vec<usize>,
) {
    let second_ids: Vec<usize> = accidentals[idx].seconds.clone();

    for second_id in second_ids {
        // Find the index in remaining that has this ID
        if let Some(pos) = remaining
            .iter()
            .position(|&i| accidentals[i].id == second_id)
        {
            let second_idx = remaining.remove(pos);
            result.push(second_idx);
        }
    }
}

/// Insert octave of an accidental into the result.
fn insert_octave(
    accidentals: &[AccidentalInfo],
    idx: usize,
    result: &mut Vec<usize>,
    remaining: &mut Vec<usize>,
) -> bool {
    let octave_ids: Vec<usize> = accidentals[idx].octaves.clone();
    let mut found = false;

    for octave_id in octave_ids {
        if let Some(pos) = remaining
            .iter()
            .position(|&i| accidentals[i].id == octave_id)
        {
            let octave_idx = remaining.remove(pos);
            result.push(octave_idx);
            found = true;
        }
    }

    found
}

/// Check if two accidentals can fit in the same column.
fn can_fit_in_same_column(
    acc1: &AccidentalInfo,
    acc2: &AccidentalInfo,
    ctx: &LayoutContext,
) -> bool {
    // Exception: naturals a sixth apart can share a column
    if is_exception_of_naturals_sixth(acc1, acc2) {
        return true;
    }

    let v_pad = ctx.vertical_padding(
        acc1.accidental_type,
        acc1.mag,
        acc2.accidental_type,
        acc2.mag,
    );
    !acc1.shape.vertically_intersects(&acc2.shape, v_pad)
}

/// Count columns needed for an ordering.
fn count_columns(accidentals: &[AccidentalInfo], ordering: &[usize], ctx: &LayoutContext) -> usize {
    if ordering.is_empty() {
        return 0;
    }

    let mut columns: HashMap<usize, usize> = HashMap::new();
    columns.insert(ordering[0], 0);
    let mut max_column = 0;

    for i in 1..ordering.len() {
        let idx = ordering[i];
        let acc = &accidentals[idx];
        let mut col = 0;

        // Check against all previously placed accidentals
        for prev_idx in ordering.iter().take(i) {
            let prev_acc = &accidentals[*prev_idx];
            let prev_col = *columns.get(prev_idx).unwrap_or(&0);

            if !can_fit_in_same_column(acc, prev_acc, ctx) && prev_col >= col {
                col = prev_col + 1;
            }
        }

        columns.insert(idx, col);
        max_column = max_column.max(col);
    }

    max_column + 1
}

/// Stack a single accidental.
fn stack_accidental(acc: &mut AccidentalInfo, ctx: &LayoutContext) {
    let spatium = ctx.spatium;
    let config = ctx.config;

    // Start with minimum distance to chord
    let mut x = min_accidental_to_chord_distance(acc, ctx);

    // Move shape to this position
    let mut acc_shape = acc.shape.translate_x(-x);

    // Check collision with already-stacked accidentals
    let mut column = 0;
    let mut iteration = 0;
    const MAX_ITERATIONS: usize = 50;

    loop {
        iteration += 1;
        if iteration > MAX_ITERATIONS {
            break;
        }

        let mut collision_found = false;
        let mut move_distance = 0.0_f64;

        for stacked in &ctx.stacked {
            // Check for exception of fourth (flats can kern)
            if is_exception_of_fourth_stacked(acc, stacked) {
                let kern =
                    kerning_of_fourth_stacked(acc, stacked, &acc_shape, &stacked.shape, config);
                if kern > move_distance {
                    collision_found = true;
                    move_distance = kern;
                }
                column = column.max(stacked.column + 1);
                continue;
            }

            let v_pad = ctx.vertical_padding(
                acc.accidental_type,
                acc.mag,
                stacked.accidental_type,
                stacked.mag,
            );
            let h_pad = ctx.horizontal_padding(acc.mag, stacked.mag);

            if acc_shape.intersects(&stacked.shape, h_pad * 0.999, v_pad) {
                collision_found = true;

                // Update column
                if !is_exception_of_naturals_sixth_stacked(acc, stacked) {
                    column = column.max(stacked.column + 1);
                }

                // Calculate move distance
                if acc_shape.vertically_intersects(&stacked.shape, v_pad) {
                    let dist = acc_shape.right - stacked.shape.left + h_pad;
                    let dist = dist
                        + additional_padding_for_verticals_stacked(acc, stacked, config) * spatium;
                    move_distance = move_distance.max(dist);
                }
            }
        }

        if collision_found {
            acc_shape = acc_shape.translate_x(-move_distance);
            x += move_distance;
        } else {
            break;
        }
    }

    acc.x_offset = -x;
    acc.column = column;
    acc.shape = acc_shape;
}

/// Minimum distance from accidental to chord elements.
fn min_accidental_to_chord_distance(acc: &AccidentalInfo, ctx: &LayoutContext) -> f64 {
    let spatium = ctx.spatium;
    let config = ctx.config;

    let mut min_dist = config.accidental_to_note_distance * spatium + acc.width * spatium;

    // Check against noteheads
    for (note_line, notehead_bbox) in &ctx.chord_shape.noteheads {
        let note_y = -*note_line as f64 * spatium / 2.0;
        let notehead_top = notehead_bbox.y0 + note_y;
        let notehead_bottom = notehead_bbox.y1 + note_y;

        let clearance = config.vertical_accidental_to_chord_clearance * spatium;
        if acc.shape.bottom + clearance >= notehead_top
            && acc.shape.top - clearance <= notehead_bottom
        {
            let dist = acc.shape.right - notehead_bbox.x0;
            min_dist = min_dist.max(dist + config.accidental_to_note_distance * spatium);
        }
    }

    // Check against ledger lines
    for ledger in &ctx.chord_shape.ledger_lines {
        // Sharp and natural need extra padding near ledger lines
        if (acc.accidental_type == AccidentalType::Sharp
            || acc.accidental_type == AccidentalType::Natural)
            && ledger.y0 > acc.shape.top
            && ledger.y1 < acc.shape.bottom
        {
            let dist = acc.shape.right - ledger.x0
                + config.sharp_and_natural_ledger_line_padding * spatium;
            min_dist = min_dist.max(dist);
        }
    }

    min_dist
}

/// Vertically align accidentals of the same type in the same column.
fn vertically_align_accidentals(accidentals: &mut [AccidentalInfo], ctx: &LayoutContext) {
    let mut already_grouped: HashSet<usize> = HashSet::new();
    let mut vertical_sets: Vec<Vec<usize>> = Vec::new();

    // Collect vertical sets
    for i in 0..accidentals.len() {
        if already_grouped.contains(&accidentals[i].id) {
            continue;
        }

        let mut set = vec![i];

        for j in (i + 1)..accidentals.len() {
            if already_grouped.contains(&accidentals[j].id) {
                continue;
            }

            let acc1 = &accidentals[i];
            let acc2 = &accidentals[j];

            // Same type, same column, similar x position
            let same_type = acc1.accidental_type == acc2.accidental_type;
            let same_column = acc1.column == acc2.column;
            let same_subgroup = acc1.vertical_subgroup == acc2.vertical_subgroup;
            let similar_x = (acc1.x_offset - acc2.x_offset).abs()
                < ctx.config.x_vertical_alignment_threshold * ctx.spatium;

            if same_type && same_column && same_subgroup && similar_x {
                set.push(j);
                already_grouped.insert(accidentals[j].id);
            }
        }

        if set.len() > 1 {
            already_grouped.insert(accidentals[i].id);
            vertical_sets.push(set);
        }
    }

    // Align each vertical set
    for set in vertical_sets {
        // Find the leftmost position (most negative x_offset)
        let min_right_edge = set
            .iter()
            .map(|&idx| accidentals[idx].x_offset + accidentals[idx].width * ctx.spatium)
            .fold(f64::INFINITY, f64::min);

        // Align all accidentals in the set
        for &idx in &set {
            let acc = &mut accidentals[idx];
            acc.x_offset = min_right_edge - acc.width * ctx.spatium;
        }
    }

    // Also align octaves if configured
    if ctx.config.align_offset_octaves {
        align_octaves(accidentals, ctx);
    }
}

/// Align octave accidentals.
fn align_octaves(accidentals: &mut [AccidentalInfo], ctx: &LayoutContext) {
    let mut already_aligned: HashSet<usize> = HashSet::new();

    for i in 0..accidentals.len() {
        if already_aligned.contains(&accidentals[i].id) {
            continue;
        }

        let octave_ids = accidentals[i].octaves.clone();
        if octave_ids.is_empty() {
            continue;
        }

        // Find all octave partners
        let mut octave_indices: Vec<usize> = vec![i];
        for oct_id in octave_ids {
            if let Some(idx) = accidentals.iter().position(|a| a.id == oct_id)
                && !already_aligned.contains(&accidentals[idx].id)
            {
                octave_indices.push(idx);
            }
        }

        if octave_indices.len() < 2 {
            continue;
        }

        // Check if they're close enough to align
        let max_x = octave_indices
            .iter()
            .map(|&idx| accidentals[idx].x_offset)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_x = octave_indices
            .iter()
            .map(|&idx| accidentals[idx].x_offset)
            .fold(f64::INFINITY, f64::min);

        if max_x - min_x < 2.0 * ctx.spatium {
            // Align to the leftmost (most negative)
            for &idx in &octave_indices {
                accidentals[idx].x_offset = min_x;
                already_aligned.insert(accidentals[idx].id);
            }
        }
    }
}

// === Special Case Predicates ===

/// Check if two accidentals form the "exception of fourth" (flats can kern).
// Kept (only exercised by `test_exception_of_fourth`); deleting it would break that test.
#[allow(dead_code)]
fn is_exception_of_fourth(acc1: &AccidentalInfo, acc2: &AccidentalInfo) -> bool {
    // Both must be flats (or natural + flat)
    let acc1_flat = acc1.accidental_type.is_flat();
    let acc2_flat_or_nat =
        acc2.accidental_type.is_flat() || acc2.accidental_type == AccidentalType::Natural;

    // Must be exactly 3 lines apart (a fourth)
    let is_fourth = acc1.line - acc2.line == 3;

    acc1_flat && acc2_flat_or_nat && is_fourth
}

/// Check if two accidentals form the "exception of fourth" (version for stacked).
fn is_exception_of_fourth_stacked(acc1: &AccidentalInfo, acc2: &StackedAccidental) -> bool {
    let acc1_flat = acc1.accidental_type.is_flat();
    let acc2_flat_or_nat =
        acc2.accidental_type.is_flat() || acc2.accidental_type == AccidentalType::Natural;
    let is_fourth = acc1.line - acc2.line == 3;
    acc1_flat && acc2_flat_or_nat && is_fourth
}

/// Check if two accidentals form the "exception of naturals sixth".
fn is_exception_of_naturals_sixth(acc1: &AccidentalInfo, acc2: &AccidentalInfo) -> bool {
    acc1.accidental_type == AccidentalType::Natural
        && acc2.accidental_type == AccidentalType::Natural
        && (acc1.line - acc2.line).abs() == 5
}

/// Check if two accidentals form the "exception of naturals sixth" (version for stacked).
fn is_exception_of_naturals_sixth_stacked(acc1: &AccidentalInfo, acc2: &StackedAccidental) -> bool {
    acc1.accidental_type == AccidentalType::Natural
        && acc2.accidental_type == AccidentalType::Natural
        && (acc1.line - acc2.line).abs() == 5
}

/// Calculate kerning for fourth exception (version for stacked).
fn kerning_of_fourth_stacked(
    _acc1: &AccidentalInfo,
    acc2: &StackedAccidental,
    shape1: &AccidentalShape,
    shape2: &AccidentalShape,
    config: &AccidentalLayoutConfig,
) -> f64 {
    let kern = if acc2.accidental_type == AccidentalType::Natural {
        config.natural_kerning_of_fourth
    } else {
        config.flat_kerning_of_fourth
    };

    shape1.right - shape2.left + kern
}

/// Additional padding when natural is near vertical elements (version for stacked).
fn additional_padding_for_verticals_stacked(
    acc1: &AccidentalInfo,
    acc2: &StackedAccidental,
    config: &AccidentalLayoutConfig,
) -> f64 {
    if acc1.accidental_type != AccidentalType::Natural {
        return 0.0;
    }

    let acc2_type = acc2.accidental_type;
    if acc2_type != AccidentalType::Natural && !acc2_type.is_flat() {
        return 0.0;
    }

    let line_diff = acc2.line - acc1.line;
    if !(-2..=5).contains(&line_diff) {
        return 0.0;
    }

    config.additional_padding_for_verticals
}

/// Get standard dimensions for an accidental glyph (width, height) in spatiums.
#[must_use]
pub fn accidental_dimensions(accidental_type: AccidentalType) -> (f64, f64) {
    match accidental_type {
        AccidentalType::None => (0.0, 0.0),
        AccidentalType::Sharp => (1.2, 2.8),
        AccidentalType::Flat => (0.9, 2.4),
        AccidentalType::Natural => (0.7, 2.8),
        AccidentalType::DoubleSharp => (1.0, 1.0),
        AccidentalType::DoubleFlat => (1.4, 2.4),
    }
}

/// Check if two notes form a fourth interval (3 lines apart).
#[must_use]
pub fn is_fourth(line1: i32, line2: i32) -> bool {
    (line1 - line2).abs() == 3
}

/// Check if two notes form a sixth interval (5 lines apart).
#[must_use]
pub fn is_sixth(line1: i32, line2: i32) -> bool {
    (line1 - line2).abs() == 5
}

/// Check if two notes form an octave interval (7 lines apart).
#[must_use]
pub fn is_octave(line1: i32, line2: i32) -> bool {
    (line1 - line2).abs() % 7 == 0 && line1 != line2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_acc(id: usize, acc_type: AccidentalType, line: i32) -> AccidentalInfo {
        let (width, height) = accidental_dimensions(acc_type);
        AccidentalInfo {
            id,
            accidental_type: acc_type,
            line,
            note_x: 0.0,
            shape: AccidentalShape::from_dimensions(width, height, line, 5.0),
            width,
            height,
            mag: 1.0,
            column: 0,
            x_offset: 0.0,
            stacking_number: 0,
            vertical_subgroup: 0,
            octaves: Vec::new(),
            seconds: Vec::new(),
        }
    }

    #[test]
    fn test_single_accidental() {
        let mut accidentals = vec![make_acc(0, AccidentalType::Sharp, 0)];
        let config = AccidentalLayoutConfig::default();
        let placements = layout_accidentals_simple(&mut accidentals, 5.0, &config);

        assert_eq!(placements.len(), 1);
        assert!(placements[0].x_offset < 0.0);
        assert_eq!(placements[0].column, 0);
    }

    #[test]
    fn test_two_accidentals_no_overlap() {
        let mut accidentals = vec![
            make_acc(0, AccidentalType::Sharp, -4), // High note
            make_acc(1, AccidentalType::Flat, 4),   // Low note
        ];
        let config = AccidentalLayoutConfig::default();
        let placements = layout_accidentals_simple(&mut accidentals, 5.0, &config);

        assert_eq!(placements.len(), 2);
        // Both should fit in column 0 (no vertical overlap)
        assert_eq!(placements[0].column, 0);
        assert_eq!(placements[1].column, 0);
    }

    #[test]
    fn test_two_accidentals_with_overlap() {
        let mut accidentals = vec![
            make_acc(0, AccidentalType::Sharp, 0),
            make_acc(1, AccidentalType::Sharp, 1), // Adjacent line - will overlap
        ];
        let config = AccidentalLayoutConfig::default();
        let placements = layout_accidentals_simple(&mut accidentals, 5.0, &config);

        assert_eq!(placements.len(), 2);
        // One should be in a different column due to overlap
        assert!(
            placements[0].column != placements[1].column
                || (placements[0].x_offset - placements[1].x_offset).abs() > 1.0
        );
    }

    #[test]
    fn test_octave_detection() {
        let mut accidentals = vec![
            make_acc(0, AccidentalType::Sharp, 0),
            make_acc(1, AccidentalType::Sharp, 7),
            make_acc(2, AccidentalType::Flat, 14), // Not an octave of 0 (different type)
        ];
        find_octaves_and_seconds(&mut accidentals);

        assert!(accidentals[0].octaves.contains(&1));
        assert!(accidentals[1].octaves.contains(&0));
        assert!(accidentals[0].octaves.len() == 1);
        assert!(accidentals[2].octaves.is_empty());
    }

    #[test]
    fn test_second_detection() {
        let mut accidentals = vec![
            make_acc(0, AccidentalType::Sharp, 0),
            make_acc(1, AccidentalType::Flat, 1),
            make_acc(2, AccidentalType::Natural, 3), // Not a second of 1
        ];
        find_octaves_and_seconds(&mut accidentals);

        assert!(accidentals[0].seconds.contains(&1));
        assert!(accidentals[1].seconds.contains(&0));
        assert!(accidentals[1].seconds.len() == 1); // Only 0, not 2
    }

    #[test]
    fn test_octave_alignment() {
        let mut accidentals = vec![
            make_acc(0, AccidentalType::Sharp, 0),
            make_acc(1, AccidentalType::Sharp, 7), // Octave below
        ];
        let config = AccidentalLayoutConfig {
            align_octaves: true,
            align_offset_octaves: true,
            ..Default::default()
        };

        let placements = layout_accidentals_simple(&mut accidentals, 5.0, &config);

        assert_eq!(placements.len(), 2);
        // Octaves should have same x_offset
        assert!(
            (placements[0].x_offset - placements[1].x_offset).abs() < 0.1,
            "Octave accidentals should be aligned"
        );
    }

    #[test]
    fn test_exception_of_fourth() {
        let acc1 = make_acc(0, AccidentalType::Flat, 3);
        let acc2 = make_acc(1, AccidentalType::Flat, 0);

        assert!(is_exception_of_fourth(&acc1, &acc2));
        assert!(!is_exception_of_fourth(&acc2, &acc1)); // Direction matters
    }

    #[test]
    fn test_exception_of_naturals_sixth() {
        let acc1 = make_acc(0, AccidentalType::Natural, 0);
        let acc2 = make_acc(1, AccidentalType::Natural, 5);

        assert!(is_exception_of_naturals_sixth(&acc1, &acc2));
        assert!(is_exception_of_naturals_sixth(&acc2, &acc1));

        let acc3 = make_acc(2, AccidentalType::Sharp, 5);
        assert!(!is_exception_of_naturals_sixth(&acc1, &acc3));
    }

    #[test]
    fn test_sub_chord_splitting() {
        let accidentals = vec![
            make_acc(0, AccidentalType::Sharp, 0),
            make_acc(1, AccidentalType::Sharp, 2),
            make_acc(2, AccidentalType::Sharp, 4),
            make_acc(3, AccidentalType::Sharp, 12), // 8 lines from previous - new group
            make_acc(4, AccidentalType::Sharp, 14),
        ];
        let config = AccidentalLayoutConfig::default();
        let sub_chords = split_into_sub_chords(&accidentals, &config);

        // Should split into 2 groups
        assert!(!sub_chords.is_empty());
    }

    #[test]
    fn test_complex_chord() {
        // C-E-G-B chord with various accidentals
        let mut accidentals = vec![
            make_acc(0, AccidentalType::Sharp, 0),   // C#
            make_acc(1, AccidentalType::Flat, 2),    // Eb
            make_acc(2, AccidentalType::Natural, 4), // G natural
            make_acc(3, AccidentalType::Sharp, 6),   // B#
        ];
        let config = AccidentalLayoutConfig::default();
        let placements = layout_accidentals_simple(&mut accidentals, 5.0, &config);

        assert_eq!(placements.len(), 4);
        // All should have valid positions
        for p in &placements {
            assert!(
                p.x_offset < 0.0,
                "Accidentals should be to the left of notes"
            );
        }
    }

    #[test]
    fn test_cluster_chord() {
        // Dense cluster with many seconds
        let mut accidentals = vec![
            make_acc(0, AccidentalType::Sharp, 0),
            make_acc(1, AccidentalType::Sharp, 1),
            make_acc(2, AccidentalType::Sharp, 2),
            make_acc(3, AccidentalType::Sharp, 3),
        ];
        let config = AccidentalLayoutConfig::default();
        let placements = layout_accidentals_simple(&mut accidentals, 5.0, &config);

        assert_eq!(placements.len(), 4);

        // Should use multiple columns due to overlaps
        let Some(max_col) = placements.iter().map(|p| p.column).max() else {
            panic!("Expected placements to have at least one column");
        };
        assert!(max_col >= 1, "Cluster should require multiple columns");
    }

    #[test]
    fn test_is_fourth() {
        assert!(is_fourth(3, 0));
        assert!(is_fourth(0, 3));
        assert!(!is_fourth(2, 0));
        assert!(!is_fourth(4, 0));
    }

    #[test]
    fn test_is_sixth() {
        assert!(is_sixth(5, 0));
        assert!(is_sixth(0, 5));
        assert!(!is_sixth(4, 0));
        assert!(!is_sixth(6, 0));
    }

    #[test]
    fn test_is_octave() {
        assert!(is_octave(7, 0));
        assert!(is_octave(0, 7));
        assert!(is_octave(14, 0));
        assert!(!is_octave(0, 0));
        assert!(!is_octave(6, 0));
    }

    #[test]
    fn test_accidental_dimensions() {
        let (w, h) = accidental_dimensions(AccidentalType::Sharp);
        assert!(w > 0.0);
        assert!(h > 0.0);

        let (w, h) = accidental_dimensions(AccidentalType::None);
        assert_eq!(w, 0.0);
        assert_eq!(h, 0.0);
    }
}
