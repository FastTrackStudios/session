//! Page and system layout descriptors.
//!
//! Plain data types describing how systems are arranged into pages, consumed
//! by the chart layout pipeline (`layout::chart`). This module previously also
//! held a `Score`-model layout engine (`LayoutEngine`/`layout_score`); that
//! path was superseded by the chart-centric pipeline and has been removed.

/// Layout information for a single page.
#[derive(Debug, Clone)]
pub struct PageLayout {
    /// Page number (1-indexed)
    pub number: u32,
    /// X offset in the scene (for multi-page layouts)
    pub x_offset: f64,
    /// Y offset in the scene (for multi-page layouts)
    pub y_offset: f64,
    /// Page dimensions
    pub width: f64,
    pub height: f64,
    /// Systems on this page
    pub systems: Vec<SystemLayout>,
    /// Page margins
    pub margins: PageMargins,
}

/// Layout information for a single system (line of music).
#[derive(Debug, Clone)]
pub struct SystemLayout {
    /// System index (0-indexed within score)
    pub index: usize,
    /// Y position on page (from top)
    pub y: f64,
    /// System width
    pub width: f64,
    /// System height
    pub height: f64,
    /// Measure indices in this system
    pub measure_indices: Vec<usize>,
}

/// Page margins.
#[derive(Debug, Clone, Copy)]
pub struct PageMargins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for PageMargins {
    fn default() -> Self {
        Self {
            top: 50.0,
            right: 50.0,
            bottom: 50.0,
            left: 50.0,
        }
    }
}
