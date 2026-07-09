//! Chart layout types and data structures.
//!
//! This module contains the core types used by the chart layout engine,
//! including result types, position data, melody processing, and push spillback.

use std::collections::HashMap;

use crate::engraver::layout::orchestrator::{PageLayout, PageMargins};
use crate::engraver::layout::tlayout::Accidental;
use crate::engraver::model::{DurationKind, NoteHead};
use crate::engraver::model::{Octave, Pitch, PitchClass};
use crate::engraver::notation::Duration;
use crate::engraver::scene::node::SceneNode;

// ============================================================================
// Layout Mode
// ============================================================================

/// Layout mode for chart rendering.
///
/// All dimensions are in PostScript points (72pt = 1 inch). Use
/// [`PaperSize`] presets via [`LayoutMode::paginated`] / [`LayoutMode::snippet_of`]
/// to avoid hard-coding sizes.
#[derive(Debug, Clone)]
pub enum LayoutMode {
    /// Paginated layout with discrete pages and page breaks.
    Paginated { page_width: f64, page_height: f64 },
    /// Continuous scroll with unbounded vertical layout.
    ContinuousScroll { width: f64 },
    /// Snippet mode: fits content height automatically.
    /// For titleless charts that should be compact without whitespace.
    Snippet { page_width: f64 },
}

impl LayoutMode {
    /// Create a paginated layout for a standard paper size.
    ///
    /// Dimensions are in points (72pt = 1 inch).
    #[must_use]
    pub fn paginated(paper: crate::engraver::model::PaperSize) -> Self {
        let (w, h) = paper.dimensions_pt();
        Self::Paginated {
            page_width: f64::from(w),
            page_height: f64::from(h),
        }
    }

    /// Paginated US Letter (8.5" × 11").
    #[must_use]
    pub fn paginated_letter() -> Self {
        Self::paginated(crate::engraver::model::PaperSize::Letter)
    }

    /// Paginated A4 (210mm × 297mm).
    #[must_use]
    pub fn paginated_a4() -> Self {
        Self::paginated(crate::engraver::model::PaperSize::A4)
    }

    /// Paginated US Legal (8.5" × 14").
    #[must_use]
    pub fn paginated_legal() -> Self {
        Self::paginated(crate::engraver::model::PaperSize::Legal)
    }

    /// Paginated Tabloid / Ledger (11" × 17").
    #[must_use]
    pub fn paginated_tabloid() -> Self {
        Self::paginated(crate::engraver::model::PaperSize::Tabloid)
    }

    /// Snippet mode at the width of a standard paper size.
    #[must_use]
    pub fn snippet_of(paper: crate::engraver::model::PaperSize) -> Self {
        let (w, _) = paper.dimensions_pt();
        Self::Snippet {
            page_width: f64::from(w),
        }
    }

    /// Snippet mode with an explicit width (points).
    /// Height is calculated to fit the content.
    #[must_use]
    pub fn snippet(width: f64) -> Self {
        Self::Snippet { page_width: width }
    }

    /// Snippet mode at US Letter width.
    #[must_use]
    pub fn snippet_letter() -> Self {
        Self::snippet_of(crate::engraver::model::PaperSize::Letter)
    }

    /// Minimum page dimension (points). Anything smaller than this is
    /// almost certainly a bug — a single staff line is ~5pt, a chord
    /// symbol ~14pt, so a page narrower than 36pt (half an inch) can't
    /// fit a single measure.
    pub const MIN_DIMENSION_PT: f64 = 36.0;

    /// Custom paginated layout with validation.
    ///
    /// Returns `None` if either dimension is non-finite, non-positive, or
    /// smaller than [`Self::MIN_DIMENSION_PT`] (½").
    #[must_use]
    pub fn paginated_custom(width_pt: f64, height_pt: f64) -> Option<Self> {
        if !Self::is_valid_dim(width_pt) || !Self::is_valid_dim(height_pt) {
            return None;
        }
        Some(Self::Paginated {
            page_width: width_pt,
            page_height: height_pt,
        })
    }

    /// Sanitize an arbitrary dimension to a layout-safe value.
    ///
    /// Replaces NaN/Inf/non-positive values with `fallback`, then clamps
    /// to at least [`Self::MIN_DIMENSION_PT`]. Use this at API boundaries
    /// where callers may pass viewport pixels that briefly hit 0 during
    /// resize.
    #[must_use]
    pub fn sanitize_dim(value: f64, fallback: f64) -> f64 {
        if Self::is_valid_dim(value) {
            value
        } else {
            fallback.max(Self::MIN_DIMENSION_PT)
        }
    }

    #[inline]
    #[must_use]
    fn is_valid_dim(d: f64) -> bool {
        d.is_finite() && d >= Self::MIN_DIMENSION_PT
    }

    /// Get the page width (points) for this layout mode.
    #[must_use]
    pub fn page_width(&self) -> f64 {
        match self {
            LayoutMode::Paginated { page_width, .. } => *page_width,
            LayoutMode::ContinuousScroll { width } => *width,
            LayoutMode::Snippet { page_width } => *page_width,
        }
    }

    /// Get the page height (points), if bounded.
    ///
    /// Returns `None` for [`LayoutMode::ContinuousScroll`] (unbounded) and
    /// [`LayoutMode::Snippet`] (auto-fits content).
    #[must_use]
    pub fn page_height(&self) -> Option<f64> {
        match self {
            LayoutMode::Paginated { page_height, .. } => Some(*page_height),
            LayoutMode::ContinuousScroll { .. } | LayoutMode::Snippet { .. } => None,
        }
    }
}

impl Default for LayoutMode {
    fn default() -> Self {
        Self::paginated_letter()
    }
}

// ============================================================================
// Beat Position
// ============================================================================

/// Beat-level position data for precise cursor/highlight positioning.
///
/// This stores the actual computed X position for each beat segment,
/// accounting for proportional spacing (32nd notes get more space than whole notes).
#[derive(Debug, Clone)]
pub struct BeatPosition {
    /// Page number (1-indexed).
    pub page: u32,
    /// System index on this page (0-indexed).
    pub system: usize,
    /// Global measure index (0-indexed across entire chart).
    pub measure: usize,
    /// Beat/segment index within measure (0-indexed).
    pub beat: usize,
    /// Tick position relative to measure start (480 ticks per quarter note).
    pub tick: i32,
    /// Duration in ticks.
    pub duration_ticks: i32,
    /// Absolute tick position from song start (for tempo-independent lookup).
    pub absolute_tick: i64,
    /// Absolute X position on page (including margins and measure offset).
    pub x: f64,
    /// Width of this beat segment.
    pub width: f64,
    /// Y position of the staff (top line).
    pub staff_y: f64,
    /// Staff height (for cursor rendering).
    pub staff_height: f64,
    /// Time in seconds from song start (pre-computed with layout tempo).
    pub time_start: f64,
    /// Time in seconds when this beat ends (pre-computed with layout tempo).
    pub time_end: f64,
    /// Glyph codepoint for the primary element at this beat (for highlight rendering).
    /// This is typically a notehead (slash, normal, etc.) or rest glyph.
    pub glyph_codepoint: Option<char>,
    /// Size of the glyph in spatiums (for scaling the outline).
    pub glyph_size: f64,
    /// Y position of the glyph center (for notehead/rest vertical positioning).
    pub glyph_y: f64,
    /// Whether this note has a stem.
    pub has_stem: bool,
    /// Stem direction: true = up, false = down. Only meaningful if has_stem is true.
    pub stem_up: bool,
    /// Number of flags (0 for quarter and longer, 1 for 8th, 2 for 16th, 3 for 32nd).
    pub flag_count: u8,
    /// Time signature for the measure containing this beat: (numerator, denominator).
    /// Used for computing musical position (M.B.TTT) from tick offsets.
    pub time_signature: (u8, u8),
}

impl BeatPosition {
    /// Check if a given time falls within this beat.
    #[must_use]
    pub fn contains_time(&self, time: f64) -> bool {
        time >= self.time_start && time < self.time_end
    }

    /// Check if a given absolute tick falls within this beat.
    #[must_use]
    pub fn contains_tick(&self, tick: i64) -> bool {
        tick >= self.absolute_tick && tick < self.absolute_tick + self.duration_ticks as i64
    }

    /// Get interpolated X position for a time within this beat.
    #[must_use]
    pub fn x_at_time(&self, time: f64) -> f64 {
        if self.time_end <= self.time_start {
            return self.x;
        }
        let progress =
            ((time - self.time_start) / (self.time_end - self.time_start)).clamp(0.0, 1.0);
        self.x + self.width * progress
    }

    /// Get interpolated X position for an absolute tick within this beat.
    /// This is tempo-independent and provides smooth cursor movement.
    #[must_use]
    pub fn x_at_tick(&self, tick: i64) -> f64 {
        if self.duration_ticks <= 0 {
            return self.x;
        }
        let progress =
            ((tick - self.absolute_tick) as f64 / self.duration_ticks as f64).clamp(0.0, 1.0);
        self.x + self.width * progress
    }
}

// ============================================================================
// Slash Glyph Helper
// ============================================================================

/// Get the appropriate slash notehead glyph codepoint for a given duration in ticks.
///
/// Uses the standard 480 ticks per quarter note resolution:
/// - 1920 ticks = whole note
/// - 960 ticks = half note
/// - 480 ticks = quarter note
/// - 240 ticks = eighth note
/// - etc.
#[must_use]
pub fn slash_glyph_for_ticks(ticks: i32) -> char {
    let kind = match ticks {
        t if t >= 1920 => DurationKind::Whole,
        t if t >= 960 => DurationKind::Half,
        _ => DurationKind::Quarter, // Quarter and shorter all use filled slash
    };
    NoteHead::slash_for_duration(kind).smufl_codepoint()
}

// ============================================================================
// Chart Layout Result
// ============================================================================

/// Result of chart layout calculation.
#[derive(Debug, Clone)]
pub struct ChartLayoutResult {
    /// Root scene node containing all rendered content.
    pub scene: SceneNode,
    /// Page layouts (for paginated mode).
    pub pages: Vec<PageLayout>,
    /// Total height of content (for continuous scroll).
    pub total_height: f64,
    /// Total width of content.
    pub total_width: f64,
    /// Beat-level positions for cursor/highlight rendering.
    /// Sorted by time_start for efficient binary search.
    pub beat_positions: Vec<BeatPosition>,
}

impl ChartLayoutResult {
    /// Tight bounding box (scene coordinates) of everything actually drawn —
    /// staff, chord symbols, section labels, melodies, annotations. Returns
    /// `None` for an empty scene.
    ///
    /// `total_width`/`total_height` describe the **page box**: content plus
    /// page margins, inter-system spacing, and below-staff reserve that an A4
    /// print wants but an inline snippet does not. For a one-system chart that
    /// padding can be ~5× the music's own height. Shrink-wrapping a snippet's
    /// SVG viewBox to these bounds (plus a little padding) drops the dead space
    /// without disturbing the layout the paginated/print paths rely on.
    #[must_use]
    pub fn content_bounds(&self) -> Option<kurbo::Rect> {
        use crate::engraver::scene::paint::{PaintCommand, TextAnchor};
        use kurbo::{Affine, Point, Rect};

        // `PaintCommand::bounding_box()` returns `None` for Text and Glyph
        // because an exact extent needs font metrics. For a crop box an
        // approximation is enough — and *necessary*: chord symbols, lyrics,
        // section labels, and melody noteheads are all text/glyphs that sit
        // OUTSIDE the staff/barline paths. Without them the crop wraps only the
        // staff and clips the chord numbers above it.
        //
        // Commands are in their node's LOCAL space; the serializer renders each
        // under `parent_transform * node.transform`, so chord symbols live at
        // (0,0) inside a translated group. We must accumulate the same affine
        // and map each local box into scene space — otherwise transformed
        // content lands at the origin and the bounds (and crop) are garbage.
        fn transform_rect(t: Affine, r: Rect) -> Rect {
            let corners = [
                t * Point::new(r.x0, r.y0),
                t * Point::new(r.x1, r.y0),
                t * Point::new(r.x0, r.y1),
                t * Point::new(r.x1, r.y1),
            ];
            let mut out = Rect::new(
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for p in corners {
                out.x0 = out.x0.min(p.x);
                out.y0 = out.y0.min(p.y);
                out.x1 = out.x1.max(p.x);
                out.y1 = out.y1.max(p.y);
            }
            out
        }

        fn text_box(text: &str, font_size: f64, pos: Point, anchor: TextAnchor) -> Rect {
            // Generous average advance / ascent / descent — over-estimating the
            // box only adds a hair of margin; under-estimating clips glyphs.
            let w = text.chars().count() as f64 * font_size * 0.62;
            let (x0, x1) = match anchor {
                TextAnchor::Start => (pos.x, pos.x + w),
                TextAnchor::Middle => (pos.x - w / 2.0, pos.x + w / 2.0),
                TextAnchor::End => (pos.x - w, pos.x),
            };
            Rect::new(x0, pos.y - font_size * 0.85, x1, pos.y + font_size * 0.25)
        }

        fn cmd_bounds(cmd: &PaintCommand) -> Option<Rect> {
            match cmd {
                PaintCommand::Text {
                    text,
                    font_size,
                    position,
                    anchor,
                    ..
                } => Some(text_box(text, *font_size, *position, *anchor)),
                PaintCommand::Glyph { position, size, .. } => {
                    // SMuFL: font-size = size * 4 (1 em = 4 staff spaces),
                    // drawn baseline-left.
                    let em = size * 4.0;
                    Some(Rect::new(
                        position.x,
                        position.y - em * 0.9,
                        position.x + em * 1.2,
                        position.y + em * 0.3,
                    ))
                }
                other => other.bounding_box(),
            }
        }

        fn walk(node: &SceneNode, parent: Affine, acc: &mut Option<Rect>) {
            if !node.visible {
                return;
            }
            let t = parent * node.transform;
            for cmd in &node.commands {
                if let Some(r) = cmd_bounds(cmd) {
                    let r = transform_rect(t, r);
                    *acc = Some(match *acc {
                        Some(a) => a.union(r),
                        None => r,
                    });
                }
            }
            for child in &node.children {
                walk(child, t, acc);
            }
        }
        let mut acc = None;
        walk(&self.scene, Affine::IDENTITY, &mut acc);
        acc
    }

    /// Get layout metrics for a specific page.
    ///
    /// Returns detailed spacing information for debugging and verification.
    pub fn page_metrics(&self, page_number: u32) -> Option<PageLayoutMetrics> {
        let page = self.pages.iter().find(|p| p.number == page_number)?;
        Some(PageLayoutMetrics::from_page(page))
    }

    /// Get layout metrics for all pages.
    pub fn all_page_metrics(&self) -> Vec<PageLayoutMetrics> {
        self.pages
            .iter()
            .map(PageLayoutMetrics::from_page)
            .collect()
    }

    /// Find the beat position at a given time.
    ///
    /// Uses binary search for efficient lookup when beat_positions is sorted by time_start.
    #[must_use]
    pub fn beat_at_time(&self, time: f64) -> Option<&BeatPosition> {
        // Binary search for the beat containing this time
        let idx = self.beat_positions.partition_point(|b| b.time_end <= time);
        self.beat_positions
            .get(idx)
            .filter(|b| b.contains_time(time))
    }

    /// Get the interpolated X position and page for a given time.
    ///
    /// Returns (page, x, y, height) for cursor rendering.
    #[must_use]
    pub fn cursor_position_at_time(&self, time: f64) -> Option<(u32, f64, f64, f64)> {
        let beat = self.beat_at_time(time)?;
        let x = beat.x_at_time(time);
        Some((beat.page, x, beat.staff_y, beat.staff_height))
    }

    /// Get all beat positions for a specific measure.
    #[must_use]
    pub fn beats_in_measure(&self, measure_index: usize) -> Vec<&BeatPosition> {
        self.beat_positions
            .iter()
            .filter(|b| b.measure == measure_index)
            .collect()
    }

    /// Find the beat position at a given absolute tick.
    ///
    /// Uses binary search for efficient lookup. This is tempo-independent.
    #[must_use]
    pub fn beat_at_tick(&self, tick: i64) -> Option<&BeatPosition> {
        // Binary search for the beat containing this tick
        let idx = self
            .beat_positions
            .partition_point(|b| b.absolute_tick + b.duration_ticks as i64 <= tick);
        self.beat_positions
            .get(idx)
            .filter(|b| b.contains_tick(tick))
    }

    /// Get the interpolated X position and page for a given absolute tick.
    ///
    /// Returns (page, x, y, height) for cursor rendering. This is tempo-independent.
    #[must_use]
    pub fn cursor_position_at_tick(&self, tick: i64) -> Option<(u32, f64, f64, f64)> {
        let beat = self.beat_at_tick(tick)?;
        let x = beat.x_at_tick(tick);
        Some((beat.page, x, beat.staff_y, beat.staff_height))
    }

    /// Get the total duration of the chart in ticks.
    #[must_use]
    pub fn total_ticks(&self) -> i64 {
        self.beat_positions
            .last()
            .map(|b| b.absolute_tick + b.duration_ticks as i64)
            .unwrap_or(0)
    }

    /// Get all beat positions on a specific page.
    #[must_use]
    pub fn beats_on_page(&self, page: u32) -> Vec<&BeatPosition> {
        self.beat_positions
            .iter()
            .filter(|b| b.page == page)
            .collect()
    }
}

// ============================================================================
// Page Layout Metrics
// ============================================================================

/// Layout metrics for a single page - used for debugging and verification.
///
/// Similar to LilyPond's debug mode, this provides detailed information about
/// system placement on a page.
#[derive(Debug, Clone)]
pub struct PageLayoutMetrics {
    /// Page number (1-indexed).
    pub page_number: u32,
    /// Page dimensions.
    pub page_width: f64,
    pub page_height: f64,
    /// Number of systems on this page.
    pub system_count: usize,
    /// Y positions of each system (from top of page).
    pub system_y_positions: Vec<f64>,
    /// Spacing between consecutive systems.
    pub inter_system_spacing: Vec<f64>,
    /// Distance from top margin to first system.
    pub top_margin_to_first_system: f64,
    /// Distance from last system bottom to page bottom.
    pub last_system_to_bottom: f64,
    /// Total content height (all systems + spacing).
    pub content_height: f64,
    /// Available content height (page height - top margin - bottom margin).
    pub available_height: f64,
    /// Page margins.
    pub margins: PageMargins,
}

impl PageLayoutMetrics {
    /// Create metrics from a PageLayout.
    pub fn from_page(page: &PageLayout) -> Self {
        let system_count = page.systems.len();
        let system_y_positions: Vec<f64> = page.systems.iter().map(|s| s.y).collect();

        // Calculate inter-system spacing
        let inter_system_spacing: Vec<f64> = if system_count > 1 {
            page.systems
                .windows(2)
                .map(|pair| {
                    let first_bottom = pair[0].y + pair[0].height;
                    let second_top = pair[1].y;
                    second_top - first_bottom
                })
                .collect()
        } else {
            Vec::new()
        };

        // Calculate distances
        let top_margin_to_first_system = if let Some(first) = page.systems.first() {
            first.y - page.margins.top
        } else {
            0.0
        };

        let last_system_to_bottom = if let Some(last) = page.systems.last() {
            let last_bottom = last.y + last.height;
            page.height - page.margins.bottom - last_bottom
        } else {
            page.height - page.margins.top - page.margins.bottom
        };

        let content_height =
            if let (Some(first), Some(last)) = (page.systems.first(), page.systems.last()) {
                (last.y + last.height) - first.y
            } else {
                0.0
            };

        let available_height = page.height - page.margins.top - page.margins.bottom;

        Self {
            page_number: page.number,
            page_width: page.width,
            page_height: page.height,
            system_count,
            system_y_positions,
            inter_system_spacing,
            top_margin_to_first_system,
            last_system_to_bottom,
            content_height,
            available_height,
            margins: page.margins,
        }
    }

    /// Print a debug summary of the page layout (similar to LilyPond debug mode).
    pub fn print_debug(&self) {
        println!("=== Page {} Layout Metrics ===", self.page_number);
        println!(
            "Page size: {:.1} × {:.1} points",
            self.page_width, self.page_height
        );
        println!(
            "Margins: top={:.1}, bottom={:.1}, left={:.1}, right={:.1}",
            self.margins.top, self.margins.bottom, self.margins.left, self.margins.right
        );
        println!("Available height: {:.1} points", self.available_height);
        println!("Systems: {}", self.system_count);
        println!();

        if !self.system_y_positions.is_empty() {
            println!("System positions (from page top):");
            for (i, y) in self.system_y_positions.iter().enumerate() {
                println!("  System {}: y={:.1}", i + 1, y);
            }
            println!();
        }

        if !self.inter_system_spacing.is_empty() {
            println!("Inter-system spacing:");
            for (i, spacing) in self.inter_system_spacing.iter().enumerate() {
                println!("  Between {} and {}: {:.1} points", i + 1, i + 2, spacing);
            }
            println!();
        }

        println!(
            "Top margin to first system: {:.1} points",
            self.top_margin_to_first_system
        );
        println!(
            "Last system to bottom: {:.1} points",
            self.last_system_to_bottom
        );
        println!("Total content height: {:.1} points", self.content_height);
        println!(
            "Remaining space: {:.1} points",
            self.available_height - self.content_height
        );
        println!();
    }

    /// Check if the page has reasonable spacing.
    ///
    /// Returns a list of warnings if spacing issues are detected.
    pub fn check_spacing(&self, min_system_spacing: f64, max_system_spacing: f64) -> Vec<String> {
        let mut warnings = Vec::new();

        // Check inter-system spacing
        for (i, &spacing) in self.inter_system_spacing.iter().enumerate() {
            if spacing < min_system_spacing {
                warnings.push(format!(
                    "Systems {} and {} are too close: {:.1} points (min: {:.1})",
                    i + 1,
                    i + 2,
                    spacing,
                    min_system_spacing
                ));
            }
            if spacing > max_system_spacing {
                warnings.push(format!(
                    "Systems {} and {} are too far apart: {:.1} points (max: {:.1})",
                    i + 1,
                    i + 2,
                    spacing,
                    max_system_spacing
                ));
            }
        }

        // Check bottom margin
        if self.last_system_to_bottom < 0.0 {
            warnings.push(format!(
                "Content extends past bottom margin by {:.1} points",
                -self.last_system_to_bottom
            ));
        }

        // Check if too much empty space at bottom
        let excessive_bottom = self.available_height * 0.3; // More than 30% empty
        if self.last_system_to_bottom > excessive_bottom {
            warnings.push(format!(
                "Excessive empty space at bottom: {:.1} points ({:.0}% of available height)",
                self.last_system_to_bottom,
                (self.last_system_to_bottom / self.available_height) * 100.0
            ));
        }

        warnings
    }
}

// ============================================================================
// Melody Spillover Types
// ============================================================================

/// A segment of a melody note that fits within a single measure.
///
/// When a melody spans multiple measures, notes are split at barlines.
/// Each segment represents the portion of a note (or complete note) that
/// fits within one measure.
#[derive(Debug, Clone)]
pub struct MelodyNoteSegment {
    /// The pitch (e.g., "C", "F#", "r" for rest)
    pub pitch: String,
    /// Duration of this segment in beats (quarter note = 1.0)
    pub beats: f64,
    /// True if this is dotted
    pub dotted: bool,
    /// True if this is a continuation from previous measure (needs incoming tie)
    pub tie_from_previous: bool,
    /// True if this continues into next measure (needs outgoing tie)
    pub tie_to_next: bool,
    /// True if this is a rest
    pub is_rest: bool,
    /// Resolved absolute octave (None for rests)
    pub octave: Option<u8>,
    /// Resolved accidental for rendering
    pub accidental: Accidental,
    /// Additional pitches stacked on this note's stem — comes from
    /// MelodyNote.extra_pitches (octave doublings, double-stops). Each
    /// entry is (pitch name, optional explicit octave) and is resolved
    /// to a staff line at render time using the active clef.
    pub extra_pitches: Vec<(String, Option<u8>)>,
    /// Relative octave modifiers for `extra_pitches`; parallel to
    /// `extra_pitches`, defaulting to none when omitted.
    pub extra_pitch_modifiers: Vec<crate::chart::melody::OctaveModifier>,
}

impl MelodyNoteSegment {
    /// Convert beats to a Duration enum value
    pub fn to_duration(&self) -> Duration {
        // Map beats to duration (approximating to nearest standard duration)
        // For dotted notes, divide by 1.5 to get the base duration
        let base_beats = if self.dotted {
            self.beats / 1.5
        } else {
            self.beats
        };

        let base_duration = match base_beats {
            b if b >= 3.5 => Duration::Whole,
            b if b >= 1.75 => Duration::Half,
            b if b >= 0.875 => Duration::Quarter,
            b if b >= 0.4375 => Duration::Eighth,
            b if b >= 0.21875 => Duration::Sixteenth,
            _ => Duration::ThirtySecond,
        };

        // Return dotted version if appropriate
        if self.dotted {
            match base_duration {
                Duration::Half => Duration::DottedHalf,
                Duration::Quarter => Duration::DottedQuarter,
                Duration::Eighth => Duration::DottedEighth,
                Duration::Sixteenth => Duration::DottedSixteenth,
                // No DottedWhole or DottedThirtySecond in engraver
                other => other,
            }
        } else {
            base_duration
        }
    }
}

/// Preprocessed melody data for a measure.
///
/// Contains all melody note segments that should appear in this measure,
/// including segments that spilled over from previous measures.
#[derive(Debug, Clone, Default)]
pub struct MeasureMelodyData {
    /// Note segments to render in this measure
    pub segments: Vec<MelodyNoteSegment>,
    /// Total beats consumed by melody segments
    pub total_beats: f64,
    /// Active clef for this measure — drives pitch→staff-line mapping.
    /// Defaults to Treble so legacy code paths that don't yet set it
    /// keep their previous behaviour.
    pub clef: crate::chart::ChartClef,
    /// Active key signature as fifths: positive sharps, negative flats.
    /// Used only to decide which accidentals must be visibly printed.
    pub key_signature: i8,
}

impl MeasureMelodyData {
    /// Check if this measure has any melody content
    pub fn has_content(&self) -> bool {
        !self.segments.is_empty()
    }
}

// ============================================================================
// Melody Pitch Helpers
// ============================================================================

/// Parse a melody pitch string into (PitchClass, alteration, Accidental).
///
/// Handles pitch names like "C", "F#", "Bb", "D##", "Ebb".
/// Returns None for rests ("r").
pub(super) fn parse_melody_pitch(pitch: &str) -> Option<(PitchClass, i8, Accidental)> {
    if pitch == "r" || pitch.is_empty() {
        return None;
    }

    let first = pitch.chars().next()?;
    let class = match first.to_ascii_uppercase() {
        'C' => PitchClass::C,
        'D' => PitchClass::D,
        'E' => PitchClass::E,
        'F' => PitchClass::F,
        'G' => PitchClass::G,
        'A' => PitchClass::A,
        'B' => PitchClass::B,
        _ => return None,
    };

    let suffix = &pitch[first.len_utf8()..];
    let (alteration, accidental) = match suffix {
        "##" | "x" => (2, Accidental::DoubleSharp),
        "#" => (1, Accidental::Sharp),
        "bb" => (-2, Accidental::DoubleFlat),
        "b" => (-1, Accidental::Flat),
        "n" => (0, Accidental::Natural),
        "" => (0, Accidental::None),
        _ => (0, Accidental::None),
    };

    Some((class, alteration, accidental))
}

fn key_signature_alteration(class: PitchClass, key_signature: i8) -> i8 {
    let letter = match class {
        PitchClass::C => 'C',
        PitchClass::D => 'D',
        PitchClass::E => 'E',
        PitchClass::F => 'F',
        PitchClass::G => 'G',
        PitchClass::A => 'A',
        PitchClass::B => 'B',
    };

    if key_signature > 0 {
        const SHARP_ORDER: [char; 7] = ['F', 'C', 'G', 'D', 'A', 'E', 'B'];
        if SHARP_ORDER[..key_signature.min(7) as usize].contains(&letter) {
            return 1;
        }
    } else if key_signature < 0 {
        const FLAT_ORDER: [char; 7] = ['B', 'E', 'A', 'D', 'G', 'C', 'F'];
        if FLAT_ORDER[..key_signature.unsigned_abs().min(7) as usize].contains(&letter) {
            return -1;
        }
    }

    0
}

fn accidental_from_alteration(alteration: i8) -> Accidental {
    match alteration {
        2 => Accidental::DoubleSharp,
        1 => Accidental::Sharp,
        -1 => Accidental::Flat,
        -2 => Accidental::DoubleFlat,
        _ => Accidental::Natural,
    }
}

fn accidental_for_key_signature(
    class: PitchClass,
    alteration: i8,
    explicit_accidental: Accidental,
    key_signature: i8,
) -> Accidental {
    let implied = key_signature_alteration(class, key_signature);

    if alteration == implied {
        return Accidental::None;
    }

    // An explicit natural should stay visible when it cancels a key-signature
    // accidental. For unmarked natural pitches that differ from the active key,
    // also print a natural so the rendered note matches the pitch spelling.
    if alteration == 0 || explicit_accidental == Accidental::Natural {
        Accidental::Natural
    } else {
        accidental_from_alteration(alteration)
    }
}

/// Resolve the octave for a melody note using relative-pitch logic.
///
/// Given the previous note's pitch class and octave, find the octave for the
/// new note that places it closest (within a 4th) to the previous note.
/// Then apply octave modifier (Up = +1, Down = -1).
pub(super) fn resolve_relative_octave(
    new_class: PitchClass,
    prev_class: PitchClass,
    prev_octave: u8,
    octave_modifier: crate::chart::melody::OctaveModifier,
) -> u8 {
    // Calculate the diatonic interval (in scale steps) from prev to new
    let prev_step = prev_class.staff_offset(); // 0-6
    let new_step = new_class.staff_offset(); // 0-6

    // Try same octave, one above, one below — pick closest
    let candidates = [prev_octave.wrapping_sub(1), prev_octave, prev_octave + 1];

    let prev_pos = prev_step + (prev_octave as i32) * 7;
    let mut best_octave = prev_octave;
    let mut best_dist = i32::MAX;

    for &oct in &candidates {
        if oct > 9 {
            continue;
        } // sanity
        let pos = new_step + (oct as i32) * 7;
        let dist = (pos - prev_pos).abs();
        if dist < best_dist {
            best_dist = dist;
            best_octave = oct;
        }
    }

    // Apply octave modifier
    match octave_modifier {
        crate::chart::melody::OctaveModifier::Up => best_octave.saturating_add(1),
        crate::chart::melody::OctaveModifier::Down => best_octave.saturating_sub(1),
        crate::chart::melody::OctaveModifier::UpBy(count) => best_octave.saturating_add(count),
        crate::chart::melody::OctaveModifier::DownBy(count) => best_octave.saturating_sub(count),
        crate::chart::melody::OctaveModifier::None => best_octave,
    }
}

/// Convert a melody pitch string + resolved octave to a staff line position
/// and accidental, accounting for the active clef.
///
/// Staff line 0 = middle line. Each clef shifts the middle-line pitch:
/// - Treble (G clef): B4 (staff_position 6) = line 0
/// - Bass (F clef): D3 (staff_position -6) = line 0
/// - Alto (C clef on line 3): C4 (staff_position 0) = line 0
/// - Tenor (C clef on line 4): A3 (staff_position -2) = line 0
///
/// Positive = up, negative = down. For backwards-compat callers that
/// don't yet know which clef they're rendering, use
/// [`melody_pitch_to_line_treble`].
pub fn melody_pitch_to_line_for_clef(
    pitch: &str,
    octave: u8,
    clef: crate::chart::ChartClef,
) -> (i32, Accidental) {
    melody_pitch_to_line_for_clef_and_key(pitch, octave, clef, 0)
}

pub fn melody_pitch_to_line_for_clef_and_key(
    pitch: &str,
    octave: u8,
    clef: crate::chart::ChartClef,
    key_signature: i8,
) -> (i32, Accidental) {
    let Some((class, alteration, accidental)) = parse_melody_pitch(pitch) else {
        return (0, Accidental::None);
    };
    let p = Pitch::with_alteration(class, Octave::new(octave as i8), alteration);
    let staff_pos = p.staff_position();
    let middle_line_pos = clef_middle_line_pos(clef);
    let line = staff_pos - middle_line_pos;
    let display_accidental =
        accidental_for_key_signature(class, alteration, accidental, key_signature);
    (line, display_accidental)
}

/// Treble-clef pitch-to-line (back-compat shim for callers that haven't
/// been threaded with clef yet).
pub fn melody_pitch_to_line(pitch: &str, octave: u8) -> (i32, Accidental) {
    melody_pitch_to_line_for_clef(pitch, octave, crate::chart::ChartClef::Treble)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaffPlacement {
    StaffLine(i32),
    StaffSpace(i32),
    LedgerLineAbove(i32),
    LedgerSpaceAbove(i32),
    LedgerLineBelow(i32),
    LedgerSpaceBelow(i32),
}

pub fn staff_placement_for_line(line: i32) -> StaffPlacement {
    match line {
        -4 | -2 | 0 | 2 | 4 => StaffPlacement::StaffLine((line + 4) / 2 + 1),
        -3 | -1 | 1 | 3 => StaffPlacement::StaffSpace((line + 3) / 2 + 1),
        l if l > 4 && l % 2 == 0 => StaffPlacement::LedgerLineAbove((l - 4) / 2),
        l if l > 4 => StaffPlacement::LedgerSpaceAbove((l - 5) / 2 + 1),
        l if l < -4 && l % 2 == 0 => StaffPlacement::LedgerLineBelow((-4 - l) / 2),
        l if l < -4 => StaffPlacement::LedgerSpaceBelow((-5 - l) / 2 + 1),
        _ => unreachable!(),
    }
}

pub fn melody_staff_placement_for_clef(
    pitch: &str,
    octave: u8,
    clef: crate::chart::ChartClef,
) -> (i32, StaffPlacement, Accidental) {
    let (line, accidental) = melody_pitch_to_line_for_clef(pitch, octave, clef);
    (line, staff_placement_for_line(line), accidental)
}

/// Staff `staff_position` (relative to middle C = 0) of the middle line
/// for each clef. Treble: B4 = 6. Bass: D3 = -6. Alto: C4 = 0.
/// Tenor: A3 = -2. Percussion has no pitched middle — fall back to
/// treble so derived layouts stay sensible.
fn clef_middle_line_pos(clef: crate::chart::ChartClef) -> i32 {
    match clef {
        crate::chart::ChartClef::Treble => 6,
        crate::chart::ChartClef::Bass => -6,
        crate::chart::ChartClef::Alto => 0,
        crate::chart::ChartClef::Tenor => -2,
        crate::chart::ChartClef::Percussion => 6,
    }
}

/// Compute extra vertical space needed above and below the staff
/// for melody notes that extend beyond the 5-line staff.
///
/// Returns `(extra_above, extra_below)` in points.
/// The staff spans lines -4 (bottom, E4) to +4 (top, F5).
/// Notes outside this range need ledger lines and extra vertical space.
pub fn melody_note_extent(
    melody_data: &MeasureMelodyData,
    spatium: f64,
    clef: crate::chart::ChartClef,
) -> (f64, f64) {
    let mut max_line: i32 = 0;
    let mut min_line: i32 = 0;

    for seg in &melody_data.segments {
        if seg.is_rest {
            continue;
        }
        if let Some(oct) = seg.octave {
            let (line, _) = melody_pitch_to_line_for_clef(&seg.pitch, oct, clef);
            max_line = max_line.max(line);
            min_line = min_line.min(line);
        }
    }

    // Staff top line = line +4, bottom line = line -4
    // Each line unit = half a spatium
    let half_sp = spatium / 2.0;

    // Extra space above: notes above line +4 (top staff line)
    // Add 0.5 spatium padding for notehead clearance
    let extra_above = if max_line > 5 {
        (max_line - 4) as f64 * half_sp + spatium * 0.5
    } else {
        0.0
    };

    // Extra space below: notes below line -4 (bottom staff line)
    // Add 0.5 spatium padding for notehead clearance
    let extra_below = if min_line < -5 {
        (-4 - min_line) as f64 * half_sp + spatium * 0.5
    } else {
        0.0
    };

    (extra_above, extra_below)
}

/// Expands melodies across measure boundaries.
///
/// This processes all melodies in a chart section and distributes them
/// across measures, splitting notes at barlines and tracking ties.
pub fn expand_melodies_across_measures(
    section_measures: &[crate::chart::types::Measure],
    beats_per_measure: f64,
    key_signature: i8,
) -> HashMap<usize, MeasureMelodyData> {
    let mut result: HashMap<usize, MeasureMelodyData> = HashMap::new();

    for (measure_idx, measure) in section_measures.iter().enumerate() {
        if measure.melodies.is_empty() {
            continue;
        }

        tracing::debug!(
            "[melody-spillover] Measure {} has {} melodies",
            measure_idx,
            measure.melodies.len()
        );

        // Process each melody attached to this measure
        for melody in &measure.melodies {
            tracing::debug!(
                "[melody-spillover]   Processing melody with {} notes, total_beats: {:.2}",
                melody.notes.len(),
                melody.notes.iter().map(|n| n.duration_beats()).sum::<f64>()
            );
            let mut current_measure = measure_idx;

            // Track running pitch state for relative octave resolution
            // Start at C4 (middle C) as the reference point
            let mut ref_class = PitchClass::C;
            let mut ref_octave: u8 = 4;

            for note in &melody.notes {
                let note_beats = note.duration_beats();
                let mut beats_to_place = note_beats;
                let mut is_first_segment = true;

                // Resolve pitch and octave for this note
                let (resolved_octave, resolved_accidental) = if note.is_rest() {
                    (None, Accidental::None)
                } else if let Some((class, _alteration, accidental)) =
                    parse_melody_pitch(&note.pitch)
                {
                    // Determine octave: explicit or relative
                    let octave = if let Some(explicit_oct) = note.octave {
                        explicit_oct
                    } else {
                        resolve_relative_octave(class, ref_class, ref_octave, note.octave_modifier)
                    };

                    // Update reference state for next note
                    ref_class = class;
                    ref_octave = octave;

                    (Some(octave), accidental)
                } else {
                    (None, Accidental::None)
                };

                while beats_to_place > 0.001 {
                    // Get or create melody data for current measure
                    let measure_data = result.entry(current_measure).or_default();
                    measure_data.key_signature = key_signature;

                    // Check remaining capacity in this measure
                    let capacity = beats_per_measure - measure_data.total_beats;

                    if capacity <= 0.001 {
                        // Measure is full, move to next
                        current_measure += 1;
                        continue;
                    }

                    // Determine how much of this note fits in current measure
                    let segment_beats = beats_to_place.min(capacity);
                    let is_last_segment = (beats_to_place - segment_beats) < 0.001;

                    // Create segment
                    let segment = MelodyNoteSegment {
                        pitch: note.pitch.clone(),
                        beats: segment_beats,
                        dotted: note.dotted && is_last_segment,
                        tie_from_previous: !is_first_segment,
                        tie_to_next: !is_last_segment && !note.is_rest(),
                        is_rest: note.is_rest(),
                        octave: resolved_octave,
                        accidental: resolved_accidental,
                        // Polyphony stack — only attach to the first segment
                        // when a note spills across barlines; otherwise the
                        // tied continuation segments would duplicate heads.
                        extra_pitches: if is_first_segment {
                            note.extra_pitches.clone()
                        } else {
                            Vec::new()
                        },
                        extra_pitch_modifiers: if is_first_segment {
                            note.extra_pitch_modifiers.clone()
                        } else {
                            Vec::new()
                        },
                    };

                    measure_data.segments.push(segment);
                    measure_data.total_beats += segment_beats;

                    beats_to_place -= segment_beats;
                    is_first_segment = false;

                    // If we consumed all capacity, move to next measure for remaining beats
                    if measure_data.total_beats >= beats_per_measure - 0.001 {
                        current_measure += 1;
                    }
                }
            }
        }
    }

    // Smart octave centering: shift all notes so the median sits near the
    // middle of the treble staff (B4, staff_position 6, line 0).
    // Only apply when no notes had explicit octave annotations.
    {
        // Collect all staff positions across all measures
        let mut positions: Vec<i32> = Vec::new();
        let mut has_explicit_octave = false;

        for data in result.values() {
            for seg in &data.segments {
                if seg.is_rest {
                    continue;
                }
                if let Some(oct) = seg.octave {
                    let (line, _) = melody_pitch_to_line(&seg.pitch, oct);
                    positions.push(line);
                }
            }
        }

        // Check if any of the original melody notes had explicit octaves
        for measure in section_measures {
            for melody in &measure.melodies {
                for note in &melody.notes {
                    if note.octave.is_some() {
                        has_explicit_octave = true;
                    }
                }
            }
        }

        if !has_explicit_octave && positions.len() >= 2 {
            positions.sort();
            let median = positions[positions.len() / 2];
            let min = *positions.first().unwrap();
            let max = *positions.last().unwrap();
            let range = max - min;

            // Range-aware centering: if the melody intentionally spans more than
            // ~2 octaves (14 staff positions), the user has chosen a wide range
            // on purpose — don't drag the whole line up or down. Likewise, if
            // the median is already within ~half an octave of the staff center,
            // skip the shift (avoids cosmetic moves that surprise users).
            const WIDE_RANGE_LINES: i32 = 14;
            const ALREADY_CENTERED_LINES: i32 = 4;

            if range > WIDE_RANGE_LINES {
                tracing::debug!(
                    "[melody-centering] Skipping shift: range {} > {} (deliberately wide melody)",
                    range,
                    WIDE_RANGE_LINES
                );
            } else if median.abs() <= ALREADY_CENTERED_LINES {
                tracing::debug!(
                    "[melody-centering] Skipping shift: median {} already near staff center",
                    median
                );
            } else {
                // Target: median near line 0 (B4, middle of treble staff).
                // Each octave = 7 staff positions.
                let octave_shift = ((0 - median) as f64 / 7.0).round() as i8;

                if octave_shift != 0 {
                    tracing::debug!(
                        "[melody-centering] Shifting all octaves by {} (median line was {}, range {})",
                        octave_shift,
                        median,
                        range
                    );
                    for data in result.values_mut() {
                        for seg in &mut data.segments {
                            if let Some(ref mut oct) = seg.octave {
                                *oct = (*oct as i8 + octave_shift).max(0) as u8;
                            }
                        }
                    }
                }
            }
        }
    }

    // Log summary of melody distribution
    for (measure_idx, data) in result.iter() {
        tracing::debug!(
            "[melody-spillover] Output measure {}: {} segments, {:.2} total beats",
            measure_idx,
            data.segments.len(),
            data.total_beats
        );
        for (i, seg) in data.segments.iter().enumerate() {
            tracing::debug!(
                "[melody-spillover]   Segment {}: pitch={}, beats={:.2}, tie_in={}, tie_out={}",
                i,
                seg.pitch,
                seg.beats,
                seg.tie_from_previous,
                seg.tie_to_next
            );
        }
    }

    result
}

// ============================================================================
// Push Spillback Types (Re-exported from chart::rhythm)
// ============================================================================
//
// The canonical spillback types and detection functions are now in
// crate::chart::rhythm. They are re-exported through this module's parent
// (engraver::layout::chart) for backward compatibility.
//
// See: crate::chart::rhythm::{Spillback, detect_push_spillbacks, detect_section_start_spillback}

#[cfg(test)]
mod layout_mode_tests {
    use super::{
        Accidental, LayoutMode, StaffPlacement, melody_pitch_to_line_for_clef,
        melody_pitch_to_line_for_clef_and_key, melody_staff_placement_for_clef,
    };
    use crate::chart::ChartClef;
    use crate::engraver::model::PaperSize;

    #[test]
    fn paper_presets_match_paper_size_dimensions() {
        let letter = LayoutMode::paginated_letter();
        let (w, h) = PaperSize::Letter.dimensions_pt();
        assert_eq!(letter.page_width(), f64::from(w));
        assert_eq!(letter.page_height(), Some(f64::from(h)));
    }

    #[test]
    fn paginated_custom_rejects_garbage() {
        assert!(LayoutMode::paginated_custom(f64::NAN, 800.0).is_none());
        assert!(LayoutMode::paginated_custom(800.0, f64::INFINITY).is_none());
        assert!(LayoutMode::paginated_custom(-1.0, 800.0).is_none());
        assert!(LayoutMode::paginated_custom(0.0, 800.0).is_none());
        // Below MIN_DIMENSION_PT
        assert!(LayoutMode::paginated_custom(20.0, 800.0).is_none());
        // Valid
        assert!(LayoutMode::paginated_custom(400.0, 600.0).is_some());
    }

    #[test]
    fn sanitize_dim_falls_back_on_garbage() {
        let fallback = 612.0;
        assert_eq!(LayoutMode::sanitize_dim(f64::NAN, fallback), fallback);
        assert_eq!(LayoutMode::sanitize_dim(0.0, fallback), fallback);
        assert_eq!(LayoutMode::sanitize_dim(-50.0, fallback), fallback);
        assert_eq!(LayoutMode::sanitize_dim(500.0, fallback), 500.0);
    }

    #[test]
    fn page_height_is_none_for_unbounded_modes() {
        assert!(LayoutMode::snippet(400.0).page_height().is_none());
        assert!(
            LayoutMode::ContinuousScroll { width: 400.0 }
                .page_height()
                .is_none()
        );
        assert!(LayoutMode::paginated_letter().page_height().is_some());
    }

    #[test]
    fn bass_clef_note_staff_positions_match_lotf_measure_6() {
        let (f_line, f_place, _) = melody_staff_placement_for_clef("F#", 2, ChartClef::Bass);
        assert_eq!(f_line, -5);
        assert_eq!(f_place, StaffPlacement::LedgerSpaceBelow(1));

        let (c_line, c_place, _) = melody_staff_placement_for_clef("C#", 4, ChartClef::Bass);
        assert_eq!(c_line, 6);
        assert_eq!(c_place, StaffPlacement::LedgerLineAbove(1));
    }

    #[test]
    fn middle_c_ledger_lines_are_clef_dependent() {
        let (treble_line, treble_place, _) =
            melody_staff_placement_for_clef("C", 4, ChartClef::Treble);
        assert_eq!(treble_line, -6);
        assert_eq!(treble_place, StaffPlacement::LedgerLineBelow(1));

        let (bass_line, bass_place, _) = melody_staff_placement_for_clef("C", 4, ChartClef::Bass);
        assert_eq!(bass_line, 6);
        assert_eq!(bass_place, StaffPlacement::LedgerLineAbove(1));
    }

    #[test]
    fn bass_middle_line_is_d3() {
        let (line, _) = melody_pitch_to_line_for_clef("D", 3, ChartClef::Bass);
        assert_eq!(line, 0);
    }

    #[test]
    fn key_signature_suppresses_implied_melody_accidentals() {
        // E major has F#, C#, G#, D#. The C# pitch still maps to the same
        // ledger-line staff position, but it should not print a redundant #.
        let (line, accidental) = melody_pitch_to_line_for_clef_and_key("C#", 4, ChartClef::Bass, 4);
        assert_eq!(line, 6);
        assert_eq!(accidental, Accidental::None);

        let (_, accidental) = melody_pitch_to_line_for_clef_and_key("C", 4, ChartClef::Bass, 4);
        assert_eq!(accidental, Accidental::Natural);
    }

    #[test]
    fn key_signature_suppresses_implied_flat_melody_accidentals() {
        // Bb major has Bb and Eb.
        let (_, accidental) = melody_pitch_to_line_for_clef_and_key("Bb", 3, ChartClef::Bass, -2);
        assert_eq!(accidental, Accidental::None);

        let (_, accidental) = melody_pitch_to_line_for_clef_and_key("B", 3, ChartClef::Bass, -2);
        assert_eq!(accidental, Accidental::Natural);
    }
}
