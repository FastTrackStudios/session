//! Lyrics-to-audio forced alignment + pluggable stem separation for Keyflow.
//!
//! Given a song's audio and its *known* lyrics (from a `.kf` chart), produce a
//! [`timing::TimingMap`] sidecar: per-word start/end times with a confidence
//! rating. That sidecar is the single source of *when*, kept separate from the
//! chart's single source of *what* — slides, karaoke, and MIDI lyric events are
//! all derivations of `chart + sidecar`.
//!
//! ## Layers
//! * [`audio`] — WAV load / resample (mono `f32`).
//! * [`separator`] — pluggable stem separation ([`separator::StemSeparator`]);
//!   isolating vocals first is the biggest alignment-quality win.
//! * [`align`] — CTC forced alignment: the pure-Rust [`align::trellis`] plus an
//!   [`align::EmissionModel`] acoustic model (wav2vec2 via ONNX behind `onnx`).
//! * [`bench`] — SI-SDR / SDR metrics for benchmarking separators head-to-head.
//! * [`timing`] — the sidecar format and the confidence→[`timing::Rating`] gate.
//! * [`models`] — registry + on-demand download of weights (never in the binary).
//! * [`pipeline`] — wires it together.
//!
//! ## Build profiles
//! * default (lean): everything except embedded inference. `kf sync` runs via an
//!   external `demucs` + the pure-Rust trellis. No native deps.
//! * `onnx`: embed wav2vec2 + HT-Demucs via ONNX Runtime — no external tools.
//! * `download`: fetch model weights (`kf models pull`).

pub mod align;
pub mod audio;
pub mod bench;
pub mod error;
pub mod karaoke;
pub mod lanes;
pub mod models;
#[cfg(feature = "onnx")]
pub mod onnx_util;
pub mod pipeline;
pub mod separator;
pub mod timing;
#[cfg(feature = "whisper")]
pub mod whisper;

pub use align::{EmissionModel, WordTiming, align_words};
pub use audio::AudioBuffer;
pub use error::{Result, SyncError};
pub use pipeline::{SectionLyrics, prepare_vocals, run_alignment};
pub use separator::{StemSelection, StemSeparator, Stems};
pub use timing::{Rating, RatingThresholds, TimingMap, WordEntry};
