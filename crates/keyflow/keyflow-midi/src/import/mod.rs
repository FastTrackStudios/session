//! MIDI import and chart-text generation.

mod midi_chart_builder;
mod midi_import;
mod pitch;

pub use midi_chart_builder::{MelodyGrid, MidiChartConfig, generate_chart_text};
pub use midi_import::{
    ChordMarker, MarkerEvent, MarkerType, MidiFile, MidiImportConfig, MidiNote, MidiTrack,
    MusicalPosition, PushPull, PushPullAmount, RhythmElement, SectionMarker, SectionType,
    TempoEvent, TimeSignatureEvent, format_duration_suffix, format_measure_rhythm, format_rest,
    generate_measure_rhythm, normalize_chord_name,
};

/// Result type for MIDI import.
pub type Result<T> = core::result::Result<T, Error>;

/// MIDI import errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// MIDI parse error.
    MidiParse(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MidiParse(err) => write!(f, "MIDI parse error: {err}"),
        }
    }
}

impl std::error::Error for Error {}
