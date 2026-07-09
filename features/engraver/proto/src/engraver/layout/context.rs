//! Layout context for orchestrating music notation layout.
//!
//! This module provides the central `LayoutContext` struct that coordinates
//! all layout operations, similar to MuseScore's LayoutContext class.
//!
//! # Architecture
//!
//! The context is split into two types:
//! - `LayoutContext<'a>` - Immutable, borrowed view used during layout
//! - `LayoutContextOwned` - Owns the data and provides borrowed contexts
//!
//! This design eliminates RefCell runtime overhead and Box::leak memory leaks.

use std::sync::Arc;

use crate::engraver::fonts::SMuFLFont;
use crate::engraver::style::MStyle;

// region:    --- Layout Mode

/// Layout view mode.
///
/// Determines how the score is laid out and rendered. Based on MuseScore's
/// LayoutMode enum with modes optimized for different use cases.
///
/// # Modes
///
/// - **Page**: Standard page view for printing, honors page and line breaks
/// - **Horizontal**: Endless horizontal scroll, one system containing all measures
/// - **Vertical**: Endless vertical scroll, multiple systems on one infinite page
/// - **Float**: Reflow mode, ignores explicit breaks (similar to Page but more flexible)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayoutMode {
    /// Standard page view (default).
    ///
    /// Creates fixed-size pages with multiple systems per page.
    /// Honors explicit page breaks and line breaks.
    /// Best for: Print output, traditional score viewing.
    #[default]
    Page,

    /// Endless horizontal scroll mode.
    ///
    /// Creates a single horizontal strip containing one system with all measures.
    /// The width grows to accommodate all content; height is fixed to one system.
    /// Ignores line breaks and page breaks.
    /// Best for: DAW integration, horizontal timeline views, practice mode.
    Horizontal,

    /// Endless vertical scroll mode.
    ///
    /// Creates one infinite page with multiple systems stacked vertically.
    /// Page breaks are converted to line breaks; content flows continuously.
    /// Width is fixed; height grows to accommodate all systems.
    /// Best for: Screen viewing, continuous scrolling, web display.
    Vertical,

    /// Reflow mode.
    ///
    /// Similar to Page mode but ignores explicit line breaks and page breaks.
    /// Content reflows to fit the available space.
    /// Best for: Dynamic resizing, responsive layouts.
    Float,
}

impl LayoutMode {
    /// Returns true if this is a linear (non-paginated) mode.
    ///
    /// Linear modes don't create page breaks and render as continuous content.
    #[must_use]
    pub const fn is_linear(&self) -> bool {
        matches!(self, Self::Horizontal | Self::Vertical)
    }

    /// Returns true if this mode creates multiple pages.
    #[must_use]
    pub const fn is_paginated(&self) -> bool {
        matches!(self, Self::Page | Self::Float)
    }

    /// Returns true if this mode respects explicit line breaks.
    #[must_use]
    pub const fn honors_line_breaks(&self) -> bool {
        matches!(self, Self::Page | Self::Vertical)
    }

    /// Returns true if this mode respects explicit page breaks.
    #[must_use]
    pub const fn honors_page_breaks(&self) -> bool {
        matches!(self, Self::Page)
    }

    /// Get a human-readable name for this mode.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Page => "Page View",
            Self::Horizontal => "Continuous Horizontal",
            Self::Vertical => "Continuous Vertical",
            Self::Float => "Reflow",
        }
    }
}

// endregion: --- Layout Mode

// region:    --- Layout Configuration

/// Configuration for layout operations (immutable).
#[derive(Debug, Clone, Default)]
pub struct LayoutConfiguration {
    /// View mode for layout
    pub view_mode: LayoutMode,
    /// Whether to show invisible elements
    pub show_invisible: bool,
    /// Note head width in spatiums (for spacing calculations)
    pub note_head_width: f64,
    /// Page number offset (for multi-document layouts)
    pub page_number_offset: usize,
}

impl LayoutConfiguration {
    /// Create a new configuration with default values.
    #[must_use]
    pub fn new() -> Self {
        Self {
            view_mode: LayoutMode::Page,
            show_invisible: false,
            note_head_width: 1.6, // Standard notehead width
            page_number_offset: 0,
        }
    }

    /// Set the view mode.
    #[must_use]
    pub fn with_view_mode(mut self, mode: LayoutMode) -> Self {
        self.view_mode = mode;
        self
    }

    /// Set whether to show invisible elements.
    #[must_use]
    pub fn with_show_invisible(mut self, show: bool) -> Self {
        self.show_invisible = show;
        self
    }
}

// endregion: --- Layout Configuration

// region:    --- Layout State

/// Mutable state during layout pass.
///
/// This is passed explicitly through layout functions rather than
/// being embedded in the context, enabling better compiler optimization
/// and clearer data flow.
#[derive(Debug, Default, Clone)]
pub struct LayoutState {
    /// Current measure being laid out
    pub current_measure: Option<usize>,
    /// Current system being laid out
    pub current_system: Option<usize>,
    /// Current page being laid out
    pub current_page: Option<usize>,
    /// Current tick position (for tempo calculations)
    pub tick: i32,
    /// Current measure number (1-indexed for display)
    pub measure_no: usize,
    /// Whether this is the first system on the page
    pub first_system: bool,
}

// endregion: --- Layout State

// region:    --- Layout Context (Borrowed)

/// Immutable layout context for layout operations.
///
/// The `LayoutContext` contains all configuration and read-only data needed for
/// laying out a musical score. It provides access to:
/// - The score data (read-only)
/// - Style properties (spacing, fonts, etc.)
/// - SMuFL font for glyph metrics
///
/// Based on MuseScore's LayoutContext + LayoutConfiguration + DomAccessor.
///
/// # Lifetime
///
/// The `'a` lifetime ties the context to the data it borrows,
/// preventing use-after-free and enabling zero-copy references.
///
/// # Example
///
/// ```ignore
/// let owned = LayoutContextOwned::new_minimal(MStyle::default());
/// let ctx = owned.as_context();
///
/// // Use spatium-based queries
/// let bar_distance = ctx.style_distance(Sid::BarNoteDistance);
/// ```
#[derive(Clone, Copy)]
pub struct LayoutContext<'a> {
    /// Configuration (immutable)
    pub config: &'a LayoutConfiguration,
    /// MStyle for all spacing/style queries
    pub style: &'a MStyle,
    /// SMuFL font for glyph metrics
    pub font: &'a SMuFLFont<'a>,
}

impl<'a> LayoutContext<'a> {
    /// Create a new layout context from borrowed references.
    #[must_use]
    pub fn new(
        config: &'a LayoutConfiguration,
        style: &'a MStyle,
        font: &'a SMuFLFont<'a>,
    ) -> Self {
        Self {
            config,
            style,
            font,
        }
    }

    /// Get the base spatium value in points.
    ///
    /// Spatium is the fundamental unit in music notation,
    /// representing one staff space (1/4 of staff height).
    #[must_use]
    pub fn spatium(&self) -> f64 {
        self.style.base_spatium() as f64
    }

    /// Get a style distance in points.
    ///
    /// Converts a spatium-based style property to points.
    #[must_use]
    pub fn style_distance(&self, sid: crate::engraver::style::Sid) -> f64 {
        self.style.spatium(sid) as f64 * self.spatium()
    }

    /// Get a real-valued style property.
    #[must_use]
    pub fn style_real(&self, sid: crate::engraver::style::Sid) -> f64 {
        self.style.real(sid) as f64
    }

    /// Get a boolean style property.
    #[must_use]
    pub fn style_bool(&self, sid: crate::engraver::style::Sid) -> bool {
        self.style.bool(sid)
    }

    /// Get the view mode.
    #[must_use]
    pub fn view_mode(&self) -> LayoutMode {
        self.config.view_mode
    }

    /// Check if invisible elements should be shown.
    #[must_use]
    pub fn show_invisible(&self) -> bool {
        self.config.show_invisible
    }
}

// endregion: --- Layout Context (Borrowed)

// region:    --- Layout Context Owned

/// Owned layout context that manages resource lifetimes.
///
/// Use this when you need to own the layout data (e.g., in tests or
/// when the context outlives the original data). It provides a borrowed
/// `LayoutContext` via `as_context()`.
///
/// # Example
///
/// ```ignore
/// let owned = LayoutContextOwned::new_minimal(MStyle::default());
/// let ctx = owned.as_context();
/// let sp = ctx.spatium();
/// ```
pub struct LayoutContextOwned {
    config: LayoutConfiguration,
    style: Arc<MStyle>,
    font: SMuFLFontOwned,
}

/// Owned SMuFL font data (font bytes + parsed font).
struct SMuFLFontOwned {
    // We store the font data inline since SMuFLFont borrows from it.
    // Using Box to keep the data stable in memory.
    #[allow(dead_code)]
    data: Option<Box<[u8]>>,
    // The parsed font - we use a raw pointer internally but expose safe API
    font: SMuFLFont<'static>,
}

impl SMuFLFontOwned {
    /// Create an empty font.
    fn empty() -> Self {
        Self {
            data: None,
            font: SMuFLFont::empty(),
        }
    }

    /// Get a reference to the font with appropriate lifetime.
    fn as_ref(&self) -> &SMuFLFont<'_> {
        // Safety: The font data lives as long as SMuFLFontOwned,
        // and we only hand out references with lifetime tied to &self.
        // The 'static lifetime in the inner font is an implementation detail.
        &self.font
    }
}

impl LayoutContextOwned {
    /// Create a new owned context with all data.
    #[must_use]
    pub fn new(config: LayoutConfiguration, style: MStyle, font: SMuFLFont<'static>) -> Self {
        Self {
            config,
            style: Arc::new(style),
            font: SMuFLFontOwned { data: None, font },
        }
    }

    /// Create a minimal owned context with just style information.
    ///
    /// This is useful for layout operations that only need spatium/style access
    /// without requiring a full Score or font.
    #[must_use]
    pub fn new_minimal(style: MStyle) -> Self {
        Self::new_minimal_arc(Arc::new(style))
    }

    /// Create a minimal owned context from a shared style.
    ///
    /// Preferred when the caller already holds an `Arc<MStyle>` — avoids
    /// cloning the style's underlying property vector.
    #[must_use]
    pub fn new_minimal_arc(style: Arc<MStyle>) -> Self {
        Self {
            config: LayoutConfiguration::default(),
            style,
            font: SMuFLFontOwned::empty(),
        }
    }

    /// Create a minimal owned context for testing.
    #[must_use]
    pub fn for_test(config: LayoutConfiguration, style: MStyle) -> Self {
        Self {
            config,
            style: Arc::new(style),
            font: SMuFLFontOwned::empty(),
        }
    }

    /// Get a borrowed context from this owned context.
    #[must_use]
    pub fn as_context(&self) -> LayoutContext<'_> {
        LayoutContext {
            config: &self.config,
            style: &self.style,
            font: self.font.as_ref(),
        }
        // NOTE: `&self.style` autoderefs Arc<MStyle> -> &MStyle
    }

    /// Get mutable access to the style.
    ///
    /// Uses `Arc::make_mut` for copy-on-write: cheap when the `Arc` is unique,
    /// clones the inner `MStyle` when shared.
    pub fn style_mut(&mut self) -> &mut MStyle {
        Arc::make_mut(&mut self.style)
    }

    /// Get mutable access to the configuration.
    pub fn config_mut(&mut self) -> &mut LayoutConfiguration {
        &mut self.config
    }
}

// endregion: --- Layout Context Owned

impl<'a> LayoutContext<'a> {
    /// Create a minimal layout context for testing.
    ///
    /// This constructor is only available in test builds and creates a context
    /// with a stub SMuFLFont reference. Use for layout tests that only need
    /// spatium/style access.
    #[cfg(test)]
    #[must_use]
    pub fn new_for_test(_config: LayoutConfiguration, style: &'a MStyle) -> Self {
        // Leak a default Config and SMuFLFont for a 'static-ish test context.
        let font = Box::leak(Box::new(SMuFLFont::empty()));
        let config = Box::leak(Box::new(LayoutConfiguration::new()));
        Self {
            config,
            style,
            font,
        }
    }
}

// endregion: --- Deprecated API

// region:    --- Test Helpers

/// Create a test context with static lifetime.
///
/// This helper is only available in tests and leaks memory intentionally
/// to provide `'static` lifetimes needed for test assertions.
#[cfg(test)]
pub fn test_context() -> LayoutContext<'static> {
    let config = Box::leak(Box::new(LayoutConfiguration::new()));
    let style = Box::leak(Box::new(crate::engraver::style::MStyle::default()));
    let font = Box::leak(Box::new(crate::engraver::fonts::SMuFLFont::empty()));
    LayoutContext::new(config, style, font)
}

/// Create a test context with custom configuration.
#[cfg(test)]
pub fn test_context_with_config(config: LayoutConfiguration) -> LayoutContext<'static> {
    let config = Box::leak(Box::new(config));
    let style = Box::leak(Box::new(crate::engraver::style::MStyle::default()));
    let font = Box::leak(Box::new(crate::engraver::fonts::SMuFLFont::empty()));
    LayoutContext::new(config, style, font)
}

/// Create a test context with custom style.
#[cfg(test)]
pub fn test_context_with_style(style: crate::engraver::style::MStyle) -> LayoutContext<'static> {
    let config = Box::leak(Box::new(LayoutConfiguration::new()));
    let style = Box::leak(Box::new(style));
    let font = Box::leak(Box::new(crate::engraver::fonts::SMuFLFont::empty()));
    LayoutContext::new(config, style, font)
}

// endregion: --- Test Helpers

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_configuration_default() {
        let config = LayoutConfiguration::default();
        assert_eq!(config.view_mode, LayoutMode::Page);
        assert!(!config.show_invisible);
        assert_eq!(config.note_head_width, 0.0); // Default derive sets to 0
    }

    #[test]
    fn test_layout_configuration_new() {
        let config = LayoutConfiguration::new();
        assert_eq!(config.view_mode, LayoutMode::Page);
        assert!(!config.show_invisible);
        assert_eq!(config.note_head_width, 1.6);
    }

    #[test]
    fn test_layout_state_default() {
        let state = LayoutState::default();
        assert_eq!(state.current_measure, None);
        assert_eq!(state.tick, 0);
        assert_eq!(state.measure_no, 0);
        assert!(!state.first_system);
    }

    #[test]
    fn test_layout_mode_is_linear() {
        assert!(!LayoutMode::Page.is_linear());
        assert!(LayoutMode::Horizontal.is_linear());
        assert!(LayoutMode::Vertical.is_linear());
        assert!(!LayoutMode::Float.is_linear());
    }

    #[test]
    fn test_layout_mode_is_paginated() {
        assert!(LayoutMode::Page.is_paginated());
        assert!(!LayoutMode::Horizontal.is_paginated());
        assert!(!LayoutMode::Vertical.is_paginated());
        assert!(LayoutMode::Float.is_paginated());
    }

    #[test]
    fn test_layout_mode_honors_breaks() {
        // Page honors both line and page breaks
        assert!(LayoutMode::Page.honors_line_breaks());
        assert!(LayoutMode::Page.honors_page_breaks());

        // Horizontal ignores all breaks
        assert!(!LayoutMode::Horizontal.honors_line_breaks());
        assert!(!LayoutMode::Horizontal.honors_page_breaks());

        // Vertical honors line breaks only (page breaks become line breaks)
        assert!(LayoutMode::Vertical.honors_line_breaks());
        assert!(!LayoutMode::Vertical.honors_page_breaks());

        // Float ignores all explicit breaks
        assert!(!LayoutMode::Float.honors_line_breaks());
        assert!(!LayoutMode::Float.honors_page_breaks());
    }

    #[test]
    fn test_layout_mode_names() {
        assert_eq!(LayoutMode::Page.name(), "Page View");
        assert_eq!(LayoutMode::Horizontal.name(), "Continuous Horizontal");
        assert_eq!(LayoutMode::Vertical.name(), "Continuous Vertical");
        assert_eq!(LayoutMode::Float.name(), "Reflow");
    }

    #[test]
    fn test_layout_context_owned_minimal() {
        let style = MStyle::default();
        let owned = LayoutContextOwned::new_minimal(style);
        let ctx = owned.as_context();

        // Should be able to access spatium
        let _sp = ctx.spatium();
    }

    #[test]
    fn test_layout_context_owned_for_test() {
        let config = LayoutConfiguration::new().with_view_mode(LayoutMode::Horizontal);
        let style = MStyle::default();
        let owned = LayoutContextOwned::for_test(config, style);
        let ctx = owned.as_context();

        assert_eq!(ctx.view_mode(), LayoutMode::Horizontal);
    }
}

// endregion: --- Tests
