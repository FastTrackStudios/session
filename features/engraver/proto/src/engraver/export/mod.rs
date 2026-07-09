//! Export module for generating output from the scene graph.
//!
//! This module provides exporters for various output formats:
//! - SVG for vector graphics with semantic IDs
//! - PDF for high-quality print output (WASM compatible via printpdf)
//!
//! All exporters take a SceneNode and produce output in the target format.

// PDF export pulls in the printpdf/svg2pdf/usvg stack; only available with the
// full `engraver` feature. SVG export is pure-CPU and available under `svg`.
#[cfg(feature = "engraver")]
pub mod pdf;
pub mod svg;

#[cfg(feature = "engraver")]
pub use pdf::{PdfExportConfig, PdfExportError, PdfSerializer};
pub use svg::{SvgExportConfig, SvgSerializer};
