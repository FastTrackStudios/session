//! REAPER action handlers for transport playback — `toggle_playback`
//! and `toggle_song_loop`.
//!
//! Sibling to `setlist_actions.rs`: runs synchronously on REAPER's main
//! thread (where the action callback fires), calling `D`'s sync
//! `Transport` methods directly — no `main_thread::query` bounce, no
//! Tokio runtime.
//!
//! ## Why only these two, and not the other 6 gap ids
//!
//! `session_actions` (`lib.rs`) also declares `SMART_NEXT`,
//! `SMART_PREVIOUS`, `NEXT_SONG`, `PREVIOUS_SONG`, `NEXT_SECTION`,
//! `PREVIOUS_SECTION`. Those all route through
//! `SetlistServiceImpl::go_to_song_impl` / `go_to_section_impl`
//! (`setlist_service/navigation.rs`), which depend on
//! `ensure_song_hydrated` (`setlist_service/hydration.rs`) — a real
//! async rebuild path (chart/fingerprint extraction via
//! `keyflow_daw_analysis::MidiChartsClient`, semaphore-bounded
//! concurrency) guarded by a 5s `architect::platform::timeout`, plus multiple
//! `daw_reaper::main_thread::query(...).await` bounces batched across
//! several steps.
//!
//! Collapsing that to a synchronous REAPER action callback has no safe
//! option: blocking on the async work (`block_on`) from the callback
//! risks deadlock, since the callback already runs ON the main thread
//! `main_thread::query` needs to reach — and a "sync fast path, silently
//! no-op on cache miss" variant would change behavior for songs that
//! haven't been chart-hydrated yet, which is a real product regression,
//! not a refactor. Fixing this properly means giving `SetlistServiceImpl`
//! a genuine synchronous fast-path (no hydration, no timeout) for
//! main-thread callers — a larger, separate change. These 6 stay
//! RPC-only (reachable via `SetlistService::next_song` etc. over vox,
//! from the CLI/desktop/web clients) and are NOT reachable as REAPER
//! named commands today; that was already true before this migration
//! (`daw_module.rs`'s dispatch chain never routed them to a sync
//! handler either — see `docs/handoff-session-thread-safety.md`).
//!
//! `toggle_playback` / `toggle_song_loop` don't have this problem:
//! `playback.rs`'s implementations are a single sync `Transport` call
//! plus a cheap in-memory cache read, no hydration, no timeout.

use std::sync::Arc;

use architect::action::ActionBackend;
use daw::service::ProjectContext;
use daw::service::transport::service::Transport as TransportService;

/// Bridges `toggle_playback` / `toggle_song_loop` onto
/// `#[architect::actions]`. Forwards to the same `Transport` calls
/// `SetlistServiceImpl::toggle_playback_impl` / `toggle_song_loop_impl`
/// make (`setlist_service/playback.rs`) — no behavior change for the
/// loop toggle.
///
/// `toggle_playback` uses `ProjectContext::Current` rather than the RPC
/// path's "cached active song" lookup: a REAPER action fires in the
/// context of whatever project tab is focused, so operating on
/// `Current` is the correct behavior for a hotkey (the cached-song
/// indirection exists to serve remote RPC callers, who have no
/// "current tab" of their own — not applicable here).
pub struct PlaybackActionsImpl<D> {
    daw: D,
}

#[architect::actions(namespace = "FTS_SESSION")]
pub trait PlaybackActions {
    #[action(
        description = "Toggle play/pause for the current project",
        category = "Transport",
        group = "Transport"
    )]
    fn toggle_playback(&self);

    #[action(
        description = "Toggle looping for the current project",
        category = "Transport",
        group = "Transport"
    )]
    fn toggle_song_loop(&self);
}

impl<D> PlaybackActions for PlaybackActionsImpl<D>
where
    D: TransportService,
{
    fn toggle_playback(&self) {
        if let Err(e) = self.daw.play_pause(ProjectContext::Current) {
            tracing::warn!("[session] toggle_playback action: failed to toggle: {e}");
        }
    }

    fn toggle_song_loop(&self) {
        if let Err(e) = self.daw.toggle_loop(ProjectContext::Current) {
            tracing::warn!("[session] toggle_song_loop action: failed to toggle: {e}");
        }
    }
}

/// Registers `toggle_playback` / `toggle_song_loop` with `backend`. Call
/// once at module init, alongside `setlist_actions::register_actions`.
pub fn register_actions<D, B>(backend: &B, daw: D)
where
    D: TransportService + Send + Sync + 'static,
    B: ActionBackend + ?Sized,
{
    register_playback_actions_actions(backend, Arc::new(PlaybackActionsImpl { daw }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generated ids must reproduce the exact REAPER command-id strings
    /// `session_actions` (`lib.rs`'s `actions_proto::define_actions!`
    /// block) already uses — anything already wired into a keybinding
    /// or menu depends on these exact strings.
    #[test]
    fn ids_match_existing_reaper_command_convention() {
        let ids: Vec<_> = PlaybackActionsActions::all()
            .iter()
            .map(|m| (m.method_name, m.id))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("toggle_playback", "FTS_SESSION_TOGGLE_PLAYBACK"),
                ("toggle_song_loop", "FTS_SESSION_TOGGLE_SONG_LOOP"),
            ]
        );
    }
}
