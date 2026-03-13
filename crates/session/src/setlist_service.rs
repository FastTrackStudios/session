//! SetlistService implementation

use crate::song_builder::SongBuilder;
use daw_control::Daw;
use roam::Tx;
use rustc_hash::{FxHashMap, FxHashSet};
use session_proto::{
    ActiveIndices, AdvanceMode, MeasureInfo, QueuedTarget, Section, Setlist, SetlistEvent,
    SetlistService, Song, SongChartHydration, SongDetectedChord, SongTransportState,
};
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore, broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

const HYDRATION_CONCURRENCY: usize = 12;
const MIDI_TRACK_TAG: &str = "CHORDS";
const ACTIVE_HYDRATION_POLL_MS: u64 = 2000;
const ACTIVE_HYDRATION_TICK_MS: u64 = 500;
const ACTIVE_HYDRATION_POLL_MAX_MS: u64 = 15000;
const ACTIVE_INDICES_PROGRESS_EMIT_MS: u64 = 100;
const CHART_REFRESH_FALLBACK_POLL_MS: u64 = 5000;
const PROJECT_SWITCH_DEBOUNCE_MS: u64 = 900;
const TRANSPORT_TIME_EPSILON_SECS: f64 = 0.002;
const TRANSPORT_PROGRESS_EPSILON: f64 = 0.0005;

#[derive(Clone)]
struct SongCacheEntry {
    project_name: String,
    chart_fingerprint: Option<String>,
    song: Song,
}

#[derive(Clone)]
struct ProjectLoad {
    index: usize,
    project: daw_control::Project,
    guid: String,
    project_name: String,
}

/// Implementation of SetlistService
#[derive(Clone)]
pub struct SetlistServiceImpl {
    /// Current setlist state
    setlist: Arc<RwLock<Option<Setlist>>>,
    /// Currently active song ID (cached locally to avoid RPC calls)
    /// Using ID instead of index ensures stability when songs are reordered
    active_song_id: Arc<RwLock<Option<String>>>,
    /// Cached active indices (updated by polling loop at 60Hz)
    /// Used for instant navigation without RPC calls
    cached_indices: Arc<RwLock<ActiveIndices>>,
    /// Queued navigation target (flashes in UI until transport reaches it)
    /// Only one target can be queued at a time
    queued_target: Arc<RwLock<Option<QueuedTarget>>>,
    /// Monotonic setlist revision stream for subscribers (hydration/build updates)
    setlist_update_tx: watch::Sender<u64>,
    /// Last setlist revision value
    setlist_revision: Arc<AtomicU64>,
    /// Full-song cache keyed by project GUID
    song_cache: Arc<RwLock<FxHashMap<String, SongCacheEntry>>>,
    /// Delta updates for individual hydrated songs
    hydration_tx: broadcast::Sender<(usize, Song)>,
    /// Delta updates for hydrated chart payloads
    chart_hydration_tx: broadcast::Sender<(usize, SongChartHydration)>,
    /// Chart payload cache keyed by project GUID
    chart_cache: Arc<RwLock<FxHashMap<String, SongChartHydration>>>,
    /// Whether the connected DAW backend supports source_fingerprint().
    /// None = unknown, Some(true) = supported, Some(false) = unsupported.
    fingerprint_method_supported: Arc<RwLock<Option<bool>>>,
    /// Last fallback chart refresh attempt when fingerprint API is unavailable.
    last_chart_refresh_attempt: Arc<RwLock<FxHashMap<String, Instant>>>,
    /// Monotonic build generation to cancel stale background hydration tasks
    build_generation: Arc<AtomicU64>,
}

impl SetlistServiceImpl {
    pub fn new() -> Self {
        let (setlist_update_tx, _setlist_update_rx) = watch::channel(0_u64);
        let (hydration_tx, _hydration_rx) = broadcast::channel(1024);
        let (chart_hydration_tx, _chart_hydration_rx) = broadcast::channel(1024);
        Self {
            setlist: Arc::new(RwLock::new(None)),
            active_song_id: Arc::new(RwLock::new(None)),
            cached_indices: Arc::new(RwLock::new(ActiveIndices::default())),
            queued_target: Arc::new(RwLock::new(None)),
            setlist_update_tx,
            setlist_revision: Arc::new(AtomicU64::new(0)),
            song_cache: Arc::new(RwLock::new(FxHashMap::default())),
            hydration_tx,
            chart_hydration_tx,
            chart_cache: Arc::new(RwLock::new(FxHashMap::default())),
            fingerprint_method_supported: Arc::new(RwLock::new(None)),
            last_chart_refresh_attempt: Arc::new(RwLock::new(FxHashMap::default())),
            build_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get the cached active indices (updated by polling loop, no RPC calls)
    async fn get_cached_indices(&self) -> ActiveIndices {
        self.cached_indices.read().await.clone()
    }

    /// Update cached indices (called by polling loop)
    async fn set_cached_indices(&self, indices: ActiveIndices) {
        *self.cached_indices.write().await = indices;
    }

    /// Set a queued navigation target
    async fn queue_target(&self, target: QueuedTarget) {
        *self.queued_target.write().await = Some(target);
    }

    /// Clear the queued navigation target
    async fn clear_queued_target(&self) {
        *self.queued_target.write().await = None;
    }

    /// Get the current queued target
    async fn get_queued_target(&self) -> Option<QueuedTarget> {
        self.queued_target.read().await.clone()
    }

    fn active_indices_structural_changed(prev: &ActiveIndices, next: &ActiveIndices) -> bool {
        prev.song_index != next.song_index
            || prev.section_index != next.section_index
            || prev.slide_index != next.slide_index
            || prev.is_playing != next.is_playing
            || prev.looping != next.looping
            || prev.loop_selection != next.loop_selection
            || prev.queued_target != next.queued_target
    }

    /// Check if the transport has reached the queued target and clear it if so
    async fn check_and_clear_queue(
        &self,
        song_index: usize,
        section_index: Option<usize>,
        position_seconds: f64,
    ) {
        let queued = self.queued_target.read().await.clone();
        if let Some(target) = queued {
            let reached = match &target {
                QueuedTarget::Section {
                    song_index: q_song,
                    section_index: q_section,
                } => song_index == *q_song && section_index == Some(*q_section),
                QueuedTarget::Time {
                    song_index: q_song,
                    position_seconds: q_pos,
                } => {
                    // Consider reached if within 0.1 seconds
                    song_index == *q_song && (position_seconds - q_pos).abs() < 0.1
                }
                QueuedTarget::Measure {
                    song_index: q_song, ..
                } => {
                    // For measures, we'd need to check musical position - for now just check song
                    song_index == *q_song
                }
                QueuedTarget::Comment {
                    song_index: q_song,
                    position_seconds: q_pos,
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

    /// Get a specific song by index (internal helper)
    async fn get_song_internal(&self, index: usize) -> Option<session_proto::Song> {
        let setlist = self.setlist.read().await;
        setlist.as_ref()?.songs.get(index).cloned()
    }

    fn song_is_hydrated(song: &Song) -> bool {
        !song.sections.is_empty() || song.end_seconds > song.start_seconds
    }

    async fn ensure_song_hydrated(&self, index: usize) -> Option<Song> {
        let current = self.get_song_internal(index).await?;
        if Self::song_is_hydrated(&current) {
            return Some(current);
        }

        let daw = Daw::get();
        let project = daw.project(&current.project_guid).await.ok()?;
        let load = ProjectLoad {
            index,
            guid: current.project_guid.clone(),
            project_name: project
                .info()
                .await
                .ok()
                .map(|info| info.name)
                .unwrap_or_else(|| current.project_guid.clone()),
            project,
        };
        let rebuilt = self.build_songs_with_cache(&load, Some(&current)).await.into_iter().next()?;
        self.cache_chart_payload_for_song(&rebuilt).await;
        let mut rebuilt_light = rebuilt.clone();
        Self::strip_song_chart_payload(&mut rebuilt_light);

        let updated_setlist = {
            let mut guard = self.setlist.write().await;
            let Some(ref mut setlist) = *guard else {
                return Some(rebuilt_light);
            };
            if index < setlist.songs.len() {
                setlist.songs[index] = rebuilt_light.clone();
                Some(setlist.clone())
            } else {
                None
            }
        };

        if updated_setlist.is_some() {
            let _ = self.hydration_tx.send((index, rebuilt_light.clone()));
            self.emit_cached_chart_payload_for_song(index, &rebuilt_light.project_guid)
                .await;
        }

        Some(rebuilt_light)
    }

    /// Get a specific song by ID (internal helper)
    async fn get_song_by_id(&self, id: &str) -> Option<Song> {
        let setlist = self.setlist.read().await;
        let setlist = setlist.as_ref()?;
        setlist.songs.iter().find(|song| song.id == id).cloned()
    }

    /// Get the active song from cached local state (no RPC calls)
    async fn get_cached_active_song(&self) -> Option<Song> {
        let song_id = self.active_song_id.read().await.clone();
        let song_id = song_id?;
        self.get_song_by_id(&song_id).await
    }

    /// Set the active song by ID (called when navigating)
    async fn set_active_song_id(&self, id: &str) {
        *self.active_song_id.write().await = Some(id.to_string());
    }

    fn notify_setlist_changed(&self) {
        let revision = self.setlist_revision.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.setlist_update_tx.send(revision);
    }

    fn parse_project_name_fallback(project_name: &str) -> String {
        let base = project_name.strip_suffix(".rpp").unwrap_or(project_name);
        base.split(" - ").next().unwrap_or(base).trim().to_string()
    }

    fn make_song_id(
        existing_song: Option<&Song>,
        cached_song: Option<&Song>,
        guid: &str,
    ) -> String {
        if let Some(song) = existing_song {
            return song.id.clone();
        }
        if let Some(song) = cached_song {
            return song.id.clone();
        }
        guid.to_string()
    }

    fn make_name_only_song(
        guid: &str,
        project_name: &str,
        existing_song: Option<&Song>,
        cached_song: Option<&Song>,
    ) -> Song {
        Song {
            id: Self::make_song_id(existing_song, cached_song, guid),
            name: Self::parse_project_name_fallback(project_name),
            project_guid: guid.to_string(),
            start_seconds: 0.0,
            end_seconds: 0.0,
            count_in_seconds: None,
            sections: Vec::new(),
            comments: Vec::new(),
            tempo: None,
            time_signature: None,
            measure_positions: Vec::new(),
            chart_text: None,
            parsed_chart: None,
            detected_chords: Vec::new(),
            chart_fingerprint: None,
            advance_mode: None,
        }
    }

    fn strip_song_chart_payload(song: &mut Song) {
        song.chart_text = None;
        song.parsed_chart = None;
        song.detected_chords.clear();
    }

    fn map_detected_chords(chords: Vec<daw_proto::MidiDetectedChord>) -> Vec<SongDetectedChord> {
        chords
            .into_iter()
            .map(|chord| SongDetectedChord {
                symbol: chord.symbol,
                start_ppq: chord.start_ppq,
                end_ppq: chord.end_ppq,
                root_pitch: chord.root_pitch,
                velocity: chord.velocity,
            })
            .collect()
    }

    async fn fetch_midi_chart_data(
        project: &daw_control::Project,
    ) -> Option<daw_proto::MidiChartData> {
        let track_tag = Some(MIDI_TRACK_TAG.to_string());
        match project.midi_analysis().generate_chart_data(track_tag).await {
            Ok(data) => Some(data),
            Err(err) => {
                debug!(
                    "MIDI chart generation unavailable for project {}: {}",
                    project.guid(),
                    err
                );
                None
            }
        }
    }

    async fn fetch_midi_source_fingerprint(
        &self,
        project: &daw_control::Project,
    ) -> Option<String> {
        let support_state = *self.fingerprint_method_supported.read().await;
        if support_state == Some(false) {
            return None;
        }

        let track_tag = Some(MIDI_TRACK_TAG.to_string());
        match project.midi_analysis().source_fingerprint(track_tag).await {
            Ok(fingerprint) => {
                if support_state != Some(true) {
                    *self.fingerprint_method_supported.write().await = Some(true);
                }
                Some(fingerprint)
            }
            Err(err) => {
                let err_text = err.to_string();
                if err_text.contains("UnknownMethod") {
                    let mut guard = self.fingerprint_method_supported.write().await;
                    if *guard != Some(false) {
                        info!(
                            "MIDI source fingerprint unsupported by DAW backend; disabling fingerprint polling"
                        );
                    }
                    *guard = Some(false);
                } else {
                    debug!(
                        "MIDI source fingerprint unavailable for project {}: {}",
                        project.guid(),
                        err_text
                    );
                }
                None
            }
        }
    }

    async fn should_run_fallback_chart_refresh(&self, project_guid: &str) -> bool {
        let now = Instant::now();
        let mut guard = self.last_chart_refresh_attempt.write().await;
        if let Some(last) = guard.get(project_guid)
            && now.duration_since(*last) < Duration::from_millis(CHART_REFRESH_FALLBACK_POLL_MS)
        {
            return false;
        }
        guard.insert(project_guid.to_string(), now);
        true
    }

    fn apply_chart_data(song: &mut Song, chart_data: daw_proto::MidiChartData) {
        song.chart_fingerprint = Some(chart_data.source_fingerprint);
        song.chart_text = Some(chart_data.chart_text);
        // Keep parsed_chart empty in session state to avoid cloning large chart
        // structures through transport/setlist event paths.
        song.parsed_chart = None;
        song.detected_chords = Self::map_detected_chords(chart_data.chords);
    }

    fn chart_payload_from_song(song: &Song) -> Option<SongChartHydration> {
        let chart_text = song.chart_text.clone()?;
        let chart_fingerprint = song.chart_fingerprint.clone()?;
        Some(SongChartHydration {
            project_guid: song.project_guid.clone(),
            chart_text,
            detected_chords: song.detected_chords.clone(),
            chart_fingerprint,
        })
    }

    async fn cache_chart_payload_for_song(&self, song: &Song) {
        if let Some(payload) = Self::chart_payload_from_song(song) {
            self.chart_cache
                .write()
                .await
                .insert(song.project_guid.clone(), payload);
        }
    }

    async fn emit_cached_chart_payload_for_song(&self, index: usize, project_guid: &str) {
        if let Some(payload) = self.chart_cache.read().await.get(project_guid).cloned() {
            let _ = self.chart_hydration_tx.send((index, payload));
        }
    }

    async fn fetch_project_loads(projects: Vec<daw_control::Project>) -> Vec<ProjectLoad> {
        let semaphore = Arc::new(Semaphore::new(HYDRATION_CONCURRENCY));
        let mut join_set = JoinSet::new();

        for (index, project) in projects.into_iter().enumerate() {
            let permit = semaphore.clone();
            join_set.spawn(async move {
                let _permit = permit.acquire_owned().await.expect("semaphore closed");
                let guid = project.guid().to_string();
                let project_name = project
                    .info()
                    .await
                    .map(|info| info.name)
                    .unwrap_or_else(|_| guid.clone());
                ProjectLoad {
                    index,
                    project,
                    guid,
                    project_name,
                }
            });
        }

        let mut loads = Vec::new();
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(load) => loads.push(load),
                Err(e) => warn!("Failed to load project metadata: {}", e),
            }
        }
        loads.sort_by_key(|load| load.index);
        loads
    }

    async fn build_songs_with_cache(
        &self,
        load: &ProjectLoad,
        existing_song: Option<&Song>,
    ) -> Vec<Song> {
        let source_fingerprint = self.fetch_midi_source_fingerprint(&load.project).await;

        if let Some(cached) = self.song_cache.read().await.get(&load.guid).cloned() {
            let fingerprint_matches = match source_fingerprint.as_ref() {
                Some(fingerprint) => cached.chart_fingerprint.as_deref() == Some(fingerprint),
                None => true,
            };

            if cached.project_name == load.project_name && fingerprint_matches {
                let mut song = cached.song;
                song.id = Self::make_song_id(existing_song, Some(&song), &load.guid);
                return vec![song];
            }
        }

        let chart_data = Self::fetch_midi_chart_data(&load.project).await;

        match SongBuilder::build(&load.project).await {
            Ok(mut songs) => {
                // Apply chart data and IDs to the first song; cache only the first song
                if let Some(first_song) = songs.first_mut() {
                    first_song.id =
                        Self::make_song_id(existing_song, None, &load.guid);
                    if let Some(data) = chart_data {
                        Self::apply_chart_data(first_song, data);
                    }
                    self.cache_chart_payload_for_song(first_song).await;
                    let mut cached_song = first_song.clone();
                    Self::strip_song_chart_payload(&mut cached_song);
                    self.song_cache.write().await.insert(
                        load.guid.clone(),
                        SongCacheEntry {
                            project_name: load.project_name.clone(),
                            chart_fingerprint: first_song.chart_fingerprint.clone(),
                            song: cached_song,
                        },
                    );
                }
                // Ensure remaining songs have unique IDs
                for song in songs.iter_mut().skip(1) {
                    if song.id.is_empty() {
                        song.id = uuid::Uuid::new_v4().to_string();
                    }
                }
                songs
            }
            Err(e) => {
                warn!(
                    "Failed to extract song from project {} ({}): {}",
                    load.index + 1,
                    load.guid,
                    e
                );
                Vec::new()
            }
        }
    }

    /// Calculate transport state for a specific song based on its project's transport
    fn calculate_song_transport(
        song: &Song,
        song_index: usize,
        position: daw_proto::Position,
        is_playing: bool,
        is_looping: bool,
        loop_region: Option<daw_proto::LoopRegion>,
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
                Some(daw_proto::LoopRegion::new(
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
        prev: &Option<daw_proto::LoopRegion>,
        next: &Option<daw_proto::LoopRegion>,
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

    fn transport_changed(prev: &SongTransportState, next: &SongTransportState) -> bool {
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
    async fn get_all_song_transports(&self) -> Vec<SongTransportState> {
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

        let semaphore = Arc::new(Semaphore::new(HYDRATION_CONCURRENCY));
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
                                let is_playing = state.play_state == daw_proto::PlayState::Playing
                                    || state.play_state == daw_proto::PlayState::Recording;

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

    async fn refresh_active_song_chart_if_changed(&self) -> bool {
        // Avoid expensive chart fingerprint/chart generation probes while playing.
        // These probes can take tens of milliseconds and compete with transport streaming.
        if self.get_cached_indices().await.is_playing {
            return false;
        }

        let active_song_id = self.active_song_id.read().await.clone();
        let Some(active_song_id) = active_song_id else {
            return false;
        };

        let (song_index, song_snapshot) = {
            let guard = self.setlist.read().await;
            let Some(ref setlist) = *guard else {
                return false;
            };
            let Some((index, song)) = setlist
                .songs
                .iter()
                .enumerate()
                .find(|(_, song)| song.id == active_song_id)
            else {
                return false;
            };
            (index, song.clone())
        };

        let fingerprint_supported = *self.fingerprint_method_supported.read().await != Some(false);
        if !fingerprint_supported
            && !self
                .should_run_fallback_chart_refresh(&song_snapshot.project_guid)
                .await
        {
            return false;
        }

        let daw = Daw::get();
        let Ok(project) = daw.project(&song_snapshot.project_guid).await else {
            return false;
        };
        let source_fingerprint = if fingerprint_supported {
            self.fetch_midi_source_fingerprint(&project).await
        } else {
            None
        };

        if let Some(ref fingerprint) = source_fingerprint
            && song_snapshot.chart_fingerprint.as_deref() == Some(fingerprint.as_str())
        {
            return false;
        }

        let Some(chart_data) = Self::fetch_midi_chart_data(&project).await else {
            return false;
        };
        if source_fingerprint.is_none()
            && song_snapshot.chart_fingerprint.as_deref()
                == Some(chart_data.source_fingerprint.as_str())
        {
            return false;
        }

        let mut updated_song = song_snapshot.clone();
        Self::apply_chart_data(&mut updated_song, chart_data);
        self.cache_chart_payload_for_song(&updated_song).await;
        let mut updated_song_light = updated_song.clone();
        Self::strip_song_chart_payload(&mut updated_song_light);

        let updated = {
            let mut guard = self.setlist.write().await;
            let Some(ref mut setlist) = *guard else {
                return false;
            };
            if song_index >= setlist.songs.len() {
                return false;
            }
            setlist.songs[song_index] = updated_song_light.clone();
            true
        };

        if updated {
            let _ = self
                .hydration_tx
                .send((song_index, updated_song_light.clone()));
            self.emit_cached_chart_payload_for_song(song_index, &updated_song_light.project_guid)
                .await;
            let project_name = project
                .info()
                .await
                .ok()
                .map(|info| info.name)
                .unwrap_or_else(|| song_snapshot.project_guid.clone());
            self.song_cache.write().await.insert(
                song_snapshot.project_guid.clone(),
                SongCacheEntry {
                    project_name,
                    chart_fingerprint: updated_song_light.chart_fingerprint.clone(),
                    song: updated_song_light,
                },
            );
            return true;
        }
        false
    }

    /// Find the active song and section based on current DAW project
    async fn calculate_active_indices(&self) -> ActiveIndices {
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

    /// Stop the previously active project if it's not playing.
    /// Called before switching to a new project so paused projects don't linger.
    async fn stop_previous_project_if_idle(&self, new_project_guid: &str) {
        let prev_song = self.get_cached_active_song().await;
        if let Some(prev) = prev_song {
            if prev.project_guid != new_project_guid {
                let daw = Daw::get();
                if let Ok(prev_project) = daw.project(&prev.project_guid).await {
                    let transport = prev_project.transport();
                    if let Ok(state) = transport.get_play_state().await {
                        if state != daw_proto::PlayState::Playing {
                            debug!(
                                "Stopping idle project {} (state={:?}) before switching",
                                prev.project_guid, state
                            );
                            let _ = transport.stop().await;
                        }
                    }
                }
            }
        }
    }

    /// Compute the seek position for a song: count-in position if available,
    /// otherwise song start (SONGSTART marker).
    ///
    /// Song.start_seconds already includes the count-in offset (it's the count-in
    /// start, not SONGSTART). So the SONGSTART position is:
    ///   start_seconds + count_in_seconds
    ///
    /// We want to seek to start_seconds (the count-in position), but if that's
    /// negative (count-in marker is before timeline origin), fall back to SONGSTART.
    fn song_seek_position(song: &Song) -> f64 {
        let count_in_pos = song.start_seconds();
        if count_in_pos >= 0.0 {
            count_in_pos
        } else {
            // Count-in is before timeline origin — fall back to SONGSTART
            let songstart = song.start_seconds() + song.count_in_seconds.unwrap_or(0.0);
            debug!(
                "song_seek_position: count-in pos {:.2} is negative, falling back to SONGSTART {:.2}",
                count_in_pos, songstart
            );
            songstart.max(0.0)
        }
    }

    /// Navigate to a song by index without requiring an RPC Context.
    /// Used by the transport loop for auto-advance.
    async fn seek_to_song_internal(&self, song_index: usize) {
        info!("seek_to_song_internal: song_index={}", song_index);

        let daw = Daw::get();

        let Some(skeleton) = self.get_song_internal(song_index).await else {
            warn!("seek_to_song_internal: song {} not found", song_index);
            return;
        };

        // Check if we're already on the correct project (skip tab switch for same-project songs)
        let current_project = daw.current_project().await.ok();
        let already_on_project = current_project
            .as_ref()
            .map(|p| p.guid().to_string() == skeleton.project_guid)
            .unwrap_or(false);

        if !already_on_project {
            self.stop_previous_project_if_idle(&skeleton.project_guid)
                .await;
        }
        self.set_active_song_id(&skeleton.id).await;

        let project = if already_on_project {
            current_project.unwrap()
        } else {
            match daw.select_project(&skeleton.project_guid).await {
                Ok(project) => project,
                Err(e) => {
                    warn!(
                        "seek_to_song_internal: failed to switch to project {}: {}",
                        skeleton.project_guid, e
                    );
                    return;
                }
            }
        };

        let song = match tokio::time::timeout(
            Duration::from_secs(5),
            self.ensure_song_hydrated(song_index),
        )
        .await
        {
            Ok(Some(song)) => song,
            _ => {
                warn!(
                    "seek_to_song_internal: hydration failed for song {} ({})",
                    song_index, skeleton.name
                );
                return;
            }
        };

        let transport = project.transport();
        let seek_pos = Self::song_seek_position(&song);
        if let Err(e) = transport.set_position(seek_pos).await {
            warn!("seek_to_song_internal: failed to seek: {}", e);
        }
        // Start playback for auto-advance
        if let Err(e) = transport.play().await {
            warn!("seek_to_song_internal: failed to start playback: {}", e);
        }
    }
}

impl SetlistService for SetlistServiceImpl {
    // =========================================================================
    // Query Methods
    // =========================================================================

    async fn get_setlist(&self) -> Option<Setlist> {
        let setlist = self.setlist.read().await;
        setlist.clone()
    }

    async fn get_songs(&self) -> Vec<Song> {
        let setlist = self.setlist.read().await;
        let Some(ref setlist) = *setlist else {
            return Vec::new();
        };

        setlist.songs.clone()
    }

    async fn get_song(&self, index: usize) -> Option<Song> {
        let setlist = self.setlist.read().await;
        setlist.as_ref()?.songs.get(index).cloned()
    }

    async fn get_sections(&self, song_index: usize) -> Vec<Section> {
        let setlist = self.setlist.read().await;
        if let Some(song) = setlist.as_ref().and_then(|s| s.songs.get(song_index)) {
            song.sections.clone()
        } else {
            Vec::new()
        }
    }

    async fn get_section(
        &self,
                song_index: usize,
        section_index: usize,
    ) -> Option<Section> {
        let setlist = self.setlist.read().await;
        let song = setlist.as_ref()?.songs.get(song_index)?;
        song.sections.get(section_index).cloned()
    }

    async fn get_measures(&self, song_index: usize) -> Vec<MeasureInfo> {
        let setlist = self.setlist.read().await;
        if let Some(song) = setlist.as_ref().and_then(|s| s.songs.get(song_index)) {
            // If we have pre-calculated measure positions, use them
            if !song.measure_positions.is_empty() {
                let ts = song
                    .time_signature
                    .unwrap_or(daw_proto::TimeSignature::new(4, 4));
                return song
                    .measure_positions
                    .iter()
                    .enumerate()
                    .map(|(idx, pos)| MeasureInfo {
                        measure: idx as i32,
                        time_seconds: pos.time.as_ref().map(|t| t.as_seconds()).unwrap_or(0.0),
                        time_sig_numerator: ts.numerator() as i32,
                        time_sig_denominator: ts.denominator() as i32,
                    })
                    .collect();
            }

            // Otherwise, calculate measures from tempo and time signature
            let ts = song
                .time_signature
                .unwrap_or(daw_proto::TimeSignature::new(4, 4));
            let tempo = song.tempo.unwrap_or(120.0);

            // Calculate measure duration in seconds
            let beats_per_measure = ts.numerator() as f64;
            let seconds_per_beat = 60.0 / tempo;
            let measure_duration = beats_per_measure * seconds_per_beat;

            // Generate measures for the song duration
            let song_duration = song.duration();
            let measure_count = (song_duration / measure_duration).ceil() as i32;

            (0..measure_count)
                .map(|idx| MeasureInfo {
                    measure: idx,
                    time_seconds: song.start_seconds + (idx as f64 * measure_duration),
                    time_sig_numerator: ts.numerator() as i32,
                    time_sig_denominator: ts.denominator() as i32,
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    async fn get_active_song(&self) -> Option<Song> {
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        let song_index = active.song_index?;
        let setlist = self.setlist.read().await;
        setlist.as_ref()?.songs.get(song_index).cloned()
    }

    async fn get_active_section(&self) -> Option<Section> {
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        let song_index = active.song_index?;
        let section_index = active.section_index?;
        let setlist = self.setlist.read().await;
        let song = setlist.as_ref()?.songs.get(song_index)?;
        song.sections.get(section_index).cloned()
    }

    async fn get_song_at(&self, seconds: f64) -> Option<Song> {
        let setlist = self.setlist.read().await;
        let (_, song) = setlist.as_ref()?.song_at(seconds)?;
        Some(song.clone())
    }

    async fn get_section_at(&self, seconds: f64) -> Option<Section> {
        let setlist = self.setlist.read().await;
        let (_, song) = setlist.as_ref()?.song_at(seconds)?;
        let (_, section) = song.section_at_position_with_index(seconds)?;
        Some(section.clone())
    }

    // =========================================================================
    // Navigation Commands
    // =========================================================================

    async fn go_to_song(&self, index: usize) {
        debug!("go_to_song: {}", index);

        let daw = Daw::get();

        // Get the skeleton song first (no RPC, just reads from the setlist).
        // This gives us the project GUID so we can switch tabs even if hydration fails.
        let Some(skeleton) = self.get_song_internal(index).await else {
            warn!("go_to_song: song {} not found in setlist", index);
            return;
        };

        // Stop previous project if it's not actively playing
        self.stop_previous_project_if_idle(&skeleton.project_guid)
            .await;

        // Update the cached active song ID for fast playback commands
        self.set_active_song_id(&skeleton.id).await;

        // Switch to the correct project tab first — this must always happen
        let project = match daw.select_project(&skeleton.project_guid).await {
            Ok(project) => project,
            Err(e) => {
                warn!(
                    "Failed to switch to project {} for song {}: {}",
                    skeleton.project_guid, index, e
                );
                return;
            }
        };

        // Now try to hydrate (best-effort, with timeout to prevent freezes)
        let song = match tokio::time::timeout(
            Duration::from_secs(5),
            self.ensure_song_hydrated(index),
        )
        .await
        {
            Ok(Some(song)) => song,
            Ok(None) => {
                warn!(
                    "go_to_song: hydration returned None for song {} ({}), staying on project tab",
                    index, skeleton.name
                );
                return;
            }
            Err(_) => {
                warn!(
                    "go_to_song: hydration timed out for song {} ({}), staying on project tab",
                    index, skeleton.name
                );
                return;
            }
        };

        let transport = project.transport();

        // Only seek if the project is NOT already playing
        // This preserves playback position when switching between songs
        let is_playing = transport.is_playing().await.unwrap_or(false);
        if !is_playing {
            let seek_pos = Self::song_seek_position(&song);
            if let Err(e) = transport.set_position(seek_pos).await {
                warn!("Failed to set position for song {}: {}", index, e);
            }
        }

        // Log the actual cursor position after navigation
        let actual_pos = transport.get_position().await.unwrap_or(f64::NAN);
        info!(
            "go_to_song: song {} ({}) — seek_target={:.2}s, actual_pos={:.2}s, is_playing={}",
            index,
            song.name,
            Self::song_seek_position(&song),
            actual_pos,
            is_playing
        );
    }

    async fn next_song(&self) {
        let active = self.get_cached_indices().await;
        info!("next_song: cached song_index={:?}", active.song_index);
        if let Some(current_idx) = active.song_index {
            let next_idx = current_idx + 1;
            self.go_to_song(next_idx).await;
        } else {
            warn!("next_song: no active song index, cannot navigate");
        }
    }

    async fn previous_song(&self) {
        let active = self.get_cached_indices().await;
        info!("previous_song: cached song_index={:?}", active.song_index);
        if let Some(current_idx) = active.song_index {
            if current_idx > 0 {
                let prev_idx = current_idx - 1;
                self.go_to_song(prev_idx).await;
            } else {
                info!("previous_song: already at first song (index 0)");
            }
        } else {
            warn!("previous_song: no active song index, cannot navigate");
        }
    }

    async fn go_to_section(&self, index: usize) {
        debug!("go_to_section: {}", index);

        let daw = Daw::get();
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;

        if let Some(song_idx) = active.song_index {
            // Queue the target immediately for visual feedback
            self.queue_target(QueuedTarget::Section {
                song_index: song_idx,
                section_index: index,
            })
            .await;

            if let Some(song) = self.ensure_song_hydrated(song_idx).await {
                if let Some(section) = song.sections.get(index) {
                    // First, switch to the correct project (in case we're on a different one)
                    match daw.select_project(&song.project_guid).await {
                        Ok(project) => {
                            // Then seek to the section's start position
                            if let Err(e) = project
                                .transport()
                                .set_position(section.start_seconds)
                                .await
                            {
                                warn!("Failed to navigate to section {}: {}", index, e);
                                // Clear queue on failure
                                self.clear_queued_target().await;
                            } else {
                                info!(
                                    "Navigated to section {} ({}) in song {} (project {})",
                                    index, section.name, song.name, song.project_guid
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to switch to project {} for section navigation: {}",
                                song.project_guid, e
                            );
                        }
                    }
                }
            }
        }
    }

    async fn next_section(&self) {
        debug!("next_section");

        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        if let Some(section_idx) = active.section_index {
            let next_idx = section_idx + 1;
            self.go_to_section(next_idx).await;
        }
    }

    async fn previous_section(&self) {
        debug!("previous_section");

        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        if let Some(section_idx) = active.section_index {
            // Smart previous: if we're past the beginning of the section (>5% progress),
            // go to the start of the current section. Only go to previous section
            // if we're already at/near the beginning.
            let at_section_start = active
                .section_progress
                .map(|p| p < 0.05) // Within first 5% of section
                .unwrap_or(true);

            if at_section_start && section_idx > 0 {
                // Already at the start, go to previous section
                let prev_idx = section_idx - 1;
                self.go_to_section(prev_idx).await;
            } else {
                // Not at start, go to beginning of current section
                self.go_to_section(section_idx).await;
            }
        }
    }

    async fn seek_to(&self, seconds: f64) {
        debug!("seek_to: {}", seconds);

        let daw = Daw::get();
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;

        if let Some(song_idx) = active.song_index {
            if let Some(song) = self.ensure_song_hydrated(song_idx).await {
                let absolute_pos = song.start_seconds() + seconds;
                match daw.project(&song.project_guid).await {
                    Ok(project) => {
                        if let Err(e) = project.transport().set_position(absolute_pos).await {
                            warn!("Failed to seek to {}: {}", seconds, e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to get project: {}", e);
                    }
                }
            }
        }
    }

    async fn seek_to_time(&self, song_index: usize, seconds: f64) {
        info!(
            "seek_to_time: song_index={}, seconds={}",
            song_index, seconds
        );

        // Queue the target immediately for visual feedback (comment marker)
        self.queue_target(QueuedTarget::Comment {
            song_index,
            position_seconds: seconds,
        })
        .await;

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            // First, switch to the correct project
            match daw.select_project(&song.project_guid).await {
                Ok(project) => {
                    // Seek to the absolute time position
                    if let Err(e) = project.transport().set_position(seconds).await {
                        warn!(
                            "Failed to seek to {} seconds in song {}: {}",
                            seconds, song_index, e
                        );
                        // Clear queue on failure
                        self.clear_queued_target().await;
                    } else {
                        info!(
                            "Seeked to {} seconds in song {} ({})",
                            seconds, song_index, song.name
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        song.project_guid, song_index, e
                    );
                }
            }
        } else {
            warn!("Song {} not found", song_index);
        }
    }

    async fn seek_to_song(&self, song_index: usize) {
        info!("seek_to_song called: song_index={}", song_index);

        let daw = Daw::get();

        // Get the skeleton song first so we can switch tabs even if hydration fails.
        let Some(skeleton) = self.get_song_internal(song_index).await else {
            warn!("seek_to_song: song {} not found in setlist", song_index);
            return;
        };

        // Check if we're already on the correct project (skip tab switch for same-project songs)
        let current_project = daw.current_project().await.ok();
        let already_on_project = current_project
            .as_ref()
            .map(|p| p.guid().to_string() == skeleton.project_guid)
            .unwrap_or(false);

        // Only stop previous project if we're actually changing projects
        if !already_on_project {
            self.stop_previous_project_if_idle(&skeleton.project_guid)
                .await;
        }

        // Update the cached active song ID for fast playback commands
        self.set_active_song_id(&skeleton.id).await;

        // Switch to the correct project tab (or reuse if already there)
        let project = if already_on_project {
            current_project.unwrap()
        } else {
            match daw.select_project(&skeleton.project_guid).await {
                Ok(project) => project,
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        skeleton.project_guid, song_index, e
                    );
                    return;
                }
            }
        };

        // Now try to hydrate (best-effort, with timeout to prevent freezes)
        let song = match tokio::time::timeout(
            Duration::from_secs(5),
            self.ensure_song_hydrated(song_index),
        )
        .await
        {
            Ok(Some(song)) => song,
            Ok(None) => {
                warn!(
                    "seek_to_song: hydration returned None for song {} ({}), staying on project tab",
                    song_index, skeleton.name
                );
                return;
            }
            Err(_) => {
                warn!(
                    "seek_to_song: hydration timed out for song {} ({}), staying on project tab",
                    song_index, skeleton.name
                );
                return;
            }
        };

        let transport = project.transport();

        // Only seek if the project is NOT already playing
        let is_playing = transport.is_playing().await.unwrap_or(false);
        if !is_playing {
            let seek_pos = Self::song_seek_position(&song);
            if let Err(e) = transport.set_position(seek_pos).await {
                warn!("Failed to seek to song {}: {}", song_index, e);
            }
        }

        // Log the actual cursor position after navigation
        let actual_pos = transport.get_position().await.unwrap_or(f64::NAN);
        info!(
            "seek_to_song: song {} ({}) — seek_target={:.2}s, actual_pos={:.2}s, is_playing={}",
            song_index,
            song.name,
            Self::song_seek_position(&song),
            actual_pos,
            is_playing
        );
    }

    async fn seek_to_section(&self, song_index: usize, section_index: usize) {
        debug!(
            "seek_to_section: song={}, section={}",
            song_index, section_index
        );

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            if let Some(section) = song.sections.get(section_index) {
                // First, switch to the correct project
                match daw.select_project(&song.project_guid).await {
                    Ok(project) => {
                        // Then seek to the section's start position
                        if let Err(e) = project
                            .transport()
                            .set_position(section.start_seconds)
                            .await
                        {
                            warn!(
                                "Failed to seek to section {} in song {}: {}",
                                section_index, song_index, e
                            );
                        } else {
                            info!(
                                "Seeked to section {} ({}) in song {} (project {})",
                                section_index, section.name, song.name, song.project_guid
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to switch to project {} for song {}: {}",
                            song.project_guid, song_index, e
                        );
                    }
                }
            } else {
                warn!("Section {} not found in song {}", section_index, song_index);
            }
        } else {
            warn!("Song {} not found", song_index);
        }
    }

    async fn seek_to_musical_position(
        &self,
                song_index: usize,
        position: daw_proto::MusicalPosition,
    ) {
        debug!(
            "seek_to_musical_position: song={}, position={}.{}.{}",
            song_index, position.measure, position.beat, position.subdivision
        );

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            // First, switch to the correct project
            match daw.select_project(&song.project_guid).await {
                Ok(project) => {
                    // The musical position is relative to the song start
                    // We need to convert it to absolute position using the tempo map
                    let fraction = position.subdivision as f64 / 1000.0;
                    let relative_seconds = project
                        .tempo_map()
                        .musical_to_time(position.measure, position.beat, fraction)
                        .await
                        .unwrap_or(0.0);

                    // Add the song start offset to get absolute position
                    let absolute_pos = song.start_seconds() + relative_seconds;

                    if let Err(e) = project.transport().set_position(absolute_pos).await {
                        warn!(
                            "Failed to seek to musical position in song {}: {}",
                            song_index, e
                        );
                    } else {
                        info!(
                            "Seeked to {}.{}.{} in song {} (project {})",
                            position.measure,
                            position.beat,
                            position.subdivision,
                            song.name,
                            song.project_guid
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        song.project_guid, song_index, e
                    );
                }
            }
        } else {
            warn!("Song {} not found", song_index);
        }
    }

    async fn goto_measure(&self, song_index: usize, measure: i32) {
        info!(
            "goto_measure: song_index={}, measure={}",
            song_index, measure
        );

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            // First, switch to the correct project
            match daw.select_project(&song.project_guid).await {
                Ok(project) => {
                    // Use the transport's goto_measure which handles tempo map conversion
                    if let Err(e) = project.transport().goto_measure(measure).await {
                        warn!(
                            "Failed to goto measure {} in song {}: {}",
                            measure, song_index, e
                        );
                    } else {
                        info!(
                            "Went to measure {} in song {} ({})",
                            measure, song.name, song.project_guid
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        song.project_guid, song_index, e
                    );
                }
            }
        } else {
            warn!("Song {} not found", song_index);
        }
    }

    // =========================================================================
    // Playback Commands
    // =========================================================================

    async fn toggle_playback(&self) {
        debug!("toggle_playback");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().play_pause().await {
                        warn!("Failed to toggle playback: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
            }
        } else {
            warn!("No active song to toggle playback (navigate to a song first)");
        }
    }

    async fn play(&self) {
        debug!("play");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().play().await {
                        warn!("Failed to play: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
            }
        } else {
            warn!("No active song to play (navigate to a song first)");
        }
    }

    async fn pause(&self) {
        debug!("pause");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().pause().await {
                        warn!("Failed to pause: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
            }
        } else {
            warn!("No active song to pause (navigate to a song first)");
        }
    }

    async fn stop(&self) {
        debug!("stop");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().stop().await {
                        warn!("Failed to stop: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
            }
        } else {
            warn!("No active song to stop (navigate to a song first)");
        }
    }

    // =========================================================================
    // Loop Control
    // =========================================================================

    async fn toggle_song_loop(&self) {
        debug!("toggle_song_loop");

        let daw = Daw::get();
        match daw.current_project().await {
            Ok(project) => {
                if let Err(e) = project.transport().toggle_loop().await {
                    warn!("Failed to toggle song loop: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to get current project: {}", e);
            }
        }
    }

    async fn toggle_section_loop(&self) {
        debug!("toggle_section_loop");
        // TODO: Implement section-specific loop using loop points
        warn!("toggle_section_loop not yet implemented");
    }

    async fn set_loop_region(&self, _start_seconds: f64, _end_seconds: f64) {
        debug!("set_loop_region: {} - {}", _start_seconds, _end_seconds);
        // TODO: Implement setting loop region
        warn!("set_loop_region not yet implemented");
    }

    async fn clear_loop(&self) {
        debug!("clear_loop");

        let daw = Daw::get();
        match daw.current_project().await {
            Ok(project) => {
                if let Err(e) = project.transport().set_loop(false).await {
                    warn!("Failed to clear loop: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to get current project: {}", e);
            }
        }
    }

    // =========================================================================
    // Build/Refresh
    // =========================================================================

    async fn build_from_open_projects(&self) {
        debug!("Building setlist from open projects...");

        // Check if DAW is initialized (it may not be ready yet after cell startup)
        let Some(daw) = Daw::try_get() else {
            warn!("DAW not initialized yet, cannot build setlist");
            return;
        };

        let build_generation = self.build_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let projects = match daw.projects().await {
            Ok(projects) => projects,
            Err(e) => {
                warn!("Failed to enumerate open projects: {}", e);
                return;
            }
        };

        let current_project_guid = daw
            .current_project()
            .await
            .ok()
            .map(|project| project.guid().to_string());

        let project_loads = Self::fetch_project_loads(projects).await;
        if project_loads.is_empty() {
            let empty = Setlist {
                id: None,
                name: "Empty Setlist".to_string(),
                advance_mode: AdvanceMode::default(),
                songs: Vec::new(),
            };
            *self.setlist.write().await = Some(empty.clone());
            self.notify_setlist_changed();
            return;
        }

        let open_guids: FxHashSet<String> =
            project_loads.iter().map(|load| load.guid.clone()).collect();
        self.song_cache
            .write()
            .await
            .retain(|guid, _| open_guids.contains(guid));
        self.chart_cache
            .write()
            .await
            .retain(|guid, _| open_guids.contains(guid));
        self.last_chart_refresh_attempt
            .write()
            .await
            .retain(|guid, _| open_guids.contains(guid));

        let existing_setlist = self.setlist.read().await.clone();
        let existing_songs_by_guid: FxHashMap<String, Song> = existing_setlist
            .as_ref()
            .map(|sl| {
                sl.songs
                    .iter()
                    .cloned()
                    .map(|song| (song.project_guid.clone(), song))
                    .collect()
            })
            .unwrap_or_default();
        let cached_songs: FxHashMap<String, Song> = self
            .song_cache
            .read()
            .await
            .iter()
            .map(|(guid, entry)| (guid.clone(), entry.song.clone()))
            .collect();

        let focused_index = current_project_guid
            .as_ref()
            .and_then(|guid| project_loads.iter().position(|load| &load.guid == guid))
            .unwrap_or(0);

        // Phase 1: focused project full details, all others names only.
        // A single project may produce multiple songs (multi-song mode).
        let mut songs = Vec::with_capacity(project_loads.len());
        let mut focused_song_count = 0usize;
        for load in &project_loads {
            let existing_song = existing_songs_by_guid.get(&load.guid);
            let cached_song = cached_songs.get(&load.guid);

            if load.index == focused_index {
                let project_songs =
                    self.build_songs_with_cache(load, existing_song).await;
                if !project_songs.is_empty() {
                    for song in &project_songs {
                        self.cache_chart_payload_for_song(song).await;
                    }
                    let light_songs: Vec<Song> = project_songs
                        .into_iter()
                        .map(|mut s| {
                            Self::strip_song_chart_payload(&mut s);
                            s
                        })
                        .collect();
                    focused_song_count = light_songs.len();
                    songs.extend(light_songs);
                    continue;
                }
            }

            songs.push(Self::make_name_only_song(
                &load.guid,
                &load.project_name,
                existing_song,
                cached_song,
            ));
        }

        // Compute the actual song index of the first focused song
        let focused_song_start = project_loads
            .iter()
            .take_while(|l| l.index != focused_index)
            .count(); // one placeholder per project before focused

        let setlist = Setlist {
            id: None,
            name: format!("Setlist - {}", chrono::Local::now().format("%Y-%m-%d")),
            advance_mode: AdvanceMode::default(),
            songs,
        };

        if let Some(active_song) = setlist.songs.get(focused_song_start) {
            *self.active_song_id.write().await = Some(active_song.id.clone());
            debug!(
                "Set initial active song to: {} ({})",
                active_song.name, active_song.id
            );
        }

        debug!(
            "Phase 1 complete: focused project ready ({} songs), {} total songs loaded",
            focused_song_count,
            setlist.songs.len()
        );
        *self.setlist.write().await = Some(setlist);
        self.notify_setlist_changed();
        if let Some(load) = project_loads.get(focused_index) {
            self.emit_cached_chart_payload_for_song(focused_song_start, &load.guid)
                .await;
        }

        // Phase 2: hydrate remaining projects in background with bounded concurrency.
        // Each project may produce multiple songs, so we splice by project_guid.
        let this = self.clone();
        tokio::spawn(async move {
            let semaphore = Arc::new(Semaphore::new(HYDRATION_CONCURRENCY));
            let mut join_set = JoinSet::new();

            for load in project_loads {
                if load.index == focused_index {
                    continue;
                }

                let permit = semaphore.clone();
                let this_clone = this.clone();
                let existing_song = existing_songs_by_guid.get(&load.guid).cloned();
                join_set.spawn(async move {
                    let _permit = permit.acquire_owned().await.expect("semaphore closed");
                    let songs = this_clone
                        .build_songs_with_cache(&load, existing_song.as_ref())
                        .await;
                    (load.guid, songs)
                });
            }

            while let Some(result) = join_set.join_next().await {
                if this.build_generation.load(Ordering::SeqCst) != build_generation {
                    info!("Skipping stale hydration results from older build generation");
                    return;
                }

                match result {
                    Ok((project_guid, hydrated_songs)) if !hydrated_songs.is_empty() => {
                        for song in &hydrated_songs {
                            this.cache_chart_payload_for_song(song).await;
                        }
                        let light_songs: Vec<Song> = hydrated_songs
                            .into_iter()
                            .map(|mut s| {
                                Self::strip_song_chart_payload(&mut s);
                                s
                            })
                            .collect();

                        // Replace the placeholder(s) for this project with hydrated songs.
                        let emitted_songs = {
                            let mut guard = this.setlist.write().await;
                            let Some(ref mut current_setlist) = *guard else {
                                return;
                            };
                            // Find the range of songs belonging to this project
                            let first = current_setlist
                                .songs
                                .iter()
                                .position(|s| s.project_guid == project_guid);
                            let Some(first_idx) = first else { continue };
                            let count = current_setlist.songs[first_idx..]
                                .iter()
                                .take_while(|s| s.project_guid == project_guid)
                                .count();
                            // Splice: remove old range, insert new songs
                            let end_idx = first_idx + count;
                            let indexed_songs: Vec<(usize, Song)> = light_songs
                                .into_iter()
                                .enumerate()
                                .map(|(i, s)| (first_idx + i, s))
                                .collect();
                            current_setlist
                                .songs
                                .splice(first_idx..end_idx, indexed_songs.iter().map(|(_, s)| s.clone()));
                            indexed_songs
                        };

                        this.notify_setlist_changed();
                        for (song_index, song) in &emitted_songs {
                            let _ = this.hydration_tx.send((*song_index, song.clone()));
                            this.emit_cached_chart_payload_for_song(*song_index, &project_guid)
                                .await;
                        }
                    }
                    Ok((_project_guid, _)) => {
                        // Keep placeholder if build returned empty.
                    }
                    Err(e) => {
                        warn!("Song hydration worker failed: {}", e);
                    }
                }
            }
        });
    }

    async fn refresh(&self) {
        info!("Refreshing setlist...");
        self.build_from_open_projects().await;
    }

    // =========================================================================
    // Subscriptions
    // =========================================================================

    async fn subscribe(&self, events: Tx<SetlistEvent>) {
        debug!("SetlistService::subscribe() - starting fully reactive event stream");
        // Clone self for the spawned task
        let this = self.clone();

        // Spawn the streaming loop so this method returns immediately
        tokio::spawn(async move {
            let mut setlist_updates = this.setlist_update_tx.subscribe();
            let mut hydration_rx = this.hydration_tx.subscribe();
            let mut chart_hydration_rx = this.chart_hydration_tx.subscribe();

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
                let chart_cache = this.chart_cache.read().await;
                for (index, song) in songs.iter().enumerate() {
                    if let Some(chart) = chart_cache.get(&song.project_guid).cloned() {
                        if events
                            .send(SetlistEvent::SongChartHydrated { index, chart })
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
                .map(|(idx, song)| (song.id.clone(), idx))
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
                    *this.active_song_id.write().await = Some(song.id.clone());
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
            let mut project_check_interval = tokio::time::interval(Duration::from_millis(500));
            project_check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut active_hydration_interval =
                tokio::time::interval(Duration::from_millis(ACTIVE_HYDRATION_TICK_MS));
            active_hydration_interval
                .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut perf_log_interval = tokio::time::interval(Duration::from_secs(5));
            perf_log_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let (chart_probe_tx, mut chart_probe_rx) =
                mpsc::unbounded_channel::<(Duration, bool)>();
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
                                .map(|(idx, song)| (song.id.clone(), idx))
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
                                    .send(SetlistEvent::SongHydrated { index, song })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                debug!("Hydration update lagged by {} messages", skipped);
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }

                    chart_hydrated = chart_hydration_rx.recv() => {
                        match chart_hydrated {
                            Ok((index, chart)) => {
                                if events
                                    .send(SetlistEvent::SongChartHydrated { index, chart })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                debug!("Chart hydration update lagged by {} messages", skipped);
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
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
                                        == daw_proto::PlayState::Playing
                                        || proj_state.transport.play_state
                                            == daw_proto::PlayState::Recording;

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
                                                        pending_section_events.push(SetlistEvent::SectionExited {
                                                            song_index,
                                                            section_index: sec_idx,
                                                        });
                                                    }

                                                    if let Some(sec_idx) = current_section {
                                                        if let Some(section) = song.sections.get(sec_idx) {
                                                            pending_section_events.push(SetlistEvent::SectionEntered {
                                                                song_index,
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
                                        if state == daw_proto::PlayState::Paused {
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
                                    let song_id = {
                                        let setlist_guard = this.setlist.read().await;
                                        setlist_guard
                                            .as_ref()
                                            .and_then(|sl| sl.songs.get(song_idx))
                                            .map(|song| song.id.clone())
                                    };
                                    if let Some(song_id) = song_id {
                                        *this.active_song_id.write().await = Some(song_id);
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
    }

    async fn subscribe_active(&self, indices: Tx<ActiveIndices>) {
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
                tokio::time::sleep(Duration::from_micros(16667)).await;

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
    }

    async fn get_audio_latency(&self) -> f64 {
        let daw = Daw::get();
        daw.audio_engine()
            .get_output_latency_seconds()
            .await
            .unwrap_or(0.0)
    }

    async fn get_audio_latency_info(&self) -> session_proto::AudioLatencyInfo {
        let daw = Daw::get();
        match daw.audio_engine().get_state().await {
            Ok(state) => session_proto::AudioLatencyInfo {
                input_samples: state.latency.input_samples,
                output_samples: state.latency.output_samples,
                output_seconds: state.latency.output_seconds,
                sample_rate: state.latency.sample_rate,
                is_running: state.is_running,
            },
            Err(_) => session_proto::AudioLatencyInfo::default(),
        }
    }
}
