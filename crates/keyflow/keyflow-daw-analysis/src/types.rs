//! Wire types for `MidiCharts` — request payload + result shapes.

use facet::Facet;

/// Request a chart for a project + optional track filter.
///
/// `project_guid` selects the project (or the current project when
/// `None`). `track_tag` is a case-insensitive substring matched against
/// track names — `None` picks the first track in the project.
#[derive(Clone, Debug, Facet)]
pub struct MidiChartRequest {
    pub project_guid: Option<String>,
    pub track_tag: Option<String>,
}

impl MidiChartRequest {
    pub fn new(project_guid: Option<String>, track_tag: Option<String>) -> Self {
        Self {
            project_guid,
            track_tag,
        }
    }
}

/// One detected chord, with PPQ-aligned start/end ticks.
#[derive(Clone, Debug, PartialEq, Facet)]
pub struct DetectedChord {
    pub symbol: String,
    pub start_ppq: i64,
    pub end_ppq: i64,
    pub root_pitch: u8,
    pub velocity: u8,
}

/// Chart-render output: keyflow-format text plus the underlying chord events.
#[derive(Clone, Debug, PartialEq, Facet)]
pub struct MidiChartData {
    pub source_track_name: String,
    /// Stable hash of the source MIDI + markers — clients invalidate caches
    /// when this changes.
    pub source_fingerprint: String,
    pub chart_text: String,
    pub chords: Vec<DetectedChord>,
}
