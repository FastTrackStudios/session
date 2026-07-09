//! Keyflow MIDI chart service — chord detection + chart text generation
//! over the public daw client API.
//!
//! # Architecture
//!
//! `daw` is intentionally domain-neutral — it knows about projects,
//! tracks, items, takes, raw MIDI notes, but not about chord symbols,
//! chart text, or any keyflow concept. Chord detection and chart-text
//! generation live in keyflow's domain, so the request/response types
//! and the service trait are owned by *this* crate, not by daw.
//!
//! Consumers wanting chart data depend on `keyflow-daw-analysis` (or a
//! types-only spinoff) and call `MidiCharts` over the same RPC
//! channel that hosts the daw services. fts-extensions registers a
//! `KeyflowMidiAnalysis` impl with the bridge's `RoutedHandler` at
//! startup.
//!
//! ```text
//!                    daw  (public facade — DAW domain only)
//!                     ▲                ▲
//!                     │                │
//!         keyflow-daw-analysis ◀── keyflow (chord/chart logic)
//!                     ▲
//!                     │
//!              fts-extensions  (composition root)
//! ```
//!
//! # Module shape
//!
//! - [`types`] : wire payload + result structs ([`MidiChartRequest`],
//!   [`MidiChartData`], [`DetectedChord`]).
//! - [`service`] : the [`MidiCharts`] trait, decorated with
//!   `#[architect::rpc]`. All-async — see the trait docs for the
//!   justification.
//! - [`backend`] : [`KeyflowMidiAnalysis`], the daw-backed impl.

pub mod backend;
pub mod service;
pub mod types;

pub use backend::KeyflowMidiAnalysis;
pub use service::MidiCharts;
pub use types::{DetectedChord, MidiChartData, MidiChartRequest};

// vox-emitted names. All-async traits route through `#[vox::service]`
// directly, so the client/dispatcher/descriptor names live on the user
// trait (no `Rpc` suffix). The `Rpc`-suffixed aliases are also emitted
// by architect so the names match the sync-trait modules in
// `keyflow-proto`.
#[cfg(feature = "vox")]
pub use service::{
    MidiChartsClient as Client, MidiChartsClient, MidiChartsDispatcher as Dispatcher,
    MidiChartsDispatcher, midi_charts_rpc_service_descriptor as descriptor,
    midi_charts_service_descriptor, serve,
};
