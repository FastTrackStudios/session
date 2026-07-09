//! Keyflow MIDI import helpers.
//!
//! This crate re-exports the MIDI import APIs from `engraver` so
//! MIDI tooling can be pulled in without the full keyflow crate.

pub use keyflow_proto as proto;

pub mod guide;
pub mod import;

use std::path::Path;

use keyflow_proto::Chart;

pub fn generate_chart_text_from_midi_bytes(bytes: &[u8]) -> Result<String, String> {
    let midi = import::MidiFile::parse(bytes).map_err(|e| e.to_string())?;
    let config = import::MidiChartConfig::default();
    Ok(import::generate_chart_text(&midi, &config))
}

pub fn generate_chart_text_from_midi_path(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    generate_chart_text_from_midi_bytes(&bytes)
}

pub fn parse_midi_bytes(bytes: &[u8]) -> Result<Chart, String> {
    let text = generate_chart_text_from_midi_bytes(bytes)?;
    keyflow_text::api::parse::chart(&text)
}

pub fn parse_midi_path(path: &Path) -> Result<Chart, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    parse_midi_bytes(&bytes)
}
