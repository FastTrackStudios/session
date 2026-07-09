//! Renderers for the staff-attached notation elements added in `chart::notations`.
//!
//! ## Skyline-based collision pass
//!
//! Models MuseScore's `Skyline` / `Autoplace` from
//! `engraving/infrastructure/skyline.{h,cpp}` and
//! `rendering/score/autoplace.cpp`. Each measure tracks two skyline lines:
//! - `north` (above-staff): the bottom edge of every placed element
//! - `south` (below-staff): the top edge of every placed element
//!
//! Adding an element: query `min_distance_to_shape_*(rect)` for the highest
//! (above) or lowest (below) y the new bbox can sit at without overlapping
//! an existing one in the same x-range. Shift the SceneNode's transform by
//! the delta and record the new bbox.
//!
//! These follow MuseScore's element-per-segment convention from `engraving/dom/`:
//! - `Dynamic` → single SMuFL glyph below the staff (mirrors `dynamic.cpp`)
//! - `StaffText` → text with optional rect frame (`stafftext.cpp` / `TextBase`)
//! - `FiguredBass` → stacked numerals above/below staff (`figuredbass.cpp`)
//! - `Hairpin` → wedge of two lines below the staff (`hairpin.cpp`)
//!
//! Volta + barline-repeat rendering live with the barline pass since they
//! integrate with system/measure layout, not with per-segment paint.

use kurbo::{Affine, Point, Rect};
use peniko::Color;

use crate::chart::notations::{
    Dynamic, DynamicLevel, FiguredBass, Hairpin, HairpinKind, Placement, StaffText,
    SuspensionFigure, Volta,
};
use crate::engraver::layout::shape::{Shape, ShapeElement};
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

const DYNAMIC_COLOR: Color = Color::from_rgb8(220, 38, 38);
const STAFF_TEXT_COLOR: Color = Color::from_rgb8(220, 38, 38);
const DYNAMIC_GLYPH_SIZE_SP: f64 = 3.05;
const STAFF_TEXT_FONT_SIZE_SP: f64 = 2.5;

#[derive(Clone)]
pub struct DynamicStackAnchor {
    pub x: f64,
    pub y_slots: Vec<f64>,
    pub fallback_gap: f64,
}

/// Layout inputs shared by every per-measure notation renderer.
#[derive(Clone)]
pub struct MeasureFrame<'a> {
    pub measure_x: f64,
    pub measure_width: f64,
    pub staff_y: f64,
    pub staff_height: f64,
    /// Baseline used by chord symbols in this measure.
    pub chord_y: f64,
    pub spatium: f64,
    pub beats_per_measure: u8,
    /// Source MusicXML measure width, in tenths, when available.
    pub source_measure_width: Option<f64>,
    /// Per-beat segment x-positions (measure-local, in spatiums) emitted by
    /// the chord-rest layout pass. When present, beat_x() snaps to the
    /// closest segment so figured-bass numerals / dynamics line up under
    /// their chord symbol instead of a uniform grid approximation.
    pub segment_positions: Option<&'a [f64]>,
    /// Optional x-position for beat-1 dynamics at the start of a system.
    /// This keeps line-opening dynamics in the prefix column under the clef
    /// instead of pushing them into the first measure's rhythmic content.
    pub system_start_dynamic_x: Option<f64>,
    /// Optional margin anchor for stacked dynamics at the start of a repeated
    /// section. This keeps pass-specific dynamics visually attached to stacked
    /// section cards such as `VS 1b` / `VS 2`.
    pub section_start_dynamic_stack: Option<DynamicStackAnchor>,
}

impl<'a> MeasureFrame<'a> {
    /// X position of a 1-based beat. Uses real chord-segment positions when
    /// available, otherwise falls back to an even grid.
    fn beat_x(&self, beat: u8) -> f64 {
        if beat <= 1
            && let Some(x) = self.system_start_dynamic_x
        {
            return x;
        }

        let beat_idx = beat.saturating_sub(1) as usize;
        if let Some(segments) = self.segment_positions
            && !segments.is_empty()
        {
            let idx = beat_idx.min(segments.len() - 1);
            return self.measure_x + segments[idx] * self.spatium;
        }
        let beats = self.beats_per_measure.max(1) as f64;
        let beat_clamped = (beat.max(1) as f64 - 1.0).min(beats - 1.0);
        let usable = (self.measure_width - self.spatium).max(0.0);
        self.measure_x + self.spatium * 0.5 + (beat_clamped / beats) * usable
    }

    fn source_x(&self, source_default_x: f64) -> Option<f64> {
        let width = self.source_measure_width?;
        if !width.is_finite() || width <= 0.0 {
            return None;
        }
        let ratio = (source_default_x / width).clamp(0.0, 1.0);
        Some(self.measure_x + ratio * self.measure_width)
    }

    fn source_dx(&self, source_relative_x: f64) -> Option<f64> {
        let width = self.source_measure_width?;
        if !width.is_finite() || width <= 0.0 {
            return None;
        }
        Some(source_relative_x / width * self.measure_width)
    }

    fn staff_top(&self) -> f64 {
        self.staff_y
    }

    fn staff_bottom(&self) -> f64 {
        self.staff_y + self.staff_height
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Dynamics
// ────────────────────────────────────────────────────────────────────────────

/// Render classical dynamic markings. One SMuFL glyph per [`Dynamic`], placed
/// below the staff (or above if `placement = Above`) at the beat's x.
pub fn render_dynamics(
    dynamics: &[Dynamic],
    frame: &MeasureFrame<'_>,
    id_counter: &mut u64,
) -> Vec<SceneNode> {
    let mut nodes = Vec::with_capacity(dynamics.len());
    let glyph_size = frame.spatium * DYNAMIC_GLYPH_SIZE_SP;
    // Start close to the staff and let skyline autoplace move only when
    // visible ink actually collides.
    let below_y = frame.staff_bottom() + frame.spatium * 2.0;
    let above_y = frame.staff_top() - frame.spatium * 1.5;

    let stack_anchor = (dynamics.len() > 1)
        .then_some(frame.section_start_dynamic_stack.as_ref())
        .flatten();

    for (dynamic_idx, dyn_) in dynamics.iter().enumerate() {
        let y = match dyn_.placement {
            Placement::Above => above_y,
            Placement::Below => below_y,
        };
        let (x, y) = if dyn_.beat == 1 {
            if let Some(anchor) = stack_anchor {
                let y = anchor.y_slots.get(dynamic_idx).copied().unwrap_or_else(|| {
                    let last = anchor.y_slots.last().copied().unwrap_or(y);
                    let overflow_idx =
                        dynamic_idx.saturating_sub(anchor.y_slots.len().saturating_sub(1));
                    last + anchor.fallback_gap * overflow_idx as f64
                });
                (anchor.x, y)
            } else {
                (frame.beat_x(dyn_.beat), y)
            }
        } else {
            (frame.beat_x(dyn_.beat), y)
        };
        let mut paints = Vec::with_capacity(1);
        paints.push(dynamic_glyph_command(dyn_.level, x, y, glyph_size));

        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::Articulation, *id_counter),
            paints,
        );
        node.set_element_type("dynamic");
        node.metadata
            .insert("dynamic_level".to_string(), dyn_.level.as_str().to_string());
        node.metadata
            .insert("dynamic_beat".to_string(), dyn_.beat.to_string());
        if let Some(anchor) = stack_anchor {
            node.metadata
                .insert("section_start_dynamic".to_string(), "true".to_string());
            node.metadata.insert(
                "section_start_dynamic_stack_count".to_string(),
                anchor.y_slots.len().to_string(),
            );
        }
        *id_counter += 1;
        nodes.push(node);
    }

    nodes
}

fn dynamic_glyph_command(level: DynamicLevel, x: f64, y: f64, size: f64) -> PaintCommand {
    // SMuFL dynamic glyphs live in the music font (Leland) — use the same
    // named font registered by ChartFontBundle so they render in italic bold
    // matching the rest of the music notation.
    PaintCommand::text(
        level.smufl_glyph(),
        "Leland",
        size,
        Point::new(x, y),
        DYNAMIC_COLOR,
    )
}

// ────────────────────────────────────────────────────────────────────────────
// Staff text (free-form directions)
// ────────────────────────────────────────────────────────────────────────────

/// Render free-form staff text. Optional rect frame matches MusicXML
/// `enclosure="rectangle"` and MuseScore's `frameType=Square` (`TextBase`).
pub fn render_staff_text(
    items: &[StaffText],
    frame: &MeasureFrame<'_>,
    text_metrics: &crate::engraver::layout::text_metrics::TextFontMetrics,
    id_counter: &mut u64,
) -> Vec<SceneNode> {
    let mut nodes = Vec::with_capacity(items.len());
    let font_size = frame.spatium * STAFF_TEXT_FONT_SIZE_SP;
    let pad_y = frame.spatium * 0.4;
    let pad_x = frame.spatium * 0.5;

    let below_baseline = frame.staff_bottom() + frame.spatium * 3.2;
    let above_baseline = frame.staff_top() - frame.spatium * 2.8;

    for item in items {
        let baseline_y = match item.placement {
            Placement::Above => above_baseline,
            Placement::Below => below_baseline,
        };
        let x = item
            .source_default_x
            .and_then(|source_x| frame.source_x(source_x))
            .unwrap_or_else(|| frame.beat_x(item.beat));

        let mut paints = Vec::with_capacity(2);

        if item.boxed {
            // Frame around the text — measure text first.
            let text_width = text_metrics.horizontal_advance(&item.text, font_size);
            let rect = kurbo::Rect::new(
                x - pad_x,
                baseline_y - font_size + pad_y * 0.5,
                x + text_width + pad_x,
                baseline_y + pad_y,
            );
            paints.push(PaintCommand::stroked_rect(
                rect,
                Color::BLACK,
                frame.spatium * 0.12,
            ));
        }

        paints.push(PaintCommand::text(
            &item.text,
            "Leland Text",
            font_size,
            Point::new(x, baseline_y),
            STAFF_TEXT_COLOR,
        ));

        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::Articulation, *id_counter),
            paints,
        );
        node.set_element_type("staff_text");
        *id_counter += 1;
        nodes.push(node);
    }

    nodes
}

// ────────────────────────────────────────────────────────────────────────────
// Figured bass
// ────────────────────────────────────────────────────────────────────────────

/// Render stacked figured-bass numerals. Each row stacks tightly below the
/// previous, matching MuseScore's `figuredbass.cpp` row layout (vertical
/// step ≈ 1 line-height with no extra leading).
pub fn render_figured_bass(
    items: &[FiguredBass],
    frame: &MeasureFrame<'_>,
    id_counter: &mut u64,
) -> Vec<SceneNode> {
    let mut nodes = Vec::with_capacity(items.len());
    let font_size = frame.spatium * 1.45;
    let line_step = font_size * 0.95;

    let above_baseline = frame.chord_y - font_size * 0.95;
    let below_baseline = frame.staff_bottom() + frame.spatium * 4.2;

    for item in items {
        let anchor_x = item
            .source_default_x
            .and_then(|source_x| frame.source_x(source_x))
            .unwrap_or_else(|| frame.beat_x(item.beat));
        let x = anchor_x
            + item
                .source_relative_x
                .and_then(|source_x| frame.source_dx(source_x))
                .unwrap_or(0.0);
        // For Above, the first row sits highest and successive rows go *down*.
        // For Below, the first row sits closest to the staff and rows go down.
        let top_y = match item.placement {
            Placement::Above if item.rows.len() == 1 => frame.chord_y,
            Placement::Above => above_baseline - line_step * (item.rows.len() - 1) as f64,
            Placement::Below => below_baseline,
        };

        let mut paints = Vec::with_capacity(item.rows.len() * 2);
        for (row_idx, row) in item.rows.iter().enumerate() {
            let row_y = top_y + line_step * row_idx as f64;
            let mut row_x = x;
            if !row.accidental.is_empty() {
                paints.push(PaintCommand::text(
                    &row.accidental,
                    "Leland",
                    font_size,
                    Point::new(row_x, row_y),
                    Color::BLACK,
                ));
                row_x += font_size * 0.4;
            }
            if !row.text.is_empty() {
                paints.push(PaintCommand::text(
                    &row.text,
                    "MuseJazz Text",
                    font_size,
                    Point::new(row_x, row_y),
                    Color::BLACK,
                ));
            }
        }

        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::Articulation, *id_counter),
            paints,
        );
        node.set_element_type("figured_bass");
        *id_counter += 1;
        nodes.push(node);
    }

    nodes
}

/// Render suspension figures (`4-3`, `2-3`, `3`) in the plain text font
/// (`FreeSans` — the chord font Leland Text has no digit glyphs).
///
/// Two layouts depending on `SuspensionFigure::standalone`:
/// - attached (`Eb2`, `F4-3`): a superscript at 80% of the chord root size,
///   hugging the upper-right of its chord symbol (matched via `chord_bounds`),
///   so it reads as `Eb²` / `F⁴⁻³`.
/// - standalone (`Bb // 4-3`, `F 4-3 ///`): its own chord-row symbol at full
///   chord size, placed at the figure's beat column — the held chord sounds
///   underneath without printing its name.
///
/// One scene node per figure, matching the order of `items` so callers can zip.
pub fn render_suspensions(
    items: &[SuspensionFigure],
    frame: &MeasureFrame<'_>,
    chord_root_size: f64,
    chord_bounds: &[Rect],
    id_counter: &mut u64,
) -> Vec<SceneNode> {
    let mut nodes = Vec::with_capacity(items.len());
    let gap = frame.spatium * 0.25;
    let below_y = frame.staff_bottom() + frame.spatium * 4.2;

    for item in items {
        let beat_x = frame.beat_x(item.beat);
        let (font_size, x, baseline) = if item.standalone {
            // Own chord-row symbol at the beat column, full chord size. Use a
            // measure-local grid fraction rather than beat_x(): in slash-filled
            // measures the segment table can place a late beat far outside the
            // bar, but a figure over "beat 3" just wants 3/N across the measure.
            let beats = f64::from(frame.beats_per_measure.max(1));
            let frac = ((f64::from(item.beat.max(1)) - 1.0) / beats).clamp(0.0, 1.0);
            let gx = frame.measure_x + frame.spatium * 0.5 + frac * frame.measure_width;
            let y = match item.placement {
                Placement::Above => frame.chord_y,
                Placement::Below => below_y,
            };
            (chord_root_size * 0.78, gx, y)
        } else {
            // Attached superscript hugging the chord whose left edge sits
            // closest to the figure's beat.
            let size = chord_root_size * 0.66;
            let chord = chord_bounds.iter().min_by(|a, b| {
                (a.x0 - beat_x)
                    .abs()
                    .partial_cmp(&(b.x0 - beat_x).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            match (chord, item.placement) {
                (Some(b), Placement::Above) => (size, b.x1 + gap, b.y0 + size),
                (Some(b), Placement::Below) => (size, b.x1 + gap, below_y),
                (None, Placement::Above) => (size, beat_x, frame.chord_y - size * 0.36),
                (None, Placement::Below) => (size, beat_x, below_y),
            }
        };
        let paints = vec![PaintCommand::text(
            &item.figure,
            "FreeSans",
            font_size,
            Point::new(x, baseline),
            Color::BLACK,
        )];
        // Oblique the figure by shearing geometrically rather than relying on
        // `font-style: italic` — FreeSans has no italic face, so resvg (PNG)
        // synthesizes a slant but svg2pdf (PDF) leaves it upright, making the
        // two exports disagree. A baseline-anchored x-shear slants identically
        // in both. shear ≈ tan(11°).
        let shear = 0.2_f64;
        let skew = Affine::new([1.0, 0.0, -shear, 1.0, shear * baseline, 0.0]);
        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::Articulation, *id_counter),
            paints,
        )
        .with_transform(skew);
        node.set_element_type("suspension");
        *id_counter += 1;
        nodes.push(node);
    }

    nodes
}

// ────────────────────────────────────────────────────────────────────────────
// Volta (1st / 2nd ending bracket)
// ────────────────────────────────────────────────────────────────────────────

/// Render a volta bracket starting at this measure. Renders only the leading
/// segment — the bracket within the measure it begins in. Multi-measure
/// voltas need spanner stitching across systems (MuseScore's `Volta` runs
/// through `SLine` → `VoltaSegment`); a follow-up pass after system layout
/// will extend the bracket across subsequent measures.
pub fn render_volta(volta: &Volta, frame: &MeasureFrame, id_counter: &mut u64) -> SceneNode {
    render_volta_span(
        volta,
        frame,
        frame.measure_x + frame.measure_width,
        volta.length_measures <= 1,
        id_counter,
    )
}

pub fn render_volta_span(
    volta: &Volta,
    frame: &MeasureFrame,
    x_end: f64,
    closes_in_system: bool,
    id_counter: &mut u64,
) -> SceneNode {
    let stroke = frame.spatium * 0.16;
    // MuseScore Sid::voltaPosAbove ≈ -3 sp from staff top, hook = 1.9 sp.
    let bracket_y = frame.staff_top() - frame.spatium * 12.0;
    let hook_y = bracket_y + frame.spatium * 1.5;
    let x_start = frame.measure_x;

    let mut paints = Vec::with_capacity(4);
    // Left hook (down).
    paints.push(PaintCommand::line(
        Point::new(x_start, bracket_y),
        Point::new(x_start, hook_y),
        Color::BLACK,
        stroke,
    ));
    // Horizontal top.
    paints.push(PaintCommand::line(
        Point::new(x_start, bracket_y),
        Point::new(x_end, bracket_y),
        Color::BLACK,
        stroke,
    ));

    // Right hook only if this volta finishes within this system.
    if closes_in_system {
        paints.push(PaintCommand::line(
            Point::new(x_end, bracket_y),
            Point::new(x_end, hook_y),
            Color::BLACK,
            stroke,
        ));
    }

    // Label text ("1.", "2.", or user-supplied).
    let label = if volta.label.is_empty() {
        let mut s = String::new();
        for (i, n) in volta.numbers.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&n.to_string());
        }
        if !s.is_empty() {
            s.push('.');
        }
        s
    } else {
        volta.label.clone()
    };
    if !label.is_empty() {
        let label_size = frame.spatium * 1.8;
        paints.push(PaintCommand::text(
            label,
            "Leland Text",
            label_size,
            Point::new(x_start + frame.spatium * 0.4, bracket_y + label_size * 0.9),
            Color::BLACK,
        ));
    }

    let mut node = SceneNode::leaf(
        SemanticId::new(ElementType::Articulation, *id_counter),
        paints,
    );
    node.set_element_type("volta");
    *id_counter += 1;
    node
}

// ────────────────────────────────────────────────────────────────────────────
// Hairpins (single-measure only for now)
// ────────────────────────────────────────────────────────────────────────────

/// Render a crescendo / decrescendo hairpin confined to a single measure.
///
/// Cross-measure hairpins need spanner machinery (system-level path layout in
/// MuseScore's `hairpin.cpp` / `HairpinSegment`); those are skipped here and
/// will be handled by a follow-up that runs after measure layout completes.
pub fn render_hairpins(
    hairpins: &[Hairpin],
    frame: &MeasureFrame<'_>,
    id_counter: &mut u64,
) -> Vec<SceneNode> {
    let mut nodes = Vec::with_capacity(hairpins.len());
    // Wedge opening = spatium (MuseScore: `Sid::hairpinHeight = 1.2sp`).
    let opening = frame.spatium * 1.2;
    let stroke = frame.spatium * 0.12;
    let baseline_y = frame.staff_bottom() + frame.spatium * 4.0;

    for hp in hairpins {
        // Single-measure only — skip multi-measure spanners.
        if hp.end_measure_offset > 0 {
            continue;
        }
        let placement_y = match hp.placement {
            Placement::Above => frame.staff_top() - frame.spatium * 2.5,
            Placement::Below => baseline_y,
        };
        let x_start = frame.beat_x(hp.start_beat);
        let x_end = frame.beat_x(hp.end_beat).max(x_start + frame.spatium);

        let (left_open, right_open) = match hp.kind {
            // Crescendo: < — closed at start, opens to the right.
            HairpinKind::Crescendo => (0.0, opening),
            // Decrescendo: > — open at start, closes to the right.
            HairpinKind::Decrescendo => (opening, 0.0),
        };

        let upper_start = Point::new(x_start, placement_y - left_open / 2.0);
        let lower_start = Point::new(x_start, placement_y + left_open / 2.0);
        let upper_end = Point::new(x_end, placement_y - right_open / 2.0);
        let lower_end = Point::new(x_end, placement_y + right_open / 2.0);

        let paints = vec![
            PaintCommand::line(upper_start, upper_end, Color::BLACK, stroke),
            PaintCommand::line(lower_start, lower_end, Color::BLACK, stroke),
        ];

        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::Articulation, *id_counter),
            paints,
        );
        node.set_element_type("hairpin");
        *id_counter += 1;
        nodes.push(node);
    }

    nodes
}

// ────────────────────────────────────────────────────────────────────────────
// Skyline-based autoplace
// ────────────────────────────────────────────────────────────────────────────

/// Per-measure skyline used by [`autoplace_node`] to push successive
/// notation elements out of each other's way. Mirrors MuseScore's
/// `engraving::Skyline` (north + south lines of placed shapes).
#[derive(Default)]
pub struct MeasureSkyline {
    /// Above-staff shapes placed so far (in world coordinates).
    above: Vec<Shape>,
    /// Below-staff shapes placed so far.
    below: Vec<Shape>,
}

impl MeasureSkyline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the skyline with an immovable rect (notes, staff, slashes, etc.)
    /// so newly placed notation elements steer around it.
    pub fn add_above(&mut self, r: Rect) {
        self.add_shape_above(Shape::from_rect(r));
    }
    pub fn add_below(&mut self, r: Rect) {
        self.add_shape_below(Shape::from_rect(r));
    }

    pub fn add_shape_above(&mut self, shape: Shape) {
        if !shape.is_empty() {
            self.above.push(shape);
        }
    }

    pub fn add_shape_below(&mut self, shape: Shape) {
        if !shape.is_empty() {
            self.below.push(shape);
        }
    }

    fn min_distance_to_shape_above(&self, shape: &Shape, horizontal_clearance: f64) -> f64 {
        self.above
            .iter()
            .map(|obstacle| shape.min_vertical_distance(obstacle, horizontal_clearance))
            .filter(|distance| distance.is_finite())
            .fold(f64::NEG_INFINITY, f64::max)
    }

    fn min_distance_to_shape_below(&self, shape: &Shape, horizontal_clearance: f64) -> f64 {
        self.below
            .iter()
            .map(|obstacle| obstacle.min_vertical_distance(shape, horizontal_clearance))
            .filter(|distance| distance.is_finite())
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

pub fn add_scene_obstacles(
    skyline: &mut MeasureSkyline,
    node: &SceneNode,
    transform: Affine,
    add_above: bool,
    add_below: bool,
) {
    fn visit(
        skyline: &mut MeasureSkyline,
        node: &SceneNode,
        transform: Affine,
        add_above: bool,
        add_below: bool,
    ) {
        let transform = transform * node.transform;
        if node
            .id
            .as_ref()
            .is_some_and(|id| id.element_type == ElementType::StaffLines)
        {
            return;
        }

        for command in &node.commands {
            if let Some(rect) = command_collision_rect(command) {
                let rect = transform.transform_rect_bbox(rect);
                if !rect.is_zero_area() {
                    if add_above {
                        skyline.add_above(rect);
                    }
                    if add_below {
                        skyline.add_below(rect);
                    }
                }
            }
        }

        for child in &node.children {
            visit(skyline, child, transform, add_above, add_below);
        }
    }

    visit(skyline, node, transform, add_above, add_below);
}

pub fn scene_ink_bounds(node: &SceneNode, transform: Affine) -> Option<Rect> {
    fn visit(node: &SceneNode, transform: Affine, bounds: &mut Option<Rect>) {
        let transform = transform * node.transform;
        if node
            .id
            .as_ref()
            .is_some_and(|id| id.element_type == ElementType::StaffLines)
        {
            return;
        }

        for command in &node.commands {
            if let Some(rect) = command_collision_rect(command) {
                let rect = transform.transform_rect_bbox(rect);
                if !rect.is_zero_area() {
                    *bounds = Some(bounds.map_or(rect, |current| current.union(rect)));
                }
            }
        }

        for child in &node.children {
            visit(child, transform, bounds);
        }
    }

    let mut bounds = None;
    visit(node, transform, &mut bounds);
    bounds
}

fn command_collision_rect(cmd: &PaintCommand) -> Option<Rect> {
    use crate::engraver::scene::paint::PaintCommand;
    match cmd {
        PaintCommand::Text {
            text,
            font_size,
            position,
            anchor,
            ..
        } => {
            // Font metrics are not available this late in the notation pass,
            // so keep this conservative. Boxed staff text contributes its
            // exact frame rect separately.
            let w = text.chars().count() as f64 * font_size * 0.48;
            let h = font_size;
            let (x0, x1) = match anchor {
                crate::engraver::scene::paint::TextAnchor::Start => (position.x, position.x + w),
                crate::engraver::scene::paint::TextAnchor::Middle => {
                    (position.x - w * 0.5, position.x + w * 0.5)
                }
                crate::engraver::scene::paint::TextAnchor::End => (position.x - w, position.x),
            };
            Some(Rect::new(
                x0,
                position.y - h * 0.65,
                x1,
                position.y + h * 0.1,
            ))
        }
        PaintCommand::Glyph { position, size, .. } => {
            let half = size * 0.6;
            Some(Rect::new(
                position.x - half,
                position.y - half,
                position.x + half,
                position.y + half,
            ))
        }
        other => other.bounding_box(),
    }
}

/// Estimate a shape for a SceneNode based on its paint commands. Unlike a
/// single bbox, this preserves separate text/frame/line rectangles so unrelated
/// vertical slices can tuck past each other like MuseScore shapes.
fn estimate_node_shape(node: &crate::engraver::scene::node::SceneNode) -> Option<Shape> {
    let mut elements = Vec::new();
    for cmd in &node.commands {
        if let Some(rect) = command_collision_rect(cmd) {
            let rect = node.transform.transform_rect_bbox(rect);
            if !rect.is_zero_area() {
                elements.push(ShapeElement::new(rect));
            }
        }
    }

    if elements.is_empty() {
        None
    } else {
        Some(Shape::from_elements(elements))
    }
}

/// Translate every paint command in a node by `(dx, dy)`. Used by the
/// autoplace pass to push an element out of the skyline without rebuilding
/// it. Affects Text / Glyph / Line / Rect / Circle / Ellipse / Fill /
/// Stroke.
fn shift_node_xy(node: &mut crate::engraver::scene::node::SceneNode, dx: f64, dy: f64) {
    use crate::engraver::scene::paint::PaintCommand;
    for cmd in node.commands.iter_mut() {
        match cmd {
            PaintCommand::Text { position, .. } | PaintCommand::Glyph { position, .. } => {
                position.x += dx;
                position.y += dy;
            }
            PaintCommand::Line { start, end, .. } => {
                start.x += dx;
                start.y += dy;
                end.x += dx;
                end.y += dy;
            }
            PaintCommand::Rect { rect, .. } => {
                rect.x0 += dx;
                rect.x1 += dx;
                rect.y0 += dy;
                rect.y1 += dy;
            }
            PaintCommand::Circle { center, .. } | PaintCommand::Ellipse { center, .. } => {
                center.x += dx;
                center.y += dy;
            }
            PaintCommand::Fill { path, .. } | PaintCommand::Stroke { path, .. } => {
                let shifted = Affine::translate((dx, dy)) * path.clone();
                *path = shifted;
            }
        }
    }
}

fn node_scale_anchor(node: &crate::engraver::scene::node::SceneNode) -> Option<Point> {
    for cmd in &node.commands {
        match cmd {
            PaintCommand::Text { position, .. } | PaintCommand::Glyph { position, .. } => {
                return Some(*position);
            }
            PaintCommand::Line { start, .. } => return Some(*start),
            PaintCommand::Rect { rect, .. } => return Some(Point::new(rect.x0, rect.y0)),
            PaintCommand::Circle { center, .. } | PaintCommand::Ellipse { center, .. } => {
                return Some(*center);
            }
            PaintCommand::Fill { .. } | PaintCommand::Stroke { .. } => {}
        }
    }
    None
}

fn scale_point_around(point: &mut Point, anchor: Point, scale: f64) {
    point.x = anchor.x + (point.x - anchor.x) * scale;
    point.y = anchor.y + (point.y - anchor.y) * scale;
}

fn scale_node_around_anchor(
    node: &mut crate::engraver::scene::node::SceneNode,
    anchor: Point,
    scale: f64,
) {
    for cmd in node.commands.iter_mut() {
        match cmd {
            PaintCommand::Text {
                position,
                font_size,
                ..
            } => {
                scale_point_around(position, anchor, scale);
                *font_size *= scale;
            }
            PaintCommand::Glyph { position, size, .. } => {
                scale_point_around(position, anchor, scale);
                *size *= scale;
            }
            PaintCommand::Line {
                start, end, width, ..
            } => {
                scale_point_around(start, anchor, scale);
                scale_point_around(end, anchor, scale);
                *width *= scale;
            }
            PaintCommand::Rect {
                rect, stroke_width, ..
            } => {
                let mut p0 = Point::new(rect.x0, rect.y0);
                let mut p1 = Point::new(rect.x1, rect.y1);
                scale_point_around(&mut p0, anchor, scale);
                scale_point_around(&mut p1, anchor, scale);
                rect.x0 = p0.x.min(p1.x);
                rect.y0 = p0.y.min(p1.y);
                rect.x1 = p0.x.max(p1.x);
                rect.y1 = p0.y.max(p1.y);
                *stroke_width *= scale;
            }
            PaintCommand::Circle { center, radius, .. } => {
                scale_point_around(center, anchor, scale);
                *radius *= scale;
            }
            PaintCommand::Ellipse {
                center,
                radius_x,
                radius_y,
                ..
            } => {
                scale_point_around(center, anchor, scale);
                *radius_x *= scale;
                *radius_y *= scale;
            }
            PaintCommand::Fill { path, .. } | PaintCommand::Stroke { path, .. } => {
                let affine = Affine::translate((anchor.x, anchor.y))
                    * Affine::scale(scale)
                    * Affine::translate((-anchor.x, -anchor.y));
                *path = affine * path.clone();
            }
        }
    }
}

pub fn shrink_node_to_max_right(
    node: &mut crate::engraver::scene::node::SceneNode,
    max_right: f64,
    min_scale: f64,
) {
    let Some(anchor) = node_scale_anchor(node) else {
        return;
    };
    let Some(shape) = estimate_node_shape(node) else {
        return;
    };
    let bbox = shape.bbox();
    if bbox.x1 <= max_right {
        return;
    }
    let overflow_width = (bbox.x1 - anchor.x).max(1.0);
    let available_width = (max_right - anchor.x).max(1.0);
    let scale = (available_width / overflow_width).clamp(min_scale, 1.0);
    if scale < 1.0 {
        scale_node_around_anchor(node, anchor, scale);
    }
}

fn placement_dy(shape: &Shape, skyline: &MeasureSkyline, above: bool, min_gap: f64) -> f64 {
    if above {
        let distance = skyline.min_distance_to_shape_above(shape, min_gap);
        if distance.is_finite() && distance > -min_gap {
            -(distance + min_gap)
        } else {
            0.0
        }
    } else {
        let distance = skyline.min_distance_to_shape_below(shape, min_gap);
        if distance.is_finite() && distance > -min_gap {
            distance + min_gap
        } else {
            0.0
        }
    }
}

/// Add `node` to the scene, shifting it vertically away from anything
/// already in the skyline that overlaps its x-range. Returns `true` if
/// the node was successfully placed.
///
/// `above` = true means the element belongs to the north (above-staff)
/// skyline, false = south (below-staff). `min_gap` is added on top of
/// the raw clearance so neighbours don't kiss.
pub fn autoplace_node(
    skyline: &mut MeasureSkyline,
    mut node: crate::engraver::scene::node::SceneNode,
    above: bool,
    min_gap: f64,
) -> Option<crate::engraver::scene::node::SceneNode> {
    let shape = estimate_node_shape(&node)?;
    let dy = placement_dy(&shape, skyline, above, min_gap);
    if dy.abs() > f64::EPSILON {
        shift_node_xy(&mut node, 0.0, dy);
    }
    let final_shape = shape.translate(Point::new(0.0, dy));
    if above {
        skyline.add_shape_above(final_shape);
    } else {
        skyline.add_shape_below(final_shape);
    }
    Some(node)
}

/// Place text-like notation, shrinking it when its full-size footprint would
/// need to move too far to avoid chord symbols, staff lines, or prior text.
pub fn autoplace_text_node(
    skyline: &mut MeasureSkyline,
    node: crate::engraver::scene::node::SceneNode,
    above: bool,
    min_gap: f64,
    max_displacement_before_shrink: f64,
) -> Option<crate::engraver::scene::node::SceneNode> {
    let anchor = node_scale_anchor(&node)?;
    let mut selected: Option<(crate::engraver::scene::node::SceneNode, Shape, f64)> = None;

    for scale in [1.0, 0.9, 0.8, 0.7, 0.6] {
        let mut candidate = node.clone();
        if scale < 1.0 {
            scale_node_around_anchor(&mut candidate, anchor, scale);
        }
        let shape = estimate_node_shape(&candidate)?;
        let dy = placement_dy(&shape, skyline, above, min_gap);
        selected = Some((candidate, shape, dy));
        if dy.abs() <= max_displacement_before_shrink {
            break;
        }
    }

    let (mut node, shape, dy) = selected?;
    if dy.abs() > f64::EPSILON {
        shift_node_xy(&mut node, 0.0, dy);
    }
    let final_shape = shape.translate(Point::new(0.0, dy));
    if above {
        skyline.add_shape_above(final_shape);
    } else {
        skyline.add_shape_below(final_shape);
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::notations::FiguredBassRow;

    #[test]
    fn classical_dynamics_render_red() {
        let dynamics = [Dynamic {
            level: DynamicLevel::Mf,
            beat: 1,
            placement: Placement::Below,
        }];
        let frame = MeasureFrame {
            measure_x: 100.0,
            measure_width: 120.0,
            staff_y: 40.0,
            staff_height: 40.0,
            chord_y: 20.0,
            spatium: 10.0,
            beats_per_measure: 4,
            source_measure_width: None,
            segment_positions: None,
            system_start_dynamic_x: None,
            section_start_dynamic_stack: None,
        };
        let mut id_counter = 1;

        let nodes = render_dynamics(&dynamics, &frame, &mut id_counter);

        let [node] = nodes.as_slice() else {
            panic!("expected one rendered dynamic");
        };
        let [PaintCommand::Text { color, .. }] = node.commands.as_slice() else {
            panic!("expected dynamic to render as a text glyph");
        };
        assert_eq!(*color, DYNAMIC_COLOR);
    }

    #[test]
    fn classical_dynamics_render_larger_by_default() {
        let dynamics = [Dynamic {
            level: DynamicLevel::Mf,
            beat: 1,
            placement: Placement::Below,
        }];
        let frame = MeasureFrame {
            measure_x: 100.0,
            measure_width: 120.0,
            staff_y: 40.0,
            staff_height: 40.0,
            chord_y: 20.0,
            spatium: 10.0,
            beats_per_measure: 4,
            source_measure_width: None,
            segment_positions: None,
            system_start_dynamic_x: None,
            section_start_dynamic_stack: None,
        };
        let mut id_counter = 1;

        let nodes = render_dynamics(&dynamics, &frame, &mut id_counter);

        let [node] = nodes.as_slice() else {
            panic!("expected one rendered dynamic");
        };
        let [PaintCommand::Text { font_size, .. }] = node.commands.as_slice() else {
            panic!("expected dynamic to render as a text glyph");
        };
        assert_eq!(*font_size, frame.spatium * DYNAMIC_GLYPH_SIZE_SP);
    }

    #[test]
    fn staff_text_renders_red() {
        let items = [StaffText {
            text: "Cresc. <...<...<".to_string(),
            beat: 1,
            placement: Placement::Below,
            source_default_x: None,
            boxed: false,
            bold: false,
            italic: false,
        }];
        let frame = MeasureFrame {
            measure_x: 100.0,
            measure_width: 120.0,
            staff_y: 40.0,
            staff_height: 40.0,
            chord_y: 20.0,
            spatium: 10.0,
            beats_per_measure: 4,
            source_measure_width: None,
            segment_positions: None,
            system_start_dynamic_x: None,
            section_start_dynamic_stack: None,
        };
        let text_metrics = crate::engraver::layout::text_metrics::TextFontMetrics::new(
            std::sync::Arc::new(Vec::new()),
        );
        let mut id_counter = 1;

        let nodes = render_staff_text(&items, &frame, &text_metrics, &mut id_counter);

        let [node] = nodes.as_slice() else {
            panic!("expected one rendered staff text node");
        };
        let text_color = node.commands.iter().find_map(|command| {
            if let PaintCommand::Text { color, .. } = command {
                Some(*color)
            } else {
                None
            }
        });
        assert_eq!(text_color, Some(STAFF_TEXT_COLOR));
    }

    #[test]
    fn staff_text_renders_larger_by_default() {
        let items = [StaffText {
            text: "Full Groove Now >>>".to_string(),
            beat: 1,
            placement: Placement::Below,
            source_default_x: None,
            boxed: false,
            bold: false,
            italic: false,
        }];
        let frame = MeasureFrame {
            measure_x: 100.0,
            measure_width: 120.0,
            staff_y: 40.0,
            staff_height: 40.0,
            chord_y: 20.0,
            spatium: 10.0,
            beats_per_measure: 4,
            source_measure_width: None,
            segment_positions: None,
            system_start_dynamic_x: None,
            section_start_dynamic_stack: None,
        };
        let text_metrics = crate::engraver::layout::text_metrics::TextFontMetrics::new(
            std::sync::Arc::new(Vec::new()),
        );
        let mut id_counter = 1;

        let nodes = render_staff_text(&items, &frame, &text_metrics, &mut id_counter);

        let [node] = nodes.as_slice() else {
            panic!("expected one rendered staff text node");
        };
        let font_size = node.commands.iter().find_map(|command| {
            if let PaintCommand::Text { font_size, .. } = command {
                Some(*font_size)
            } else {
                None
            }
        });
        assert_eq!(font_size, Some(frame.spatium * STAFF_TEXT_FONT_SIZE_SP));
    }

    #[test]
    fn repeated_section_start_dynamics_can_stack_under_section_cards() {
        let dynamics = [
            Dynamic {
                level: DynamicLevel::Mf,
                beat: 1,
                placement: Placement::Below,
            },
            Dynamic {
                level: DynamicLevel::F,
                beat: 1,
                placement: Placement::Below,
            },
        ];
        let frame = MeasureFrame {
            measure_x: 100.0,
            measure_width: 120.0,
            staff_y: 40.0,
            staff_height: 40.0,
            chord_y: 20.0,
            spatium: 10.0,
            beats_per_measure: 4,
            source_measure_width: None,
            segment_positions: None,
            system_start_dynamic_x: Some(40.0),
            section_start_dynamic_stack: Some(DynamicStackAnchor {
                x: 24.0,
                y_slots: vec![92.0, 128.0],
                fallback_gap: 18.0,
            }),
        };
        let mut id_counter = 1;

        let nodes = render_dynamics(&dynamics, &frame, &mut id_counter);

        let positions: Vec<Point> = nodes
            .iter()
            .map(|node| {
                let [PaintCommand::Text { position, .. }] = node.commands.as_slice() else {
                    panic!("expected dynamic to render as a text glyph");
                };
                *position
            })
            .collect();
        assert_eq!(
            positions,
            vec![Point::new(24.0, 92.0), Point::new(24.0, 128.0)]
        );
    }

    #[test]
    fn figured_bass_uses_source_default_x_when_available() {
        let items = [FiguredBass {
            rows: vec![FiguredBassRow {
                accidental: "#".to_string(),
                text: "4-3".to_string(),
            }],
            beat: 1,
            placement: Placement::Above,
            source_default_x: Some(136.0),
            source_relative_x: None,
        }];
        let frame = MeasureFrame {
            measure_x: 100.0,
            measure_width: 200.0,
            staff_y: 40.0,
            staff_height: 40.0,
            chord_y: 20.0,
            spatium: 10.0,
            beats_per_measure: 6,
            source_measure_width: Some(272.0),
            segment_positions: None,
            system_start_dynamic_x: None,
            section_start_dynamic_stack: None,
        };
        let mut id_counter = 1;

        let nodes = render_figured_bass(&items, &frame, &mut id_counter);
        let [node] = nodes.as_slice() else {
            panic!("expected one rendered figured-bass node");
        };
        let PaintCommand::Text { position, .. } = &node.commands[0] else {
            panic!("expected figured bass to render text");
        };

        assert!(
            (position.x - 200.0).abs() < f64::EPSILON,
            "default-x 136 in source width 272 should map to measure midpoint"
        );
        assert!(
            position.y < frame.staff_y,
            "figured bass should render in the chord-symbol lane above the staff"
        );
    }
}
