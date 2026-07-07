//! Take ranking actions — set a take's REAPER rank marker (`:)`, `:))`,
//! `:)))`, or `:(`) to a specific level in one shot. REAPER's native
//! ranking actions only bump one step at a time; we manipulate the take
//! markers directly via `SetTakeMarker`.
//!
//! Three scopes × four levels = 12 actions:
//!
//! - **Play-position-minus-2s**: target = `(play_pos - 2s)` if playing,
//!   else edit cursor. Acts on the active take of every selected item
//!   that contains that project position.
//! - **Item-wide**: acts on the active take of every selected item; the
//!   marker is anchored at source position 0.
//! - **Mouse**: acts on the take under the mouse cursor; marker is
//!   placed at the mouse's project-time position.
//!
//! Marker write semantics: any existing rank marker (name parses as a
//! [`TakeRating`]) within ±0.5s of the target source position is
//! renamed in place; otherwise a new marker is appended. This keeps
//! repeated keystrokes at the same spot idempotent while still allowing
//! a take to carry multiple rank regions for comping workflows.

use daw_proto::TakeRating;
use daw_reaper::safe_wrappers::item as item_sw;
use daw_reaper::safe_wrappers::mouse::MouseSnapshot;
use reaper_high::Reaper;
use reaper_low::raw;
use reaper_medium::MediaItemTake;
use std::ffi::CString;
use tracing::{debug, info};

const REPLACE_WINDOW_SECS: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankAction {
    PlayPos1,
    PlayPos2,
    PlayPos3,
    PlayPosDown,
    Item1,
    Item2,
    Item3,
    ItemDown,
    Mouse1,
    Mouse2,
    Mouse3,
    MouseDown,
}

impl RankAction {
    fn scope(self) -> Scope {
        match self {
            Self::PlayPos1 | Self::PlayPos2 | Self::PlayPos3 | Self::PlayPosDown => {
                Scope::PlayPosMinus2s
            }
            Self::Item1 | Self::Item2 | Self::Item3 | Self::ItemDown => Scope::ItemWide,
            Self::Mouse1 | Self::Mouse2 | Self::Mouse3 | Self::MouseDown => Scope::MouseCursor,
        }
    }

    fn rating(self) -> TakeRating {
        match self {
            Self::PlayPos1 | Self::Item1 | Self::Mouse1 => TakeRating::UpRank(1),
            Self::PlayPos2 | Self::Item2 | Self::Mouse2 => TakeRating::UpRank(2),
            Self::PlayPos3 | Self::Item3 | Self::Mouse3 => TakeRating::UpRank(3),
            Self::PlayPosDown | Self::ItemDown | Self::MouseDown => TakeRating::DownRank,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Scope {
    PlayPosMinus2s,
    ItemWide,
    MouseCursor,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::PlayPosMinus2s => "play-pos-2s",
            Scope::ItemWide => "item-wide",
            Scope::MouseCursor => "mouse",
        }
    }
}

/// Parse a session action id (e.g. `fts.session.take_rank_playpos_1`)
/// into a [`RankAction`]. Returns `None` for unrelated ids.
pub fn action_for_id(action_id: &str) -> Option<RankAction> {
    let slug = action_id
        .trim()
        .to_lowercase()
        .strip_prefix("fts.session.")
        .map(str::to_string)
        .unwrap_or_else(|| action_id.to_lowercase());
    match slug.as_str() {
        "take_rank_playpos_1" => Some(RankAction::PlayPos1),
        "take_rank_playpos_2" => Some(RankAction::PlayPos2),
        "take_rank_playpos_3" => Some(RankAction::PlayPos3),
        "take_rank_playpos_down" => Some(RankAction::PlayPosDown),
        "take_rank_item_1" => Some(RankAction::Item1),
        "take_rank_item_2" => Some(RankAction::Item2),
        "take_rank_item_3" => Some(RankAction::Item3),
        "take_rank_item_down" => Some(RankAction::ItemDown),
        "take_rank_mouse_1" => Some(RankAction::Mouse1),
        "take_rank_mouse_2" => Some(RankAction::Mouse2),
        "take_rank_mouse_3" => Some(RankAction::Mouse3),
        "take_rank_mouse_down" => Some(RankAction::MouseDown),
        _ => None,
    }
}

pub fn dispatch(action: RankAction) {
    apply(action.scope(), action.rating());
}

// ─── Core ────────────────────────────────────────────────────────────────────

fn apply(scope: Scope, rating: TakeRating) {
    let reaper = Reaper::get();
    let medium = reaper.medium_reaper();
    let low = medium.low();
    let marker_name = rating.to_marker_name();
    let undo_c = CString::new(format!("Take rank: {} ({})", marker_name, scope.label())).unwrap();

    unsafe {
        low.Undo_BeginBlock();
        low.PreventUIRefresh(1);

        let targets: Vec<(MediaItemTake, f64)> = match scope {
            Scope::PlayPosMinus2s => targets_play_pos_minus_2s(low),
            Scope::ItemWide => targets_item_wide(low),
            Scope::MouseCursor => targets_mouse_cursor().into_iter().collect(),
        };
        // medium handle is only used by the play-pos / item scopes via low.
        let _ = medium;

        if targets.is_empty() {
            low.PreventUIRefresh(-1);
            low.Undo_EndBlock(undo_c.as_ptr(), 0);
            debug!(scope = scope.label(), rank = %marker_name, "[take-rank] No targets");
            return;
        }

        for (take, source_pos) in &targets {
            write_rank_marker(low, *take, &marker_name, *source_pos);
        }

        low.UpdateArrange();
        low.PreventUIRefresh(-1);
        low.Undo_EndBlock(undo_c.as_ptr(), 4); // 4 = item changes
        info!(
            scope = scope.label(),
            rank = %marker_name,
            targets = targets.len(),
            "[take-rank] Applied"
        );
    }
}

fn write_rank_marker(low: &reaper_low::Reaper, take: MediaItemTake, name: &str, source_pos: f64) {
    let markers = item_sw::list_take_markers(low, take);
    let existing = markers.iter().find(|m| {
        TakeRating::from_marker_name(&m.name).is_some()
            && (m.source_position_seconds - source_pos).abs() <= REPLACE_WINDOW_SECS
    });
    match existing {
        Some(m) => {
            let ok =
                item_sw::update_take_marker(low, take, m.index, Some(name), Some(source_pos), None);
            info!(
                action = "update",
                ok,
                index = m.index,
                source_pos,
                old = %m.name,
                new = name,
                marker_count_before = markers.len(),
                "[take-rank] Wrote marker"
            );
        }
        None => {
            let idx = item_sw::add_take_marker(low, take, name, source_pos, None);
            info!(
                action = "add",
                idx = ?idx,
                source_pos,
                name = name,
                marker_count_before = markers.len(),
                "[take-rank] Wrote marker"
            );
        }
    }
}

// ─── Scope target resolvers ─────────────────────────────────────────────────

unsafe fn targets_play_pos_minus_2s(low: &reaper_low::Reaper) -> Vec<(MediaItemTake, f64)> {
    let play_state = unsafe { low.GetPlayStateEx(std::ptr::null_mut()) };
    let is_playing = play_state & 1 != 0;
    let target_proj_pos = if is_playing {
        (unsafe { low.GetPlayPositionEx(std::ptr::null_mut()) } - 2.0).max(0.0)
    } else {
        unsafe { low.GetCursorPositionEx(std::ptr::null_mut()) }
    };
    unsafe { collect_selected_active_takes_at(low, target_proj_pos) }
}

unsafe fn targets_item_wide(low: &reaper_low::Reaper) -> Vec<(MediaItemTake, f64)> {
    let n = unsafe { low.CountSelectedMediaItems(std::ptr::null_mut()) };
    let mut out = Vec::with_capacity(n.max(0) as usize);
    for i in 0..n {
        let item = unsafe { low.GetSelectedMediaItem(std::ptr::null_mut(), i) };
        if item.is_null() {
            continue;
        }
        let take_ptr = unsafe { low.GetActiveTake(item) };
        if let Some(take) = MediaItemTake::new(take_ptr) {
            out.push((take, 0.0));
        }
    }
    out
}

fn targets_mouse_cursor() -> Option<(MediaItemTake, f64)> {
    let snap = MouseSnapshot::capture();
    if snap.take.is_none() {
        info!(
            screen_x = snap.screen_pos.x,
            screen_y = snap.screen_pos.y,
            window = ?snap.window,
            segment = ?snap.segment,
            item_some = snap.item.is_some(),
            track_some = snap.track.is_some(),
            "[take-rank] mouse: no take under cursor"
        );
        return None;
    }
    let source_pos = snap.take_source_position();
    if source_pos.is_none() {
        info!(
            screen_x = snap.screen_pos.x,
            screen_y = snap.screen_pos.y,
            project_position = ?snap.project_position,
            item_some = snap.item.is_some(),
            "[take-rank] mouse: take found but source_pos resolves to None"
        );
        return None;
    }
    Some((snap.take?, source_pos?))
}

unsafe fn collect_selected_active_takes_at(
    low: &reaper_low::Reaper,
    target_proj_pos: f64,
) -> Vec<(MediaItemTake, f64)> {
    let n = unsafe { low.CountSelectedMediaItems(std::ptr::null_mut()) };
    let mut out = Vec::new();
    for i in 0..n {
        let item = unsafe { low.GetSelectedMediaItem(std::ptr::null_mut(), i) };
        if item.is_null() {
            continue;
        }
        let item_start = unsafe { low.GetMediaItemInfo_Value(item, c_str_d_position()) };
        let item_length = unsafe { low.GetMediaItemInfo_Value(item, c_str_d_length()) };
        if target_proj_pos < item_start || target_proj_pos > item_start + item_length {
            continue;
        }
        let take_ptr = unsafe { low.GetActiveTake(item) };
        if let Some(take) = MediaItemTake::new(take_ptr)
            && let Some(src) =
                unsafe { project_pos_to_source_pos(low, item, take_ptr, target_proj_pos) }
        {
            out.push((take, src));
        }
    }
    out
}

unsafe fn project_pos_to_source_pos(
    low: &reaper_low::Reaper,
    item: *mut raw::MediaItem,
    take: *mut raw::MediaItem_Take,
    proj_pos: f64,
) -> Option<f64> {
    let item_start = unsafe { low.GetMediaItemInfo_Value(item, c_str_d_position()) };
    let play_rate = unsafe { low.GetMediaItemTakeInfo_Value(take, c_str_d_playrate()) };
    let start_offset = unsafe { low.GetMediaItemTakeInfo_Value(take, c_str_d_startoffs()) };
    let src = (proj_pos - item_start) * play_rate + start_offset;
    if src < 0.0 { None } else { Some(src) }
}

// ─── Static C-string accessors ──────────────────────────────────────────────

fn c_str_d_position() -> *const std::os::raw::c_char {
    b"D_POSITION\0".as_ptr() as *const _
}
fn c_str_d_length() -> *const std::os::raw::c_char {
    b"D_LENGTH\0".as_ptr() as *const _
}
fn c_str_d_playrate() -> *const std::os::raw::c_char {
    b"D_PLAYRATE\0".as_ptr() as *const _
}
fn c_str_d_startoffs() -> *const std::os::raw::c_char {
    b"D_STARTOFFS\0".as_ptr() as *const _
}

// ── architect::actions declaration ──────────────────────────────────────
//
// `RankAction` / `action_for_id` / `dispatch` above stay put — still the
// live path `daw_module.rs`'s dispatch chain calls into. Additive
// declarative layer only, mirroring `setlist_actions`'s migration.

/// Bridges the twelve take-ranking actions onto `#[architect::actions]`.
/// Every method forwards to the existing synchronous `dispatch` — no
/// behavior change, just a declarative front door with real metadata.
pub struct TakeRankingActionsImpl;

#[architect::actions(namespace = "FTS_SESSION")]
pub trait TakeRankingActions {
    #[action(
        description = "Set the active take's rank marker to :) at (play-pos - 2s) on every selected item, or at edit cursor if not playing",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_playpos_1(&self);
    #[action(
        description = "Set the active take's rank marker to :)) at (play-pos - 2s) on every selected item, or at edit cursor if not playing",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_playpos_2(&self);
    #[action(
        description = "Set the active take's rank marker to :))) at (play-pos - 2s) on every selected item, or at edit cursor if not playing",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_playpos_3(&self);
    #[action(
        description = "Set the active take's rank marker to :( at (play-pos - 2s) on every selected item, or at edit cursor if not playing",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_playpos_down(&self);
    #[action(
        description = "Set the active take's rank marker to :) at item start for every selected item",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_item_1(&self);
    #[action(
        description = "Set the active take's rank marker to :)) at item start for every selected item",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_item_2(&self);
    #[action(
        description = "Set the active take's rank marker to :))) at item start for every selected item",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_item_3(&self);
    #[action(
        description = "Set the active take's rank marker to :( at item start for every selected item",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_item_down(&self);
    #[action(
        description = "Set the rank marker to :) on the take under the mouse at the mouse's project-time position",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_mouse_1(&self);
    #[action(
        description = "Set the rank marker to :)) on the take under the mouse at the mouse's project-time position",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_mouse_2(&self);
    #[action(
        description = "Set the rank marker to :))) on the take under the mouse at the mouse's project-time position",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_mouse_3(&self);
    #[action(
        description = "Set the rank marker to :( on the take under the mouse at the mouse's project-time position",
        category = "Project",
        group = "Take Ranking"
    )]
    fn take_rank_mouse_down(&self);
}

impl TakeRankingActions for TakeRankingActionsImpl {
    fn take_rank_playpos_1(&self) {
        dispatch(RankAction::PlayPos1);
    }
    fn take_rank_playpos_2(&self) {
        dispatch(RankAction::PlayPos2);
    }
    fn take_rank_playpos_3(&self) {
        dispatch(RankAction::PlayPos3);
    }
    fn take_rank_playpos_down(&self) {
        dispatch(RankAction::PlayPosDown);
    }
    fn take_rank_item_1(&self) {
        dispatch(RankAction::Item1);
    }
    fn take_rank_item_2(&self) {
        dispatch(RankAction::Item2);
    }
    fn take_rank_item_3(&self) {
        dispatch(RankAction::Item3);
    }
    fn take_rank_item_down(&self) {
        dispatch(RankAction::ItemDown);
    }
    fn take_rank_mouse_1(&self) {
        dispatch(RankAction::Mouse1);
    }
    fn take_rank_mouse_2(&self) {
        dispatch(RankAction::Mouse2);
    }
    fn take_rank_mouse_3(&self) {
        dispatch(RankAction::Mouse3);
    }
    fn take_rank_mouse_down(&self) {
        dispatch(RankAction::MouseDown);
    }
}

/// Registers all twelve take-ranking actions with `backend`.
pub fn register_actions<B>(backend: &B)
where
    B: ::architect::action::ActionBackend + ?Sized,
{
    register_take_ranking_actions_actions(backend, std::sync::Arc::new(TakeRankingActionsImpl));
}
