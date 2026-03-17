//! SetlistService implementation
//!
//! Split into focused sub-modules:
//! - `hydration` — Song hydration, chart extraction, caching
//! - `navigation` — Song/section navigation and seeking
//! - `playback` — Play, pause, stop, loop controls
//! - `build` — Setlist assembly from open DAW projects
//! - `polling` — Active indices calculation, transport streaming, subscriptions

mod build;
mod combined;
pub mod file_watcher;
mod hydration;
pub mod live_daw_sync;
pub mod live_sync;
mod navigation;
mod playback;
mod polling;
pub mod position_sync;

use crate::cache::Cache;
use crate::event_bus::{EventBus, WatchBus};
use moire::sync::RwLock;
use session_proto::{ActiveIndices, QueuedTarget, Setlist, Song, SongChartHydration};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use position_sync::PositionSyncBridge;

pub(crate) use hydration::SongCacheEntry;

pub(crate) const HYDRATION_CONCURRENCY: usize = 12;
pub(crate) const MIDI_TRACK_TAG: &str = "CHORDS";
pub(crate) const ACTIVE_HYDRATION_POLL_MS: u64 = 2000;
pub(crate) const ACTIVE_HYDRATION_TICK_MS: u64 = 500;
pub(crate) const ACTIVE_HYDRATION_POLL_MAX_MS: u64 = 15000;
pub(crate) const ACTIVE_INDICES_PROGRESS_EMIT_MS: u64 = 100;
pub(crate) const CHART_REFRESH_FALLBACK_POLL_MS: u64 = 5000;
pub(crate) const PROJECT_SWITCH_DEBOUNCE_MS: u64 = 900;
pub(crate) const TRANSPORT_TIME_EPSILON_SECS: f64 = 0.002;
pub(crate) const TRANSPORT_PROGRESS_EPSILON: f64 = 0.0005;

/// Implementation of SetlistService
#[derive(Clone)]
pub struct SetlistServiceImpl {
    /// Current setlist state
    pub(crate) setlist: Arc<RwLock<Option<Setlist>>>,
    /// Currently active song ID (cached locally to avoid RPC calls)
    /// Using ID instead of index ensures stability when songs are reordered
    pub(crate) active_song_id: Arc<RwLock<Option<String>>>,
    /// Cached active indices (updated by polling loop at 60Hz)
    /// Used for instant navigation without RPC calls
    pub(crate) cached_indices: Arc<RwLock<ActiveIndices>>,
    /// Queued navigation target (flashes in UI until transport reaches it)
    /// Only one target can be queued at a time
    pub(crate) queued_target: Arc<RwLock<Option<QueuedTarget>>>,
    /// Monotonic setlist revision stream for subscribers (hydration/build updates)
    pub(crate) setlist_update_bus: Arc<WatchBus<u64>>,
    /// Last setlist revision value
    pub(crate) setlist_revision: Arc<AtomicU64>,
    /// Full-song cache keyed by project GUID
    pub(crate) song_cache: Cache<String, SongCacheEntry>,
    /// Delta updates for individual hydrated songs
    pub(crate) hydration_bus: Arc<EventBus<(usize, Song)>>,
    /// Delta updates for hydrated chart payloads
    pub(crate) chart_hydration_bus: Arc<EventBus<(usize, SongChartHydration)>>,
    /// Chart payload cache keyed by project GUID
    pub(crate) chart_cache: Cache<String, SongChartHydration>,
    /// Whether the connected DAW backend supports source_fingerprint().
    /// None = unknown, Some(true) = supported, Some(false) = unsupported.
    pub(crate) fingerprint_method_supported: Arc<RwLock<Option<bool>>>,
    /// Last fallback chart refresh attempt when fingerprint API is unavailable.
    pub(crate) last_chart_refresh_attempt: Cache<String, Instant>,
    /// Monotonic build generation to cancel stale background hydration tasks
    pub(crate) build_generation: Arc<AtomicU64>,
    /// Bidirectional position sync between song tabs and setlist tab.
    /// None until a combined setlist is generated.
    pub(crate) position_sync: Arc<RwLock<Option<PositionSyncBridge>>>,
}

impl SetlistServiceImpl {
    pub fn new() -> Self {
        Self {
            setlist: Arc::new(RwLock::new("session.setlist", None)),
            active_song_id: Arc::new(RwLock::new("session.setlist.active_song_id", None)),
            cached_indices: Arc::new(RwLock::new(
                "session.setlist.cached_indices",
                ActiveIndices::default(),
            )),
            queued_target: Arc::new(RwLock::new("session.setlist.queued_target", None)),
            setlist_update_bus: Arc::new(WatchBus::new("session.setlist.updates", 0_u64)),
            setlist_revision: Arc::new(AtomicU64::new(0)),
            song_cache: Cache::named("session.setlist.song_cache"),
            hydration_bus: Arc::new(EventBus::new("session.setlist.hydration", 1024)),
            chart_hydration_bus: Arc::new(EventBus::new("session.setlist.chart_hydration", 1024)),
            chart_cache: Cache::named("session.setlist.chart_cache"),
            fingerprint_method_supported: Arc::new(RwLock::new(
                "session.setlist.fingerprint_supported",
                None,
            )),
            last_chart_refresh_attempt: Cache::named("session.setlist.chart_refresh_attempts"),
            build_generation: Arc::new(AtomicU64::new(0)),
            position_sync: Arc::new(RwLock::new("session.setlist.position_sync", None)),
        }
    }

    /// Get the cached active indices (updated by polling loop, no RPC calls)
    pub(crate) async fn get_cached_indices(&self) -> ActiveIndices {
        self.cached_indices.read().await.clone()
    }

    /// Update cached indices (called by polling loop)
    pub(crate) async fn set_cached_indices(&self, indices: ActiveIndices) {
        *self.cached_indices.write().await = indices;
    }

    /// Set a queued navigation target
    pub(crate) async fn queue_target(&self, target: QueuedTarget) {
        *self.queued_target.write().await = Some(target);
    }

    /// Clear the queued navigation target
    pub(crate) async fn clear_queued_target(&self) {
        *self.queued_target.write().await = None;
    }

    /// Get the current queued target
    pub(crate) async fn get_queued_target(&self) -> Option<QueuedTarget> {
        self.queued_target.read().await.clone()
    }

    /// Get a specific song by index (internal helper)
    pub(crate) async fn get_song_internal(&self, index: usize) -> Option<Song> {
        let setlist = self.setlist.read().await;
        setlist.as_ref()?.songs.get(index).cloned()
    }

    /// Get a specific song by ID (internal helper)
    pub(crate) async fn get_song_by_id(&self, id: &str) -> Option<Song> {
        let setlist = self.setlist.read().await;
        let setlist = setlist.as_ref()?;
        setlist
            .songs
            .iter()
            .find(|song| song.id.as_str() == id)
            .cloned()
    }

    /// Get the active song from cached local state (no RPC calls)
    pub(crate) async fn get_cached_active_song(&self) -> Option<Song> {
        let song_id = self.active_song_id.read().await.clone();
        let song_id = song_id?;
        self.get_song_by_id(&song_id).await
    }

    /// Set the active song by ID (called when navigating)
    pub(crate) async fn set_active_song_id(&self, id: &str) {
        *self.active_song_id.write().await = Some(id.to_string());
    }

    pub(crate) fn notify_setlist_changed(&self) {
        let revision = self.setlist_revision.fetch_add(1, Ordering::SeqCst) + 1;
        self.setlist_update_bus.send(revision);
    }
}
