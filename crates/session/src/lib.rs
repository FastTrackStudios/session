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
//!
//! See `docs/design/session.md`.

pub mod ext;
pub mod id;
mod session;
pub mod time;
pub mod timeline;
pub mod transport;
pub mod value;

#[cfg(feature = "architect")]
pub mod doc;
#[cfg(feature = "sync")]
pub mod sync;

pub use ext::{Entry, ExtensionData};
pub use id::{ExtensionId, ItemId, LaneId, MarkId};
pub use session::Session;
pub use timeline::{BarBeat, Item, Lane, Marker, Region, TempoMap, Time};
pub use transport::Transport;
pub use value::Value;
