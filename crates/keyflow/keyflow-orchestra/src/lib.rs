//! Keyflow Orchestra — MusicXML score → articulated sample-library MIDI.
//!
//! Rust port of the FTS CSS Orchestrator engine (`reference-data/css-orchestrator/
//! css_engine.lua`): a pure, DAW-agnostic pipeline that turns a MusicXML score
//! into channel-split notes plus CC curves (dynamics, vibrato, keyswitches)
//! tuned for Cinematic Studio Strings / Brass / Woodwinds — with an instrument-
//! profile system so unknown patches degrade to safe generic output.
//!
//! Everything downstream of parsing works in **quarter-notes (QN)**; writers
//! (RPP, SMF, live Reaper) convert QN → PPQ/seconds at the edge.
//!
//! Layers:
//! - [`score`] — score analysis primitives: parse `.mxl`/`.musicxml` into a
//!   QN-domain [`score::Score`] (parts, notes with voice/slur/articulation
//!   detail, dynamics, tempo map, meters, markings) plus instrumentation /
//!   articulation-inventory reports.
//! - [`profile`] — instrument profiles (CSS/CSW/CSB/generic/harp) and
//!   track-name family detection.
//! - [`config`] — the engine tunables (port of `defaultConfig`).
//! - [`engine`] — the processing pipeline: ties → channels → legato →
//!   phrasing/micro-dynamics → velocity → timing compensation → note + CC
//!   emission.

pub mod analysis;
pub mod config;
pub mod engine;
pub mod mirror;
pub mod profile;
pub mod score;

pub use config::Config;
pub use engine::{PartOutput, process_part};
pub use mirror::{MidiNote, MirrorOutput, mirror_part};
pub use profile::{Profile, ProfileKind, detect_profile};
pub use score::Score;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("musicxml parse error: {0}")]
    Parse(String),
}
