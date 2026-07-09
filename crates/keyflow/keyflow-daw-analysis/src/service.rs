//! `MidiCharts` — chart/chord analysis over a connected DAW.
//!
//! All-async by design. The impl ([`crate::backend::KeyflowMidiAnalysis`])
//! composes 4+ async calls into the `daw` facade per request (project →
//! track → take → MIDI notes → markers → tempo map), so the trait
//! cannot reasonably be sync without `block_on` inside every method.
//!
//! `#[architect::rpc]` on an all-async trait emits a pass-through
//! `#[vox::service]` decoration — same vox client/dispatcher names as
//! before, plus the `serve`/`layer`/`descriptor` aliases that mirror
//! the sync-trait modules in `keyflow-proto`.

use crate::types::{MidiChartData, MidiChartRequest};

#[architect::rpc]
pub trait MidiCharts {
    /// Cheap fingerprint of the source MIDI + markers. Doesn't run chord
    /// detection — useful for "did anything change?" cache invalidation.
    async fn source_fingerprint(&self, request: MidiChartRequest) -> Result<String, String>;

    /// Run keyflow chord detection + chart text generation. Heavier than
    /// `source_fingerprint`.
    async fn generate_chart_data(&self, request: MidiChartRequest)
    -> Result<MidiChartData, String>;
}
