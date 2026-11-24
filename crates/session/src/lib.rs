//! The session: the primitive everything agrees on.
//!
//! - [`time`]: positions, tempo, meters and the tempo map (from `daw-proto`).
//! - [`TempoMap`], [`Time`], [`Marker`], [`Region`]: the song structure.
//! - [`Lane`], [`Item`]: the generic timeline; a kind and a [`Value`] the
//!   session does not interpret.
//! - [`ExtensionData`]: each extension's namespace, merged as it chooses.
//! - [`Transport`]: transport as data.
//! - [`sync`] (feature `sync`): playback sync with realtime guarantees.
//! - [`doc`] (feature `architect`): the session as an Architect (Loro)
//!   document.
//! - [`service`] (feature `service`): what a runtime extension may do to
//!   the session, answered the same way by every host.
//! - [`extension`] (feature `extension`): that, as an Architect extension
//!   service, for the provider's actions.
//! - [`host`] (feature `vox`): that, served as `fts.session` over vox, the
//!   caller taken from the call's principal.
//!
//! See `docs/design/session.md`.

pub mod ext;
pub mod id;
pub mod presence;
pub mod section_kinds;
mod session;
pub mod time;
pub mod timeline;
pub mod transport;
pub mod value;

#[cfg(feature = "architect")]
pub mod changes;
#[cfg(feature = "architect")]
pub mod doc;
#[cfg(feature = "extension")]
pub mod extension;
#[cfg(all(feature = "architect", not(target_arch = "wasm32")))]
pub mod file;
#[cfg(feature = "vox")]
pub mod host;
#[cfg(all(feature = "lead", not(target_arch = "wasm32")))]
pub mod lead;
#[cfg(feature = "service")]
pub mod service;
#[cfg(feature = "sync")]
pub mod sync;

pub use ext::{Entry, ExtensionData};
pub use id::{ExtensionId, ItemId, LaneId, MarkId};
pub use session::Session;
pub use timeline::{BarBeat, Item, Lane, Marker, Region, TempoMap, Time};
pub use transport::Transport;
pub use value::Value;
