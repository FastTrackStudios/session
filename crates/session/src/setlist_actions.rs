//! REAPER action handlers for setlist-level operations.
//!
//! The `BUILD_SETLIST` action ID has been registered in
//! `session_actions!` for a while, but `daw_module::actions` walks a
//! chain of `*_actions::action_for_id` lookups and *none* of those
//! owned the setlist domain. Hitting the hotkey in REAPER fell
//! through to the "No DAW handler registered" log message — the
//! action was a no-op.
//!
//! This module fills the gap, mirroring the `keyflow_actions` shape:
//!
//!   pub fn dispatch<D>(daw: &D, action: SetlistAction)
//!       where D: Projects + …
//!
//! Runs **synchronously on REAPER's main thread** (where the action
//! callback fires), so we can use the sync trait methods on
//! `daw_reaper::Reaper` directly — no `main_thread::query` bounce,
//! no Tokio runtime, no `moire::task::spawn`. This is the same
//! pattern keyflow_actions uses and the same reason it works.
//!
//! Setlist storage: at mount time `register` stashes the
//! `SetlistServiceImpl`'s `Arc<RwLock<Option<Setlist>>>` so the
//! sync writer here and the RPC reader (`SetlistService::setlist`)
//! share one piece of state.

use std::sync::Arc;
use std::sync::OnceLock;

use daw::service::ProjectContext;
use daw::service::transport::service::Transport as TransportService;
use daw::service::{Markers, Projects, Regions, TempoMap};
use moire::sync::RwLock;
use session_proto::{AdvanceMode, Setlist, Song};

use crate::setlist_service::SetlistServiceImpl;
use crate::song_builder::SongBuilder;

/// The setlist storage cell shared with `SetlistServiceImpl`.
static SETLIST_STORE: OnceLock<Arc<RwLock<Option<Setlist>>>> = OnceLock::new();

/// Stash the `SetlistServiceImpl`'s setlist cell so the action
/// handler writes to the same `Arc<RwLock<>>` the RPC service reads.
/// Idempotent — first call wins; later calls ignored so re-mounting
/// (shouldn't happen but defensive) can't blow up plugin startup.
pub fn register<D>(svc: &SetlistServiceImpl<D>) {
    let _ = SETLIST_STORE.set(svc.setlist.clone());
}

/// REAPER action dispatch table for the setlist domain. Returned to
/// `daw_module::actions`'s `action_for_id` chain; `None` means
/// "someone else's action".
pub enum SetlistAction {
    /// Scan open project tabs, parse SONGSTART/SONGEND markers + section
    /// regions, populate the cached `Setlist`. Idempotent — rerun to
    /// pick up edits.
    Build,
    /// Stamp the canonical demo markers + section regions into the
    /// current REAPER project (3 songs of typical worship-set
    /// structure: count-in, song-start, verse/pre/chorus/bridge/outro
    /// regions, song-end, =END render bound), then immediately rebuild
    /// the cached `Setlist`. Use for a one-keypress "give me a working
    /// setlist to test against" — handy when iterating on UI or RPC
    /// without recording markers by hand.
    LoadDemo,
    /// Log every marker and region in the current project with its
    /// position, name, color and ruler lane index. Diagnostic — the
    /// vox response-store encoder still panics on Markers::all /
    /// Regions::all so there's no read-side CLI path; dump-via-log
    /// is the workaround for inspecting actual lane assignments.
    DumpRulerState,
}

pub fn action_for_id(action_id: &str) -> Option<SetlistAction> {
    // Action IDs flow in either the dotted form `fts.session.build_setlist`
    // (definition-time) or the upper-snake REAPER command form
    // `FTS_SESSION_BUILD_SETLIST` (runtime callback). Accept both.
    let trimmed = action_id.trim();
    let lower = trimmed.to_lowercase();
    let bare = lower
        .strip_prefix("fts.session.")
        .or_else(|| lower.strip_prefix("fts_session_"))
        .unwrap_or(lower.as_str());
    match bare {
        "build_setlist" => Some(SetlistAction::Build),
        "load_demo_setlist" => Some(SetlistAction::LoadDemo),
        "dump_ruler_state" => Some(SetlistAction::DumpRulerState),
        _ => None,
    }
}

/// Run a setlist action on the calling thread (REAPER main thread).
/// All work is sync; no spawning, no Tokio runtime needed.
pub fn dispatch<D>(daw: &D, action: SetlistAction)
where
    D: Projects + TransportService + Markers + Regions + TempoMap,
{
    match action {
        SetlistAction::DumpRulerState => dump_ruler_state(daw),
        SetlistAction::LoadDemo => {
            tracing::info!("[session] load_demo_setlist action — stamping demo markers/regions");
            let started = std::time::Instant::now();
            match crate::setlist_service::demo::stamp_demo_setlist_with(daw) {
                Ok(()) => {
                    tracing::info!(
                        "[session] demo markers stamped in {:?}; rebuilding setlist",
                        started.elapsed()
                    );
                    // Fall through into the Build path so the cached
                    // setlist immediately reflects what we just stamped.
                    dispatch(daw, SetlistAction::Build);
                }
                Err(e) => {
                    tracing::warn!("[session] load_demo_setlist: stamping failed: {e:?}");
                }
            }
        }
        SetlistAction::Build => {
            tracing::info!("[session] build_setlist action — building synchronously");
            let started = std::time::Instant::now();
            let setlist = build_setlist_sync(daw);
            let song_count = setlist.songs.len();
            match SETLIST_STORE.get() {
                Some(slot) => match slot.try_write() {
                    Ok(mut guard) => {
                        *guard = Some(setlist);
                        tracing::info!(
                            "[session] build_setlist action completed ({} songs in {:?})",
                            song_count,
                            started.elapsed()
                        );
                    }
                    Err(_) => {
                        tracing::warn!(
                            "[session] build_setlist: setlist cell busy — \
                             another writer is mid-build, skipping"
                        );
                    }
                },
                None => {
                    tracing::warn!(
                        "[session] build_setlist action: setlist store not \
                         registered (mounted_services_with_daw never called?)"
                    );
                }
            }
        }
    }
}

/// Walk every open REAPER project tab and synthesise a `Song` from
/// each via `SongBuilder::build_native` (markers + regions + lanes —
/// no chart / fingerprint hydration). Returns an empty setlist when
/// nothing is open. Sync because callers are REAPER action handlers.
fn build_setlist_sync<D>(daw: &D) -> Setlist
where
    D: Projects,
{
    let projects = daw.list();
    let mut songs: Vec<Song> = Vec::with_capacity(projects.len());
    for project in projects {
        let ctx = ProjectContext::Project(project.guid.clone());
        match SongBuilder::build_native(ctx) {
            Ok(mut project_songs) => {
                tracing::debug!(
                    project = %project.name,
                    guid = %project.guid,
                    songs = project_songs.len(),
                    "[session] built songs from project"
                );
                songs.append(&mut project_songs);
            }
            Err(err) => {
                tracing::warn!(
                    project = %project.name,
                    guid = %project.guid,
                    "[session] SongBuilder failed for project: {err}"
                );
            }
        }
    }

    Setlist {
        id: None,
        name: format!("Setlist - {}", chrono::Local::now().format("%Y-%m-%d")),
        advance_mode: AdvanceMode::default(),
        songs,
    }
}

/// Log every marker and region in the current REAPER project, sorted
/// by lane then position, so the user (or me, debugging via tail-log)
/// can confirm what landed where after a build / demo / hand-edit.
/// Sync on the main thread — same reason the rest of dispatch is sync.
fn dump_ruler_state<D>(daw: &D)
where
    D: Projects + Markers + Regions,
{
    let project = ProjectContext::Current;

    // Lane table first. Probe 0..=10 because REAPER may have an
    // implicit lane at index 0 (the "automatic" slot) that
    // `ruler_lane_count` (which stops at the first empty name)
    // hides from us, and the user wants to know about it.
    tracing::info!("[session] === lanes ===");
    for idx in 0u32..=10 {
        let name = daw.get_ruler_lane_name(project.clone(), idx);
        tracing::info!("[session] lane {idx}: name={:?}", name);
    }

    let markers = Markers::all(daw, project.clone());
    let regions = Regions::all(daw, project);

    tracing::info!(
        "[session] ruler state: {} marker(s), {} region(s)",
        markers.len(),
        regions.len()
    );

    // Markers — sort by (lane, position) so the dump groups by lane.
    let mut sorted_markers: Vec<_> = markers.iter().collect();
    sorted_markers.sort_by(|a, b| {
        let lane_ord = a.lane.unwrap_or(0).cmp(&b.lane.unwrap_or(0));
        lane_ord.then_with(|| {
            let ap = a.position.time.map(|t| t.as_seconds()).unwrap_or(0.0);
            let bp = b.position.time.map(|t| t.as_seconds()).unwrap_or(0.0);
            ap.partial_cmp(&bp).unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    tracing::info!("[session] === markers ===");
    for m in &sorted_markers {
        let pos = m.position.time.map(|t| t.as_seconds()).unwrap_or(0.0);
        tracing::info!(
            "[session] marker  lane={:?}  pos={:>7.3}  color={:?}  name={:?}  id={:?}",
            m.lane,
            pos,
            m.color,
            m.name,
            m.id,
        );
    }

    // Regions — same shape, plus end position.
    let mut sorted_regions: Vec<_> = regions.iter().collect();
    sorted_regions.sort_by(|a, b| {
        let lane_ord = a.lane.unwrap_or(0).cmp(&b.lane.unwrap_or(0));
        lane_ord.then_with(|| {
            let ap = a
                .time_range
                .start
                .time
                .map(|t| t.as_seconds())
                .unwrap_or(0.0);
            let bp = b
                .time_range
                .start
                .time
                .map(|t| t.as_seconds())
                .unwrap_or(0.0);
            ap.partial_cmp(&bp).unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    tracing::info!("[session] === regions ===");
    for r in &sorted_regions {
        let start = r
            .time_range
            .start
            .time
            .map(|t| t.as_seconds())
            .unwrap_or(0.0);
        let end = r.time_range.end.time.map(|t| t.as_seconds()).unwrap_or(0.0);
        tracing::info!(
            "[session] region  lane={:?}  start={:>7.3}  end={:>7.3}  color={:?}  name={:?}  id={:?}",
            r.lane,
            start,
            end,
            r.color,
            r.name,
            r.id,
        );
    }
}
