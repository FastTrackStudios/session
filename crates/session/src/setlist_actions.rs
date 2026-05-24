//! REAPER action handlers for setlist-level operations.
//!
//! The `BUILD_SETLIST` action ID has been registered in
//! `session_actions!` for a while, but `daw_module::actions` walks a
//! chain of `*_actions::action_for_id` lookups (keyflow, auto_color,
//! preroll, mode, take_ranking, record) and *none* of those owned the
//! setlist domain. Hitting the hotkey in REAPER fell through to the
//! "No DAW handler registered" log message — the action was a no-op.
//!
//! This module fills the gap. It:
//!
//! 1. Holds a singleton `Arc<dyn SetlistDispatcher>` registered at
//!    mount time so the action handler runs against the same in-memory
//!    setlist the RPC service exposes.
//! 2. Provides the `action_for_id` / `dispatch` shape every other
//!    `*_actions` module uses, so wiring is one line in
//!    `daw_module::actions`.
//!
//! Why a trait-object singleton rather than a concrete
//! `SetlistServiceImpl<D>`: the session crate is generic over a DAW
//! backend `D`, but the action handler can't be generic (it ends up
//! in a `Box<dyn Fn()>` cell on the REAPER side). Erasing to a small
//! `SetlistDispatcher` trait keeps the static type-free.

use std::sync::Arc;
use std::sync::OnceLock;

use crate::setlist_service::SetlistServiceImpl;

/// Type-erased "what to do when a setlist action fires" handle.
/// Implemented by `SetlistServiceImpl<D>` for any `D: Send + Sync +
/// Clone + 'static` that satisfies the service's own bounds.
pub trait SetlistDispatcher: Send + Sync + 'static {
    /// Trigger `build_from_open_projects` on this dispatcher's setlist
    /// service. Synchronous from the caller's POV — bounces work onto
    /// the moire runtime so the REAPER main thread keeps pumping.
    fn trigger_build(&self);
}

impl<D> SetlistDispatcher for SetlistServiceImpl<D>
where
    D: Clone + daw::service::Projects + Send + Sync + 'static,
{
    fn trigger_build(&self) {
        // Spawn on the moire runtime so we don't block REAPER's main
        // thread (action handlers run synchronously on it). Call the
        // `_impl` variant directly — going through the trait method
        // dispatches via Vox in some setups; the impl is the work.
        let svc = self.clone();
        moire::task::spawn(async move {
            match svc.build_from_open_projects_impl().await {
                Ok(()) => tracing::info!("[session] build_setlist action completed"),
                Err(e) => tracing::warn!("[session] build_setlist action failed: {e:?}"),
            }
        });
    }
}

static SETLIST_DISPATCHER: OnceLock<Arc<dyn SetlistDispatcher>> = OnceLock::new();

/// Stash the shared SetlistServiceImpl so the action handler can find
/// it. Called once from `SessionServices::mounted_services_with_daw`.
/// Re-registration is a silent no-op — first wins (we never expect to
/// mount twice, but being lenient beats a panic at plugin load).
pub fn register(impl_handle: impl SetlistDispatcher) {
    let _ = SETLIST_DISPATCHER.set(Arc::new(impl_handle));
}

/// REAPER action dispatch table for the setlist domain. Returned to
/// `daw_module::actions`'s `action_for_id` chain; `None` means
/// "someone else's action".
pub enum SetlistAction {
    Build,
}

pub fn action_for_id(action_id: &str) -> Option<SetlistAction> {
    // The action ID strings come from the `define_actions!` macro in
    // session_actions! — keep these in sync with the IDs declared
    // there. (`build_setlist` is the action's `id`, namespaced to
    // `fts.session.build_setlist` at command-id construction time.)
    let trimmed = action_id.trim();
    let bare = trimmed.strip_prefix("fts.session.").unwrap_or(trimmed);
    match bare {
        "build_setlist" => Some(SetlistAction::Build),
        _ => None,
    }
}

pub fn dispatch(action: SetlistAction) {
    let Some(dispatcher) = SETLIST_DISPATCHER.get() else {
        tracing::warn!(
            "[session] setlist action fired but no dispatcher registered \
             — session services were not mounted (or were mounted via a \
             code path that skipped setlist_actions::register)"
        );
        return;
    };
    match action {
        SetlistAction::Build => {
            tracing::info!("[session] build_setlist action — triggering");
            dispatcher.trigger_build();
        }
    }
}
