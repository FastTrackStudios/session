//! Measurement pass for multi-pass chart layout.
//!
//! This module implements Pass 1 of the Measure → Layout → Paint pipeline.
//! It pre-computes exact sizes of all elements before layout, replacing
//! the previous estimation-based approach.
//!
//! # Why Measurement Caching?
//!
//! The old approach estimated chord widths multiple times:
//! 1. `estimate_measure_content_weight()` - predict relative weight
//! 2. `compute_minimum_measure_width()` - predict collision space
//! 3. `compute_chord_min_widths()` - predict segment minimums
//! 4. Render-time measurement in `layout_harmony()`
//!
//! This caused issues:
//! - Same chord measured 3-4x with different code paths
//! - Estimates didn't match actual rendered widths
//! - Post-hoc collision fixes broke barline positions
//!
//! The new approach measures everything once, caches it, and uses
//! real measurements throughout layout and rendering.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use kurbo::Rect;

use crate::chart::types::Measure;
use crate::engraver::layout::shape::Shape;
use crate::engraver::layout::text_metrics::TextFontMetrics;
use crate::engraver::layout::tlayout::HarmonyStyle;

use super::rhythm_builder::{self, RhythmBuildConfig, RhythmSource};

/// Cache key for harmony layout data.
///
/// This key uniquely identifies a chord symbol with its style parameters,
/// enabling accurate cache lookups even when styles change.
///
/// # Style Hashing
///
/// The style_hash captures font size, superscript scale, and other layout-affecting
/// parameters. This ensures cache invalidation when style changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HarmonyKey {
    /// The chord symbol string (e.g., "Cmaj7", "F#m7b5")
    pub symbol: String,
    /// Quantized font size (font_size * 10, preserving 0.1pt precision)
    pub font_size_q10: i32,
    /// Style hash combining superscript_scale, bass_scale, and notation type
    pub style_hash: u64,
}

impl HarmonyKey {
    /// Create a new harmony key from symbol and style.
    #[must_use]
    pub fn new(symbol: &str, style: &HarmonyStyle) -> Self {
        Self {
            symbol: symbol.to_string(),
            font_size_q10: quantize_font_size(style.root_size),
            style_hash: compute_style_hash(style),
        }
    }
}

/// Quantize font size for use as a hash key.
/// Multiplies by 10 to preserve 0.1pt precision.
#[inline]
fn quantize_font_size(font_size: f64) -> i32 {
    (font_size * 10.0).round() as i32
}

/// Compute a hash of style parameters that affect layout.
fn compute_style_hash(style: &HarmonyStyle) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Quantize scale factors to avoid floating-point precision issues
    let superscript_q = (style.superscript_scale * 1000.0).round() as i32;
    let bass_q = (style.bass_scale * 1000.0).round() as i32;
    let offset_q = (style.superscript_offset * 1000.0).round() as i32;
    superscript_q.hash(&mut hasher);
    bass_q.hash(&mut hasher);
    offset_q.hash(&mut hasher);
    std::mem::discriminant(&style.notation).hash(&mut hasher);
    std::mem::discriminant(&style.symbol_set).hash(&mut hasher);
    hasher.finish()
}

/// Cached layout data for a chord symbol.
///
/// This stores the full layout result from `layout_harmony()`, enabling
/// the rendering pass to reuse exact measurements without re-computing.
#[derive(Debug, Clone)]
pub struct CachedHarmonyLayout {
    /// Bounding box of the chord symbol (in local coordinates, origin at baseline-left)
    pub bounds: Rect,
    /// Total width of the chord symbol
    pub width: f64,
    /// Height (from baseline to top of superscripts)
    pub height: f64,
    /// Baseline Y position (relative to local origin)
    pub baseline: f64,
    /// Collision shape for accurate collision detection
    pub shape: Shape,
}

impl CachedHarmonyLayout {
    /// Create a new cached layout from layout data.
    #[must_use]
    pub fn new(bounds: Rect, width: f64, height: f64, baseline: f64) -> Self {
        // Create a simple rectangular shape from bounds for collision detection
        Self {
            bounds,
            width,
            height,
            baseline,
            shape: Shape::from_rect(bounds),
        }
    }

    /// Create a cached layout with a custom collision shape.
    #[must_use]
    pub fn with_shape(bounds: Rect, width: f64, height: f64, baseline: f64, shape: Shape) -> Self {
        Self {
            bounds,
            width,
            height,
            baseline,
            shape,
        }
    }
}

/// Cache for measured element sizes.
///
/// This cache is session-scoped: created fresh for each `layout_chart()` call,
/// used throughout the pass, then dropped. No staleness issues.
///
/// # Cache Keys
///
/// - Chord widths: `(symbol, font_size_quantized)` → width in points
/// - Harmony layouts: `HarmonyKey` → full `CachedHarmonyLayout` with bounds and shape
/// - Font size is quantized to 0.1pt precision (multiply by 10, cast to i32)
///
/// # Usage Pattern
///
/// 1. **Measure pass**: Call `measure_harmony()` to populate the cache
/// 2. **Layout pass**: Call `get_harmony_layout()` to retrieve cached data
/// 3. **Render pass**: Use cached bounds for collision-free positioning
#[derive(Debug, Default)]
pub struct MeasurementCache {
    /// Chord symbol widths: (symbol, quantized_font_size) → width in points
    /// This is the legacy cache, kept for backward compatibility
    chord_widths: HashMap<(String, i32), f64>,

    /// Full harmony layout data: HarmonyKey → CachedHarmonyLayout
    /// This is the new, richer cache that stores bounds and shapes
    harmony_layouts: HashMap<HarmonyKey, CachedHarmonyLayout>,
}

impl MeasurementCache {
    /// Create a new empty measurement cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Measure a chord symbol's width, returning cached value if available.
    ///
    /// # Arguments
    /// * `symbol` - The chord symbol string (e.g., "Cmaj7", "F#m7b5")
    /// * `font_size` - Font size in points
    /// * `metrics` - Text font metrics for measurement
    ///
    /// # Returns
    /// Width in points
    pub fn measure_chord_width(
        &mut self,
        symbol: &str,
        font_size: f64,
        metrics: &TextFontMetrics,
    ) -> f64 {
        let quantized_size = quantize_font_size(font_size);
        let key = (symbol.to_string(), quantized_size);

        *self
            .chord_widths
            .entry(key)
            .or_insert_with(|| metrics.horizontal_advance(symbol, font_size))
    }

    /// Get a cached chord width without measuring.
    /// Returns None if not cached.
    #[must_use]
    pub fn get_chord_width(&self, symbol: &str, font_size: f64) -> Option<f64> {
        let quantized_size = quantize_font_size(font_size);
        let key = (symbol.to_string(), quantized_size);
        self.chord_widths.get(&key).copied()
    }

    /// Store a harmony layout in the cache.
    ///
    /// # Arguments
    /// * `key` - The harmony key identifying this chord symbol + style
    /// * `layout` - The cached layout data to store
    pub fn store_harmony_layout(&mut self, key: HarmonyKey, layout: CachedHarmonyLayout) {
        self.harmony_layouts.insert(key, layout);
    }

    /// Get a cached harmony layout.
    ///
    /// # Arguments
    /// * `key` - The harmony key to look up
    ///
    /// # Returns
    /// The cached layout if found, None otherwise
    #[must_use]
    pub fn get_harmony_layout(&self, key: &HarmonyKey) -> Option<&CachedHarmonyLayout> {
        self.harmony_layouts.get(key)
    }

    /// Check if a harmony layout is cached.
    #[must_use]
    pub fn has_harmony_layout(&self, key: &HarmonyKey) -> bool {
        self.harmony_layouts.contains_key(key)
    }

    /// Measure and cache a harmony layout, returning cached value if available.
    ///
    /// This is the primary entry point for measuring chord symbols during the
    /// measure pass. It returns a reference to the cached layout.
    ///
    /// # Arguments
    /// * `symbol` - The chord symbol string
    /// * `style` - The harmony style (provides font size and other parameters)
    /// * `measure_fn` - A closure that measures the chord and returns layout data
    ///
    /// # Returns
    /// Reference to the cached layout data
    pub fn measure_harmony<F>(
        &mut self,
        symbol: &str,
        style: &HarmonyStyle,
        measure_fn: F,
    ) -> &CachedHarmonyLayout
    where
        F: FnOnce() -> CachedHarmonyLayout,
    {
        let key = HarmonyKey::new(symbol, style);
        self.harmony_layouts.entry(key).or_insert_with(measure_fn)
    }

    /// Number of cached chord width entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chord_widths.len()
    }

    /// Number of cached harmony layout entries.
    #[must_use]
    pub fn harmony_layout_count(&self) -> usize {
        self.harmony_layouts.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chord_widths.is_empty() && self.harmony_layouts.is_empty()
    }

    /// Clear all cached data.
    ///
    /// Call this when the layout context changes (e.g., style update).
    pub fn clear(&mut self) {
        self.chord_widths.clear();
        self.harmony_layouts.clear();
    }
}

/// Per-chord layout data for collision detection.
///
/// This stores the layout information for a single chord symbol,
/// including its position, bounds, and collision shape.
#[derive(Debug, Clone)]
pub struct ChordLayoutData {
    /// Position of the chord (in local measure coordinates)
    pub position: kurbo::Point,
    /// Bounding box of the chord symbol
    pub bbox: Rect,
    /// Collision shape for accurate collision detection
    pub shape: Shape,
    /// Text width of the chord symbol
    pub text_width: f64,
    /// Segment index where this chord is placed
    pub segment_index: usize,
    /// Whether this chord is visible (not a placeholder)
    pub visible: bool,
    /// Index in the original chord array
    pub chord_index: usize,
}

/// Measurement data for a single measure.
///
/// Contains pre-computed chord width measurements. This is the output of Pass 1
/// and provides accurate sizing data for the layout pass.
///
/// Note: Rhythm-based measurements (segment count, triplet detection) are intentionally
/// NOT included here. The rhythm builder already handles those correctly during layout,
/// and duplicating that logic would be error-prone. The measure pass focuses exclusively
/// on chord symbol width caching.
#[derive(Debug, Clone)]
pub struct MeasureMeasurements {
    /// Actual widths of each visible chord symbol (in points).
    /// Indexed by chord position within the measure.
    pub chord_widths: Vec<f64>,

    /// Total minimum width needed for this measure (in points).
    /// Calculated from actual chord widths + minimum gaps.
    pub min_width: f64,

    /// Number of visible chords in this measure.
    pub visible_chord_count: usize,

    /// Per-chord layout data for collision detection.
    /// Indexed by chord position within the measure (only visible chords).
    pub chord_layouts: Vec<ChordLayoutData>,
}

impl Default for MeasureMeasurements {
    fn default() -> Self {
        Self {
            chord_widths: Vec::new(),
            min_width: 0.0,
            visible_chord_count: 0,
            chord_layouts: Vec::new(),
        }
    }
}

/// Measurements for an entire chart.
///
/// This is the result of Pass 1 (Measure pass), containing pre-computed
/// measurements for all measures across all sections.
#[derive(Debug, Default)]
pub struct ChartMeasurements {
    /// Measurements for each measure, in order.
    /// Index corresponds to global measure index across all sections.
    pub measures: Vec<MeasureMeasurements>,
}

impl ChartMeasurements {
    /// Create empty chart measurements.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add measurements for a measure.
    pub fn push(&mut self, measurements: MeasureMeasurements) {
        self.measures.push(measurements);
    }

    /// Get measurements for a specific measure index.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&MeasureMeasurements> {
        self.measures.get(index)
    }

    /// Total number of measures.
    #[must_use]
    pub fn len(&self) -> usize {
        self.measures.len()
    }

    /// Whether there are no measurements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.measures.is_empty()
    }
}

/// Check if a chord symbol is a placeholder (space/rest).
#[inline]
fn is_placeholder(symbol: &str) -> bool {
    symbol.is_empty() || symbol == "s" || symbol == "r"
}

/// Measure a single measure's content.
///
/// This replaces the estimation logic in `compute_minimum_measure_width()`,
/// `compute_chord_min_widths()`, and parts of `estimate_measure_content_weight()`.
///
/// # Arguments
/// * `measure` - The measure to measure
/// * `style` - Harmony style (provides font size)
/// * `cache` - Measurement cache to use/populate
///
/// # Returns
/// Measurement data for the measure
pub fn measure_measure(
    measure: &Measure,
    style: &HarmonyStyle,
    cache: &mut MeasurementCache,
) -> MeasureMeasurements {
    // Detect if this measure has triplet pushes - if so, we need stemmed notation
    // which creates 2 segments per triplet beat instead of 1
    let has_triplet_push = measure.chords.iter().any(|c| {
        c.push_pull.as_ref().is_some_and(|(is_push, amount)| {
            *is_push && amount.base == crate::chord::PushPullBase::Triplet && amount.level == 1
        })
    });

    // Use the extended version with detected use_stems (no spillbacks at this stage)
    // Default to not being at a section boundary - caller should use measure_measure_with_config
    // directly if they need boundary-aware behavior.
    measure_measure_with_config(measure, style, cache, has_triplet_push, None, false)
}

/// Measure a single measure's content with full configuration.
///
/// This version accepts additional parameters for accurate segment-count calculation
/// when the measure contains push/pull timing.
///
/// # Arguments
/// * `measure` - The measure to measure
/// * `style` - Harmony style (provides font size)
/// * `cache` - Measurement cache to use/populate
/// * `use_stems` - Whether stemmed notation is used (affects triplet segment count)
/// * `spillbacks` - Optional spillback chords from next measure
/// * `is_section_boundary` - Whether this is the first measure in a section (affects spillback handling)
///
/// # Returns
/// Measurement data for the measure
pub fn measure_measure_with_config(
    measure: &Measure,
    style: &HarmonyStyle,
    cache: &mut MeasurementCache,
    use_stems: bool,
    spillbacks: Option<&[super::PushSpillback]>,
    is_section_boundary: bool,
) -> MeasureMeasurements {
    let text_metrics = match style.text_font_metrics.as_ref() {
        Some(m) => m,
        None => {
            // No metrics available - return defaults
            return MeasureMeasurements::default();
        }
    };

    let font_size = style.root_size;
    // Minimum gap between chord symbols for min_width calculation.
    // Uses NON_KERNING semantics (MuseScore): adjacent chord symbols always
    // maintain full padding, never compress against each other.
    let min_gap = font_size * 0.35;

    // Collect visible chord widths and layout data
    let mut chord_widths = Vec::new();
    let mut chord_layouts = Vec::new();
    let mut visible_chord_count = 0;

    // Track if we've seen a real (non-placeholder) chord yet
    let mut seen_real_chord = false;

    for (chord_idx, chord) in measure.chords.iter().enumerate() {
        let is_visible = !is_placeholder(&chord.full_symbol);

        // Check if this is a pushed chord that will spill back to the previous measure.
        // Pushed chords that are the first real chord in a measure render as spillbacks
        // in the PREVIOUS measure. However, at section boundaries, they ALSO render in
        // the current measure (at segment 0) to avoid confusion, so we need to reserve width.
        let is_pushed = is_visible
            && !seen_real_chord
            && chord
                .push_pull
                .as_ref()
                .is_some_and(|(is_push, _)| *is_push);

        // Only skip if pushed AND not at a boundary (at boundaries, chord renders in both places)
        let should_skip_for_spillback = is_pushed && !is_section_boundary;

        if is_visible {
            seen_real_chord = true;
        }

        // Skip pushed spillback chords that don't render in this measure
        if should_skip_for_spillback {
            tracing::debug!(
                "[measure-pass] Skipping pushed spillback chord '{}' - renders in previous measure",
                chord.full_symbol
            );
            continue;
        }

        if is_visible {
            let base_width = cache.measure_chord_width(&chord.full_symbol, font_size, text_metrics);

            // Adjust width for rendered chord components that are wider than plain text:
            // - Accidentals (b, #) render as ♭, ♯ symbols which are wider
            // - Extensions (7, 9, 11, 13) and alterations (#5, b9) need space
            // - Quality markers (maj, dim, aug) have specific widths
            let symbol = &chord.full_symbol;
            let has_flat = symbol.contains('b') && symbol.len() > 1; // 'b' as flat, not 'B' root
            let has_sharp = symbol.contains('#');
            let has_extension = symbol.chars().any(|c| c.is_ascii_digit());

            // Add padding for each component that renders wider than text
            let accidental_padding = if has_flat || has_sharp {
                font_size * 0.3
            } else {
                0.0
            };
            let extension_padding = if has_extension { font_size * 0.2 } else { 0.0 };

            // Push/pull indicator (apostrophe marks) needs extra horizontal space
            let push_pull_padding = if chord.push_pull.as_ref().is_some_and(|(_, _)| true) {
                font_size * 0.3
            } else {
                0.0
            };

            let width = (base_width + accidental_padding + extension_padding + push_pull_padding)
                .max(font_size * 1.5);
            chord_widths.push(width);

            // Create local-coordinate bounding box (origin at baseline-left)
            let bbox = Rect::new(0.0, -font_size, width, font_size * 0.2);
            let shape = Shape::from_rect(bbox);

            chord_layouts.push(ChordLayoutData {
                position: kurbo::Point::ZERO, // Will be set during layout pass
                bbox,
                shape,
                text_width: width,
                segment_index: chord_idx, // Initial estimate, updated below
                visible: true,
                chord_index: chord_idx,
            });

            visible_chord_count += 1;
        }
    }

    // Run the rhythm builder to get the ACTUAL segment count.
    // This is critical because triplet beats create 2 segments instead of 1.
    let has_explicit = rhythm_builder::measure_has_explicit_chord_rhythm(measure);
    let source = if has_explicit {
        RhythmSource::ExplicitRhythm {
            elements: &measure.rhythm_elements,
            spillbacks,
        }
    } else {
        RhythmSource::SlashNotation {
            chords: &measure.chords,
            spillbacks,
        }
    };

    let config = RhythmBuildConfig {
        time_signature: (measure.time_signature.0, 4),
        use_stems,
        auto_rhythm_slashes: false,
        push_alters_rhythm: true, // Default to altering rhythm for triplet pushes
    };

    let rhythm_result = rhythm_builder::build_rhythm(source, &config);
    let num_segments = rhythm_result.len();
    let mut segment_mins_for_log = Vec::new();

    // Calculate minimum width from actual measurements using segment-based layout.
    // This accounts for the fact that chords are placed at specific segments,
    // so a wide chord might need more than its "fair share" of segment space.
    //
    // IMPORTANT: Minimum width is ONLY about preventing chord symbol collision.
    // Rhythm notation (rests, noteheads, triplet brackets) can compress to fit
    // whatever space is allocated. The spring distribution handles proportional
    // spacing - min_width is just the floor below which chords would overlap.
    let min_width = if num_segments == 0 {
        // No segments at all - shouldn't happen, but handle gracefully
        0.0
    } else if chord_layouts.is_empty() {
        // No visible chords - rhythm notation can compress to any width
        // Use a small baseline to avoid zero-width measures
        font_size * 0.5
    } else if chord_layouts.len() == 1 {
        // Single chord - just need space for that chord symbol, no collision possible
        // Chord can overhang into adjacent space, so minimal padding needed
        chord_layouts[0].text_width + font_size * 0.3
    } else {
        // Compute per-segment minimum widths.
        // For slash notation, chord index typically maps directly to beat, and each beat
        // may be 1 or 2 segments depending on triplets.
        let mut segment_mins = vec![0.0_f64; num_segments];

        // Build mapping from chord index to segment index
        // For triplet beats, chord at beat N maps to segment 0 of that beat's triplet group
        // This is a simplified mapping - actual positions depend on push/pull
        let chord_to_segment: Vec<usize> = if has_explicit {
            // For explicit rhythm, chord indices map directly to rhythm entry indices
            (0..measure.chords.len()).collect()
        } else {
            measure
                .chords
                .iter()
                .map(|chord| {
                    let beat = chord.position.beats() as f64
                        + chord.position.subdivisions() as f64 / 1000.0;
                    let beats_per_segment =
                        f64::from(measure.time_signature.0) / num_segments as f64;
                    if beats_per_segment > 0.0 {
                        (beat / beats_per_segment)
                            .round()
                            .clamp(0.0, (num_segments.saturating_sub(1)) as f64)
                            as usize
                    } else {
                        0
                    }
                })
                .collect()
        };

        // Update chord_layouts with correct segment indices
        for layout in &mut chord_layouts {
            if layout.chord_index < chord_to_segment.len() {
                layout.segment_index = chord_to_segment[layout.chord_index];
            }
        }

        // Build list of (segment_index, width) for visible chords
        let visible_chord_info: Vec<(usize, f64)> = chord_layouts
            .iter()
            .map(|c| (c.segment_index, c.text_width))
            .collect();

        // For each pair of adjacent visible chords, compute segment minimums.
        // With duration-proportional spacing, intermediate segments may have
        // very different widths. We distribute the required collision-prevention
        // space across the segments between two chords.
        for i in 0..visible_chord_info.len() - 1 {
            let (idx1, width1) = visible_chord_info[i];
            let (idx2, _) = visible_chord_info[i + 1];

            let segment_gap = idx2.saturating_sub(idx1);
            if segment_gap == 0 {
                continue; // Same segment, can't help
            }

            // The first chord can overhang left into the clef area, so we don't
            // need to reserve its full width. Allow 50% overhang for segment 0.
            let effective_width = if idx1 == 0 {
                width1 * 0.5 // First chord can overhang 50% left
            } else {
                width1
            };

            // Required space for this chord + gap before next chord.
            // NON_KERNING: chord symbols always maintain full padding.
            let required_space = effective_width + min_gap;

            // Distribute required_space across the segments between the two chords.
            // With proportional spacing, putting all space on the first segment
            // isn't sufficient — a short-duration first segment might get less
            // actual width than its min. Instead, apply the requirement to the
            // first segment (which positions the chord) and set a small baseline
            // for intermediate segments so they contribute minimum space.
            if idx1 < segment_mins.len() {
                segment_mins[idx1] = segment_mins[idx1].max(required_space);
            }

            // For intermediate segments (between two chords), ensure a small
            // baseline width so they're not zero-width in the min calculation.
            // This prevents short-duration segments from being so narrow that
            // their proportional contribution can't prevent chord collision.
            let intermediate_baseline = min_gap * 0.5;
            for seg_idx in (idx1 + 1)..idx2 {
                if seg_idx < segment_mins.len() && segment_mins[seg_idx] == 0.0 {
                    segment_mins[seg_idx] = segment_mins[seg_idx].max(intermediate_baseline);
                }
            }
        }

        // Handle the last visible chord: reserve space for its width + trailing
        if let Some(&(last_idx, last_width)) = visible_chord_info.last()
            && last_idx < segment_mins.len()
        {
            let trailing_for_last = last_width * 0.5; // Half-width trailing
            segment_mins[last_idx] = segment_mins[last_idx].max(trailing_for_last);
        }

        // Sum segment minimums to get total min_width.
        // ONLY segments with chord collisions contribute to min_width.
        // Segments without chords (rests, spaces) contribute NOTHING because
        // rhythm notation compresses to fit whatever space is allocated.
        //
        // Previous bug: baseline_segment_width was applied to ALL segments,
        // causing triplet-heavy measures (r8t >Cm_8t r8t ...) to get huge
        // min_widths even though most segments are just rests.

        // Add minimal trailing padding for visual breathing room
        let trailing_padding = font_size * 0.3;

        // Only sum segments that have actual chord collision requirements
        let segment_total: f64 = segment_mins.iter().filter(|&&m| m > 0.0).sum();
        segment_mins_for_log = segment_mins;

        segment_total + trailing_padding
    };

    if tracing::enabled!(
        target: "engraver_proto::engraver::layout::chart::spacing",
        tracing::Level::DEBUG
    ) {
        let chord_width_summary = chord_layouts
            .iter()
            .map(|layout| {
                format!(
                    "#{}@seg{}:{:.1}pt",
                    layout.chord_index, layout.segment_index, layout.text_width
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let symbol_summary = measure
            .chords
            .iter()
            .filter(|c| !is_placeholder(&c.full_symbol))
            .map(|c| c.full_symbol.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        tracing::debug!(
            target: "engraver_proto::engraver::layout::chart::spacing",
            source_measure = ?measure.source_measure_number,
            time_signature = ?measure.time_signature,
            use_stems,
            has_explicit_rhythm = has_explicit,
            is_section_boundary,
            segment_count = num_segments,
            visible_chord_count,
            symbols = %symbol_summary,
            chord_widths = %chord_width_summary,
            segment_mins = ?segment_mins_for_log,
            min_width,
            "[spacing-measure-pass] measured measure minimum"
        );
    }

    MeasureMeasurements {
        chord_widths,
        min_width,
        visible_chord_count,
        chord_layouts,
    }
}

/// Measure all content in a chart.
///
/// This is the main entry point for Pass 1 (Measure pass).
/// Pre-measures all elements in the chart and returns cached measurements.
///
/// # Arguments
/// * `sections` - Iterator over chart sections with measures
/// * `style` - Harmony style for chord symbols
/// * `cache` - Measurement cache to populate
///
/// # Returns
/// Complete chart measurements
pub fn measure_chart<I, M>(
    sections: I,
    style: &HarmonyStyle,
    cache: &mut MeasurementCache,
) -> ChartMeasurements
where
    I: Iterator<Item = M>,
    M: AsRef<[Measure]>,
{
    let mut measurements = ChartMeasurements::new();

    for section_measures in sections {
        for (measure_idx, measure) in section_measures.as_ref().iter().enumerate() {
            // First measure of a section is at a section boundary
            let is_section_boundary = measure_idx == 0;

            // Detect if this measure has triplet pushes
            let has_triplet_push = measure.chords.iter().any(|c| {
                c.push_pull.as_ref().is_some_and(|(is_push, amount)| {
                    *is_push
                        && amount.base == crate::chord::PushPullBase::Triplet
                        && amount.level == 1
                })
            });

            let measure_data = measure_measure_with_config(
                measure,
                style,
                cache,
                has_triplet_push,
                None,
                is_section_boundary,
            );
            measurements.push(measure_data);
        }
    }

    measurements
}

/// Calculate measure content weight from pre-computed measurements.
///
/// This provides a base weight based on chord complexity. The rhythm builder
/// may add additional weight during layout for triplets and complex rhythms.
///
/// # Arguments
/// * `measurements` - Pre-computed measurements for this measure
/// * `segment_count` - Number of rhythm segments (from rhythm builder)
/// * `triplet_count` - Number of triplet elements (from rhythm builder)
///
/// # Returns
/// Weight value for spring-based width distribution (typically 0.5-4.0)
#[must_use]
pub fn compute_measure_weight(
    _measurements: &MeasureMeasurements,
    segment_count: usize,
    triplet_count: usize,
) -> f64 {
    // Base weight from segment count (time signature).
    // 4/4 measures get weight 1.0 as baseline.
    let segment_weight = segment_count as f64 / 4.0;

    // Small triplet bonus - triplet brackets need a little extra breathing room,
    // but not much. The old value (0.15 per triplet) was too aggressive
    // and caused measures to overflow. This bonus (0.08 per triplet)
    // gives enough extra space for the bracket notation without
    // stealing too much from adjacent measures.
    const TRIPLET_BONUS: f64 = 0.08;
    let triplet_bonus = triplet_count as f64 * TRIPLET_BONUS;

    // Combine and clamp to reasonable range
    let weight = segment_weight + triplet_bonus;
    weight.clamp(0.5, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // Create a minimal test HarmonyStyle
    fn make_test_style() -> HarmonyStyle {
        let font_data = Arc::new(crate::engraver::fonts::EMPTY_FONT_DATA_FOR_TESTS.to_vec());
        HarmonyStyle::default().with_text_font_metrics(TextFontMetrics::new(font_data))
    }

    #[test]
    fn test_measurement_cache_basic() {
        let mut cache = MeasurementCache::new();
        let style = make_test_style();
        let metrics = style.text_font_metrics.as_ref().unwrap();

        // First call measures
        let width1 = cache.measure_chord_width("Cmaj7", 14.0, metrics);
        assert!(width1 > 0.0);

        // Second call returns cached value
        let width2 = cache.measure_chord_width("Cmaj7", 14.0, metrics);
        assert!((width1 - width2).abs() < 0.001);

        // Different font size = different cache entry
        let width3 = cache.measure_chord_width("Cmaj7", 12.0, metrics);
        assert!((width1 - width3).abs() > 0.1);

        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_is_placeholder() {
        assert!(is_placeholder(""));
        assert!(is_placeholder("s"));
        assert!(is_placeholder("r"));
        assert!(!is_placeholder("C"));
        assert!(!is_placeholder("Am7"));
    }

    #[test]
    fn test_compute_measure_weight() {
        // Simple measure with 4 segments (4/4 time)
        let measurements = MeasureMeasurements {
            chord_widths: vec![50.0],
            min_width: 0.0,
            visible_chord_count: 1,
            chord_layouts: Vec::new(),
        };
        let weight = compute_measure_weight(&measurements, 4, 0);
        assert!((weight - 1.0).abs() < 0.01); // 4/4 = 1.0 base weight

        // Measure with triplets gets a small bonus (0.08 per triplet)
        let measurements_triplet = MeasureMeasurements {
            chord_widths: vec![50.0, 50.0],
            min_width: 100.0,
            visible_chord_count: 2,
            chord_layouts: Vec::new(),
        };
        let weight_with_triplets = compute_measure_weight(&measurements_triplet, 4, 3);
        // 1.0 base + 3 * 0.08 = 1.24
        assert!((weight_with_triplets - 1.24).abs() < 0.01);
        assert!(weight_with_triplets > weight); // Slightly more than no triplets

        // 6/8 measure with 6 segments
        let weight_6_segments = compute_measure_weight(&measurements_triplet, 6, 0);
        assert!((weight_6_segments - 1.5).abs() < 0.01); // 6/4 = 1.5 weight
    }

    #[test]
    fn dense_chord_measure_sums_segment_minimums() {
        use crate::chord::{Chord, ChordQuality, ChordRhythm};
        use crate::primitives::RootNotation;
        use crate::time::{AbsolutePosition, MusicalDuration};

        fn chord(symbol: &str) -> crate::chart::types::ChordInstance {
            let root = RootNotation::from_string("C").expect("test root should parse");
            crate::chart::types::ChordInstance::new(
                root.clone(),
                symbol.to_string(),
                Chord::new(root, ChordQuality::Major),
                ChordRhythm::Default,
                symbol.to_string(),
                MusicalDuration::new(0, 1, 0),
                AbsolutePosition::at_beginning(),
            )
        }

        let style = make_test_style();
        let mut cache = MeasurementCache::new();

        let mut dense = Measure::new();
        dense.time_signature = (4, 4);
        dense.chords = vec![
            chord("Cmaj13#11"),
            chord("F#m7b5"),
            chord("B13b9"),
            chord("Emaj9/G#"),
        ];

        let mut filler = Measure::new();
        filler.time_signature = (4, 4);
        filler.chords = vec![chord(""), chord("")];

        let dense_measure = measure_measure(&dense, &style, &mut cache);
        let filler_measure = measure_measure(&filler, &style, &mut cache);

        assert_eq!(dense_measure.visible_chord_count, 4);
        assert_eq!(filler_measure.visible_chord_count, 0);
        assert!(
            dense_measure.min_width > filler_measure.min_width * 3.0,
            "dense chord clusters should reserve a summed per-segment floor: dense={} filler={}",
            dense_measure.min_width,
            filler_measure.min_width
        );
    }

    #[test]
    fn test_chart_measurements() {
        let mut measurements = ChartMeasurements::new();
        assert!(measurements.is_empty());

        measurements.push(MeasureMeasurements::default());
        measurements.push(MeasureMeasurements::default());

        assert_eq!(measurements.len(), 2);
        assert!(measurements.get(0).is_some());
        assert!(measurements.get(2).is_none());
    }
}
