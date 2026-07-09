//! Vox RPC service implementations for session control surfaces.
//!
//! Three services exposed today, all mounted by `fts-extensions` on
//! the published Unix socket:
//!
//! - [`SessionModeServiceImpl`] — get / set / list FTS session modes
//! - [`TakeRankingServiceImpl`] — apply a rank at a given scope
//! - [`RecordControlServiceImpl`] — restart_recording + monitor toggles
//!
//! Every call bounces to REAPER's main thread via
//! `daw_reaper::main_thread::query` because the underlying handlers
//! call raw REAPER FFI that's main-thread-only. The async RPC
//! dispatcher runs on the tokio runtime; without the bounce, calling
//! e.g. `set_mode` from a remote client would touch REAPER from the
//! wrong thread.

use daw_reaper::main_thread;
use session_proto::SessionServiceError;
use session_proto::services::{
    RecordControlService, SessionModeService, TakeRankLevel, TakeRankScope, TakeRankingService,
};
use tokio::sync::broadcast::error::RecvError;
use vox::Tx;

use crate::mode_actions::{self, Mode};
use crate::record_actions::{self, RecordAction};
use crate::take_ranking::{self, RankAction};

// ─── SessionModeService ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct SessionModeServiceImpl {
    /// `#[subscribe]` hub for mode changes; a pump bridges the internal
    /// broadcast into it (replay 1 = late subscribers get current mode).
    modes_hub: architect::PubSub<String>,
}

impl Default for SessionModeServiceImpl {
    fn default() -> Self {
        let hub = architect::PubSub::sliding(1);
        // Seed + pump: current mode first, then every flip.
        hub.publish(mode_actions::current_mode().slug().to_string());
        let pump = hub.clone();
        let mut rx = mode_actions::mode_broadcast().subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(slug) => {
                        pump.publish(slug);
                    }
                    Err(RecvError::Closed) => return,
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "SessionMode pump lagged");
                    }
                }
            }
        });
        Self { modes_hub: hub }
    }
}

impl session_proto::services::session_mode_service::SessionModeServiceStreamSource
    for SessionModeServiceImpl
{
    fn mode_changes_hub(&self) -> &architect::PubSub<String> {
        &self.modes_hub
    }
}

impl SessionModeService for SessionModeServiceImpl {
    async fn current_mode(&self) -> Result<String, SessionServiceError> {
        Ok(mode_actions::current_mode().slug().to_string())
    }

    async fn set_mode(&self, slug: String) -> Result<(), SessionServiceError> {
        let mode = Mode::from_slug(&slug).ok_or_else(|| SessionServiceError::NotFound {
            entity: "Mode".to_string(),
            id: slug.clone(),
        })?;
        main_thread::query(move || mode_actions::set_mode(mode))
            .await
            .ok_or_else(|| {
                SessionServiceError::DawError("TaskSupport not initialised".to_string())
            })?;
        Ok(())
    }

    async fn list_modes(&self) -> Result<Vec<String>, SessionServiceError> {
        Ok(Mode::ALL.iter().map(|m| m.slug().to_string()).collect())
    }

}

// ─── TakeRankingService ────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct TakeRankingServiceImpl;

impl TakeRankingService for TakeRankingServiceImpl {
    async fn apply_rank(
        &self,
        scope: TakeRankScope,
        level: TakeRankLevel,
    ) -> Result<(), SessionServiceError> {
        let action = rank_action_for(scope, level);
        main_thread::query(move || take_ranking::dispatch(action))
            .await
            .ok_or_else(|| {
                SessionServiceError::DawError("TaskSupport not initialised".to_string())
            })?;
        Ok(())
    }
}

fn rank_action_for(scope: TakeRankScope, level: TakeRankLevel) -> RankAction {
    use TakeRankLevel::*;
    use TakeRankScope::*;
    match (scope, level) {
        (PlayPosMinus2s, One) => RankAction::PlayPos1,
        (PlayPosMinus2s, Two) => RankAction::PlayPos2,
        (PlayPosMinus2s, Three) => RankAction::PlayPos3,
        (PlayPosMinus2s, Down) => RankAction::PlayPosDown,
        (ItemWide, One) => RankAction::Item1,
        (ItemWide, Two) => RankAction::Item2,
        (ItemWide, Three) => RankAction::Item3,
        (ItemWide, Down) => RankAction::ItemDown,
        (MouseCursor, One) => RankAction::Mouse1,
        (MouseCursor, Two) => RankAction::Mouse2,
        (MouseCursor, Three) => RankAction::Mouse3,
        (MouseCursor, Down) => RankAction::MouseDown,
    }
}

// ─── RecordControlService ──────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct RecordControlServiceImpl;

impl RecordControlService for RecordControlServiceImpl {
    async fn restart_recording(&self) -> Result<(), SessionServiceError> {
        dispatch_record(RecordAction::RestartRecording).await
    }

    async fn toggle_monitor_on_off(&self) -> Result<(), SessionServiceError> {
        dispatch_record(RecordAction::ToggleMonitorOnOff).await
    }

    async fn toggle_monitor_tape_off(&self) -> Result<(), SessionServiceError> {
        dispatch_record(RecordAction::ToggleMonitorTapeOff).await
    }
}

async fn dispatch_record(action: RecordAction) -> Result<(), SessionServiceError> {
    main_thread::query(move || record_actions::dispatch(action))
        .await
        .ok_or_else(|| SessionServiceError::DawError("TaskSupport not initialised".to_string()))?;
    Ok(())
}
