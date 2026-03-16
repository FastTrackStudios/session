//! Song hydration logic: cache lookups, chart extraction, fingerprint checking

use super::{
    SetlistServiceImpl, CHART_REFRESH_FALLBACK_POLL_MS, HYDRATION_CONCURRENCY, MIDI_TRACK_TAG,
};
use crate::song_builder::SongBuilder;
use daw::Daw;
use session_proto::{Song, SongChartHydration, SongDetectedChord, SongId};
use std::sync::Arc;
use std::time::{Duration, Instant};
use moire::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

#[derive(Clone)]
pub(crate) struct SongCacheEntry {
    pub(crate) project_name: String,
    pub(crate) chart_fingerprint: Option<String>,
    pub(crate) song: Song,
}

#[derive(Clone)]
pub(crate) struct ProjectLoad {
    pub(crate) index: usize,
    pub(crate) project: daw::Project,
    pub(crate) guid: String,
    pub(crate) project_name: String,
}

impl SetlistServiceImpl {
    pub(crate) fn song_is_hydrated(song: &Song) -> bool {
        !song.sections.is_empty() || song.end_seconds > song.start_seconds
    }

    pub(crate) async fn ensure_song_hydrated(&self, index: usize) -> Option<Song> {
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
        let rebuilt = self
            .build_songs_with_cache(&load, Some(&current))
            .await
            .into_iter()
            .next()?;
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
            self.hydration_bus.emit((index, rebuilt_light.clone()));
            self.emit_cached_chart_payload_for_song(index, &rebuilt_light.project_guid)
                .await;
        }

        Some(rebuilt_light)
    }

    pub(crate) fn parse_project_name_fallback(project_name: &str) -> String {
        let base = project_name.strip_suffix(".rpp").unwrap_or(project_name);
        base.split(" - ").next().unwrap_or(base).trim().to_string()
    }

    pub(crate) fn make_song_id(
        existing_song: Option<&Song>,
        cached_song: Option<&Song>,
        _guid: &str,
    ) -> SongId {
        if let Some(song) = existing_song {
            return song.id.clone();
        }
        if let Some(song) = cached_song {
            return song.id.clone();
        }
        SongId::new()
    }

    pub(crate) fn make_name_only_song(
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

    pub(crate) fn strip_song_chart_payload(song: &mut Song) {
        song.chart_text = None;
        song.parsed_chart = None;
        song.detected_chords.clear();
    }

    fn map_detected_chords(chords: Vec<daw::service::MidiDetectedChord>) -> Vec<SongDetectedChord> {
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

    pub(crate) async fn fetch_midi_chart_data(
        project: &daw::Project,
    ) -> Option<daw::service::MidiChartData> {
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

    pub(crate) async fn fetch_midi_source_fingerprint(
        &self,
        project: &daw::Project,
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

    pub(crate) async fn should_run_fallback_chart_refresh(&self, project_guid: &str) -> bool {
        let now = Instant::now();
        self.last_chart_refresh_attempt
            .with_write(|map| {
                if let Some(last) = map.get(project_guid)
                    && now.duration_since(*last)
                        < Duration::from_millis(CHART_REFRESH_FALLBACK_POLL_MS)
                {
                    return false;
                }
                map.insert(project_guid.to_string(), now);
                true
            })
            .await
    }

    pub(crate) fn apply_chart_data(song: &mut Song, chart_data: daw::service::MidiChartData) {
        song.chart_fingerprint = Some(chart_data.source_fingerprint);
        song.chart_text = Some(chart_data.chart_text);
        // Keep parsed_chart empty in session state to avoid cloning large chart
        // structures through transport/setlist event paths.
        song.parsed_chart = None;
        song.detected_chords = Self::map_detected_chords(chart_data.chords);
    }

    pub(crate) fn chart_payload_from_song(song: &Song) -> Option<SongChartHydration> {
        let chart_text = song.chart_text.clone()?;
        let chart_fingerprint = song.chart_fingerprint.clone()?;
        Some(SongChartHydration {
            project_guid: song.project_guid.clone(),
            chart_text,
            detected_chords: song.detected_chords.clone(),
            chart_fingerprint,
        })
    }

    pub(crate) async fn cache_chart_payload_for_song(&self, song: &Song) {
        if let Some(payload) = Self::chart_payload_from_song(song) {
            self.chart_cache
                .insert(song.project_guid.clone(), payload)
                .await;
        }
    }

    pub(crate) async fn emit_cached_chart_payload_for_song(
        &self,
        index: usize,
        project_guid: &str,
    ) {
        if let Some(payload) = self.chart_cache.get(&project_guid.to_string()).await {
            self.chart_hydration_bus.emit((index, payload));
        }
    }

    pub(crate) async fn fetch_project_loads(
        projects: Vec<daw::Project>,
    ) -> Vec<ProjectLoad> {
        let semaphore = Arc::new(Semaphore::new("session.setlist.hydration.fetch_projects", HYDRATION_CONCURRENCY));
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

    pub(crate) async fn build_songs_with_cache(
        &self,
        load: &ProjectLoad,
        existing_song: Option<&Song>,
    ) -> Vec<Song> {
        let source_fingerprint = self.fetch_midi_source_fingerprint(&load.project).await;

        if let Some(cached) = self.song_cache.get(&load.guid).await {
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
                    first_song.id = Self::make_song_id(existing_song, None, &load.guid);
                    if let Some(data) = chart_data {
                        Self::apply_chart_data(first_song, data);
                    }
                    self.cache_chart_payload_for_song(first_song).await;
                    let mut cached_song = first_song.clone();
                    Self::strip_song_chart_payload(&mut cached_song);
                    self.song_cache
                        .insert(
                            load.guid.clone(),
                            SongCacheEntry {
                                project_name: load.project_name.clone(),
                                chart_fingerprint: first_song.chart_fingerprint.clone(),
                                song: cached_song,
                            },
                        )
                        .await;
                }
                // SongBuilder now assigns SongId::new() to each song,
                // so remaining songs already have unique IDs.
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

    pub(crate) async fn refresh_active_song_chart_if_changed(&self) -> bool {
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
                .find(|(_, song)| song.id.as_str() == active_song_id)
            else {
                return false;
            };
            (index, song.clone())
        };

        let fingerprint_supported =
            *self.fingerprint_method_supported.read().await != Some(false);
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
            self.hydration_bus
                .emit((song_index, updated_song_light.clone()));
            self.emit_cached_chart_payload_for_song(song_index, &updated_song_light.project_guid)
                .await;
            let project_name = project
                .info()
                .await
                .ok()
                .map(|info| info.name)
                .unwrap_or_else(|| song_snapshot.project_guid.clone());
            self.song_cache
                .insert(
                    song_snapshot.project_guid.clone(),
                    SongCacheEntry {
                        project_name,
                        chart_fingerprint: updated_song_light.chart_fingerprint.clone(),
                        song: updated_song_light,
                    },
                )
                .await;
            return true;
        }
        false
    }
}
