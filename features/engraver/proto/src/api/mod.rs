//! Clean facade API for engraving workflows.
//!
//! This module provides a small, ergonomic layer over the engraver pipeline,
//! leaving the full surface area available under `engraver::*`.

use std::error::Error;
use std::fmt;

/// Commonly used types for chart engraving.
pub mod prelude {
    pub use crate::api::style::{
        leak_default_style, leak_jazz_lead_sheet_style, leak_lead_sheet_style,
    };
    pub use crate::engraver::fonts::ChartFontBundle;
    pub use crate::engraver::layout::chart::{ChartLayoutEngine, ChartLayoutResult, LayoutMode};
    pub use crate::engraver::style::MStyle;
    pub use keyflow_proto::{Chart, ParseError};
}

/// Convenience helpers for obtaining a `'static` style.
pub mod style {
    use crate::engraver::style::MStyle;

    /// Leak a style to obtain a `'static` reference for chart layout engines.
    #[must_use]
    pub fn leak_style(style: MStyle) -> &'static MStyle {
        Box::leak(Box::new(style))
    }

    /// Default engraving style.
    #[must_use]
    pub fn leak_default_style() -> &'static MStyle {
        leak_style(MStyle::default())
    }

    /// Lead sheet style preset.
    #[must_use]
    pub fn leak_lead_sheet_style() -> &'static MStyle {
        leak_style(MStyle::lead_sheet())
    }

    /// Jazz lead sheet style preset.
    #[must_use]
    pub fn leak_jazz_lead_sheet_style() -> &'static MStyle {
        leak_style(MStyle::jazz_lead_sheet())
    }
}

/// Errors returned by the facade helpers.
#[derive(Debug)]
pub enum ChartLayoutError {
    Parse(String),
    Fonts(String),
}

impl fmt::Display for ChartLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "chart parse error: {err}"),
            Self::Fonts(err) => write!(f, "font bundle error: {err}"),
        }
    }
}

impl Error for ChartLayoutError {}

/// Chart engraving helpers (keyflow chart → layout).
pub mod chart {
    use super::{ChartLayoutError, style};
    use crate::engraver::fonts::ChartFontBundle;
    use crate::engraver::layout::chart::{ChartLayoutEngine, ChartLayoutResult, LayoutMode};
    use crate::engraver::style::MStyle;
    use keyflow_proto::Chart;

    /// Create a chart layout engine from a style and font bundle.
    #[must_use]
    pub fn engine(style: &'static MStyle, fonts: &ChartFontBundle) -> ChartLayoutEngine {
        fonts.create_layout_engine(style)
    }

    /// Layout an already-parsed chart using the provided engine and layout mode.
    #[must_use]
    pub fn layout(
        chart: &Chart,
        engine: &ChartLayoutEngine,
        mode: &LayoutMode,
    ) -> ChartLayoutResult {
        engine.layout_chart(chart, mode)
    }

    /// Parse and layout chart text with default fonts and a lead sheet style.
    pub fn layout_text(
        text: &str,
        mode: &LayoutMode,
    ) -> Result<ChartLayoutResult, ChartLayoutError> {
        let chart = keyflow_text::chart::parse_chart(text).map_err(ChartLayoutError::Parse)?;
        layout_chart(&chart, mode)
    }

    /// Layout a chart with default fonts and a lead sheet style.
    pub fn layout_chart(
        chart: &Chart,
        mode: &LayoutMode,
    ) -> Result<ChartLayoutResult, ChartLayoutError> {
        let fonts = ChartFontBundle::new().map_err(ChartLayoutError::Fonts)?;
        let style = style::leak_lead_sheet_style();
        let engine = fonts.create_layout_engine(style);
        Ok(engine.layout_chart(chart, mode))
    }
}
