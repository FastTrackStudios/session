//! Active indices polling/calculation, transport state tracking, subscriptions

use super::{
    SetlistServiceImpl, ACTIVE_HYDRATION_POLL_MAX_MS, ACTIVE_HYDRATION_POLL_MS,
    ACTIVE_HYDRATION_TICK_MS, ACTIVE_INDICES_PROGRESS_EMIT_MS, HYDRATION_CONCURRENCY,
    PROJECT_SWITCH_DEBOUNCE_MS, TRANSPORT_PROGRESS_EPSILON, TRANSPORT_TIME_EPSILON_SECS,
};
use daw::Daw;
use roam::Tx;
use rustc_hash::FxHashMap;
use session_proto::{
    ActiveIndices, AdvanceMode, MeasureInfo, Section, SessionServiceError, SetlistEvent,
    SetlistService, Song, SongTransportState,
};
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use moire::sync::{Semaphore, mpsc};
use tokio::sync::broadcast::error::RecvError as BroadcastRecvError;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

impl SetlistServiceImpl {
    pub(crate) fn active_indices_structural_changed(
        prev: &ActiveIndices,
        next: &ActiveIndices,
    ) -> bool {
        prev.song_index != next.song_index
            || prev.section_index != next.section_index
            || prev.slide_index != next.slide_index
            || prev.is_playing != next.is_playing
            || prev.looping != next.looping
            || prev.loop_selection != next.loop_selection
            || prev.queued_target != next.queued_target
    }

    /// Check if the transport has reached the queued target and clear it if so
    pub(crate) async fn check_and_clear_queue(
        &self,
        song_index: usize,
        section_index: Option<usize>,
        position_seconds: f64,
    ) {
        let queued = self.queued_target.read().await.clone();
        if let Some(target) = queued {
            let reached = match &target {
                session_proto::QueuedTarget::Section {
                    song_index: q_song,
                    section_index: q_section,
                    ..
                } => song_index == *q_song && section_index == Some(*q_section),
                session_proto::QueuedTarget::Time {
                    song_index: q_song,
                    position_seconds: q_pos,
                    ..
                } => {
                    // Consider reached if within 0.1 seconds
                    song_index == *q_song && (position_seconds - q_pos).abs() < 0.1
                }
                session_proto::QueuedTarget::Measure {
                    song_index: q_song,
                    ..
                } => {
                    // For measures, we'd need to check musical position - for now just check song
                    song_index == *q_song
                }
                session_proto::QueuedTarget::Comment {
                    song_index: q_song,
                    position_seconds: q_pos,
                    ..
                } => {
                    // Consider reached if within 0.1 seconds
                    song_index == *q_song && (position_seconds - q_pos).abs() < 0.1
                }
            };

            if reached {
                self.clear_queued_target().await;
            }
        }
    }

    /// Calculate transport state for a specific song based on its project's transport
    pub(crate) fn calculate_song_transport(
        song: &Song,
        song_index: usize,
        position: daw::service::Position,
        is_playing: bool,
        is_looping: bool,
        loop_region: Option<daw::service::LoopRegion>,
        tempo: f64,
        time_sig: (u32, u32),
    ) -> SongTransportState {
        let song_duration = song.duration();
        let song_start = song.start_seconds();

        // Get position in seconds for progress calculations
        let position_seconds = position.time.map(|t| t.as_seconds()).unwrap_or(0.0);

        // Calculate progress within song
        let relative_pos = position_seconds - song_start;
        let progress = if song_duration > 0.0 {
            (relative_pos / song_duration).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Find section at position
        let (section_index, section_progress) = if let Some((sec_idx, section)) =
            song.section_at_position_with_index(position_seconds)
        {
            let sec_duration = section.duration();
            let sec_progress = if sec_duration > 0.0 {
                ((position_seconds - section.start_seconds) / sec_duration).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (Some(sec_idx), Some(sec_progress))
        } else {
            (None, None)
        };

        // Convert loop region from project-absolute to song-relative coordinates
        let song_loop_region = loop_region.and_then(|region| {
            // Only include loop region if it overlaps with the song
            let region_start_relative = region.start_seconds - song_start;
            let region_end_relative = region.end_seconds - song_start;

            // Check if loop region is within song bounds (with some tolerance)
            if region_end_relative > 0.0 && region_start_relative < song_duration {
                Some(daw::service::LoopRegion::new(
                    region_start_relative.max(0.0),
                    region_end_relative.min(song_duration),
                ))
            } else {
                None
            }
        });

        SongTransportState {
            song_index,
            position,
            progress,
            section_index,
            section_progress,
            is_playing,
            is_looping,
            loop_region: song_loop_region,
            bpm: tempo,
            time_sig_num: time_sig.0,
            time_sig_denom: time_sig.1,
        }
    }

    fn f64_changed(prev: f64, next: f64, epsilon: f64) -> bool {
        (prev - next).abs() > epsilon
    }

    fn option_f64_changed(prev: Option<f64>, next: Option<f64>, epsilon: f64) -> bool {
        match (prev, next) {
            (Some(a), Some(b)) => Self::f64_changed(a, b, epsilon),
            (None, None) => false,
            _ => true,
        }
    }

    fn loop_region_changed(
        prev: &Option<daw::service::LoopRegion>,
        next: &Option<daw::service::LoopRegion>,
    ) -> bool {
        match (prev, next) {
            (Some(a), Some(b)) => {
                Self::f64_changed(
                    a.start_seconds,
                    b.start_seconds,
                    TRANSPORT_TIME_EPSILON_SECS,
                ) || Self::f64_changed(a.end_seconds, b.end_seconds, TRANSPORT_TIME_EPSILON_SECS)
            }
            (None, None) => false,
            _ => true,
        }
    }

    pub(crate) fn transport_changed(
        prev: &SongTransportState,
        next: &SongTransportState,
    ) -> bool {
        if prev.song_index != next.song_index
            || prev.section_index != next.section_index
            || prev.is_playing != next.is_playing
            || prev.is_looping != next.is_looping
            || prev.time_sig_num != next.time_sig_num
            || prev.time_sig_denom != next.time_sig_denom
        {
            return true;
        }

        if Self::f64_changed(prev.bpm, next.bpm, 0.001)
            || Self::f64_changed(prev.progress, next.progress, TRANSPORT_PROGRESS_EPSILON)
            || Self::option_f64_changed(
                prev.section_progress,
                next.section_progress,
                TRANSPORT_PROGRESS_EPSILON,
            )
            || Self::loop_region_changed(&prev.loop_region, &next.loop_region)
        {
            return true;
        }

        let prev_time = prev.position.time.map(|t| t.as_seconds());
        let next_time = next.position.time.map(|t| t.as_seconds());
        if Self::option_f64_changed(prev_time, next_time, TRANSPORT_TIME_EPSILON_SECS) {
            return true;
        }

        prev.position.musical != next.position.musical || prev.position.midi != next.position.midi
    }

    /// Get transport state for ALL songs by querying each project
    ///
    /// Uses `get_state()` which returns all transport info in ONE RPC call per project,
    /// rather than making 5 separate calls (position, is_playing, is_looping, tempo, time_sig).
    pub(crate) async fn get_all_song_transports(&self) -> Vec<SongTransportState> {
        let daw = Daw::get();
        let songs = {
            let setlist = self.setlist.read().await;
            let Some(ref setlist) = *setlist else {
                return Vec::new();
            };
            setlist.songs.clone()
        };
        if songs.is_empty() {
            return Vec::new();
        }

        let semaphore = Arc::new(Semaphore::new("session.setlist.polling.hydration", HYDRATION_CONCURRENCY));
        let mut join_set = JoinSet::new();

        for (song_index, song) in songs.into_iter().enumerate() {
            let daw_clone = daw.clone();
            let permit = semaphore.clone();
            join_set.spawn(async move {
                let _permit = permit.acquire_owned().await.expect("semaphore closed");
                match daw_clone.project(&song.project_guid).await {
                    Ok(project) => {
                        let transport = project.transport();

                        // Get full transport state in ONE RPC call
                        match transport.get_state().await {
                            Ok(state) => {
                                let is_playing = state.play_state == daw::service::PlayState::Playing
                                    || state.play_state == daw::service::PlayState::Recording;

                                // Use playhead position when playing, edit cursor when stopped
                                // The Position includes both time and musical position from REAPER's tempo map
                                let position = if is_playing {
                                    state.playhead_position.clone()
                                } else {
                                    state.edit_position.clone()
                                };

                                let is_looping = state.looping;
                                let loop_region = state.loop_region.clone();
                                let tempo = state.tempo.bpm();
                                let time_sig = (
                                    state.time_signature.numerator(),
                                    state.time_signature.denominator(),
                                );

                                let song_transport = Self::calculate_song_transport(
                                    &song,
                                    song_index,
                                    position,
                                    is_playing,
                                    is_looping,
                                    loop_region,
                                    tempo,
                                    time_sig,
                                );

                                (song_index, song_transport)
                            }
                            Err(e) => {
                                debug!(
                                    "Could not get transport state for project {}: {}",
                                    song.project_guid, e
                                );
                                (
                                    song_index,
                                    SongTransportState {
                                        song_index,
                                        ..Default::default()
                                    },
                                )
                            }
                        }
                    }
                    Err(e) => {
                        debug!(
                            "Could not get project {} for song {}: {}",
                            song.project_guid, song.name, e
                        );
                        (
                            song_index,
                            SongTransportState {
                                song_index,
                                ..Default::default()
                            },
                        )
                    }
                }
            });
        }

        let mut transports = vec![SongTransportState::default(); join_set.len()];
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((song_index, song_transport)) => {
                    if song_index < transports.len() {
                        transports[song_index] = song_transport;
                    }
                }
                Err(e) => {
                    warn!("Transport worker failed: {}", e);
                }
            }
        }
        for (index, transport) in transports.iter_mut().enumerate() {
            transport.song_index = index;
        }

        transports
    }

    /// Find the active song and section based on current DAW project
    pub(crate) async fn calculate_active_indices(&self) -> ActiveIndices {
        let daw = Daw::get();

        // Get current project (the one that's selected/focused in the DAW)
        let current_project = match daw.current_project().await {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to get current project: {}", e);
                return ActiveIndices::default();
            }
        };

        let current_guid = current_project.guid().to_string();

        // Get transport state for current project
        let transport = current_project.transport();
        let position = transport.get_position().await.unwrap_or(0.0);
        let is_playing = transport.is_playing().await.unwrap_or(false);
        let looping = transport.is_looping().await.unwrap_or(false);

        // Find which song corresponds to the current project
        let setlist = self.setlist.read().await;
        let Some(ref setlist) = *setlist else {
            return ActiveIndices {
                is_playing,
                looping,
                ..Default::default()
            };
        };

        // Find the song that matches the current project and position.
        // When multiple songs share a project GUID, resolve by playhead position.
        let song_data = setlist
            .songs
            .iter()
            .enumerate()
            .filter(|(_, song)| song.project_guid == current_guid)
            .find(|(_, song)| position >= song.start_seconds && position < song.end_seconds)
            .or_else(|| {
                // Fallback: nearest song in the same project
                setlist
                    .songs
                    .iter()
                    .enumerate()
                    .filter(|(_, song)| song.project_guid == current_guid)
                    .min_by(|(_, a), (_, b)| {
                        let dist_a = (position - a.start_seconds)
                            .abs()
                            .min((position - a.end_seconds).abs());
                        let dist_b = (position - b.start_seconds)
                            .abs()
                            .min((position - b.end_seconds).abs());
                        dist_a
                            .partial_cmp(&dist_b)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });

        if let Some((song_index, song)) = song_data {
            // Calculate song progress
            let song_duration = song.duration();
            let song_relative_pos = position - song.start_seconds();
            let song_progress = if song_duration > 0.0 {
                Some((song_relative_pos / song_duration).clamp(0.0, 1.0))
            } else {
                None
            };

            // Find section at current position
            if let Some((section_index, section)) = song.section_at_position_with_index(position) {
                // Calculate section progress
                let section_duration = section.duration();
                let section_relative_pos = position - section.start_seconds;
                let section_progress = if section_duration > 0.0 {
                    Some((section_relative_pos / section_duration).clamp(0.0, 1.0))
                } else {
                    None
                };

                ActiveIndices {
                    song_index: Some(song_index),
                    section_index: Some(section_index),
                    slide_index: None,
                    song_progress,
                    section_progress,
                    is_playing,
                    looping,
                    loop_selection: None,
                    queued_target: None,
                }
            } else {
                // In song but not in a specific section
                ActiveIndices {
                    song_index: Some(song_index),
                    section_index: None,
                    slide_index: None,
                    song_progress,
                    section_progress: None,
                    is_playing,
                    looping,
                    loop_selection: None,
                    queued_target: None,
                }
            }
        } else {
            // Current project doesn't match any song in setlist
            ActiveIndices {
                song_index: None,
                section_index: None,
                slide_index: None,
                song_progress: None,
                section_progress: None,
                is_playing,
                looping,
                loop_selection: None,
                queued_target: None,
            }
        }
    }

    // =========================================================================
    // SetlistService trait: Subscriptions
    // =========================================================================

    pub(crate) async fn subscribe_impl(
        &self,
        events: Tx<SetlistEvent>,
    ) -> Result<(), SessionServiceError> {
        debug!("SetlistService::subscribe() - starting fully reactive event stream");
        // Clone self for the spawned task
        let this = self.clone();

        // Spawn the streaming loop so this method returns immediately
        tokio::spawn(async move {
            let mut setlist_updates = this.setlist_update_bus.subscribe();
            let mut hydration_rx = this.hydration_bus.subscribe();
            let mut chart_hydration_rx = this.chart_hydration_bus.subscribe();

            // Get songs for GUID -> index mapping
            let songs: Vec<Song>;
            {
                let setlist = this.setlist.read().await;
                if let Some(ref sl) = *setlist {
                    songs = sl.songs.clone();
                    // Send initial setlist state
                    if events
                        .send(SetlistEvent::SetlistChanged(sl.clone()))
                        .await
                        .is_err()
                    {
                        debug!(
                            "SetlistService::subscribe() - client disconnected during initial send"
                        );
                        return;
                    }
                } else {
                    info!("SetlistService::subscribe() - no setlist available");
                    return;
                }
            }

            // Send cached chart payloads for current songs so subscribers can
            // render charts without bloating SetlistChanged payloads.
            {
                let chart_snapshot = this.chart_cache.snapshot().await;
                for (index, song) in songs.iter().enumerate() {
                    if let Some(chart) = chart_snapshot.get(&song.project_guid).cloned() {
                        if events
                            .send(SetlistEvent::SongChartHydrated { song_id: song.id.clone(), index, chart })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }

            // Build project GUID -> song indices mapping (multiple songs may share a GUID).
            let mut guid_to_indices: FxHashMap<String, Vec<usize>> = FxHashMap::default();
            for (idx, song) in songs.iter().enumerate() {
                guid_to_indices
                    .entry(song.project_guid.clone())
                    .or_default()
                    .push(idx);
            }
            let mut song_id_to_index: FxHashMap<String, usize> = songs
                .iter()
                .enumerate()
                .map(|(idx, song)| (song.id.to_string(), idx))
                .collect();

            // Subscribe to the reactive per-project transport stream
            let daw = Daw::get();
            let transport_rx = match daw.current_project().await {
                Ok(project) => match project.transport().subscribe_all_projects().await {
                    Ok(rx) => rx,
                    Err(e) => {
                        warn!("Failed to subscribe to all projects transport: {}", e);
                        return;
                    }
                },
                Err(e) => {
                    warn!(
                        "Failed to get current project for transport subscription: {}",
                        e
                    );
                    return;
                }
            };

            // Get initial active indices from REAPER's current project (makes RPC calls)
            // This ensures the UI shows the correct song/section on startup
            let mut last_indices = this.calculate_active_indices().await;

            // Update cached indices so navigation works correctly
            this.set_cached_indices(last_indices.clone()).await;

            // Also update active_song_id if we found a song
            if let Some(song_idx) = last_indices.song_index {
                if let Some(song) = songs.get(song_idx) {
                    *this.active_song_id.write().await = Some(song.id.to_string());
                }
            }
            if events
                .send(SetlistEvent::ActiveIndicesChanged(last_indices.clone()))
                .await
                .is_err()
            {
                debug!("SetlistService::subscribe() - client disconnected");
                return;
            }

            // Send initial transport state for all songs (one-time poll)
            let initial_transports = this.get_all_song_transports().await;
            let mut last_transport_by_song: FxHashMap<usize, SongTransportState> =
                initial_transports
                    .iter()
                    .cloned()
                    .map(|transport| (transport.song_index, transport))
                    .collect();
            if events
                .send(SetlistEvent::TransportUpdate(initial_transports))
                .await
                .is_err()
            {
                debug!("SetlistService::subscribe() - client disconnected");
                return;
            }

            // Track current song/section for enter/exit events
            // Keyed by song_index to track section per song
            let mut last_section_by_song: FxHashMap<usize, Option<usize>> = FxHashMap::default();

            // Track last known current project GUID for detecting tab switches
            let mut last_current_project_guid: Option<String> = {
                let daw = Daw::get();
                daw.current_project()
                    .await
                    .ok()
                    .map(|p| p.guid().to_string())
            };
            let mut pending_project_switch: Option<(String, Instant)> = None;

            // Timer for checking project tab switches (every 500ms)
            // This is separate from transport updates which come at ~30Hz
            let mut project_check_interval =
                tokio::time::interval(Duration::from_millis(500));
            project_check_interval
                .set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut active_hydration_interval =
                tokio::time::interval(Duration::from_millis(ACTIVE_HYDRATION_TICK_MS));
            active_hydration_interval
                .set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut perf_log_interval = tokio::time::interval(Duration::from_secs(5));
            perf_log_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let (chart_probe_tx, mut chart_probe_rx) =
                mpsc::unbounded_channel::<(Duration, bool)>("session.setlist.polling.chart_probe");
            let chart_probe_inflight = Arc::new(AtomicBool::new(false));

            let mut transport_tick_count: u64 = 0;
            let mut transport_events_sent: u64 = 0;
            let mut transport_song_updates_sent: usize = 0;
            let mut transport_compute_total = Duration::ZERO;
            let mut chart_check_count: u64 = 0;
            let mut chart_update_count: u64 = 0;
            let mut chart_compute_total = Duration::ZERO;
            let mut last_indices_progress_emit = Instant::now();
            let mut chart_probe_poll_ms = ACTIVE_HYDRATION_POLL_MS;
            let mut chart_probe_last_started =
                Instant::now() - Duration::from_millis(ACTIVE_HYDRATION_POLL_MS);
            // Track which song index we already auto-advanced from, to prevent retriggering.
            let mut auto_advance_triggered_for: Option<usize> = None;

            // Fully reactive loop - processes transport stream updates and periodic project checks
            let mut transport_rx = transport_rx;

            loop {
                tokio::select! {
                    changed = setlist_updates.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        if let Some(setlist) = this.setlist.read().await.clone() {
                            guid_to_indices = FxHashMap::default();
                            for (idx, song) in setlist.songs.iter().enumerate() {
                                guid_to_indices
                                    .entry(song.project_guid.clone())
                                    .or_default()
                                    .push(idx);
                            }
                            song_id_to_index = setlist
                                .songs
                                .iter()
                                .enumerate()
                                .map(|(idx, song)| (song.id.to_string(), idx))
                                .collect();
                            if events
                                .send(SetlistEvent::SetlistChanged(setlist))
                                .await
                            .is_err()
                            {
                                break;
                            }
                        }
                    }

                    hydrated = hydration_rx.recv() => {
                        match hydrated {
                            Ok((index, song)) => {
                                if events
                                    .send(SetlistEvent::SongHydrated { song_id: song.id.clone(), index, song })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(BroadcastRecvError::Lagged(skipped)) => {
                                debug!("Hydration update lagged by {} messages", skipped);
                            }
                            Err(BroadcastRecvError::Closed) => break,
                        }
                    }

                    chart_hydrated = chart_hydration_rx.recv() => {
                        match chart_hydrated {
                            Ok((index, chart)) => {
                                let song_id = {
                                    let setlist = this.setlist.read().await;
                                    setlist.as_ref()
                                        .and_then(|sl| sl.songs.get(index))
                                        .map(|s| s.id.clone())
                                        .unwrap_or_else(session_proto::SongId::new)
                                };
                                if events
                                    .send(SetlistEvent::SongChartHydrated { song_id, index, chart })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(BroadcastRecvError::Lagged(skipped)) => {
                                debug!("Chart hydration update lagged by {} messages", skipped);
                            }
                            Err(BroadcastRecvError::Closed) => break,
                        }
                    }

                    // Handle transport updates from the broadcast channel
                    result = transport_rx.recv() => {
                        match result {
                    Ok(Some(update)) => {
                        let transport_started = Instant::now();
                        let active_song_id = this.active_song_id.read().await.clone();
                        let active_song_index: Option<usize>;
                        let mut song_transports: SmallVec<[SongTransportState; 16]> =
                            SmallVec::new();
                        let mut pending_section_events: SmallVec<[SetlistEvent; 8]> =
                            SmallVec::new();

                        {
                            let setlist_guard = this.setlist.read().await;
                            let Some(ref setlist_ref) = *setlist_guard else {
                                continue;
                            };

                            active_song_index = active_song_id.as_ref().and_then(|id| {
                                song_id_to_index.get(id).copied()
                            });

                            for proj_state in &update.projects {
                                if let Some(song_indices) = guid_to_indices.get(&proj_state.project_guid) {
                                    let is_playing = proj_state.transport.play_state
                                        == daw::service::PlayState::Playing
                                        || proj_state.transport.play_state
                                            == daw::service::PlayState::Recording;

                                    // Use playhead position when playing, edit cursor when stopped
                                    let position = if is_playing {
                                        proj_state.transport.playhead_position.clone()
                                    } else {
                                        proj_state.transport.edit_position.clone()
                                    };

                                    let pos_secs = position
                                        .time
                                        .as_ref()
                                        .map(|t| t.as_seconds())
                                        .unwrap_or(0.0);

                                    // Resolve which song the playhead is in (position-based)
                                    let resolved_index = song_indices
                                        .iter()
                                        .copied()
                                        .find(|&idx| {
                                            if let Some(s) = setlist_ref.songs.get(idx) {
                                                pos_secs >= s.start_seconds && pos_secs < s.end_seconds
                                            } else {
                                                false
                                            }
                                        })
                                        // Fallback: nearest song in this project
                                        .or_else(|| {
                                            song_indices
                                                .iter()
                                                .copied()
                                                .min_by(|&a, &b| {
                                                    let dist_a = setlist_ref.songs.get(a).map(|s| {
                                                        (pos_secs - s.start_seconds).abs()
                                                            .min((pos_secs - s.end_seconds).abs())
                                                    }).unwrap_or(f64::MAX);
                                                    let dist_b = setlist_ref.songs.get(b).map(|s| {
                                                        (pos_secs - s.start_seconds).abs()
                                                            .min((pos_secs - s.end_seconds).abs())
                                                    }).unwrap_or(f64::MAX);
                                                    dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
                                                })
                                        });

                                    // Emit transport for all songs in this project
                                    for &song_index in song_indices {
                                        if let Some(song) = setlist_ref.songs.get(song_index) {
                                            let song_transport = Self::calculate_song_transport(
                                                song,
                                                song_index,
                                                position.clone(),
                                                is_playing,
                                                proj_state.transport.looping,
                                                proj_state.transport.loop_region.clone(),
                                                proj_state.transport.tempo.bpm(),
                                                (
                                                    proj_state.transport.time_signature.numerator(),
                                                    proj_state.transport.time_signature.denominator(),
                                                ),
                                            );

                                            // Track section changes only for the resolved active song
                                            if Some(song_index) == resolved_index {
                                                let current_section = song_transport.section_index;
                                                let last_section =
                                                    last_section_by_song.get(&song_index).copied().flatten();

                                                if current_section != last_section {
                                                    if let Some(sec_idx) = last_section {
                                                        let section_id = song.sections.get(sec_idx)
                                                            .map(|s| s.section_id.clone())
                                                            .unwrap_or_else(session_proto::SectionId::new);
                                                        pending_section_events.push(SetlistEvent::SectionExited {
                                                            song_id: song.id.clone(),
                                                            song_index,
                                                            section_id,
                                                            section_index: sec_idx,
                                                        });
                                                    }

                                                    if let Some(sec_idx) = current_section {
                                                        if let Some(section) = song.sections.get(sec_idx) {
                                                            pending_section_events.push(SetlistEvent::SectionEntered {
                                                                song_id: song.id.clone(),
                                                                song_index,
                                                                section_id: section.section_id.clone(),
                                                                section_index: sec_idx,
                                                                section: section.clone(),
                                                            });
                                                        }
                                                    }

                                                    last_section_by_song.insert(song_index, current_section);
                                                }
                                            }
                                            song_transports.push(song_transport);
                                        }
                                    }
                                }
                            }
                        }

                        if !song_transports.is_empty() {
                            for section_event in pending_section_events {
                                if events.send(section_event).await.is_err() {
                                    break;
                                }
                            }

                            // Update ActiveIndices from the first playing song (or first song with updates)
                            // Find the currently active song based on cached ID
                            // Find transport for active song, or use first available
                            let active_transport = active_song_index
                                .and_then(|idx| {
                                    song_transports.iter().find(|t| t.song_index == idx)
                                })
                                .or_else(|| song_transports.first());

                            if let Some(transport) = active_transport {
                                // Get current position in seconds for queue checking
                                let position_seconds = transport
                                    .position
                                    .time
                                    .map(|t| t.as_seconds())
                                    .unwrap_or(0.0);

                                // Check if we've reached the queued target
                                this.check_and_clear_queue(
                                    transport.song_index,
                                    transport.section_index,
                                    position_seconds,
                                )
                                .await;

                                // Get current queued target to include in indices
                                let queued_target = this.get_queued_target().await;

                                let current_indices = ActiveIndices {
                                    song_index: Some(transport.song_index),
                                    section_index: transport.section_index,
                                    slide_index: None,
                                    song_progress: Some(transport.progress),
                                    section_progress: transport.section_progress,
                                    is_playing: transport.is_playing,
                                    looping: transport.is_looping,
                                    loop_selection: None,
                                    queued_target,
                                };

                                // Cache indices for instant navigation (no RPC needed)
                                this.set_cached_indices(current_indices.clone()).await;

                                // Only send if changed. Structural changes are immediate;
                                // progress-only changes are throttled to reduce UI churn.
                                let structural_change = Self::active_indices_structural_changed(
                                    &last_indices,
                                    &current_indices,
                                );
                                let progress_change = current_indices.song_progress
                                    != last_indices.song_progress
                                    || current_indices.section_progress
                                        != last_indices.section_progress;
                                let can_emit_progress = last_indices_progress_emit.elapsed()
                                    >= Duration::from_millis(ACTIVE_INDICES_PROGRESS_EMIT_MS);
                                let song_changed =
                                    current_indices.song_index != last_indices.song_index;

                                // ── Auto-advance: when a song ends, optionally navigate to next ──
                                if let Some(song_idx) = current_indices.song_index {
                                    let near_end = current_indices
                                        .song_progress
                                        .map(|p| p >= 0.999)
                                        .unwrap_or(false);
                                    let stopped = !current_indices.is_playing;

                                    if song_changed {
                                        // Reset auto-advance guard when the song changes.
                                        auto_advance_triggered_for = None;
                                    }

                                    if near_end
                                        && stopped
                                        && auto_advance_triggered_for != Some(song_idx)
                                    {
                                        // Extract advance decision under the read lock, then drop it
                                        // before performing any async navigation.
                                        let should_advance = {
                                            let setlist = this.setlist.read().await;
                                            setlist.as_ref().and_then(|sl| {
                                                let song = sl.songs.get(song_idx)?;
                                                let mode = song.effective_advance_mode(sl.advance_mode);
                                                if mode == AdvanceMode::AutoPlay {
                                                    let next_idx = song_idx + 1;
                                                    (next_idx < sl.songs.len()).then_some(next_idx)
                                                } else {
                                                    None
                                                }
                                            })
                                        };

                                        if let Some(next_idx) = should_advance {
                                            auto_advance_triggered_for = Some(song_idx);
                                            info!(
                                                "Auto-advance: song {} ended, advancing to song {}",
                                                song_idx, next_idx
                                            );
                                            this.seek_to_song_internal(next_idx).await;
                                        }
                                    }
                                }

                                if structural_change || (progress_change && can_emit_progress) {
                                    if events
                                        .send(SetlistEvent::ActiveIndicesChanged(
                                            current_indices.clone(),
                                        ))
                                        .await
                                    .is_err()
                                    {
                                        break;
                                    }
                                    if progress_change {
                                        last_indices_progress_emit = Instant::now();
                                    }
                                    last_indices = current_indices;
                                    if song_changed {
                                        chart_probe_poll_ms = ACTIVE_HYDRATION_POLL_MS;
                                        chart_probe_last_started = Instant::now()
                                            - Duration::from_millis(ACTIVE_HYDRATION_POLL_MS);
                                    }
                                }
                            }

                            let mut changed_transports: SmallVec<[SongTransportState; 16]> =
                                SmallVec::with_capacity(song_transports.len());
                            for transport in song_transports {
                                let changed = last_transport_by_song
                                    .get(&transport.song_index)
                                    .map(|prev| Self::transport_changed(prev, &transport))
                                    .unwrap_or(true);
                                if changed {
                                    last_transport_by_song
                                        .insert(transport.song_index, transport.clone());
                                    changed_transports.push(transport);
                                }
                            }

                            let changed_count = changed_transports.len();
                            if changed_count > 0 {
                                let changed_transports = changed_transports.into_vec();
                                if events
                                    .send(SetlistEvent::TransportUpdate(changed_transports))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                transport_events_sent += 1;
                                transport_song_updates_sent += changed_count;
                            }
                        }
                        transport_tick_count += 1;
                        transport_compute_total += transport_started.elapsed();
                    }
                    Ok(None) => {
                        // Stream ended
                        info!("SetlistService::subscribe() - transport stream ended");
                        break;
                    }
                    Err(e) => {
                        warn!(
                            "SetlistService::subscribe() - transport stream error: {}",
                            e
                        );
                        break;
                    }
                        }
                    }

                    // Periodically check if the current REAPER project tab has changed
                    _ = project_check_interval.tick() => {
                        // Avoid tab-switch churn while transport is active.
                        if this.get_cached_indices().await.is_playing {
                            pending_project_switch = None;
                            continue;
                        }

                        let daw = Daw::get();
                        if let Ok(current_project) = daw.current_project().await {
                            let current_guid = current_project.guid().to_string();

                            // No change.
                            if last_current_project_guid.as_ref() == Some(&current_guid) {
                                pending_project_switch = None;
                                continue;
                            }

                            // Debounce tab switches so transient backend jitter doesn't thrash active state.
                            let switch_stable = match pending_project_switch.as_ref() {
                                Some((pending_guid, started)) if pending_guid == &current_guid => {
                                    started.elapsed()
                                        >= Duration::from_millis(PROJECT_SWITCH_DEBOUNCE_MS)
                                }
                                _ => false,
                            };
                            if !switch_stable {
                                pending_project_switch = Some((current_guid, Instant::now()));
                                continue;
                            }

                            let (confirmed_guid, _) =
                                pending_project_switch.take().unwrap_or((current_guid, Instant::now()));
                            debug!(
                                "Project tab changed from {:?} to {}",
                                last_current_project_guid, confirmed_guid
                            );

                            // Check if previously active project was paused - if so, stop it
                            if let Some(prev_guid) = &last_current_project_guid {
                                if let Ok(prev_project) = daw.project(prev_guid).await {
                                    let transport = prev_project.transport();
                                    if let Ok(state) = transport.get_play_state().await {
                                        if state == daw::service::PlayState::Paused {
                                            // Stop the paused project
                                            debug!("Stopping paused project {}", prev_guid);
                                            let _ = transport.stop().await;
                                        }
                                    }
                                }
                            }

                            // Find the song that matches the new current project
                            if guid_to_indices.contains_key(&confirmed_guid) {
                                // Let calculate_active_indices resolve the correct
                                // song by position (handles multi-song projects).
                                let new_indices = this.calculate_active_indices().await;
                                this.set_cached_indices(new_indices.clone()).await;

                                // Update active song ID from resolved index
                                if let Some(song_idx) = new_indices.song_index {
                                    let song_id_str = {
                                        let setlist_guard = this.setlist.read().await;
                                        setlist_guard
                                            .as_ref()
                                            .and_then(|sl| sl.songs.get(song_idx))
                                            .map(|song| song.id.to_string())
                                    };
                                    if let Some(song_id_str) = song_id_str {
                                        *this.active_song_id.write().await = Some(song_id_str);
                                    }
                                }

                                if new_indices != last_indices {
                                    if events
                                        .send(SetlistEvent::ActiveIndicesChanged(new_indices.clone()))
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                    if new_indices.song_index != last_indices.song_index {
                                        chart_probe_poll_ms = ACTIVE_HYDRATION_POLL_MS;
                                        chart_probe_last_started = Instant::now()
                                            - Duration::from_millis(ACTIVE_HYDRATION_POLL_MS);
                                    }
                                    last_indices = new_indices;
                                }
                            }

                            last_current_project_guid = Some(confirmed_guid);
                        }
                    }

                    _ = active_hydration_interval.tick() => {
                        if this.get_cached_indices().await.is_playing {
                            continue;
                        }
                        if chart_probe_last_started.elapsed()
                            < Duration::from_millis(chart_probe_poll_ms)
                        {
                            continue;
                        }
                        // Keep chart probing off the hot transport/select loop.
                        // If a prior probe is still running, skip this tick.
                        if !chart_probe_inflight.swap(true, Ordering::SeqCst) {
                            chart_probe_last_started = Instant::now();
                            let this_clone = this.clone();
                            let chart_probe_tx_clone = chart_probe_tx.clone();
                            let inflight = chart_probe_inflight.clone();
                            tokio::spawn(async move {
                                let chart_started = Instant::now();
                                let updated = this_clone.refresh_active_song_chart_if_changed().await;
                                let _ = chart_probe_tx_clone.send((chart_started.elapsed(), updated));
                                inflight.store(false, Ordering::SeqCst);
                            });
                        }
                    }

                    chart_probe = chart_probe_rx.recv() => {
                        match chart_probe {
                            Some((elapsed, updated)) => {
                                chart_check_count += 1;
                                if updated {
                                    chart_update_count += 1;
                                    chart_probe_poll_ms = ACTIVE_HYDRATION_POLL_MS;
                                } else if elapsed >= Duration::from_millis(25) {
                                    chart_probe_poll_ms = (chart_probe_poll_ms.saturating_mul(2))
                                        .min(ACTIVE_HYDRATION_POLL_MAX_MS);
                                } else {
                                    // Keep lightweight probes responsive while idle.
                                    chart_probe_poll_ms = (chart_probe_poll_ms + 1000)
                                        .min(ACTIVE_HYDRATION_POLL_MAX_MS);
                                }
                                chart_compute_total += elapsed;
                            }
                            None => {
                                warn!("Chart probe channel closed");
                                break;
                            }
                        }
                    }

                    _ = perf_log_interval.tick() => {
                        if transport_tick_count == 0 && chart_check_count == 0 {
                            continue;
                        }

                        let transport_avg_ms = if transport_tick_count > 0 {
                            (transport_compute_total.as_secs_f64() * 1000.0)
                                / transport_tick_count as f64
                        } else {
                            0.0
                        };
                        let chart_avg_ms = if chart_check_count > 0 {
                            (chart_compute_total.as_secs_f64() * 1000.0)
                                / chart_check_count as f64
                        } else {
                            0.0
                        };
                        let fingerprint_support = match *this.fingerprint_method_supported.read().await {
                            Some(true) => "supported",
                            Some(false) => "unsupported",
                            None => "unknown",
                        };

                        debug!(
                            "Setlist perf (5s): transport_ticks={}, transport_events={}, transport_song_updates={}, transport_avg_ms={:.2}, chart_checks={}, chart_updates={}, chart_avg_ms={:.2}, fingerprint={}",
                            transport_tick_count,
                            transport_events_sent,
                            transport_song_updates_sent,
                            transport_avg_ms,
                            chart_check_count,
                            chart_update_count,
                            chart_avg_ms,
                            fingerprint_support
                        );

                        transport_tick_count = 0;
                        transport_events_sent = 0;
                        transport_song_updates_sent = 0;
                        transport_compute_total = Duration::ZERO;
                        chart_check_count = 0;
                        chart_update_count = 0;
                        chart_compute_total = Duration::ZERO;
                    }
                }
            }

            info!("SetlistService::subscribe() - stream ended");
        });
        Ok(())
    }

    pub(crate) async fn subscribe_active_impl(
        &self,
        indices: Tx<ActiveIndices>,
    ) -> Result<(), SessionServiceError> {
        info!("SetlistService::subscribe_active() - starting active indices stream");

        // Clone self for the spawned task
        let this = self.clone();

        // Spawn the streaming loop so this method returns immediately
        tokio::spawn(async move {
            // Send initial state
            let mut last_indices = this.calculate_active_indices().await;
            if indices.send(last_indices.clone()).await.is_err() {
                debug!(
                    "SetlistService::subscribe_active() - client disconnected during initial send"
                );
                return;
            }

            // Poll for changes at 60Hz (smooth updates during playback)
            loop {
                moire::time::sleep(Duration::from_micros(16667)).await;

                let current_indices = this.calculate_active_indices().await;

                // Only send if something changed
                if current_indices != last_indices {
                    if indices.send(current_indices.clone()).await.is_err() {
                        debug!("SetlistService::subscribe_active() - client disconnected");
                        break;
                    }
                    last_indices = current_indices;
                }
            }

            info!("SetlistService::subscribe_active() - stream ended");
        });
        Ok(())
    }

    pub(crate) async fn get_audio_latency_impl(&self) -> Result<f64, SessionServiceError> {
        let daw = Daw::get();
        Ok(daw
            .audio_engine()
            .output_latency_seconds()
            .await
            .unwrap_or(0.0))
    }

    pub(crate) async fn get_audio_latency_info_impl(
        &self,
    ) -> Result<session_proto::AudioLatencyInfo, SessionServiceError> {
        let daw = Daw::get();
        Ok(match daw.audio_engine().get_state().await {
            Ok(state) => session_proto::AudioLatencyInfo {
                input_samples: state.latency.input_samples,
                output_samples: state.latency.output_samples,
                output_seconds: state.latency.output_seconds,
                sample_rate: state.latency.sample_rate,
                is_running: state.is_running,
            },
            Err(_) => session_proto::AudioLatencyInfo::default(),
        })
    }
}

// =============================================================================
// SetlistService trait implementation — delegates to sub-module methods
// =============================================================================

impl SetlistService for SetlistServiceImpl {
    // =========================================================================
    // Query Methods
    // =========================================================================

    async fn get_setlist(&self) -> Result<session_proto::Setlist, SessionServiceError> {
        let setlist = self.setlist.read().await;
        setlist
            .clone()
            .ok_or_else(|| SessionServiceError::not_found("Setlist", "current"))
    }

    async fn get_songs(&self) -> Result<Vec<Song>, SessionServiceError> {
        let setlist = self.setlist.read().await;
        let Some(ref setlist) = *setlist else {
            return Ok(Vec::new());
        };

        Ok(setlist.songs.clone())
    }

    async fn get_song(&self, index: usize) -> Result<Song, SessionServiceError> {
        let setlist = self.setlist.read().await;
        setlist
            .as_ref()
            .and_then(|s| s.songs.get(index).cloned())
            .ok_or_else(|| SessionServiceError::not_found("Song", index))
    }

    async fn get_sections(
        &self,
        song_index: usize,
    ) -> Result<Vec<Section>, SessionServiceError> {
        let setlist = self.setlist.read().await;
        let song = setlist
            .as_ref()
            .and_then(|s| s.songs.get(song_index))
            .ok_or_else(|| SessionServiceError::not_found("Song", song_index))?;
        Ok(song.sections.clone())
    }

    async fn get_section(
        &self,
        song_index: usize,
        section_index: usize,
    ) -> Result<Section, SessionServiceError> {
        let setlist = self.setlist.read().await;
        let song = setlist
            .as_ref()
            .and_then(|s| s.songs.get(song_index))
            .ok_or_else(|| SessionServiceError::not_found("Song", song_index))?;
        song.sections
            .get(section_index)
            .cloned()
            .ok_or_else(|| SessionServiceError::not_found("Section", section_index))
    }

    async fn get_measures(
        &self,
        song_index: usize,
    ) -> Result<Vec<MeasureInfo>, SessionServiceError> {
        let setlist = self.setlist.read().await;
        let song = setlist
            .as_ref()
            .and_then(|s| s.songs.get(song_index))
            .ok_or_else(|| SessionServiceError::not_found("Song", song_index))?;

        // If we have pre-calculated measure positions, use them
        if !song.measure_positions.is_empty() {
            let ts = song
                .time_signature
                .unwrap_or(daw::service::TimeSignature::new(4, 4));
            return Ok(song
                .measure_positions
                .iter()
                .enumerate()
                .map(|(idx, pos)| MeasureInfo {
                    measure: idx as i32,
                    time_seconds: pos.time.as_ref().map(|t| t.as_seconds()).unwrap_or(0.0),
                    time_sig_numerator: ts.numerator() as i32,
                    time_sig_denominator: ts.denominator() as i32,
                })
                .collect());
        }

        // Otherwise, calculate measures from tempo and time signature
        let ts = song
            .time_signature
            .unwrap_or(daw::service::TimeSignature::new(4, 4));
        let tempo = song.tempo.unwrap_or(120.0);

        // Calculate measure duration in seconds
        let beats_per_measure = ts.numerator() as f64;
        let seconds_per_beat = 60.0 / tempo;
        let measure_duration = beats_per_measure * seconds_per_beat;

        // Generate measures for the song duration
        let song_duration = song.duration();
        let measure_count = (song_duration / measure_duration).ceil() as i32;

        Ok((0..measure_count)
            .map(|idx| MeasureInfo {
                measure: idx,
                time_seconds: song.start_seconds + (idx as f64 * measure_duration),
                time_sig_numerator: ts.numerator() as i32,
                time_sig_denominator: ts.denominator() as i32,
            })
            .collect())
    }

    async fn get_active_song(&self) -> Result<Song, SessionServiceError> {
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        let song_index = active
            .song_index
            .ok_or_else(|| SessionServiceError::not_found("Song", "active"))?;
        let setlist = self.setlist.read().await;
        setlist
            .as_ref()
            .and_then(|s| s.songs.get(song_index).cloned())
            .ok_or_else(|| SessionServiceError::not_found("Song", song_index))
    }

    async fn get_active_section(&self) -> Result<Section, SessionServiceError> {
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        let song_index = active
            .song_index
            .ok_or_else(|| SessionServiceError::not_found("Song", "active"))?;
        let section_index = active
            .section_index
            .ok_or_else(|| SessionServiceError::not_found("Section", "active"))?;
        let setlist = self.setlist.read().await;
        let song = setlist
            .as_ref()
            .and_then(|s| s.songs.get(song_index))
            .ok_or_else(|| SessionServiceError::not_found("Song", song_index))?;
        song.sections
            .get(section_index)
            .cloned()
            .ok_or_else(|| SessionServiceError::not_found("Section", section_index))
    }

    async fn get_song_at(&self, seconds: f64) -> Result<Song, SessionServiceError> {
        let setlist = self.setlist.read().await;
        let (_, song) = setlist
            .as_ref()
            .and_then(|s| s.song_at(seconds))
            .ok_or_else(|| SessionServiceError::not_found("Song", format!("at {seconds}s")))?;
        Ok(song.clone())
    }

    async fn get_section_at(&self, seconds: f64) -> Result<Section, SessionServiceError> {
        let setlist = self.setlist.read().await;
        let (_, song) = setlist
            .as_ref()
            .and_then(|s| s.song_at(seconds))
            .ok_or_else(|| SessionServiceError::not_found("Song", format!("at {seconds}s")))?;
        let (_, section) = song
            .section_at_position_with_index(seconds)
            .ok_or_else(|| {
                SessionServiceError::not_found("Section", format!("at {seconds}s"))
            })?;
        Ok(section.clone())
    }

    // =========================================================================
    // Navigation Commands — delegate to navigation.rs
    // =========================================================================

    async fn go_to_song(&self, index: usize) -> Result<(), SessionServiceError> {
        self.go_to_song_impl(index).await
    }

    async fn next_song(&self) -> Result<(), SessionServiceError> {
        self.next_song_impl().await
    }

    async fn previous_song(&self) -> Result<(), SessionServiceError> {
        self.previous_song_impl().await
    }

    async fn go_to_section(&self, index: usize) -> Result<(), SessionServiceError> {
        self.go_to_section_impl(index).await
    }

    async fn next_section(&self) -> Result<(), SessionServiceError> {
        self.next_section_impl().await
    }

    async fn previous_section(&self) -> Result<(), SessionServiceError> {
        self.previous_section_impl().await
    }

    async fn seek_to(&self, seconds: f64) -> Result<(), SessionServiceError> {
        self.seek_to_impl(seconds).await
    }

    async fn seek_to_time(
        &self,
        song_index: usize,
        seconds: f64,
    ) -> Result<(), SessionServiceError> {
        self.seek_to_time_impl(song_index, seconds).await
    }

    async fn seek_to_song(&self, song_index: usize) -> Result<(), SessionServiceError> {
        self.seek_to_song_impl(song_index).await
    }

    async fn seek_to_section(
        &self,
        song_index: usize,
        section_index: usize,
    ) -> Result<(), SessionServiceError> {
        self.seek_to_section_impl(song_index, section_index).await
    }

    async fn seek_to_musical_position(
        &self,
        song_index: usize,
        position: daw::service::MusicalPosition,
    ) -> Result<(), SessionServiceError> {
        self.seek_to_musical_position_impl(song_index, position)
            .await
    }

    async fn goto_measure(
        &self,
        song_index: usize,
        measure: i32,
    ) -> Result<(), SessionServiceError> {
        self.goto_measure_impl(song_index, measure).await
    }

    // =========================================================================
    // Playback Commands — delegate to playback.rs
    // =========================================================================

    async fn toggle_playback(&self) -> Result<(), SessionServiceError> {
        self.toggle_playback_impl().await
    }

    async fn play(&self) -> Result<(), SessionServiceError> {
        self.play_impl().await
    }

    async fn pause(&self) -> Result<(), SessionServiceError> {
        self.pause_impl().await
    }

    async fn stop(&self) -> Result<(), SessionServiceError> {
        self.stop_impl().await
    }

    async fn toggle_song_loop(&self) -> Result<(), SessionServiceError> {
        self.toggle_song_loop_impl().await
    }

    async fn toggle_section_loop(&self) -> Result<(), SessionServiceError> {
        self.toggle_section_loop_impl().await
    }

    async fn set_loop_region(
        &self,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Result<(), SessionServiceError> {
        self.set_loop_region_impl(start_seconds, end_seconds).await
    }

    async fn clear_loop(&self) -> Result<(), SessionServiceError> {
        self.clear_loop_impl().await
    }

    // =========================================================================
    // Build/Refresh — delegate to build.rs
    // =========================================================================

    async fn build_from_open_projects(&self) -> Result<(), SessionServiceError> {
        self.build_from_open_projects_impl().await
    }

    async fn refresh(&self) -> Result<(), SessionServiceError> {
        self.refresh_impl().await
    }

    // =========================================================================
    // Subscriptions — delegate to polling.rs
    // =========================================================================

    async fn subscribe(&self, events: Tx<SetlistEvent>) -> Result<(), SessionServiceError> {
        self.subscribe_impl(events).await
    }

    async fn subscribe_active(
        &self,
        indices: Tx<ActiveIndices>,
    ) -> Result<(), SessionServiceError> {
        self.subscribe_active_impl(indices).await
    }

    async fn get_audio_latency(&self) -> Result<f64, SessionServiceError> {
        self.get_audio_latency_impl().await
    }

    async fn get_audio_latency_info(
        &self,
    ) -> Result<session_proto::AudioLatencyInfo, SessionServiceError> {
        self.get_audio_latency_info_impl().await
    }
}
