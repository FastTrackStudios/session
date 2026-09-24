//! The Session watch app's guide feed.
//!
//! The watch (apps/session-watch) shows where the band is in the song and
//! taps the beat on the wrist, in time with the tracks. It computes none
//! of it: this crate does, on the iPhone, from the code the desktop's guide
//! runs on, and relays it over `WatchConnectivity` as a few seconds of beats
//! with every instant already in the watch's own clock.
//!
//! - [`timeline`] — a song's beats, bars, sections and count-in.
//! - [`relay`] — the leader's playhead and three clocks (Task's, the
//!   phone's, the watch's) folded into a [`WatchGuideFeed`](session_proto::watch::WatchGuideFeed).
//! - [`driver`] — the relay on its own thread, over a [`driver::WatchLink`].
//! - `link` (iOS, feature `link`) — the `WatchConnectivity` [`driver::WatchLink`].

pub mod driver;
#[cfg(all(target_os = "ios", feature = "link"))]
pub mod link;
pub mod relay;
pub mod timeline;
pub mod wire;

// The positions a lead is stamped in, and the per-buffer backends that
// stamp them — for a host building a `Lead` from its own engine.
pub use daw_transport_sync;
pub use driver::{Relay, Snapshot, SongSnapshot, WatchLink};
pub use relay::{Lead, RelayInput, SongRef, WatchRelay};
pub use session_guide::midi::TempoMark;
pub use timeline::GuideTimeline;
