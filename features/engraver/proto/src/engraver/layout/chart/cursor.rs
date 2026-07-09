//! Renderer-agnostic playback cursor for chart highlighting.
//!
//! Provides a `ChartCursor` abstraction that computes cursor position and
//! highlight commands from a `ChartLayoutResult` and a tick position. The
//! output is a set of `HighlightCommand` values that any renderer (Vello,
//! Canvas, SVG, etc.) can translate into draw calls.
//!
//! Colors use `[u8; 4]` RGBA to avoid depending on any rendering framework.

use super::types::{BeatPosition, ChartLayoutResult};

// ============================================================================
// Color Utility
// ============================================================================

/// RGBA color as `[r, g, b, a]` bytes.
pub type Rgba = [u8; 4];

/// Multiply the alpha channel of an RGBA color by a factor in `0.0..=1.0`.
#[must_use]
fn multiply_alpha(color: Rgba, factor: f32) -> Rgba {
    let a = (color[3] as f32 * factor).round().min(255.0) as u8;
    [color[0], color[1], color[2], a]
}

// ============================================================================
// Cursor Style
// ============================================================================

/// Visual style for the playback cursor.
#[derive(Debug, Clone)]
pub enum CursorStyle {
    /// Thin vertical line at the interpolated playhead position.
    VerticalLine {
        /// Line width in layout points.
        width: f64,
    },
    /// Translucent rectangle spanning the full current measure.
    MeasureHighlight,
    /// Rounded rectangle spanning the current beat/chord slot.
    BeatBox {
        /// Corner radius in layout points.
        corner_radius: f64,
    },
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self::VerticalLine { width: 6.0 }
    }
}

// ============================================================================
// Cursor Configuration
// ============================================================================

/// Configuration for cursor rendering.
#[derive(Debug, Clone)]
pub struct CursorConfig {
    /// Visual style.
    pub style: CursorStyle,
    /// Primary accent color (RGBA).
    pub accent_color: Rgba,
    /// Alpha multiplier for filled regions (measure/beat highlight).
    pub fill_alpha: f32,
    /// Vertical extension factor for the cursor line (e.g. 0.25 = 25% above
    /// and below the staff).
    pub vertical_extension: f64,
    /// Whether to highlight the current beat's notehead glyph.
    pub highlight_notehead: bool,
    /// Glow stroke width around the highlighted notehead.
    pub glow_width: f64,
    /// Glow alpha multiplier.
    pub glow_alpha: f32,
    /// Whether to show the cursor when playback is stopped.
    pub show_when_stopped: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            style: CursorStyle::default(),
            accent_color: [255, 60, 60, 255],
            fill_alpha: 0.35,
            vertical_extension: 0.25,
            highlight_notehead: true,
            glow_width: 6.0,
            glow_alpha: 0.4,
            show_when_stopped: true,
        }
    }
}

// ============================================================================
// Highlight Commands
// ============================================================================

/// Renderer-agnostic draw command produced by the cursor.
#[derive(Debug, Clone)]
pub enum HighlightCommand {
    /// Stroke a vertical line segment.
    StrokeLine {
        x: f64,
        y_top: f64,
        y_bottom: f64,
        color: Rgba,
        width: f64,
    },
    /// Fill an axis-aligned rectangle.
    FillRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: Rgba,
    },
    /// Fill a rounded rectangle.
    FillRoundedRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
        color: Rgba,
    },
    /// Stroke the outline of a SMuFL glyph (for glow effect).
    StrokeGlyph {
        codepoint: char,
        font_size: f64,
        x: f64,
        y: f64,
        stroke_width: f64,
        color: Rgba,
    },
    /// Fill a SMuFL glyph with a solid color.
    FillGlyph {
        codepoint: char,
        font_size: f64,
        x: f64,
        y: f64,
        color: Rgba,
    },
}

// ============================================================================
// Cursor State
// ============================================================================

/// Result of a cursor computation: position metadata plus draw commands.
#[derive(Debug, Clone)]
pub struct CursorState {
    /// Page the cursor is on (1-indexed).
    pub page: u32,
    /// System index within the page (0-indexed).
    pub system: usize,
    /// Current measure index (0-indexed, global).
    pub measure: usize,
    /// Interpolated X position in layout coordinates.
    pub cursor_x: f64,
    /// Y position (top of staff) in layout coordinates.
    pub cursor_y: f64,
    /// Staff height at the cursor position.
    pub cursor_height: f64,
    /// Draw commands to render the cursor and highlights.
    pub commands: Vec<HighlightCommand>,
}

// ============================================================================
// Chart Cursor
// ============================================================================

/// Renderer-agnostic playback cursor that computes highlight commands from
/// a layout and a tick position.
#[derive(Debug, Clone, Default)]
pub struct ChartCursor {
    pub config: CursorConfig,
}

impl ChartCursor {
    /// Create a new cursor with the given configuration.
    #[must_use]
    pub fn new(config: CursorConfig) -> Self {
        Self { config }
    }

    /// Compute cursor state for a given absolute tick.
    ///
    /// Returns `None` if the tick is outside the layout's beat range.
    #[must_use]
    pub fn compute(&self, layout: &ChartLayoutResult, tick: i64) -> Option<CursorState> {
        let beat = layout.beat_at_tick(tick)?;
        let cursor_x = beat.x_at_tick(tick);

        let mut commands = Vec::new();

        // Build style-specific commands
        match &self.config.style {
            CursorStyle::VerticalLine { width } => {
                self.build_vertical_line(&mut commands, beat, cursor_x, *width);
            }
            CursorStyle::MeasureHighlight => {
                self.build_measure_highlight(&mut commands, beat, layout);
            }
            CursorStyle::BeatBox { corner_radius } => {
                self.build_beat_box(&mut commands, beat, *corner_radius);
            }
        }

        // Notehead highlight (common to all styles)
        if self.config.highlight_notehead {
            self.build_notehead_highlight(&mut commands, beat);
        }

        Some(CursorState {
            page: beat.page,
            system: beat.system,
            measure: beat.measure,
            cursor_x,
            cursor_y: beat.staff_y,
            cursor_height: beat.staff_height,
            commands,
        })
    }

    /// Compute cursor state for a given time in seconds.
    ///
    /// Convenience wrapper when only time-based lookup is available.
    #[must_use]
    pub fn compute_at_time(&self, layout: &ChartLayoutResult, time: f64) -> Option<CursorState> {
        let beat = layout.beat_at_time(time)?;
        let cursor_x = beat.x_at_time(time);
        let _tick = beat.absolute_tick;

        let mut commands = Vec::new();

        match &self.config.style {
            CursorStyle::VerticalLine { width } => {
                self.build_vertical_line(&mut commands, beat, cursor_x, *width);
            }
            CursorStyle::MeasureHighlight => {
                self.build_measure_highlight(&mut commands, beat, layout);
            }
            CursorStyle::BeatBox { corner_radius } => {
                self.build_beat_box(&mut commands, beat, *corner_radius);
            }
        }

        if self.config.highlight_notehead {
            self.build_notehead_highlight(&mut commands, beat);
        }

        Some(CursorState {
            page: beat.page,
            system: beat.system,
            measure: beat.measure,
            cursor_x,
            cursor_y: beat.staff_y,
            cursor_height: beat.staff_height,
            commands,
        })
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    fn build_vertical_line(
        &self,
        commands: &mut Vec<HighlightCommand>,
        beat: &BeatPosition,
        cursor_x: f64,
        width: f64,
    ) {
        let ext = beat.staff_height * self.config.vertical_extension;
        commands.push(HighlightCommand::StrokeLine {
            x: cursor_x,
            y_top: beat.staff_y - ext,
            y_bottom: beat.staff_y + beat.staff_height + ext,
            color: self.config.accent_color,
            width,
        });
    }

    fn build_measure_highlight(
        &self,
        commands: &mut Vec<HighlightCommand>,
        beat: &BeatPosition,
        layout: &ChartLayoutResult,
    ) {
        let measure_beats = layout.beats_in_measure(beat.measure);
        if measure_beats.is_empty() {
            return;
        }

        let min_x = measure_beats
            .iter()
            .map(|b| b.x)
            .fold(f64::INFINITY, f64::min);
        let max_x = measure_beats
            .iter()
            .map(|b| b.x + b.width)
            .fold(f64::NEG_INFINITY, f64::max);

        let color = multiply_alpha(self.config.accent_color, self.config.fill_alpha);
        commands.push(HighlightCommand::FillRect {
            x: min_x,
            y: beat.staff_y,
            width: max_x - min_x,
            height: beat.staff_height,
            color,
        });
    }

    fn build_beat_box(
        &self,
        commands: &mut Vec<HighlightCommand>,
        beat: &BeatPosition,
        corner_radius: f64,
    ) {
        let color = multiply_alpha(self.config.accent_color, self.config.fill_alpha);
        commands.push(HighlightCommand::FillRoundedRect {
            x: beat.x,
            y: beat.staff_y,
            width: beat.width,
            height: beat.staff_height,
            radius: corner_radius,
            color,
        });
    }

    fn build_notehead_highlight(&self, commands: &mut Vec<HighlightCommand>, beat: &BeatPosition) {
        let Some(codepoint) = beat.glyph_codepoint else {
            return;
        };

        // SMuFL convention: 1 em = 4 staff spaces, so font_size = spatium * 4
        let font_size = beat.glyph_size * 4.0;
        let x = beat.x;
        // Notehead at vertical center of staff
        let y = beat.staff_y + beat.staff_height * 0.5;

        // Glow outline (underneath)
        let glow_color = multiply_alpha(self.config.accent_color, self.config.glow_alpha);
        commands.push(HighlightCommand::StrokeGlyph {
            codepoint,
            font_size,
            x,
            y,
            stroke_width: self.config.glow_width,
            color: glow_color,
        });

        // Solid fill on top
        commands.push(HighlightCommand::FillGlyph {
            codepoint,
            font_size,
            x,
            y,
            color: self.config.accent_color,
        });
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal layout with a few beats for testing.
    fn test_layout() -> ChartLayoutResult {
        use crate::engraver::scene::node::SceneNode;

        let beats = vec![
            BeatPosition {
                page: 1,
                system: 0,
                measure: 0,
                beat: 0,
                tick: 0,
                duration_ticks: 480,
                absolute_tick: 0,
                x: 100.0,
                width: 50.0,
                staff_y: 200.0,
                staff_height: 20.0,
                time_start: 0.0,
                time_end: 0.5,
                glyph_codepoint: Some('\u{E101}'),
                glyph_size: 5.0,
                glyph_y: 210.0,
                has_stem: false,
                stem_up: true,
                flag_count: 0,
                time_signature: (4, 4),
            },
            BeatPosition {
                page: 1,
                system: 0,
                measure: 0,
                beat: 1,
                tick: 480,
                duration_ticks: 480,
                absolute_tick: 480,
                x: 150.0,
                width: 50.0,
                staff_y: 200.0,
                staff_height: 20.0,
                time_start: 0.5,
                time_end: 1.0,
                glyph_codepoint: Some('\u{E101}'),
                glyph_size: 5.0,
                glyph_y: 210.0,
                has_stem: false,
                stem_up: true,
                flag_count: 0,
                time_signature: (4, 4),
            },
            BeatPosition {
                page: 1,
                system: 0,
                measure: 1,
                beat: 0,
                tick: 0,
                duration_ticks: 480,
                absolute_tick: 1920,
                x: 300.0,
                width: 50.0,
                staff_y: 200.0,
                staff_height: 20.0,
                time_start: 2.0,
                time_end: 2.5,
                glyph_codepoint: None,
                glyph_size: 5.0,
                glyph_y: 210.0,
                has_stem: false,
                stem_up: true,
                flag_count: 0,
                time_signature: (4, 4),
            },
        ];

        ChartLayoutResult {
            scene: SceneNode::anonymous_leaf(vec![]),
            pages: vec![],
            total_height: 800.0,
            total_width: 612.0,
            beat_positions: beats,
        }
    }

    #[test]
    fn compute_returns_none_outside_range() {
        let cursor = ChartCursor::default();
        let layout = test_layout();
        assert!(cursor.compute(&layout, -1000).is_none());
        assert!(cursor.compute(&layout, 99999).is_none());
    }

    #[test]
    fn compute_vertical_line_at_first_beat() {
        let cursor = ChartCursor::default();
        let layout = test_layout();

        let state = cursor.compute(&layout, 0).unwrap();
        assert_eq!(state.page, 1);
        assert_eq!(state.system, 0);
        assert_eq!(state.measure, 0);
        assert!((state.cursor_x - 100.0).abs() < 0.01);
        assert!((state.cursor_y - 200.0).abs() < 0.01);

        // Should have a StrokeLine + StrokeGlyph + FillGlyph (notehead highlight enabled by default)
        assert!(
            state
                .commands
                .iter()
                .any(|c| matches!(c, HighlightCommand::StrokeLine { .. }))
        );
        assert!(
            state
                .commands
                .iter()
                .any(|c| matches!(c, HighlightCommand::StrokeGlyph { .. }))
        );
        assert!(
            state
                .commands
                .iter()
                .any(|c| matches!(c, HighlightCommand::FillGlyph { .. }))
        );
    }

    #[test]
    fn compute_interpolates_within_beat() {
        let cursor = ChartCursor::default();
        let layout = test_layout();

        // Tick 240 is halfway through beat 0 (duration 480)
        let state = cursor.compute(&layout, 240).unwrap();
        // x should be 100 + 50 * 0.5 = 125
        assert!((state.cursor_x - 125.0).abs() < 0.01);
    }

    #[test]
    fn compute_measure_highlight() {
        let config = CursorConfig {
            style: CursorStyle::MeasureHighlight,
            highlight_notehead: false,
            ..CursorConfig::default()
        };
        let cursor = ChartCursor::new(config);
        let layout = test_layout();

        let state = cursor.compute(&layout, 0).unwrap();
        // Should have a FillRect spanning measure 0 (beats at x=100 and x=150, each width 50)
        let fill = state
            .commands
            .iter()
            .find(|c| matches!(c, HighlightCommand::FillRect { .. }));
        assert!(fill.is_some());
        if let Some(HighlightCommand::FillRect {
            x, width, color, ..
        }) = fill
        {
            assert!((*x - 100.0).abs() < 0.01);
            // max_x = 150 + 50 = 200, so width = 200 - 100 = 100
            assert!((*width - 100.0).abs() < 0.01);
            // Alpha should be multiplied
            assert!(color[3] < 255);
        }
    }

    #[test]
    fn compute_beat_box() {
        let config = CursorConfig {
            style: CursorStyle::BeatBox { corner_radius: 3.0 },
            highlight_notehead: false,
            ..CursorConfig::default()
        };
        let cursor = ChartCursor::new(config);
        let layout = test_layout();

        let state = cursor.compute(&layout, 0).unwrap();
        let rounded = state
            .commands
            .iter()
            .find(|c| matches!(c, HighlightCommand::FillRoundedRect { .. }));
        assert!(rounded.is_some());
        if let Some(HighlightCommand::FillRoundedRect {
            x, width, radius, ..
        }) = rounded
        {
            assert!((*x - 100.0).abs() < 0.01);
            assert!((*width - 50.0).abs() < 0.01);
            assert!((*radius - 3.0).abs() < 0.01);
        }
    }

    #[test]
    fn no_notehead_highlight_when_glyph_missing() {
        let cursor = ChartCursor::default();
        let layout = test_layout();

        // Beat at tick 1920 has no glyph_codepoint
        let state = cursor.compute(&layout, 1920).unwrap();
        assert!(
            !state
                .commands
                .iter()
                .any(|c| matches!(c, HighlightCommand::StrokeGlyph { .. }))
        );
        assert!(
            !state
                .commands
                .iter()
                .any(|c| matches!(c, HighlightCommand::FillGlyph { .. }))
        );
    }

    #[test]
    fn notehead_disabled_produces_no_glyph_commands() {
        let config = CursorConfig {
            highlight_notehead: false,
            ..CursorConfig::default()
        };
        let cursor = ChartCursor::new(config);
        let layout = test_layout();

        let state = cursor.compute(&layout, 0).unwrap();
        assert!(
            !state
                .commands
                .iter()
                .any(|c| matches!(c, HighlightCommand::StrokeGlyph { .. }))
        );
    }

    #[test]
    fn compute_at_time_works() {
        let cursor = ChartCursor::default();
        let layout = test_layout();

        let state = cursor.compute_at_time(&layout, 0.25).unwrap();
        assert_eq!(state.page, 1);
        assert_eq!(state.measure, 0);
        // time 0.25 is halfway through beat 0 (0.0..0.5)
        assert!((state.cursor_x - 125.0).abs() < 0.01);
    }

    #[test]
    fn negative_tick_count_in() {
        // Simulate a count-in beat at negative tick
        use crate::engraver::scene::node::SceneNode;

        let beats = vec![BeatPosition {
            page: 1,
            system: 0,
            measure: 0,
            beat: 0,
            tick: 0,
            duration_ticks: 480,
            absolute_tick: -1920,
            x: 50.0,
            width: 30.0,
            staff_y: 100.0,
            staff_height: 20.0,
            time_start: -2.0,
            time_end: -1.5,
            glyph_codepoint: Some('\u{E101}'),
            glyph_size: 3.0,
            glyph_y: 110.0,
            has_stem: false,
            stem_up: true,
            flag_count: 0,
            time_signature: (4, 4),
        }];

        let layout = ChartLayoutResult {
            scene: SceneNode::anonymous_leaf(vec![]),
            pages: vec![],
            total_height: 400.0,
            total_width: 300.0,
            beat_positions: beats,
        };

        let cursor = ChartCursor::default();
        let state = cursor.compute(&layout, -1920).unwrap();
        assert_eq!(state.measure, 0);
        assert!((state.cursor_x - 50.0).abs() < 0.01);
    }

    #[test]
    fn multiply_alpha_clamps() {
        let color: Rgba = [255, 0, 0, 200];
        let result = multiply_alpha(color, 0.5);
        assert_eq!(result, [255, 0, 0, 100]);

        let full = multiply_alpha(color, 1.0);
        assert_eq!(full, [255, 0, 0, 200]);

        let zero = multiply_alpha(color, 0.0);
        assert_eq!(zero, [255, 0, 0, 0]);
    }
}
