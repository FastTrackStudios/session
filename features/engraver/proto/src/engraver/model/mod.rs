//! Shared notation primitives used by the chart layout pipeline.
//!
//! This module once held a full MuseScore-style score model
//! (`Score`/`Part`/`Voice`/`Measure`/`Note`/…). That model was superseded by
//! the chart-centric pipeline, which operates directly on
//! `keyflow_proto::Chart`. Only the small, self-contained leaf types still
//! consumed by layout remain here:
//!
//! - [`DurationKind`]/[`Duration`] — note/rest duration values (quantize + notation)
//! - [`PaperSize`]/[`Margins`] — page geometry presets (chart layout config + style)
//! - [`NoteHead`] — notehead glyph selection (melody/rhythm rendering)
//! - [`Pitch`]/[`PitchClass`]/[`Octave`] — melody pitch helpers (chart layout)
//! - [`ElementId`] — opaque element handle used by collision/skyline layout

mod duration;
mod note;
mod page_style;
mod pitch;

pub use duration::{Duration, DurationKind};
pub use note::NoteHead;
pub use page_style::{Margins, PaperSize};
pub use pitch::{Octave, Pitch, PitchClass};

use serde::{Deserialize, Serialize};

/// Unique identifier for a music element (for layout references).
///
/// Used by the layout system to track elements during positioning
/// and collision detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementId(pub usize);

impl ElementId {
    /// Create a new element ID.
    #[must_use]
    pub const fn new(id: usize) -> Self {
        Self(id)
    }
}
