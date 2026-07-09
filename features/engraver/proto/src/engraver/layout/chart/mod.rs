//! Chart Layout Engine
//!
//! Converts Keyflow Chart data to engraver SceneNode trees for rendering.
//! Supports both paginated (MuseScore-style) and continuous scroll modes.

pub mod adapters;
pub mod chord_layout;
pub mod chord_renderer;
pub mod collision;
pub mod config;
pub mod config_presets;
pub mod constants;
pub mod count_in_renderer;
pub mod cursor;
pub mod engine_helpers;
pub mod layout_state;
pub mod measure_layout;
pub mod measure_pass;
pub mod measure_render;
pub mod notation_renderer;
pub mod page_rendering;
pub mod pipeline;
pub mod prefix_renderer;
pub mod rhythm_builder;
pub mod section_layout;
pub mod spacing;
pub mod tie_pass;
pub mod types;
pub mod width_dist;

// Re-export new config types (ChartLayoutConfig is still defined locally for backward compatibility)
pub use config::{BehavioralFlags, DEFAULT_MIN_CHORD_SYMBOL_GAP, LayoutParams, RenderOptions};
// Note: config::ChartLayoutConfig and config::FlatChartLayoutConfig are available
// but not re-exported to avoid conflict with the legacy struct below

// Re-export main types for convenience
pub use types::{
    BeatPosition, ChartLayoutResult, LayoutMode, MeasureMelodyData, MelodyNoteSegment,
    PageLayoutMetrics, expand_melodies_across_measures, melody_note_extent, melody_pitch_to_line,
    slash_glyph_for_ticks,
};

// Re-export rhythm types from chart module (canonical source)
// These were previously in types.rs but now live in chart::rhythm
pub use crate::chart::rhythm::{
    BeatStructure, ResolvedRhythm, SectionRhythms, Spillback as PushSpillback,
    detect_push_spillbacks, detect_section_start_spillback, resolve_measure_rhythm,
    resolve_section_rhythms,
};

// Re-export measurement pass types for multi-pass layout
pub use measure_pass::{
    CachedHarmonyLayout, ChartMeasurements, ChordLayoutData, HarmonyKey, MeasureMeasurements,
    MeasurementCache, compute_measure_weight, measure_chart, measure_measure,
};

// Re-export cursor types for renderer-agnostic playback highlighting
pub use cursor::{
    ChartCursor, CursorConfig, CursorState, CursorStyle, HighlightCommand, Rgba as CursorRgba,
};

// Re-export collision detection types
pub use collision::{ChordCollisionContext, resolve_chord_positions};

use std::sync::Arc;

use crate::Chart;
use crate::engraver::layout::context::{LayoutContext, LayoutContextOwned};
use crate::engraver::layout::orchestrator::{PageLayout, PageMargins, SystemLayout};
use crate::engraver::layout::segment::SegmentType;
use crate::engraver::layout::text_metrics::TextFontMetrics;
use crate::engraver::layout::tlayout::{BarlineType, HarmonyStyle};
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;
use crate::engraver::scene::traverse::SceneNodeExt;
use crate::engraver::style::MStyle;
use crate::key::KeySpelling;
use crate::sections::SectionType;
use kurbo::{Affine, Rect};
use tracing::debug;

fn key_signature_fifths(chart: &Chart) -> i8 {
    chart
        .initial_key
        .as_ref()
        .map(|key| {
            let is_major = key.mode.name() == "Ionian";
            KeySpelling::new(key.root(), is_major).accidental_count()
        })
        .unwrap_or(0)
}

/// Prevailing key-signature fifths in effect at a section-local measure,
/// applying every key change at or before it (in playback order). Lets each
/// system's prefix show the key actually sounding there rather than always the
/// chart's opening key.
fn prevailing_fifths_at(chart: &Chart, section_idx: usize, local_measure: usize) -> i8 {
    let mut fifths = chart
        .initial_key
        .as_ref()
        .map(prefix_renderer::key_to_fifths)
        .unwrap_or(0);
    for kc in &chart.key_changes {
        let kc_measure = kc.position.total_duration.measure as usize;
        let applies = kc.section_index < section_idx
            || (kc.section_index == section_idx && kc_measure <= local_measure);
        if applies {
            fifths = prefix_renderer::key_to_fifths(&kc.to_key);
        }
    }
    fifths
}

fn collect_system_ink_bounds(scene: &SceneNode, page: &PageLayout) -> Vec<Option<Rect>> {
    let mut bounds_by_system = vec![None; page.systems.len()];
    let content_x0 = page.x_offset + page.margins.left;
    let content_x1 = page.x_offset + page.width - page.margins.right;
    let margin_label_cutoff_x = content_x0 + 80.0;

    for (node, transform) in scene.iter_with_transforms() {
        if node.commands.is_empty() {
            continue;
        }
        if node
            .metadata
            .get("section_start_dynamic")
            .is_some_and(|value| value == "true")
        {
            continue;
        }
        let Some(bounds) = notation_renderer::scene_ink_bounds(node, transform) else {
            continue;
        };
        if is_page_background(bounds, page) {
            continue;
        }
        if node.id.is_none() && bounds.x1 <= margin_label_cutoff_x {
            continue;
        }
        if bounds.x1 <= content_x0 || bounds.x0 >= content_x1 {
            continue;
        }

        let center_x = (bounds.x0 + bounds.x1) * 0.5;
        let center_y = (bounds.y0 + bounds.y1) * 0.5;
        if center_x < page.x_offset
            || center_x > page.x_offset + page.width
            || center_y < page.y_offset
            || center_y > page.y_offset + page.height
        {
            continue;
        }

        let y_on_page = center_y - page.y_offset;
        let Some((system_idx, _)) = page.systems.iter().enumerate().min_by(|(_, a), (_, b)| {
            let a_center = a.y + a.height * 0.5;
            let b_center = b.y + b.height * 0.5;
            (y_on_page - a_center)
                .abs()
                .total_cmp(&(y_on_page - b_center).abs())
        }) else {
            continue;
        };

        let slot = &mut bounds_by_system[system_idx];
        *slot = Some(slot.map_or(bounds, |current: kurbo::Rect| current.union(bounds)));
    }

    bounds_by_system
}

fn shift_scene_systems(
    scene: &mut SceneNode,
    pages: &[PageLayout],
    page_shift_plans: &[(u32, Vec<(usize, f64)>)],
) {
    for child in &mut scene.children {
        let Some(bounds) = child_world_bounds(child) else {
            continue;
        };
        let center_y = (bounds.y0 + bounds.y1) * 0.5;
        let center_x = (bounds.x0 + bounds.x1) * 0.5;

        let Some(page) = child
            .get_page()
            .and_then(|page_number| pages.iter().find(|page| page.number == page_number))
            .or_else(|| {
                pages.iter().find(|page| {
                    center_x >= page.x_offset
                        && center_x <= page.x_offset + page.width
                        && center_y >= page.y_offset
                        && center_y <= page.y_offset + page.height
                })
            })
        else {
            continue;
        };

        if is_page_background(bounds, page) {
            continue;
        }

        let Some((_, shifts)) = page_shift_plans
            .iter()
            .find(|(page_number, _)| *page_number == page.number)
        else {
            continue;
        };

        let y_on_page = center_y - page.y_offset;
        let Some(system) = child
            .get_system()
            .and_then(|system_index| {
                page.systems
                    .iter()
                    .find(|system| system.index == system_index as usize)
            })
            .or_else(|| {
                page.systems.iter().min_by(|a, b| {
                    let a_center = a.y + a.height * 0.5;
                    let b_center = b.y + b.height * 0.5;
                    (y_on_page - a_center)
                        .abs()
                        .total_cmp(&(y_on_page - b_center).abs())
                })
            })
        else {
            continue;
        };

        if let Some((_, shift)) = shifts
            .iter()
            .rev()
            .find(|(system_index, _)| system.index >= *system_index)
        {
            child.transform = Affine::translate((0.0, -*shift)) * child.transform;
        }
    }
}

fn child_world_bounds(node: &SceneNode) -> Option<Rect> {
    let bounds = node.compute_bounds();
    if bounds.is_zero_area() {
        None
    } else {
        Some(node.transform.transform_rect_bbox(bounds))
    }
}

fn is_page_background(bounds: Rect, page: &PageLayout) -> bool {
    bounds.width() > page.width * 0.8 && bounds.height() > page.height * 0.8
}

fn node_spacing_reason(node: &SceneNode, fallback: &str) -> String {
    let element = node
        .metadata
        .get("element_type")
        .map(String::as_str)
        .or_else(|| node.id.as_ref().map(|id| id.element_type.svg_type_name()))
        .unwrap_or(fallback);
    let content = node.commands.iter().find_map(|command| {
        if let PaintCommand::Text { text, .. } = command {
            Some(text.as_str())
        } else {
            None
        }
    });
    if let Some(content) = content {
        format!("{fallback}:{element}:{content}")
    } else {
        format!("{fallback}:{element}")
    }
}

fn should_count_for_system_height(node: &SceneNode) -> bool {
    if node
        .metadata
        .get("section_start_dynamic")
        .is_some_and(|value| value == "true")
    {
        let stack_count = node
            .metadata
            .get("section_start_dynamic_stack_count")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        return stack_count > 1;
    }
    true
}

fn record_system_ink_bottom(
    system_ink_bottom: &mut f64,
    contributors: &mut Vec<String>,
    node: &SceneNode,
    reason: &str,
) {
    if !should_count_for_system_height(node) {
        return;
    }
    if let Some(bounds) = notation_renderer::scene_ink_bounds(node, Affine::IDENTITY)
        && bounds.y1 > *system_ink_bottom
    {
        *system_ink_bottom = bounds.y1;
        contributors.push(format!(
            "{} y1={:.1} bounds=({:.1},{:.1},{:.1},{:.1})",
            node_spacing_reason(node, reason),
            bounds.y1,
            bounds.x0,
            bounds.y0,
            bounds.x1,
            bounds.y1
        ));
    }
}

/// Chart layout engine configuration.
#[derive(Debug, Clone)]
pub struct ChartLayoutConfig {
    /// Page margins.
    pub margins: PageMargins,
    /// Staff space (spatium) in points.
    pub spatium: f64,
    /// Spacing between systems.
    pub system_spacing: f64,
    /// Maximum measures per system.
    pub max_measures_per_system: usize,
    /// Minimum measure width.
    pub min_measure_width: f64,
    /// Harmony style for chord symbols.
    pub harmony_style: HarmonyStyle,
    /// Hide repeated consecutive chord symbols.
    /// When true, if a chord is the same as the previous chord, it won't be displayed.
    /// This reduces visual clutter in charts with many repeated chords.
    pub hide_repeated_chords: bool,
    /// Use stemmed rhythm notation (for charts with push/pull timing).
    /// When false (default), uses stemless slash notation.
    /// When true, uses stemmed rhythmic notation with beams and triplet brackets.
    pub use_stems: bool,
    /// Automatically fill whole/half notes with quarter note slashes.
    /// When true (default), chords with whole note or half note durations
    /// are automatically expanded to quarter note slashes:
    /// - A whole note chord (4 beats in 4/4) becomes 4 quarter slashes
    /// - A half note chord (2 beats) becomes 2 quarter slashes
    ///
    /// This is standard notation for master rhythm charts.
    /// Can be disabled with `/AUTO_RHYTHM_SLASHES=false` in the chart.
    pub auto_rhythm_slashes: bool,
    /// Show measure numbers above the first measure of each system.
    /// When true, displays the measure number (accounting for offset) above bars.
    pub show_measure_numbers: bool,
    /// Offset for measure numbering (from REAPER's projmeasoffs or song start).
    /// If the project starts at measure 5, set this to 4 so measure 1 displays as "5".
    pub measure_number_offset: i32,
    /// Number of count-in measures (0, 1, or 2).
    /// Determined by the distance between Count-In marker and SONGSTART marker.
    /// Count-in measures are the N measures immediately before measure 1.
    pub count_in_measures: u8,
    /// How count-in measures participate in measure numbering. See
    /// [`config::RenderOptions::count_in_uses_negative_numbers`].
    pub count_in_uses_negative_numbers: bool,
    /// Snippet mode: use simple white background without shadow.
    pub snippet_mode: bool,
    /// Minimum horizontal gap between adjacent chord symbols (in points).
    /// When chord symbols would overlap or be closer than this gap,
    /// they are automatically pushed apart during rendering.
    /// Default is 4.0 points.
    pub min_chord_symbol_gap: f64,
    /// Whether push/pull notation alters the rhythm display.
    /// When true (default), pushed chords create triplet/syncopated notation.
    /// When false, pushed chords show apostrophe markers on chord symbols.
    pub push_alters_rhythm: bool,
    /// Whether to add page offsets for multi-page viewing.
    /// When true (default), pages are positioned at (20, 20) with gaps for multi-page layouts.
    /// When false, the page is positioned at (0, 0) for single-page PDF export or edge-to-edge rendering.
    pub use_page_offsets: bool,
    /// Duration-to-space power law slope (default 1.2).
    /// Controls how aggressively longer notes get more space.
    pub spacing_slope: f64,
    /// Spacing density (default 1.0). Higher values = tighter spacing.
    pub spacing_density: f64,
    /// Fill limit for last system justification (default 0.3).
    /// Systems below this fill ratio are left ragged.
    pub last_system_fill_limit: f64,
    /// Whether to draw tie arcs across barlines for melody notes that were
    /// split by `expand_melodies_across_measures`. Default `true` (standard
    /// notation). Set to `false` for lead-sheet styles where the second piece
    /// is rendered as a fresh attack without a tie.
    pub draw_melody_barline_ties: bool,
    /// Beam-grouping strategy for melody eighths/sixteenths within a measure.
    /// Default `Standard` (break at every beat in 4/4). `JazzHalfBar` groups
    /// across beat-2 / beat-4 boundaries (eighths in beats 1-2 and 3-4 form
    /// single beam groups), as is common in jazz lead sheets. `FullBar`
    /// groups all eighths in a measure into one beam (highly compressed).
    pub beam_grouping: BeamGroupingMode,
}

/// Viewport size class for responsive chart layout.
///
/// Each class maps to a [`ChartLayoutConfig`] tuned for that device's
/// reading distance and screen real estate. Use [`Breakpoint::from_viewport_pt`]
/// to classify a viewport width, or pick one directly for testing.
///
/// Thresholds are in points (72pt = 1 inch). At 96 DPI:
/// - Phone: < 480pt (~640 CSS px)
/// - Tablet: 480-900pt (~640-1200 CSS px)
/// - Desktop: ≥ 900pt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Breakpoint {
    /// Small screen: phones in portrait, narrow popovers.
    /// Optimized for one-handed reading at arm's length.
    Phone,
    /// Medium screen: tablets, laptops in split view.
    /// Standard rehearsal-screen sizing.
    Tablet,
    /// Large screen: monitors, full-window desktop.
    Desktop,
}

impl Breakpoint {
    /// Classify a viewport width (points) into a size class.
    #[must_use]
    pub fn from_viewport_pt(width_pt: f64) -> Self {
        if width_pt < 480.0 {
            Self::Phone
        } else if width_pt < 900.0 {
            Self::Tablet
        } else {
            Self::Desktop
        }
    }

    /// Target measures per system for this breakpoint.
    ///
    /// iReal Pro convention: 2 cells (4 measures) on phone, 4 cells
    /// (4 measures) on tablet/desktop. Returns measure count, not cell count.
    #[must_use]
    pub fn measures_per_system(self) -> usize {
        match self {
            Self::Phone => 2,
            Self::Tablet | Self::Desktop => 4,
        }
    }
}

/// Beam grouping strategy for melody notes within a measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BeamGroupingMode {
    /// Break beams at every beat boundary (standard classical convention).
    #[default]
    Standard,
    /// Group eighths within each half-bar (beats 1-2 and 3-4 in 4/4).
    /// Common in jazz lead sheets.
    JazzHalfBar,
    /// Beam every eighth/sixteenth in the measure together. Compressed.
    FullBar,
}

// `impl Default for ChartLayoutConfig` and all preset / builder methods
// live in `config_presets.rs`.

/// Parameters for `ChartLayoutEngine::layout_measure`.
struct MeasureLayoutParams<'a> {
    measure: &'a crate::chart::types::Measure,
    melody_data: Option<&'a MeasureMelodyData>,
    spillbacks: Option<&'a [PushSpillback]>,
    measure_width: f64,
    include_clef: bool,
    include_time_sig: bool,
    time_signature: (u8, u8),
    /// When `include_time_sig` is set, the color of the rendered meter glyph.
    /// `None` = default black; a mid-chart meter change passes red.
    time_sig_color: Option<peniko::Color>,
    /// Chart-level clef used to map melody pitches to staff lines so the
    /// rendered note sits at the correct height for the chosen clef.
    clef: crate::chart::ChartClef,
    ctx: &'a LayoutContext<'a>,
    id_base: u64,
    is_boundary: bool,
}

/// Red used to highlight mid-chart meter and key changes so they are obvious.
fn change_highlight_color() -> peniko::Color {
    peniko::Color::from_rgba8(0xCC, 0x00, 0x00, 0xFF)
}

/// Build a red, in-place key-change indicator (cancelling naturals + the new
/// key's accidentals, MuseScore-style) sitting in the barline gap just before
/// `measure_x`, on the staff. Returns the positioned scene node.
fn key_change_indicator_node(
    kc: &keyflow_proto::chart::types::KeyChange,
    measure_x: f64,
    staff_y: f64,
    spatium: f64,
    ctx: &LayoutContext,
    id: u64,
) -> SceneNode {
    use crate::engraver::layout::tlayout::keysig::{
        ClefContext, KeySigParams, KeySigType, layout_keysig,
    };
    use prefix_renderer::key_to_fifths;

    let to_fifths = key_to_fifths(&kc.to_key);
    let prev_fifths = kc.from_key.as_ref().map(key_to_fifths);
    let params = KeySigParams {
        id,
        key: KeySigType::Standard(to_fifths),
        clef: ClefContext::Treble,
        show_naturals: prev_fifths.is_some(),
        prev_key: prev_fifths.map(KeySigType::Standard),
        color: Some(change_highlight_color()),
    };
    let (layout, mut node) = layout_keysig(&params, ctx);
    let width = layout.bbox.width();
    let x = (measure_x - width - spatium * 0.5).max(0.0);
    node.transform = Affine::translate((x, staff_y + 2.0 * spatium));
    node
}

/// Chart layout engine.
///
/// Converts Keyflow Chart data structures to engraver SceneNode trees.
#[derive(Clone)]
pub struct ChartLayoutEngine {
    config: ChartLayoutConfig,
    style: &'static MStyle,
    text_font_data: Arc<Vec<u8>>,
    symbol_font_data: Arc<Vec<u8>>,
}

impl ChartLayoutEngine {
    /// Create a new chart layout engine.
    pub fn new(
        style: &'static MStyle,
        text_font_data: Arc<Vec<u8>>,
        symbol_font_data: Arc<Vec<u8>>,
    ) -> Self {
        Self {
            config: ChartLayoutConfig::default(),
            style,
            text_font_data,
            symbol_font_data,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(
        config: ChartLayoutConfig,
        style: &'static MStyle,
        text_font_data: Arc<Vec<u8>>,
        symbol_font_data: Arc<Vec<u8>>,
    ) -> Self {
        Self {
            config,
            style,
            text_font_data,
            symbol_font_data,
        }
    }

    /// Set the harmony style for chord symbols.
    pub fn set_harmony_style(&mut self, style: HarmonyStyle) {
        self.config.harmony_style = style;
    }

    /// Build an MStyle override carrying `self.config.spatium`.
    ///
    /// Glyph-rendering paths (clef, time signature, slash, note heads)
    /// read `ctx.spatium()` which resolves to `MStyle::Sid::Spatium`
    /// — defaulting to 5pt. Without this override they ignore the
    /// per-breakpoint config.spatium and render at the wrong scale.
    fn styled_for_config(&self) -> MStyle {
        use crate::engraver::style::{Sid, StyleValue};
        let mut style = (*self.style).clone();
        style.set(Sid::Spatium, StyleValue::Real(self.config.spatium as f32));
        style
    }

    /// Layout a chart in the specified mode.
    pub fn layout_chart(&self, chart: &Chart, mode: &LayoutMode) -> ChartLayoutResult {
        // Apply chart settings to a temporary config
        let config_with_settings = self.config.clone().with_chart_settings(&chart.settings);
        let temp_engine = ChartLayoutEngine {
            config: config_with_settings,
            style: self.style,
            text_font_data: self.text_font_data.clone(),
            symbol_font_data: self.symbol_font_data.clone(),
        };

        match mode {
            LayoutMode::Paginated {
                page_width,
                page_height,
            } => temp_engine.layout_paginated(chart, *page_width, *page_height),
            LayoutMode::ContinuousScroll { width } => temp_engine.layout_continuous(chart, *width),
            LayoutMode::Snippet { page_width } => {
                // Use tighter margins for snippets
                temp_engine.layout_snippet(chart, *page_width)
            }
        }
    }

    /// Layout a snippet with bounds-based sizing.
    /// Renders content once, measures bounds, then adjusts page background to fit.
    fn layout_snippet(&self, chart: &Chart, page_width: f64) -> ChartLayoutResult {
        // layout_paginated uses a fixed page_offset of 20.0 for positioning
        let page_offset = 20.0;

        // Step 1: Layout with snippet mode and small margins
        let snippet_margins = PageMargins {
            top: 5.0,
            bottom: 5.0,
            left: 50.0,
            right: 5.0,
        };

        let measure_config = ChartLayoutConfig {
            margins: snippet_margins,
            snippet_mode: true, // Simple white background
            ..self.config.clone()
        };

        let measure_engine = ChartLayoutEngine {
            config: measure_config,
            style: self.style,
            text_font_data: self.text_font_data.clone(),
            symbol_font_data: self.symbol_font_data.clone(),
        };

        // Layout with the provided page width (constrain measure widths to this)
        // Use a tall page height to fit all content vertically
        let layout_width = if page_width > 0.0 { page_width } else { 612.0 }; // Default to letter width
        let mut result = measure_engine.layout_paginated(chart, layout_width, 10000.0);

        // Step 2: Compute actual content bounds by examining children
        // Skip the first child which is page background (white rect in snippet mode)
        let mut content_bounds = Rect::ZERO;
        for (i, child) in result.scene.children.iter().enumerate() {
            // Skip first child (page background)
            if i < 1 {
                continue;
            }
            let child_bounds = child.compute_bounds();
            if !child_bounds.is_zero_area() {
                let transformed = child.transform.transform_rect_bbox(child_bounds);
                if content_bounds.is_zero_area() {
                    content_bounds = transformed;
                } else {
                    content_bounds = content_bounds.union(transformed);
                }
            }
        }

        // Step 3: Calculate final page dimensions from content bounds
        let padding = 5.0;
        let clef_bottom_padding = 8.0;

        // Content might extend above page_offset (chord symbols)
        let content_above_page = (page_offset - content_bounds.y0).max(0.0);

        // Final dimensions based on actual content
        let final_width = content_bounds.x1 + padding;
        let final_height = content_bounds.y1 + padding + clef_bottom_padding;

        // Step 4: Replace the page background with correctly sized one
        // The first child is the white background rect - replace it
        if !result.scene.children.is_empty() {
            let new_background = SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
                Rect::new(page_offset, page_offset, final_width, final_height),
                peniko::Color::WHITE,
            )]);
            result.scene.children[0] = new_background;
        }

        // If content extends above page_offset, shift all content down
        if content_above_page > 0.0 {
            let shift = Affine::translate((0.0, content_above_page));
            for (i, child) in result.scene.children.iter_mut().enumerate() {
                if i > 0 {
                    // Skip background
                    child.transform = shift * child.transform;
                }
            }
            // Update background to account for shifted content
            if !result.scene.children.is_empty() {
                let new_background = SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
                    Rect::new(
                        page_offset,
                        page_offset,
                        final_width,
                        final_height + content_above_page,
                    ),
                    peniko::Color::WHITE,
                )]);
                result.scene.children[0] = new_background;
            }
            // Update total dimensions with shifted content
            result.total_width = final_width + page_offset;
            result.total_height = final_height + content_above_page + page_offset;
        } else {
            // Update total dimensions to match actual content
            result.total_width = final_width + page_offset;
            result.total_height = final_height + page_offset;
        }

        result
    }

    /// Layout a chart with explicit configuration.
    ///
    /// This temporarily applies the given config for layout, then restores
    /// the engine's default config. Useful for charts with rhythmic complexity
    /// that need stemmed notation.
    pub fn layout_chart_with_config(
        &self,
        chart: &Chart,
        mode: &LayoutMode,
        config: &ChartLayoutConfig,
    ) -> ChartLayoutResult {
        // Create a temporary engine with the custom config
        let temp_engine = ChartLayoutEngine {
            config: config.clone(),
            style: self.style,
            text_font_data: self.text_font_data.clone(),
            symbol_font_data: self.symbol_font_data.clone(),
        };
        temp_engine.layout_chart(chart, mode)
    }

    /// Layout chart in paginated mode with page breaks.
    fn layout_paginated(
        &self,
        chart: &Chart,
        page_width: f64,
        page_height: f64,
    ) -> ChartLayoutResult {
        let ctx_owned = LayoutContextOwned::new_minimal(self.styled_for_config());
        let ctx = ctx_owned.as_context();
        let text_metrics = TextFontMetrics::new(self.text_font_data.clone());
        // Use text font for symbols when symbol_font_family is None (e.g., MuseJazz uses same font)
        let symbol_metrics = if self.config.harmony_style.symbol_font_family.is_none() {
            text_metrics.clone()
        } else {
            TextFontMetrics::new(self.symbol_font_data.clone())
        };

        let harmony_style = self
            .config
            .harmony_style
            .clone()
            .with_text_font_metrics(text_metrics.clone())
            .with_symbol_font_metrics(symbol_metrics.clone());

        let content_width = page_width - self.config.margins.left - self.config.margins.right;
        let content_height = page_height - self.config.margins.top - self.config.margins.bottom;
        let staff_height = self.config.spatium * 4.0;

        let mut root = SceneNode::group(SemanticId::new(ElementType::Page, 0));
        let mut pages: Vec<PageLayout> = Vec::new();
        let mut current_page_systems: Vec<SystemLayout> = Vec::new();
        let mut page_number = 1u32;
        let mut page_y = self.config.margins.top;
        let mut global_system_index = 0usize;
        let mut id_counter = 100u64;

        // Track previous chord to hide duplicates (reset at each system start)
        let mut previous_chord_symbol: Option<String>;

        // Beat position collection for cursor/highlight rendering
        let mut beat_positions: Vec<BeatPosition> = Vec::new();
        let mut global_measure_index: usize = 0;

        // Calculate seconds per tick from tempo (480 ticks per quarter note)
        let tempo_bpm = chart.tempo.map(|t| t.bpm).unwrap_or(120.0);
        let seconds_per_quarter = 60.0 / tempo_bpm;
        let seconds_per_tick = seconds_per_quarter / 480.0;

        // Get time signature for count-in duration calculation
        let time_signature = chart
            .time_signature
            .map(|ts| (ts.numerator as u8, ts.denominator as u8))
            .unwrap_or((4u8, 4u8));

        // Prevailing meter, carried across sections/systems in playback order so a
        // measure whose `time_signature` differs from its predecessor renders an
        // inline meter change. Seeded with the chart's initial meter.
        // `prev_prevailing_ts` + `ts_run_len` detect a one-measure excursion (e.g.
        // the auto-revert after `!T2/4`): that revert is drawn but NOT highlighted,
        // since the return-to-normal is adjacent and obvious. A revert that is
        // several measures after the change is highlighted like any other change.
        let mut prevailing_ts = time_signature;
        let mut prev_prevailing_ts: Option<(u8, u8)> = None;
        let mut ts_run_len: usize = 0;

        // Detect count-in from parsed chart sections if not configured explicitly.
        // If the chart has a CountIn section, use its measure count.
        let count_in_measures = if self.config.count_in_measures > 0 {
            self.config.count_in_measures as usize
        } else {
            chart
                .sections
                .iter()
                .find(|s| matches!(s.section.section_type, SectionType::CountIn))
                .map(|s| s.measures().len())
                .unwrap_or(0)
        };

        // Calculate count-in duration in ticks and seconds
        // Count-in should have NEGATIVE time values so that:
        // - Count-in measures have negative time (before SONGSTART)
        // - Real measures start at time 0 (at SONGSTART)
        let ticks_per_measure = time_signature.0 as i64 * (1920i64 / time_signature.1 as i64);
        let count_in_ticks = count_in_measures as i64 * ticks_per_measure;
        let count_in_seconds = count_in_ticks as f64 * seconds_per_tick;

        // Track cumulative time and ticks through the song
        // Start at negative offset for count-in so real measures begin at 0
        let mut cumulative_time: f64 = -count_in_seconds;
        let mut cumulative_ticks: i64 = -count_in_ticks;

        // Calculate page offset for multi-page rendering
        // When use_page_offsets is false, render at origin for single-page export
        let page_offset_x = if self.config.use_page_offsets {
            20.0
        } else {
            0.0
        };
        let page_offset_y = if self.config.use_page_offsets {
            20.0
        } else {
            0.0
        };
        let page_gap = if self.config.use_page_offsets {
            40.0
        } else {
            0.0
        };

        // Track the X offset for the current page (updated when a new page starts)
        let mut current_page_x = page_offset_x;

        // Pre-compute section letters for consecutive repeats
        let section_letters = self.compute_section_letters(&chart.sections);

        // === PASS 1: MEASURE ===
        // Pre-measure all chord symbols to get accurate widths for layout
        // This replaces the estimate_measure_content_weight/compute_minimum_measure_width calls
        let mut measurement_cache = measure_pass::MeasurementCache::new();
        let chart_measurements = measure_pass::measure_chart(
            chart
                .sections
                .iter()
                .filter(|s| {
                    !s.section.section_type.is_compact()
                        && !matches!(s.section.section_type, SectionType::End)
                })
                .map(|s| s.measures()),
            &harmony_style,
            &mut measurement_cache,
        );

        debug!(
            "[measure-pass] Pre-measured {} measures, cached {} chord widths",
            chart_measurements.len(),
            measurement_cache.len()
        );

        // Track if we've added the title header (only on first page)
        let mut title_header_added = false;

        // Track global measure offset for looking up pre-measured values
        let mut global_section_measure_offset: usize = 0;

        // Process each section
        for (section_idx, chart_section) in chart.sections.iter().enumerate() {
            // Skip count-in sections - we synthesize count-in from config.count_in_measures.
            // Advance cumulative counters by the skipped section's duration so the
            // next section starts at the correct tick (tick 0 = SONGSTART).
            if chart_section.section.section_type.is_compact() {
                let skipped_measures = chart_section.measures().len() as i64;
                cumulative_ticks += skipped_measures * ticks_per_measure;
                cumulative_time +=
                    skipped_measures as f64 * ticks_per_measure as f64 * seconds_per_tick;
                continue;
            }

            // Skip End sections entirely - they are only for progress bars, not charts
            if matches!(chart_section.section.section_type, SectionType::End) {
                continue;
            }

            // Preprocess melodies to handle cross-measure spillover
            let beats_per_measure = time_signature.0 as f64;
            let key_signature = key_signature_fifths(chart);
            let mut melody_data_map = expand_melodies_across_measures(
                chart_section.measures(),
                beats_per_measure,
                key_signature,
            );
            // Stamp the chart-level clef onto each measure's melody data so
            // pitch→staff-line mapping picks the correct middle-line pitch.
            let chart_proto_clef = self.chart_proto_clef_for(chart);
            for md in melody_data_map.values_mut() {
                md.clef = chart_proto_clef;
            }

            // Detect push spillbacks (chords from next measure that push back)
            let mut push_spillback_map = detect_push_spillbacks(chart_section.measures());

            if !push_spillback_map.is_empty() {
                debug!(
                    "[spillback-detection] Section {} detected {} spillbacks: {:?}",
                    chart_section.section.section_type.full_name(),
                    push_spillback_map.values().map(|v| v.len()).sum::<usize>(),
                    push_spillback_map
                        .iter()
                        .map(|(k, v)| (
                            k,
                            v.iter()
                                .map(|s| (&s.chord_symbol, &s.push_base))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }

            // Check for cross-section spillback: if the NEXT section starts with a pushed chord,
            // it spills back to THIS section's last measure
            let next_non_compact_section = chart.sections.iter().skip(section_idx + 1).find(|s| {
                !s.section.section_type.is_compact()
                    && !matches!(s.section.section_type, SectionType::End)
            });
            if let Some(next_section) = next_non_compact_section
                && let Some(mut spillback) = detect_section_start_spillback(next_section.measures())
            {
                // Adjust beat position based on this section's last measure time signature
                if let Some(last_measure) = chart_section.measures().last() {
                    let ts = last_measure.time_signature;
                    spillback.beat_position = (ts.0 as usize).saturating_sub(1);
                }
                let last_measure_idx = chart_section.measures().len().saturating_sub(1);

                debug!(
                    "[cross-section-spillback] Section {} -> {} spillback: '{}' at measure {} beat {}",
                    chart_section.section.section_type.full_name(),
                    next_section.section.section_type.full_name(),
                    spillback.chord_symbol,
                    last_measure_idx,
                    spillback.beat_position
                );

                push_spillback_map
                    .entry(last_measure_idx)
                    .or_default()
                    .push(spillback);
            }

            // Mid-chart key changes in this section, keyed by section-local
            // measure index (the parser stores the change's measure ordinal in
            // total_duration.measures).
            let key_changes_in_section: std::collections::HashMap<usize, _> = chart
                .key_changes
                .iter()
                .filter(|kc| kc.section_index == section_idx)
                .map(|kc| (kc.position.total_duration.measure as usize, kc))
                .collect();

            // Group measures into systems (count-based for consistent layout)
            let systems = self.group_measures_into_systems(chart_section.measures(), content_width);

            for (sys_idx, measure_indices) in systems.iter().enumerate() {
                // Reset chord tracking at line breaks (new systems)
                // so repeated chords are always visible at the start of each line
                previous_chord_symbol = None;

                // Compute extra space for melody notes extending beyond the staff
                let mut melody_extra_above = 0.0f64;
                let mut melody_extra_below = 0.0f64;
                for &m_idx in measure_indices {
                    if let Some(md) = melody_data_map.get(&m_idx) {
                        let (above, below) = melody_note_extent(
                            md,
                            self.config.spatium,
                            self.chart_proto_clef_for(chart),
                        );
                        melody_extra_above = melody_extra_above.max(above);
                        melody_extra_below = melody_extra_below.max(below);
                    }
                }
                // MuseScore spaces systems from accumulated skyline extents
                // rather than from staff lines alone. Until this chart path has
                // a full page-level skyline pass, reserve explicit north/south
                // notation bands: chord symbols live above the staff, while
                // dynamics/text/figured bass live below.
                let system_top_reserve = self.config.spatium * 2.0;
                let system_bottom_reserve = self.config.spatium * 4.5;
                let system_height = system_top_reserve
                    + melody_extra_above
                    + staff_height
                    + melody_extra_below
                    + system_bottom_reserve;

                // Check for page overflow (don't include spacing - last system doesn't need it)
                if page_y + system_height > self.config.margins.top + content_height {
                    // Finalize current page (background was already added when page started)
                    if !current_page_systems.is_empty() {
                        pages.push(PageLayout {
                            number: page_number,
                            x_offset: current_page_x,
                            y_offset: page_offset_y,
                            width: page_width,
                            height: page_height,
                            systems: std::mem::take(&mut current_page_systems),
                            margins: self.config.margins,
                        });
                        page_number += 1;
                        page_y = self.config.margins.top;
                    }
                }

                // Calculate page position
                let page_x = page_offset_x + (page_number as f64 - 1.0) * (page_width + page_gap);
                let content_x = page_x + self.config.margins.left;

                // Add page background if this is first system on page
                if current_page_systems.is_empty() {
                    // Track the page X offset for this page
                    current_page_x = page_x;
                    self.add_page_background(
                        &mut root,
                        page_x,
                        page_offset_y,
                        page_width,
                        page_height,
                    );
                    self.add_page_footer(
                        &mut root,
                        page_x,
                        page_offset_y,
                        page_width,
                        page_height,
                        &chart.metadata,
                    );

                    // Add title header on first page only
                    // Include count-in snippet in header instead of on first system
                    if !title_header_added {
                        let has_pushed_first_chord = chart_first_chord_is_pushed(chart);
                        // Per-measure labels above the count-in snippet — pulled
                        // from the CountIn section's measures so the overlay
                        // shows their source measure numbers (1, 2 for LotF).
                        let count_in_labels: Vec<String> = chart
                            .sections
                            .iter()
                            .find(|s| matches!(s.section.section_type, SectionType::CountIn))
                            .map(|s| {
                                s.measures()
                                    .iter()
                                    .enumerate()
                                    .map(|(i, m)| {
                                        m.source_measure_number
                                            .map(|n| n.to_string())
                                            .unwrap_or_else(|| (i + 1).to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let (header_height, count_in_geos) = self.add_title_header(
                            &mut root,
                            page_x,
                            page_offset_y,
                            page_width,
                            &chart.metadata,
                            chart.tempo.as_ref(),
                            count_in_measures,
                            time_signature,
                            has_pushed_first_chord,
                            count_in_labels,
                        );
                        page_y += header_height;
                        title_header_added = true;

                        // Create BeatPosition entries for count-in beats so the
                        // cursor can highlight them during the count-in period.
                        let ticks_per_beat = 1920i64 / time_signature.1 as i64;
                        for geo in &count_in_geos {
                            let beat_offset_in_measure = geo.beat_index as i64 * ticks_per_beat;
                            let measure_offset = geo.measure_index as i64 * ticks_per_measure;
                            let absolute_tick =
                                -count_in_ticks + measure_offset + beat_offset_in_measure;
                            beat_positions.push(BeatPosition {
                                page: page_number,
                                system: 0,
                                measure: geo.measure_index,
                                beat: geo.beat_index,
                                tick: beat_offset_in_measure as i32,
                                duration_ticks: ticks_per_beat as i32,
                                absolute_tick,
                                x: geo.x,
                                width: geo.width,
                                staff_y: geo.staff_y,
                                staff_height: geo.staff_height,
                                time_start: absolute_tick as f64 * seconds_per_tick,
                                time_end: (absolute_tick + ticks_per_beat) as f64
                                    * seconds_per_tick,
                                glyph_codepoint: Some(geo.glyph_codepoint),
                                glyph_size: geo.glyph_size,
                                glyph_y: geo.glyph_y,
                                has_stem: false,
                                stem_up: true,
                                flag_count: 0,
                                time_signature,
                            });
                        }
                    }
                }

                // Calculate staff_y (after potential header adjustment)
                // Shift down by melody_extra_above and the above-staff reserve
                // so chord symbols on this system cannot intrude into the
                // previous system's south skyline.
                let staff_y = page_offset_y + page_y + system_top_reserve + melody_extra_above;
                let system_page_top = page_offset_y + page_y;
                let mut system_ink_bottom = staff_y + staff_height;
                let mut system_height_contributors = Vec::new();

                // Calculate system prefix width (clef + key sig + time sig) FIRST
                // Needed to determine staff line width for short systems
                let include_clef = true; // Always show clef at system start
                let include_key_sig = true; // Key sig on every system (standard notation)
                let include_time_sig = global_system_index == 0; // Time sig only on first system

                // Key signature prevailing at this system's downbeat (follows
                // mid-chart key changes), and whether the change happens right
                // here — if so the prefix is drawn red in place rather than
                // stacking a separate red indicator over it.
                let system_first_measure = measure_indices.first().copied().unwrap_or(0);
                let key_signature: i8 =
                    prevailing_fifths_at(chart, section_idx, system_first_measure);
                let system_starts_with_key_change = measure_indices
                    .first()
                    .is_some_and(|m| key_changes_in_section.contains_key(m));

                let (clef_width, key_sig_width, time_sig_width, prefix_width) =
                    prefix_renderer::calculate_prefix_width(
                        self.config.spatium,
                        include_clef,
                        include_key_sig,
                        key_signature,
                        include_time_sig,
                    );

                // Calculate measure width using spring-based distribution
                let measures_area_width = content_width - prefix_width;
                let num_measures = measure_indices.len();
                let max_measures = self.config.max_measures_per_system;

                // Count-in is now rendered in the header, not on the first system.
                // We no longer need to account for count-in measures in width distribution.

                // Calculate content weights for spring-based distribution
                // Also check for spillbacks from the next measure (triplet pushes that affect current measure width)
                let all_measures = chart_section.measures();

                // Calculate weights and minimum widths for each measure
                // Weights come from rhythm builder (handles triplets, etc.)
                // Min widths come from pre-measured chord symbol widths (Pass 1 results)
                let (measure_weights, measure_min_widths): (Vec<f64>, Vec<f64>) = measure_indices
                    .iter()
                    .filter_map(|&idx| all_measures.get(idx).map(|m| (idx, m)))
                    .map(|(idx, m)| {
                        // Weight still uses rhythm builder (correct for triplets/complex rhythms)
                        let weight = self.estimate_measure_content_weight(m, &text_metrics);

                        // Min width from pre-measured chord widths (Pass 1)
                        let global_idx = global_section_measure_offset + idx;
                        let min_width = chart_measurements
                            .get(global_idx)
                            .map(|m| m.min_width)
                            .unwrap_or(0.0);

                        (weight, min_width)
                    })
                    .unzip();

                // Calculate base measure width based on mode
                let base_measure_width = if self.config.snippet_mode {
                    // Snippet mode: use duration-proportional natural width
                    // for a quarter-note measure as the base unit
                    spacing::natural_width(
                        constants::TICKS_PER_QUARTER,
                        self.config.spatium,
                        self.config.spacing_slope,
                        self.config.spacing_density,
                        1.0,
                    ) * 4.0 // 4 quarter notes per 4/4 measure
                } else {
                    // Normal mode: distribute across available space
                    measures_area_width / max_measures as f64
                };

                // For width calculation, just use the regular measure count
                let effective_measure_count = num_measures as f64;
                // Snippet mode always treated as short system (no stretching)
                let is_short_system =
                    self.config.snippet_mode || effective_measure_count < max_measures as f64;

                // Distribute width proportionally using spring physics
                // For full systems, distribute the entire measures_area_width
                // For short systems, distribute only the proportional amount
                let has_spacing_expansion = self.has_spacing_expander(
                    &measure_weights,
                    &measure_min_widths,
                    base_measure_width,
                );
                let total_width_to_distribute = if self.config.snippet_mode {
                    // Snippet mode: use duration-proportional natural widths
                    // No justification stretching — just natural widths
                    num_measures as f64 * base_measure_width
                } else if is_short_system && has_spacing_expansion {
                    measures_area_width
                } else if is_short_system {
                    // Short system: only use width proportional to measure count
                    num_measures as f64 * base_measure_width
                } else {
                    measures_area_width
                };

                let distributed_widths = self.distribute_measure_widths(
                    &measure_weights,
                    0, // No count-in measures in first system (now rendered in header)
                    total_width_to_distribute,
                    0.4, // compact_scale (not used when num_count_in=0)
                    base_measure_width,
                    &measure_min_widths,
                );
                self.log_system_width_decisions(
                    section_idx,
                    sys_idx,
                    measure_indices,
                    all_measures,
                    &measure_weights,
                    &measure_min_widths,
                    &distributed_widths,
                    total_width_to_distribute,
                    base_measure_width,
                    is_short_system,
                );

                // Calculate actual system width.
                // For short systems, use the sum of distributed_widths (not total_width_to_distribute)
                // because min_widths may force measures to expand beyond the planned width.
                let actual_system_width = if is_short_system {
                    prefix_width + distributed_widths.iter().sum::<f64>()
                } else {
                    content_width
                };

                // Draw staff lines (shortened for short systems)
                root.add_child(SceneNode::anonymous_leaf(self.draw_staff_lines(
                    content_x,
                    staff_y,
                    actual_system_width,
                )));

                // System-wide chord-symbol Y baseline (used as a fallback for the
                // section label and for measures without melodies). Per-measure
                // chord_y is computed inside the measure loop below using each
                // measure's local melody extent, so a single high note in one
                // measure no longer pushes chord symbols up across the whole
                // system.
                let _chord_y = staff_y + constants::CHORD_Y_OFFSET - melody_extra_above;

                // Add section label for first system of section (skip for count-in)
                let mut section_start_dynamic_stack = None;
                if sys_idx == 0
                    && !chart_section.implicit
                    && chart_section.section.section_type.should_show_header()
                {
                    let letter = section_letters.get(&section_idx).copied();
                    let y_slots = self.repeat_pass_dynamic_slots(
                        &chart_section.section,
                        page_x,
                        self.config.margins.left,
                        staff_y,
                        staff_height,
                        &ctx,
                    );
                    section_start_dynamic_stack = Some(notation_renderer::DynamicStackAnchor {
                        x: page_x + self.config.margins.left * 0.5,
                        y_slots,
                        fallback_gap: self.config.spatium * 2.2,
                    });
                    let mut section_label = self.create_section_label(
                        &chart_section.section,
                        page_x,
                        self.config.margins.left,
                        staff_y,
                        staff_height,
                        letter,
                        &ctx,
                        id_counter,
                    );
                    id_counter += 1;
                    section_label.set_system(global_system_index as u32);
                    section_label
                        .metadata
                        .insert("page".to_string(), page_number.to_string());
                    root.add_child(section_label);
                }

                // Render system prefix (clef, key signature, and time signature)
                let ts = chart
                    .time_signature
                    .map(|ts| (ts.numerator as u8, ts.denominator as u8))
                    .unwrap_or((4u8, 4u8));

                let prefix_ctx = prefix_renderer::PrefixRenderContext {
                    x: content_x,
                    staff_y,
                    spatium: self.config.spatium,
                    include_clef,
                    include_key_sig,
                    include_time_sig,
                    key_signature,
                    key_sig_color: system_starts_with_key_change.then(change_highlight_color),
                    time_signature: ts,
                    clef_width,
                    clef_type: self.chart_clef_for(chart),
                    key_sig_width,
                    time_sig_width,
                    page_number: Some(page_number),
                };

                let prefix_result =
                    prefix_renderer::render_system_prefix(&prefix_ctx, id_counter, &ctx);

                for node in prefix_result.nodes {
                    root.add_child(node);
                }
                id_counter = prefix_result.next_id;

                // Start measures after prefix
                let mut measure_x = content_x + prefix_width;
                let time_signature = chart
                    .time_signature
                    .map(|ts| (ts.numerator as u8, ts.denominator as u8))
                    .unwrap_or((4u8, 4u8));

                // Count-in is now rendered in the header (via add_title_header_with_count_in),
                // so we no longer render count-in measures inline on the first system.
                // Measures now start directly at measure 1.

                let mut system_skyline = notation_renderer::MeasureSkyline::new();
                let mut pending_notation: Vec<(SceneNode, bool, bool)> = Vec::new();
                for (local_measure_idx, &measure_idx) in measure_indices.iter().enumerate() {
                    if let Some(measure) = chart_section.measures().get(measure_idx) {
                        // Get preprocessed melody data for this measure (handles spillover)
                        let melody_data = melody_data_map.get(&measure_idx);

                        // Per-measure chord-symbol Y: use this measure's own melody
                        // extent rather than the system-wide max, so adjacent low
                        // measures don't get pushed up by one high measure.
                        let melody_extent = melody_data
                            .map(|md| {
                                melody_note_extent(
                                    md,
                                    self.config.spatium,
                                    self.chart_proto_clef_for(chart),
                                )
                                .0
                            })
                            .unwrap_or(0.0);
                        // Floor for measures that have melody notes at all: even
                        // notes inside the staff need clearance from the chord
                        // symbol above. Without this, chord text overlaps high
                        // note heads/stems when the clef-aware extent reports 0.
                        let has_melody_notes = measure
                            .melodies
                            .iter()
                            .any(|m| m.notes.iter().any(|n| !n.is_rest()));
                        let melody_floor = if has_melody_notes {
                            self.config.spatium * 1.8
                        } else {
                            0.0
                        };
                        let this_measure_extra_above = melody_extent.max(melody_floor);
                        let this_chord_y =
                            staff_y + constants::CHORD_Y_OFFSET - this_measure_extra_above;

                        // Get distributed width for this measure
                        let this_measure_width = distributed_widths
                            .get(local_measure_idx)
                            .copied()
                            .unwrap_or(base_measure_width);

                        // Get spillback chords for this measure (from next measure pushing back)
                        let spillbacks = push_spillback_map.get(&measure_idx).map(|v| v.as_slice());

                        // Layout measure content (rhythm slashes) - no clef/time sig inside measure
                        // is_boundary: first measure of section needs width for pushed chords
                        // that render both here AND spill back to previous section
                        let is_section_boundary = measure_idx == 0;
                        // A measure whose meter differs from the prevailing one
                        // renders the new time signature inline. It is highlighted
                        // red unless it merely closes a one-measure excursion (the
                        // auto-revert after `!T2/4`): that return-to-normal renders
                        // in default black because it is adjacent and obvious.
                        let measure_ts = measure.time_signature;
                        let ts_changed = measure_ts != prevailing_ts;
                        let is_oneshot_revert =
                            ts_changed && ts_run_len == 1 && prev_prevailing_ts == Some(measure_ts);
                        let highlight = ts_changed && !is_oneshot_revert;
                        let measure_result = self.layout_measure(MeasureLayoutParams {
                            measure,
                            melody_data,
                            spillbacks,
                            measure_width: this_measure_width,
                            include_clef: false,
                            include_time_sig: ts_changed,
                            time_signature: measure_ts,
                            time_sig_color: highlight.then(change_highlight_color),
                            clef: self.chart_proto_clef_for(chart),
                            ctx: &ctx,
                            id_base: id_counter,
                            is_boundary: is_section_boundary,
                        });
                        id_counter += 10;
                        if ts_changed {
                            prev_prevailing_ts = Some(prevailing_ts);
                            prevailing_ts = measure_ts;
                            ts_run_len = 1;
                        } else {
                            ts_run_len += 1;
                        }

                        // Red key-change indicator when a key change lands on a
                        // measure mid-system. A change on the system's first
                        // measure is shown by the red prefix key signature
                        // instead (see `system_starts_with_key_change`), so skip
                        // it here to avoid stacking two key sigs.
                        if let Some(kc) = key_changes_in_section
                            .get(&measure_idx)
                            .filter(|_| local_measure_idx != 0)
                        {
                            let kc_node = key_change_indicator_node(
                                kc,
                                measure_x,
                                staff_y,
                                self.config.spatium,
                                &ctx,
                                id_counter,
                            );
                            id_counter += 1;
                            root.add_child(kc_node);
                        }

                        // Get ChordRest segment x-positions (already in points after spacing)
                        let segment_positions: Vec<f64> =
                            self.get_chord_rest_positions(&measure_result);

                        let mut measure_container =
                            SceneNode::group(SemanticId::new(ElementType::Measure, id_counter));
                        id_counter += 1;
                        measure_container.transform =
                            Affine::translate((measure_x, staff_y + 2.0 * self.config.spatium));
                        measure_container
                            .metadata
                            .insert("page".to_string(), page_number.to_string());
                        notation_renderer::add_scene_obstacles(
                            &mut system_skyline,
                            &measure_result.scene,
                            measure_container.transform,
                            true,
                            true,
                        );
                        measure_container.add_child(measure_result.scene);
                        record_system_ink_bottom(
                            &mut system_ink_bottom,
                            &mut system_height_contributors,
                            &measure_container,
                            "measure",
                        );
                        root.add_child(measure_container);

                        // Add measure number on first measure of each system.
                        // Prefer the import-time source number (e.g. MusicXML
                        // `<measure number="…">`) so count-in numbering carries
                        // through — LotF's first musical bar is xml measure 3.
                        let display_measure_num = measure
                            .source_measure_number
                            .map(|n| n as i32)
                            .unwrap_or((global_measure_index as i32) + 1);
                        let is_first_of_system = local_measure_idx == 0;
                        if self.config.show_measure_numbers && is_first_of_system {
                            let measure_num_node = self.create_measure_number(
                                display_measure_num,
                                content_x,
                                staff_y,
                                id_counter,
                            );
                            id_counter += 1;
                            record_system_ink_bottom(
                                &mut system_ink_bottom,
                                &mut system_height_contributors,
                                &measure_num_node,
                                "measure_number",
                            );
                            root.add_child(measure_num_node);
                        }

                        // Collect beat positions from actual segment data
                        let measure_time_start = cumulative_time;
                        let measure_tick_start = cumulative_ticks;

                        for (beat_idx, segment) in measure_result
                            .segments
                            .iter_type(SegmentType::CHORD_REST)
                            .enumerate()
                        {
                            let time_start =
                                measure_time_start + (segment.tick as f64 * seconds_per_tick);
                            let time_end = measure_time_start
                                + ((segment.tick + segment.ticks) as f64 * seconds_per_tick);
                            let absolute_tick = measure_tick_start + segment.tick as i64;

                            // Compute stem/flag info based on duration
                            // Whole (1920) and half (960) are stemless in slash notation
                            // Quarter (480) and shorter have stems
                            let has_stem = segment.ticks < 960;
                            let flag_count = match segment.ticks {
                                t if t >= 480 => 0, // Quarter or longer: no flags
                                t if t >= 240 => 1, // Eighth: 1 flag
                                t if t >= 120 => 2, // 16th: 2 flags
                                t if t >= 60 => 3,  // 32nd: 3 flags
                                _ => 4,             // 64th: 4 flags
                            };

                            beat_positions.push(BeatPosition {
                                page: page_number,
                                system: global_system_index,
                                measure: global_measure_index,
                                beat: beat_idx,
                                tick: segment.tick,
                                duration_ticks: segment.ticks,
                                absolute_tick,
                                x: measure_x + segment.x,
                                width: segment.width,
                                staff_y,
                                staff_height,
                                time_start,
                                time_end,
                                glyph_codepoint: Some(slash_glyph_for_ticks(segment.ticks)),
                                glyph_size: self.config.spatium,
                                glyph_y: staff_y + staff_height / 2.0, // Center on staff
                                has_stem,
                                stem_up: true, // Default to stem up
                                flag_count,
                                time_signature,
                            });
                        }

                        // Calculate measure duration in ticks and update cumulative time/ticks
                        let measure_duration_ticks =
                            time_signature.0 as i32 * (1920 / time_signature.1 as i32);
                        cumulative_time += measure_duration_ticks as f64 * seconds_per_tick;
                        cumulative_ticks += measure_duration_ticks as i64;
                        global_measure_index += 1;

                        // Render chord symbols using chord_renderer module
                        // Look up pre-computed measurements for this measure
                        let global_idx = global_section_measure_offset + measure_idx;
                        let measure_measurements = chart_measurements.get(global_idx);

                        let chord_ctx = chord_renderer::ChordRenderContext {
                            measure_x,
                            measure_width: this_measure_width,
                            chord_y: this_chord_y,
                            page_number: Some(page_number),
                            global_system_index,
                            measure_idx,
                            local_measure_idx,
                            section_name: &chart_section.section.section_type.full_name(),
                            segment_positions: &segment_positions,
                            internal_push_positions: &measure_result.internal_push_positions,
                            harmony_style: &harmony_style,
                            time_signature,
                            hide_repeated_chords: self.config.hide_repeated_chords,
                            min_chord_symbol_gap: self.config.min_chord_symbol_gap,
                            push_alters_rhythm: self.config.push_alters_rhythm,
                            spatium: self.config.spatium,
                            measure_measurements,
                            spillback_positions: &measure_result.spillback_positions,
                            note_line_stacks: &measure_result.note_line_stacks,
                        };

                        let chord_result = chord_renderer::render_chord_symbols(
                            &chord_ctx,
                            measure,
                            previous_chord_symbol.as_deref(),
                            id_counter,
                            &ctx,
                        );
                        let chord_obstacles = chord_result.chord_bounds;
                        // Kept for anchoring suspension figures to their chords;
                        // `chord_obstacles` itself is consumed as obstacles below.
                        let suspension_chord_bounds = chord_obstacles.clone();

                        for node in &chord_result.nodes {
                            notation_renderer::add_scene_obstacles(
                                &mut system_skyline,
                                node,
                                Affine::IDENTITY,
                                true,
                                true,
                            );
                        }

                        for node in chord_result.nodes {
                            record_system_ink_bottom(
                                &mut system_ink_bottom,
                                &mut system_height_contributors,
                                &node,
                                "chord",
                            );
                            root.add_child(node);
                        }
                        previous_chord_symbol = chord_result.last_chord_symbol;
                        id_counter = chord_result.next_id;

                        // Render spillback chord symbols (from next measure pushing back)
                        if let Some(spillback_list) = spillbacks {
                            let spillback_result = chord_renderer::render_spillback_chords(
                                &chord_ctx,
                                spillback_list,
                                previous_chord_symbol.as_deref(),
                                id_counter,
                                &ctx,
                            );

                            for node in &spillback_result.nodes {
                                notation_renderer::add_scene_obstacles(
                                    &mut system_skyline,
                                    node,
                                    Affine::IDENTITY,
                                    true,
                                    true,
                                );
                            }

                            for node in spillback_result.nodes {
                                record_system_ink_bottom(
                                    &mut system_ink_bottom,
                                    &mut system_height_contributors,
                                    &node,
                                    "spillback_chord",
                                );
                                root.add_child(node);
                            }
                            previous_chord_symbol = spillback_result.last_chord_symbol;
                            id_counter = spillback_result.next_id;
                        }

                        // Render text cues below the staff. They are routed
                        // through the notation skyline below so they avoid
                        // dynamics, figured bass, hairpins, and staff text.
                        let cue_nodes = if !measure.text_cues.is_empty() {
                            chord_renderer::render_text_cues(
                                &measure.text_cues,
                                measure_x,
                                this_measure_width,
                                staff_y,
                                staff_height,
                                self.config.spatium,
                                &text_metrics,
                                &mut id_counter,
                            )
                        } else {
                            Vec::new()
                        };

                        // Staff-attached notations (dynamics, staff text, figured bass, hairpins).
                        let notation_frame = notation_renderer::MeasureFrame {
                            measure_x,
                            measure_width: this_measure_width,
                            staff_y,
                            staff_height,
                            chord_y: this_chord_y,
                            spatium: self.config.spatium,
                            beats_per_measure: time_signature.0,
                            source_measure_width: measure.source_measure_width,
                            segment_positions: Some(&segment_positions),
                            system_start_dynamic_x: (local_measure_idx == 0)
                                .then_some(content_x + clef_width * 0.5),
                            section_start_dynamic_stack: (sys_idx == 0 && local_measure_idx == 0)
                                .then_some(section_start_dynamic_stack.clone())
                                .flatten(),
                        };
                        // Per-system skyline (mirrors MuseScore Skyline + Autoplace).
                        // Seed with the actual rendered chord bounds so
                        // notations avoid chord text without treating the
                        // whole measure as occupied.
                        for bounds in chord_obstacles {
                            system_skyline.add_above(bounds);
                            system_skyline.add_below(bounds);
                        }
                        // Seed below skyline with the staff itself so below
                        // elements don't intrude into the staff lines.
                        let staff_band = kurbo::Rect::new(
                            if local_measure_idx == 0 {
                                content_x
                            } else {
                                measure_x
                            },
                            staff_y - self.config.spatium * 0.2,
                            measure_x + this_measure_width,
                            staff_y + staff_height + self.config.spatium * 0.4,
                        );
                        system_skyline.add_above(staff_band);
                        system_skyline.add_below(staff_band);
                        let autoplace_gap = self.config.spatium * 0.1;
                        for mut node in notation_renderer::render_dynamics(
                            &measure.classical_dynamics,
                            &notation_frame,
                            &mut id_counter,
                        ) {
                            node.set_page(page_number);
                            node.set_system(global_system_index as u32);
                            node.set_measure(measure_idx as u32);
                            pending_notation.push((node, false, false));
                        }
                        for (item, mut node) in
                            measure
                                .staff_text
                                .iter()
                                .zip(notation_renderer::render_staff_text(
                                    &measure.staff_text,
                                    &notation_frame,
                                    &text_metrics,
                                    &mut id_counter,
                                ))
                        {
                            let above =
                                matches!(item.placement, crate::chart::notations::Placement::Above);
                            notation_renderer::shrink_node_to_max_right(
                                &mut node,
                                measure_x + this_measure_width - autoplace_gap,
                                0.45,
                            );
                            pending_notation.push((node, above, true));
                        }
                        // Figured bass is deferred to the skyline autoplace pass
                        // like the other annotations. Its natural baseline can
                        // sit on the chord symbol (a single Above row anchors at
                        // chord_y), so it must be pushed clear of chord ink
                        // rather than painted directly on top of it.
                        for (item, node) in
                            measure
                                .figured_bass
                                .iter()
                                .zip(notation_renderer::render_figured_bass(
                                    &measure.figured_bass,
                                    &notation_frame,
                                    &mut id_counter,
                                ))
                        {
                            let above =
                                matches!(item.placement, crate::chart::notations::Placement::Above);
                            pending_notation.push((node, above, false));
                        }
                        for (item, node) in
                            measure
                                .suspensions
                                .iter()
                                .zip(notation_renderer::render_suspensions(
                                    &measure.suspensions,
                                    &notation_frame,
                                    self.config.harmony_style.root_size,
                                    &suspension_chord_bounds,
                                    &mut id_counter,
                                ))
                        {
                            let above =
                                matches!(item.placement, crate::chart::notations::Placement::Above);
                            pending_notation.push((node, above, false));
                        }
                        for (item, node) in
                            measure
                                .hairpins
                                .iter()
                                .zip(notation_renderer::render_hairpins(
                                    &measure.hairpins,
                                    &notation_frame,
                                    &mut id_counter,
                                ))
                        {
                            let above =
                                matches!(item.placement, crate::chart::notations::Placement::Above);
                            pending_notation.push((node, above, false));
                        }
                        for mut node in cue_nodes {
                            notation_renderer::shrink_node_to_max_right(
                                &mut node,
                                measure_x + this_measure_width - autoplace_gap,
                                0.45,
                            );
                            pending_notation.push((node, false, true));
                        }
                        if let Some(volta) = &measure.volta_start {
                            let volta_end_local = (local_measure_idx
                                + usize::from(volta.length_measures))
                            .min(distributed_widths.len());
                            let volta_x_end = measure_x
                                + distributed_widths[local_measure_idx..volta_end_local]
                                    .iter()
                                    .sum::<f64>();
                            let closes_in_system = local_measure_idx
                                + usize::from(volta.length_measures)
                                <= distributed_widths.len();
                            let volta_node = notation_renderer::render_volta_span(
                                volta,
                                &notation_frame,
                                volta_x_end,
                                closes_in_system,
                                &mut id_counter,
                            );
                            if let Some(volta_node) = notation_renderer::autoplace_node(
                                &mut system_skyline,
                                volta_node,
                                true,
                                autoplace_gap,
                            ) {
                                record_system_ink_bottom(
                                    &mut system_ink_bottom,
                                    &mut system_height_contributors,
                                    &volta_node,
                                    "volta",
                                );
                                root.add_child(volta_node);
                            }
                        }

                        // Start-of-measure forward repeat (draw at left edge of this measure).
                        if matches!(
                            measure.start_repeat,
                            crate::chart::notations::RepeatMark::Forward
                        ) {
                            let barline = self.draw_barline(
                                measure_x,
                                staff_y,
                                staff_height,
                                BarlineType::StartRepeat,
                            );
                            record_system_ink_bottom(
                                &mut system_ink_bottom,
                                &mut system_height_contributors,
                                &barline,
                                "start_repeat",
                            );
                            root.add_child(barline);
                        }

                        measure_x += this_measure_width;

                        // Closing barline — style + end-repeat aware.
                        let barline = self.draw_barline(
                            measure_x,
                            staff_y,
                            staff_height,
                            Self::end_barline_type(measure),
                        );
                        record_system_ink_bottom(
                            &mut system_ink_bottom,
                            &mut system_height_contributors,
                            &barline,
                            "barline",
                        );
                        root.add_child(barline);
                    }
                }

                let autoplace_gap = self.config.spatium * 0.1;
                let text_shrink_displacement = self.config.spatium * 4.0;
                for (node, above, shrinkable) in pending_notation {
                    let placed = if shrinkable {
                        notation_renderer::autoplace_text_node(
                            &mut system_skyline,
                            node,
                            above,
                            autoplace_gap,
                            text_shrink_displacement,
                        )
                    } else {
                        notation_renderer::autoplace_node(
                            &mut system_skyline,
                            node,
                            above,
                            autoplace_gap,
                        )
                    };
                    if let Some(node) = placed {
                        record_system_ink_bottom(
                            &mut system_ink_bottom,
                            &mut system_height_contributors,
                            &node,
                            "notation",
                        );
                        root.add_child(node);
                    }
                }

                // Track system layout
                let base_system_height = system_top_reserve + staff_height;
                let ink_system_height = system_ink_bottom - system_page_top;
                let actual_system_height = ink_system_height.max(base_system_height);
                let extra_system_height = (actual_system_height - base_system_height).max(0.0);
                if extra_system_height > self.config.spatium {
                    let source_measures = measure_indices
                        .iter()
                        .filter_map(|idx| chart_section.measures().get(*idx))
                        .filter_map(|measure| measure.source_measure_number)
                        .collect::<Vec<_>>();
                    debug!(
                        target: "keyflow::layout::system_spacing",
                        page = page_number,
                        system = global_system_index,
                        section = ?chart_section.section.section_type,
                        measures = ?source_measures,
                        base_height = base_system_height,
                        ink_height = ink_system_height,
                        actual_height = actual_system_height,
                        extra_height = extra_system_height,
                        contributors = ?system_height_contributors,
                        "system height expanded by rendered ink"
                    );
                }

                current_page_systems.push(SystemLayout {
                    index: global_system_index,
                    y: page_y,
                    width: content_width,
                    height: actual_system_height,
                    measure_indices: measure_indices.clone(),
                });

                page_y += actual_system_height + self.config.system_spacing;
                global_system_index += 1;
            }

            // Update global measure offset for next section (for chart_measurements lookup)
            global_section_measure_offset += chart_section.measures().len();
        }

        // Finalize last page
        if !current_page_systems.is_empty() {
            pages.push(PageLayout {
                number: page_number,
                x_offset: current_page_x,
                y_offset: page_offset_y,
                width: page_width,
                height: page_height,
                systems: current_page_systems,
                margins: self.config.margins,
            });
        }

        let total_width = page_offset_x * 2.0 + page_number as f64 * (page_width + page_gap);
        let total_height = page_height + page_offset_y * 2.0;

        // Post-process beat positions to make them contiguous (no gaps at barlines).
        // Each beat's width is extended to reach the next beat's x position,
        // ensuring smooth cursor movement across measure boundaries.
        for i in 0..beat_positions.len().saturating_sub(1) {
            let next_x = beat_positions[i + 1].x;
            let current_x = beat_positions[i].x;
            // Only extend if next beat is on same system (same y position)
            // and to the right (not a system break)
            if beat_positions[i + 1].staff_y == beat_positions[i].staff_y && next_x > current_x {
                beat_positions[i].width = next_x - current_x;
            }
        }

        let mut result = ChartLayoutResult {
            scene: root,
            pages,
            total_height,
            total_width,
            beat_positions,
        };
        self.compact_paginated_vertical_dead_space(&mut result);
        result
    }

    fn compact_paginated_vertical_dead_space(&self, result: &mut ChartLayoutResult) {
        // Compaction should preserve ink clearance, not a second system-spacing
        // reserve. Larger north/south reserves are useful during initial system
        // construction, but after all notation is rendered we can safely pack
        // systems by visible ink bounds. We still hold a baseline gap equal to
        // the configured `system_spacing` so compacted lines never sit tighter
        // than the pre-compactor layout — only genuine dead space beyond that
        // baseline is removed.
        let target_gap = self
            .config
            .system_spacing
            .max(self.config.spatium * 1.5)
            .max(8.0);

        for _ in 0..1 {
            let mut page_shift_plans: Vec<(u32, Vec<(usize, f64)>)> = Vec::new();

            for page in &result.pages {
                let system_bounds = collect_system_ink_bounds(&result.scene, page);
                let mut cumulative_shift = 0.0;
                let mut shifts = Vec::new();

                for system_idx in 1..page.systems.len() {
                    let lower_system_index = page.systems[system_idx].index;
                    let removable = system_bounds
                        .get(system_idx - 1)
                        .and_then(|upper| upper.as_ref())
                        .zip(
                            system_bounds
                                .get(system_idx)
                                .and_then(|lower| lower.as_ref()),
                        )
                        .map(|(upper, lower)| (lower.y0 - upper.y1 - target_gap).max(0.0))
                        .unwrap_or(0.0);

                    if removable > 0.5 {
                        cumulative_shift += removable;
                    }
                    shifts.push((lower_system_index, cumulative_shift));
                }

                if cumulative_shift > 0.0 {
                    page_shift_plans.push((page.number, shifts));
                }
            }

            if page_shift_plans.is_empty() {
                return;
            }

            let original_pages = result.pages.clone();

            for page in &mut result.pages {
                let Some((_, shifts)) = page_shift_plans
                    .iter()
                    .find(|(page_number, _)| *page_number == page.number)
                else {
                    continue;
                };

                for system in &mut page.systems {
                    if let Some((_, shift)) = shifts
                        .iter()
                        .rev()
                        .find(|(system_index, _)| system.index >= *system_index)
                    {
                        system.y -= shift;
                    }
                }
            }

            for beat in &mut result.beat_positions {
                let Some((_, shifts)) = page_shift_plans
                    .iter()
                    .find(|(page_number, _)| *page_number == beat.page)
                else {
                    continue;
                };
                if let Some((_, shift)) = shifts
                    .iter()
                    .rev()
                    .find(|(system_index, _)| beat.system >= *system_index)
                {
                    beat.staff_y -= shift;
                    beat.glyph_y -= shift;
                }
            }

            shift_scene_systems(&mut result.scene, &original_pages, &page_shift_plans);
        }
    }

    /// Layout chart in continuous scroll mode.
    fn layout_continuous(&self, chart: &Chart, width: f64) -> ChartLayoutResult {
        let ctx_owned = LayoutContextOwned::new_minimal(self.styled_for_config());
        let ctx = ctx_owned.as_context();
        let text_metrics = TextFontMetrics::new(self.text_font_data.clone());
        // Use text font for symbols when symbol_font_family is None (e.g., MuseJazz uses same font)
        let symbol_metrics = if self.config.harmony_style.symbol_font_family.is_none() {
            text_metrics.clone()
        } else {
            TextFontMetrics::new(self.symbol_font_data.clone())
        };

        let harmony_style = self
            .config
            .harmony_style
            .clone()
            .with_text_font_metrics(text_metrics.clone())
            .with_symbol_font_metrics(symbol_metrics.clone());

        let content_width = width - self.config.margins.left - self.config.margins.right;
        let content_x = self.config.margins.left;
        let staff_height = self.config.spatium * 4.0;

        let mut root = SceneNode::group(SemanticId::new(ElementType::Page, 0));
        let mut total_height = self.config.margins.top;
        let mut id_counter = 100u64;
        let mut global_system_index = 0usize;
        let mut global_measure_index = 0usize;

        // Title header (title / artist / tempo / key) above the music — the same
        // block the paginated path draws, but only when the chart actually names
        // itself. A bare chord snippet (`Cmaj7 | …`) stays header-free and tight;
        // a titled chart (`Vienna - Billy Joel`) shows its header in the inline
        // render too. No count-in here (continuous mode synthesizes none), so the
        // time signature passed is unused.
        if chart.metadata.title.is_some() {
            let (header_height, _) = self.add_title_header(
                &mut root,
                0.0,
                total_height,
                width,
                &chart.metadata,
                chart.tempo.as_ref(),
                0,
                (4, 4),
                false,
                Vec::new(),
            );
            total_height += header_height;
        }

        // Track previous chord to hide duplicates (reset at each system start)
        let mut previous_chord_symbol: Option<String>;

        // Beat position collection for cursor / highlight rendering / DAW transport sync.
        let mut beat_positions: Vec<BeatPosition> = Vec::new();

        // Tempo / tick bookkeeping (same conventions as paginated path):
        // 480 ticks per quarter note. Continuous mode has no count-in synthesis,
        // so the song timeline starts at t=0, tick=0.
        let tempo_bpm = chart.tempo.map(|t| t.bpm).unwrap_or(120.0);
        let seconds_per_tick = (60.0 / tempo_bpm) / 480.0;
        let mut cumulative_time = 0.0f64;
        let mut cumulative_ticks = 0i64;

        // Pre-compute section letters for consecutive repeats
        let section_letters = self.compute_section_letters(&chart.sections);

        // === PASS 1: MEASURE ===
        // Pre-measure all chord symbols to get accurate widths for layout
        let mut measurement_cache = measure_pass::MeasurementCache::new();
        let chart_measurements = measure_pass::measure_chart(
            chart
                .sections
                .iter()
                .filter(|s| {
                    !s.section.section_type.is_compact()
                        && !matches!(s.section.section_type, SectionType::End)
                })
                .map(|s| s.measures()),
            &harmony_style,
            &mut measurement_cache,
        );

        // Get time signature for beat calculations
        let time_signature = chart
            .time_signature
            .map(|ts| (ts.numerator as u8, ts.denominator as u8))
            .unwrap_or((4u8, 4u8));

        // Prevailing meter, carried across sections/systems in playback order so a
        // measure whose `time_signature` differs from its predecessor renders an
        // inline meter change. Seeded with the chart's initial meter.
        // `prev_prevailing_ts` + `ts_run_len` detect a one-measure excursion (e.g.
        // the auto-revert after `!T2/4`): that revert is drawn but NOT highlighted,
        // since the return-to-normal is adjacent and obvious. A revert that is
        // several measures after the change is highlighted like any other change.
        let mut prevailing_ts = time_signature;
        let mut prev_prevailing_ts: Option<(u8, u8)> = None;
        let mut ts_run_len: usize = 0;

        // Detect count-in from parsed chart sections if not configured explicitly.
        let _count_in_measures = if self.config.count_in_measures > 0 {
            self.config.count_in_measures as usize
        } else {
            chart
                .sections
                .iter()
                .find(|s| matches!(s.section.section_type, SectionType::CountIn))
                .map(|s| s.measures().len())
                .unwrap_or(0)
        };

        // Track offset into `chart_measurements`. Compact and End sections are
        // filtered out of the measure pass, so this offset only advances for
        // sections that were actually pre-measured.
        let mut global_section_measure_offset: usize = 0;

        // Process each section
        for (section_idx, chart_section) in chart.sections.iter().enumerate() {
            // Skip count-in sections - we synthesize count-in from config.
            if chart_section.section.section_type.is_compact() {
                continue;
            }

            // Skip End sections entirely - they are only for progress bars, not charts
            if matches!(chart_section.section.section_type, SectionType::End) {
                continue;
            }

            // Preprocess melodies to handle cross-measure spillover
            let beats_per_measure = time_signature.0 as f64;
            let key_signature = key_signature_fifths(chart);
            let mut melody_data_map = expand_melodies_across_measures(
                chart_section.measures(),
                beats_per_measure,
                key_signature,
            );
            // Stamp the chart-level clef onto each measure's melody data so
            // pitch→staff-line mapping picks the correct middle-line pitch.
            let chart_proto_clef = self.chart_proto_clef_for(chart);
            for md in melody_data_map.values_mut() {
                md.clef = chart_proto_clef;
            }

            // Detect push spillbacks (chords from next measure that push back)
            let mut push_spillback_map = detect_push_spillbacks(chart_section.measures());

            if !push_spillback_map.is_empty() {
                debug!(
                    "[spillback-detection] Section {} detected {} spillbacks: {:?}",
                    chart_section.section.section_type.full_name(),
                    push_spillback_map.values().map(|v| v.len()).sum::<usize>(),
                    push_spillback_map
                        .iter()
                        .map(|(k, v)| (
                            k,
                            v.iter()
                                .map(|s| (&s.chord_symbol, &s.push_base))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }

            // Check for cross-section spillback: if the NEXT section starts with a pushed chord,
            // it spills back to THIS section's last measure
            let next_non_compact_section = chart.sections.iter().skip(section_idx + 1).find(|s| {
                !s.section.section_type.is_compact()
                    && !matches!(s.section.section_type, SectionType::End)
            });
            if let Some(next_section) = next_non_compact_section
                && let Some(mut spillback) = detect_section_start_spillback(next_section.measures())
            {
                // Adjust beat position based on this section's last measure time signature
                if let Some(last_measure) = chart_section.measures().last() {
                    let ts = last_measure.time_signature;
                    spillback.beat_position = (ts.0 as usize).saturating_sub(1);
                }
                let last_measure_idx = chart_section.measures().len().saturating_sub(1);
                push_spillback_map
                    .entry(last_measure_idx)
                    .or_default()
                    .push(spillback);
            }

            // Mid-chart key changes in this section, keyed by section-local
            // measure index (the parser stores the change's measure ordinal in
            // total_duration.measures).
            let key_changes_in_section: std::collections::HashMap<usize, _> = chart
                .key_changes
                .iter()
                .filter(|kc| kc.section_index == section_idx)
                .map(|kc| (kc.position.total_duration.measure as usize, kc))
                .collect();

            // Group measures into systems (count-based for consistent layout)
            let systems = self.group_measures_into_systems(chart_section.measures(), content_width);

            for (sys_idx, measure_indices) in systems.iter().enumerate() {
                // Reset chord tracking at line breaks (new systems)
                // so repeated chords are always visible at the start of each line
                previous_chord_symbol = None;

                // Compute extra space for melody notes extending beyond the staff
                let mut melody_extra_above = 0.0f64;
                let mut melody_extra_below = 0.0f64;
                for &m_idx in measure_indices {
                    if let Some(md) = melody_data_map.get(&m_idx) {
                        let (above, below) = melody_note_extent(
                            md,
                            self.config.spatium,
                            self.chart_proto_clef_for(chart),
                        );
                        melody_extra_above = melody_extra_above.max(above);
                        melody_extra_below = melody_extra_below.max(below);
                    }
                }

                // Match the paginated path: reserve skyline-like north/south
                // bands so adjacent systems cannot overlap above/below staff
                // notation before a full page-level skyline pass exists.
                let system_top_reserve = self.config.spatium * 2.0;
                let system_bottom_reserve = self.config.spatium * 4.5;
                let staff_y = total_height + system_top_reserve + melody_extra_above;
                let system_height = system_top_reserve
                    + melody_extra_above
                    + staff_height
                    + melody_extra_below
                    + system_bottom_reserve;

                // Calculate system prefix width (clef + key sig + time sig) FIRST
                // Needed to determine staff line width for short systems
                let include_clef = true;
                let include_key_sig = true; // Key sig on every system (standard notation)
                let include_time_sig = global_system_index == 0;

                // Key signature prevailing at this system's downbeat (follows
                // mid-chart key changes), and whether the change happens right
                // here — if so the prefix is drawn red in place rather than
                // stacking a separate red indicator over it.
                let system_first_measure = measure_indices.first().copied().unwrap_or(0);
                let key_signature: i8 =
                    prevailing_fifths_at(chart, section_idx, system_first_measure);
                let system_starts_with_key_change = measure_indices
                    .first()
                    .is_some_and(|m| key_changes_in_section.contains_key(m));

                let (clef_width, key_sig_width, time_sig_width, prefix_width) =
                    prefix_renderer::calculate_prefix_width(
                        self.config.spatium,
                        include_clef,
                        include_key_sig,
                        key_signature,
                        include_time_sig,
                    );

                // Calculate measure width using spring-based distribution
                let measures_area_width = content_width - prefix_width;
                let num_measures = measure_indices.len();
                let max_measures = self.config.max_measures_per_system;

                // Count-in is now rendered in the header, not on the first system.
                // We no longer need to account for count-in measures in width distribution.

                // Calculate base measure width (what a normal measure would be)
                let base_measure_width = measures_area_width / max_measures as f64;

                // For width calculation, just use the regular measure count
                let effective_measure_count = num_measures as f64;
                let is_short_system = effective_measure_count < max_measures as f64;

                // Calculate content weights for spring-based distribution
                // Also check for spillbacks from the next measure (triplet pushes that affect current measure width)
                let all_measures = chart_section.measures();

                // Calculate weights and minimum widths for each measure
                // Weights come from rhythm builder (handles triplets, etc.)
                // Min widths come from pre-measured chord symbol widths (Pass 1 results)
                let (measure_weights, measure_min_widths): (Vec<f64>, Vec<f64>) = measure_indices
                    .iter()
                    .filter_map(|&idx| all_measures.get(idx).map(|m| (idx, m)))
                    .map(|(idx, m)| {
                        // Weight still uses rhythm builder (correct for triplets/complex rhythms)
                        let weight = self.estimate_measure_content_weight(m, &text_metrics);

                        // Min width from pre-measured chord widths (Pass 1)
                        let global_idx = global_section_measure_offset + idx;
                        let min_width = chart_measurements
                            .get(global_idx)
                            .map(|m| m.min_width)
                            .unwrap_or(0.0);

                        (weight, min_width)
                    })
                    .unzip();

                // Distribute width proportionally using spring physics
                let has_spacing_expansion = self.has_spacing_expander(
                    &measure_weights,
                    &measure_min_widths,
                    base_measure_width,
                );
                let total_width_to_distribute = if is_short_system && has_spacing_expansion {
                    measures_area_width
                } else if is_short_system {
                    num_measures as f64 * base_measure_width
                } else {
                    measures_area_width
                };

                let distributed_widths = self.distribute_measure_widths(
                    &measure_weights,
                    0, // No count-in measures in first system (now rendered in header)
                    total_width_to_distribute,
                    0.4, // compact_scale (not used when num_count_in=0)
                    base_measure_width,
                    &measure_min_widths,
                );
                self.log_system_width_decisions(
                    section_idx,
                    sys_idx,
                    measure_indices,
                    all_measures,
                    &measure_weights,
                    &measure_min_widths,
                    &distributed_widths,
                    total_width_to_distribute,
                    base_measure_width,
                    is_short_system,
                );

                // Calculate actual system width.
                // For short systems, use the sum of distributed_widths (not total_width_to_distribute)
                // because min_widths may force measures to expand beyond the planned width.
                let actual_system_width = if is_short_system {
                    prefix_width + distributed_widths.iter().sum::<f64>()
                } else {
                    content_width
                };

                // Draw staff lines (shortened for short systems)
                root.add_child(SceneNode::anonymous_leaf(self.draw_staff_lines(
                    content_x,
                    staff_y,
                    actual_system_width,
                )));

                // Place chord symbols above the highest note content (MuseScore skyline approach)
                let _chord_y = staff_y + constants::CHORD_Y_OFFSET - melody_extra_above;

                // Add section label for first system of section (skip for count-in)
                let mut section_start_dynamic_stack = None;
                if sys_idx == 0
                    && !chart_section.implicit
                    && chart_section.section.section_type.should_show_header()
                {
                    let letter = section_letters.get(&section_idx).copied();
                    let y_slots = self.repeat_pass_dynamic_slots(
                        &chart_section.section,
                        0.0,
                        self.config.margins.left,
                        staff_y,
                        staff_height,
                        &ctx,
                    );
                    section_start_dynamic_stack = Some(notation_renderer::DynamicStackAnchor {
                        x: self.config.margins.left * 0.5,
                        y_slots,
                        fallback_gap: self.config.spatium * 2.2,
                    });
                    let section_label = self.create_section_label(
                        &chart_section.section,
                        0.0,
                        self.config.margins.left,
                        staff_y,
                        staff_height,
                        letter,
                        &ctx,
                        id_counter,
                    );
                    id_counter += 1;
                    root.add_child(section_label);
                }

                // Render system prefix (clef, key signature, and time signature)
                let ts = chart
                    .time_signature
                    .map(|ts| (ts.numerator as u8, ts.denominator as u8))
                    .unwrap_or((4u8, 4u8));

                let prefix_ctx = prefix_renderer::PrefixRenderContext {
                    x: content_x,
                    staff_y,
                    spatium: self.config.spatium,
                    include_clef,
                    include_key_sig,
                    include_time_sig,
                    key_signature,
                    key_sig_color: system_starts_with_key_change.then(change_highlight_color),
                    time_signature: ts,
                    clef_width,
                    clef_type: self.chart_clef_for(chart),
                    key_sig_width,
                    time_sig_width,
                    page_number: None, // Continuous mode has no pages
                };

                let prefix_result =
                    prefix_renderer::render_system_prefix(&prefix_ctx, id_counter, &ctx);

                for node in prefix_result.nodes {
                    root.add_child(node);
                }
                id_counter = prefix_result.next_id;

                // Start measures after prefix
                let mut measure_x = content_x + prefix_width;
                let time_signature = chart
                    .time_signature
                    .map(|ts| (ts.numerator as u8, ts.denominator as u8))
                    .unwrap_or((4u8, 4u8));

                // Count-in is now rendered in the header (via add_title_header_with_count_in),
                // so we no longer render count-in measures inline on the first system.
                // Measures now start directly at measure 1.

                let mut system_skyline = notation_renderer::MeasureSkyline::new();
                for (local_measure_idx, &measure_idx) in measure_indices.iter().enumerate() {
                    if let Some(measure) = chart_section.measures().get(measure_idx) {
                        let melody_data = melody_data_map.get(&measure_idx);

                        // Per-measure chord-symbol Y (see paginated path for rationale).
                        let this_measure_extra_above = melody_data
                            .map(|md| {
                                melody_note_extent(
                                    md,
                                    self.config.spatium,
                                    self.chart_proto_clef_for(chart),
                                )
                                .0
                            })
                            .unwrap_or(0.0);
                        let this_chord_y =
                            staff_y + constants::CHORD_Y_OFFSET - this_measure_extra_above;

                        // Get distributed width for this measure
                        let this_measure_width = distributed_widths
                            .get(local_measure_idx)
                            .copied()
                            .unwrap_or(base_measure_width);

                        // Get spillback chords for this measure (from next measure pushing back)
                        let spillbacks = push_spillback_map.get(&measure_idx).map(|v| v.as_slice());

                        // is_boundary: first measure of section needs width for pushed chords
                        let is_section_boundary = measure_idx == 0;
                        // A measure whose meter differs from the prevailing one
                        // renders the new time signature inline. It is highlighted
                        // red unless it merely closes a one-measure excursion (the
                        // auto-revert after `!T2/4`): that return-to-normal renders
                        // in default black because it is adjacent and obvious.
                        let measure_ts = measure.time_signature;
                        let ts_changed = measure_ts != prevailing_ts;
                        let is_oneshot_revert =
                            ts_changed && ts_run_len == 1 && prev_prevailing_ts == Some(measure_ts);
                        let highlight = ts_changed && !is_oneshot_revert;
                        let measure_result = self.layout_measure(MeasureLayoutParams {
                            measure,
                            melody_data,
                            spillbacks,
                            measure_width: this_measure_width,
                            include_clef: false,
                            include_time_sig: ts_changed,
                            time_signature: measure_ts,
                            time_sig_color: highlight.then(change_highlight_color),
                            clef: self.chart_proto_clef_for(chart),
                            ctx: &ctx,
                            id_base: id_counter,
                            is_boundary: is_section_boundary,
                        });
                        id_counter += 10;
                        if ts_changed {
                            prev_prevailing_ts = Some(prevailing_ts);
                            prevailing_ts = measure_ts;
                            ts_run_len = 1;
                        } else {
                            ts_run_len += 1;
                        }

                        // Red key-change indicator when a key change lands on a
                        // measure mid-system. A change on the system's first
                        // measure is shown by the red prefix key signature
                        // instead (see `system_starts_with_key_change`), so skip
                        // it here to avoid stacking two key sigs.
                        if let Some(kc) = key_changes_in_section
                            .get(&measure_idx)
                            .filter(|_| local_measure_idx != 0)
                        {
                            let kc_node = key_change_indicator_node(
                                kc,
                                measure_x,
                                staff_y,
                                self.config.spatium,
                                &ctx,
                                id_counter,
                            );
                            id_counter += 1;
                            root.add_child(kc_node);
                        }

                        let segment_positions: Vec<f64> =
                            self.get_chord_rest_positions(&measure_result);

                        let mut measure_container =
                            SceneNode::group(SemanticId::new(ElementType::Measure, id_counter));
                        id_counter += 1;
                        measure_container.transform =
                            Affine::translate((measure_x, staff_y + 2.0 * self.config.spatium));
                        notation_renderer::add_scene_obstacles(
                            &mut system_skyline,
                            &measure_result.scene,
                            measure_container.transform,
                            true,
                            true,
                        );
                        measure_container.add_child(measure_result.scene);
                        root.add_child(measure_container);

                        // Add measure number on first measure of each system.
                        // Prefer the import-time source number (e.g. MusicXML
                        // `<measure number="…">`) so count-in numbering carries
                        // through — LotF's first musical bar is xml measure 3.
                        let display_measure_num = measure
                            .source_measure_number
                            .map(|n| n as i32)
                            .unwrap_or((global_measure_index as i32) + 1);
                        let is_first_of_system = local_measure_idx == 0;
                        if self.config.show_measure_numbers && is_first_of_system {
                            let measure_num_node = self.create_measure_number(
                                display_measure_num,
                                content_x,
                                staff_y,
                                id_counter,
                            );
                            id_counter += 1;
                            root.add_child(measure_num_node);
                        }

                        // Collect beat positions from actual segment data (mirrors paginated path).
                        // Required for DAW transport sync and cursor highlighting in scroll mode.
                        let measure_time_start = cumulative_time;
                        let measure_tick_start = cumulative_ticks;
                        for (beat_idx, segment) in measure_result
                            .segments
                            .iter_type(SegmentType::CHORD_REST)
                            .enumerate()
                        {
                            let time_start =
                                measure_time_start + (segment.tick as f64 * seconds_per_tick);
                            let time_end = measure_time_start
                                + ((segment.tick + segment.ticks) as f64 * seconds_per_tick);
                            let absolute_tick = measure_tick_start + segment.tick as i64;
                            let has_stem = segment.ticks < 960;
                            let flag_count = match segment.ticks {
                                t if t >= 480 => 0,
                                t if t >= 240 => 1,
                                t if t >= 120 => 2,
                                t if t >= 60 => 3,
                                _ => 4,
                            };
                            beat_positions.push(BeatPosition {
                                page: 1, // continuous mode has no real pages
                                system: global_system_index,
                                measure: global_measure_index,
                                beat: beat_idx,
                                tick: segment.tick,
                                duration_ticks: segment.ticks,
                                absolute_tick,
                                x: measure_x + segment.x,
                                width: segment.width,
                                staff_y,
                                staff_height,
                                time_start,
                                time_end,
                                glyph_codepoint: Some(slash_glyph_for_ticks(segment.ticks)),
                                glyph_size: self.config.spatium,
                                glyph_y: staff_y + staff_height / 2.0,
                                has_stem,
                                stem_up: true,
                                flag_count,
                                time_signature,
                            });
                        }

                        // Advance song-time cursor by the measure's full duration.
                        let measure_duration_ticks =
                            time_signature.0 as i32 * (1920 / time_signature.1 as i32);
                        cumulative_time += measure_duration_ticks as f64 * seconds_per_tick;
                        cumulative_ticks += measure_duration_ticks as i64;

                        // Render chord symbols using chord_renderer module
                        // Look up pre-computed measurements for this measure
                        let global_idx = global_section_measure_offset + measure_idx;
                        let measure_measurements = chart_measurements.get(global_idx);

                        let chord_ctx = chord_renderer::ChordRenderContext {
                            measure_x,
                            measure_width: this_measure_width,
                            chord_y: this_chord_y,
                            page_number: None, // Continuous mode has no pages
                            global_system_index,
                            measure_idx,
                            local_measure_idx,
                            section_name: &chart_section.section.section_type.full_name(),
                            segment_positions: &segment_positions,
                            internal_push_positions: &measure_result.internal_push_positions,
                            harmony_style: &harmony_style,
                            time_signature,
                            hide_repeated_chords: self.config.hide_repeated_chords,
                            min_chord_symbol_gap: self.config.min_chord_symbol_gap,
                            push_alters_rhythm: self.config.push_alters_rhythm,
                            spatium: self.config.spatium,
                            measure_measurements,
                            spillback_positions: &measure_result.spillback_positions,
                            note_line_stacks: &measure_result.note_line_stacks,
                        };

                        let chord_result = chord_renderer::render_chord_symbols(
                            &chord_ctx,
                            measure,
                            previous_chord_symbol.as_deref(),
                            id_counter,
                            &ctx,
                        );
                        let chord_obstacles = chord_result.chord_bounds;
                        // Kept for anchoring suspension figures to their chords;
                        // `chord_obstacles` itself is consumed as obstacles below.
                        let suspension_chord_bounds = chord_obstacles.clone();

                        for node in &chord_result.nodes {
                            notation_renderer::add_scene_obstacles(
                                &mut system_skyline,
                                node,
                                Affine::IDENTITY,
                                true,
                                true,
                            );
                        }

                        for node in chord_result.nodes {
                            root.add_child(node);
                        }
                        previous_chord_symbol = chord_result.last_chord_symbol;
                        id_counter = chord_result.next_id;

                        // Render spillback chord symbols (from next measure pushing back)
                        if let Some(spillback_list) = spillbacks {
                            let spillback_result = chord_renderer::render_spillback_chords(
                                &chord_ctx,
                                spillback_list,
                                previous_chord_symbol.as_deref(),
                                id_counter,
                                &ctx,
                            );

                            for node in &spillback_result.nodes {
                                notation_renderer::add_scene_obstacles(
                                    &mut system_skyline,
                                    node,
                                    Affine::IDENTITY,
                                    true,
                                    true,
                                );
                            }

                            for node in spillback_result.nodes {
                                root.add_child(node);
                            }
                            previous_chord_symbol = spillback_result.last_chord_symbol;
                            id_counter = spillback_result.next_id;
                        }

                        // Render text cues below the staff. They are routed
                        // through the notation skyline below so they avoid
                        // dynamics, figured bass, hairpins, and staff text.
                        let cue_nodes = if !measure.text_cues.is_empty() {
                            chord_renderer::render_text_cues(
                                &measure.text_cues,
                                measure_x,
                                this_measure_width,
                                staff_y,
                                staff_height,
                                self.config.spatium,
                                &text_metrics,
                                &mut id_counter,
                            )
                        } else {
                            Vec::new()
                        };

                        // Staff-attached notations (dynamics, staff text, figured bass, hairpins).
                        let notation_frame = notation_renderer::MeasureFrame {
                            measure_x,
                            measure_width: this_measure_width,
                            staff_y,
                            staff_height,
                            chord_y: this_chord_y,
                            spatium: self.config.spatium,
                            beats_per_measure: time_signature.0,
                            source_measure_width: measure.source_measure_width,
                            segment_positions: Some(&segment_positions),
                            system_start_dynamic_x: (local_measure_idx == 0)
                                .then_some(content_x + clef_width * 0.5),
                            section_start_dynamic_stack: (sys_idx == 0 && local_measure_idx == 0)
                                .then_some(section_start_dynamic_stack.clone())
                                .flatten(),
                        };
                        // Per-system skyline (mirrors MuseScore Skyline + Autoplace).
                        // Seed with the actual rendered chord bounds so
                        // notations avoid chord text without treating the
                        // whole measure as occupied.
                        for bounds in chord_obstacles {
                            system_skyline.add_above(bounds);
                            system_skyline.add_below(bounds);
                        }
                        // Seed below skyline with the staff itself so below
                        // elements don't intrude into the staff lines.
                        let staff_band = kurbo::Rect::new(
                            if local_measure_idx == 0 {
                                content_x
                            } else {
                                measure_x
                            },
                            staff_y - self.config.spatium * 0.2,
                            measure_x + this_measure_width,
                            staff_y + staff_height + self.config.spatium * 0.4,
                        );
                        system_skyline.add_above(staff_band);
                        system_skyline.add_below(staff_band);
                        let autoplace_gap = self.config.spatium * 0.1;
                        let text_shrink_displacement = self.config.spatium * 4.0;
                        // Collect (node, is_above) then drain into root after
                        // skyline runs so closures don't fight over &mut root.
                        let mut placed: Vec<SceneNode> = Vec::new();
                        for node in notation_renderer::render_dynamics(
                            &measure.classical_dynamics,
                            &notation_frame,
                            &mut id_counter,
                        ) {
                            if let Some(n) = notation_renderer::autoplace_node(
                                &mut system_skyline,
                                node,
                                false,
                                autoplace_gap,
                            ) {
                                placed.push(n);
                            }
                        }
                        for (item, mut node) in
                            measure
                                .staff_text
                                .iter()
                                .zip(notation_renderer::render_staff_text(
                                    &measure.staff_text,
                                    &notation_frame,
                                    &text_metrics,
                                    &mut id_counter,
                                ))
                        {
                            let above =
                                matches!(item.placement, crate::chart::notations::Placement::Above);
                            notation_renderer::shrink_node_to_max_right(
                                &mut node,
                                measure_x + this_measure_width - autoplace_gap,
                                0.45,
                            );
                            if let Some(n) = notation_renderer::autoplace_text_node(
                                &mut system_skyline,
                                node,
                                above,
                                autoplace_gap,
                                text_shrink_displacement,
                            ) {
                                placed.push(n);
                            }
                        }
                        for (item, node) in
                            measure
                                .figured_bass
                                .iter()
                                .zip(notation_renderer::render_figured_bass(
                                    &measure.figured_bass,
                                    &notation_frame,
                                    &mut id_counter,
                                ))
                        {
                            let _ = item;
                            if let Some(bounds) =
                                notation_renderer::scene_ink_bounds(&node, Affine::IDENTITY)
                            {
                                system_skyline.add_above(bounds);
                                system_skyline.add_below(bounds);
                            }
                            placed.push(node);
                        }
                        for (item, node) in
                            measure
                                .suspensions
                                .iter()
                                .zip(notation_renderer::render_suspensions(
                                    &measure.suspensions,
                                    &notation_frame,
                                    self.config.harmony_style.root_size,
                                    &suspension_chord_bounds,
                                    &mut id_counter,
                                ))
                        {
                            let _ = item;
                            if let Some(bounds) =
                                notation_renderer::scene_ink_bounds(&node, Affine::IDENTITY)
                            {
                                system_skyline.add_above(bounds);
                                system_skyline.add_below(bounds);
                            }
                            placed.push(node);
                        }
                        for (item, node) in
                            measure
                                .hairpins
                                .iter()
                                .zip(notation_renderer::render_hairpins(
                                    &measure.hairpins,
                                    &notation_frame,
                                    &mut id_counter,
                                ))
                        {
                            let above =
                                matches!(item.placement, crate::chart::notations::Placement::Above);
                            if let Some(n) = notation_renderer::autoplace_node(
                                &mut system_skyline,
                                node,
                                above,
                                autoplace_gap,
                            ) {
                                placed.push(n);
                            }
                        }
                        for mut node in cue_nodes {
                            notation_renderer::shrink_node_to_max_right(
                                &mut node,
                                measure_x + this_measure_width - autoplace_gap,
                                0.45,
                            );
                            if let Some(n) = notation_renderer::autoplace_text_node(
                                &mut system_skyline,
                                node,
                                false,
                                autoplace_gap,
                                text_shrink_displacement,
                            ) {
                                placed.push(n);
                            }
                        }
                        for n in placed {
                            root.add_child(n);
                        }
                        if let Some(volta) = &measure.volta_start {
                            let volta_end_local = (local_measure_idx
                                + usize::from(volta.length_measures))
                            .min(distributed_widths.len());
                            let volta_x_end = measure_x
                                + distributed_widths[local_measure_idx..volta_end_local]
                                    .iter()
                                    .sum::<f64>();
                            let closes_in_system = local_measure_idx
                                + usize::from(volta.length_measures)
                                <= distributed_widths.len();
                            let volta_node = notation_renderer::render_volta_span(
                                volta,
                                &notation_frame,
                                volta_x_end,
                                closes_in_system,
                                &mut id_counter,
                            );
                            if let Some(volta_node) = notation_renderer::autoplace_node(
                                &mut system_skyline,
                                volta_node,
                                true,
                                autoplace_gap,
                            ) {
                                root.add_child(volta_node);
                            }
                        }

                        // Start-of-measure forward repeat (left edge).
                        if matches!(
                            measure.start_repeat,
                            crate::chart::notations::RepeatMark::Forward
                        ) {
                            root.add_child(self.draw_barline(
                                measure_x,
                                staff_y,
                                staff_height,
                                BarlineType::StartRepeat,
                            ));
                        }

                        measure_x += this_measure_width;

                        root.add_child(self.draw_barline(
                            measure_x,
                            staff_y,
                            staff_height,
                            Self::end_barline_type(measure),
                        ));

                        global_measure_index += 1;
                    }
                }

                total_height += system_height + self.config.system_spacing;
                global_system_index += 1;
            }

            // Update global measure offset for next section (for chart_measurements lookup)
            global_section_measure_offset += chart_section.measures().len();
        }

        total_height += self.config.margins.bottom;

        ChartLayoutResult {
            scene: root,
            pages: Vec::new(),
            total_height,
            total_width: width,
            beat_positions,
        }
    }

    /// Group measures into systems based on maximum measures per system.
    ///
    /// Always uses count-based grouping to maintain consistent layout.
    /// Rhythm compression handles fitting content within the allocated width.
    fn group_measures_into_systems(
        &self,
        measures: &[crate::chart::types::Measure],
        _content_width: f64,
    ) -> Vec<Vec<usize>> {
        let mut systems = Vec::new();
        let mut current_system = Vec::new();

        for (idx, measure) in measures.iter().enumerate() {
            if starts_long_volta(measure) && !current_system.is_empty() {
                systems.push(std::mem::take(&mut current_system));
            }

            current_system.push(idx);
            if current_system.len() >= self.config.max_measures_per_system {
                systems.push(std::mem::take(&mut current_system));
            }
        }

        if !current_system.is_empty() {
            systems.push(current_system);
        }

        systems
    }
}

fn starts_long_volta(measure: &crate::chart::types::Measure) -> bool {
    measure
        .volta_start
        .as_ref()
        .is_some_and(|volta| volta.length_measures > 1)
}

// region:    --- Test Utilities

#[cfg(test)]
pub mod test_utils;

#[cfg(test)]
mod tests;

// endregion: --- Test Utilities

/// Detect whether the chart's *first* chord (skipping count-in / compact
/// sections) carries a push (anticipation) flag. Used to render the count-in
/// snippet's beat-4 indicator when the song starts on a pushed chord.
fn chart_first_chord_is_pushed(chart: &crate::chart::Chart) -> bool {
    use crate::sections::SectionType;
    chart
        .sections
        .iter()
        .find(|s| {
            !s.section.section_type.is_compact()
                && !matches!(s.section.section_type, SectionType::End)
        })
        .and_then(|s| s.measures().iter().find(|m| !m.chords.is_empty()))
        .and_then(|m| m.chords.first())
        .and_then(|c| c.push_pull)
        .map(|(is_push, _)| is_push)
        .unwrap_or(false)
}
