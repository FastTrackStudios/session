// Lint debt: workspace flipped dead_code/unused to warn (task cleanup);
// this crate predates that — burn down separately.
#![allow(dead_code, unused)]

//! Standalone DAW Implementation
//!
//! This is a minimal DAW implementation that runs standalone without any external DAW.
//! It serves as both the reference implementation and the mock for testing.
//!
//! The implementations in this module (`StandaloneTransport`, `StandaloneProject`) can be
//! used directly in tests without spawning a separate cell process.
//!
//! ## Mock Data
//!
//! The standalone implementation includes mock data for testing:
//! - **3 songs** with markers (SONGSTART/SONGEND)
//! - **Sections** as regions (Intro, Verse, Chorus, Bridge, Outro, Solo)
//! - **Tempo/time signature changes** throughout the timeline
//!
//! This allows testing the full fts-control-web experience without a real DAW.

#![deny(unsafe_code)]
// Stylistic lints kept tolerant on this crate; standalone is the
// reference/mock implementation and uses ergonomic patterns
// (`let mut x = Default::default(); x.field = ...`) extensively.
#![allow(
    clippy::field_reassign_with_default,
    clippy::manual_clamp,
    clippy::needless_range_loop
)]

mod action_registry;
mod audio_accessor;
#[cfg(any(
    feature = "audio",
    feature = "decode",
    feature = "clap-host",
    feature = "vst3-host"
))]
pub mod audio_engine;
mod audio_engine_svc;
mod automation;
mod automation_touch;
#[cfg(feature = "rpp-save")]
pub mod save;
mod stretch_marker;
#[cfg(any(feature = "audio", feature = "decode"))]
mod take_reader;
pub use automation_touch::TouchableParam;
mod batch;
#[cfg(feature = "bootstrap")]
pub mod bootstrap;
mod dawfile_service;
mod diagnostics;
mod event_bus;
mod ext_state;
mod fx;
mod fx_chains;
mod fx_params;
mod health;
mod input;
mod item;
mod live_midi;
mod marker;
pub mod media_bay;
/// Seed a project with real on-disk media (grouped tracks + mmap'd stems).
#[cfg(any(feature = "audio", feature = "decode"))]
pub mod media_seed;
pub mod metering;
mod midi;
mod peak;
/// Persistent source-level peaks: REAPER `.reapeaks` sidecars next to
/// each on-disk media file, shared with REAPER where projects overlap.
#[cfg(all(
    feature = "reapeaks",
    any(feature = "audio", feature = "decode"),
    not(target_arch = "wasm32")
))]
mod peak_store;
pub(crate) mod platform;
pub mod plugin;
mod plugin_loader;
mod position_conversion;
mod project;
#[cfg(any(feature = "rpp-project", feature = "rpp-project-wasm"))]
pub mod project_loader;
mod region;
mod resource;
mod routing;
mod routing_sync;
pub mod rpp_state;
mod screenset;
mod services;
mod shared_state;
pub mod sync;
mod take;
mod tempo_map;
mod toolbar;
mod track;
mod transport;
pub mod transport_engine;
mod ui;
mod window_geometry;
mod window_manager;

// All per-service impls are on `Standalone` directly post-port.
// Old `Standalone*` per-service structs retired.
pub use project::project_guids;
pub use shared_state::SharedProjectState;
pub use sync::Standalone;

/// REAPER `.reapeaks` peak-cache reading (the `reapeaks` feature) — draw
/// waveforms from REAPER's own peak mipmaps.
#[cfg(feature = "reapeaks")]
pub use dawfile_reaper::reapeaks;
