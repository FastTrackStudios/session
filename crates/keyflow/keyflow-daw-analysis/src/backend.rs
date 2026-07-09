//! `MidiCharts` backend — fetches MIDI through the `daw` client
//! API and runs keyflow's chord/chart analysis on the result.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use daw::rpc::{Daw, Project};
use keyflow::chord::{MidiNote as KeyflowMidiNote, detect_chords_from_midi_notes};
use keyflow::engraver::import::{
    MarkerEvent, MarkerType, MidiChartConfig, MidiFile, MidiNote as ImportMidiNote, MidiTrack,
    TempoEvent, TimeSignatureEvent, generate_chart_text,
};
use tracing::warn;

use crate::service::MidiCharts;
use crate::types::{DetectedChord, MidiChartData, MidiChartRequest};

/// Default PPQ used when serializing notes into a `MidiFile` for keyflow.
const KEYFLOW_PPQ: u32 = 960;
const MIN_CHORD_DURATION_PPQ: i64 = 120;

#[derive(Clone)]
pub struct KeyflowMidiAnalysis {
    daw: Option<Arc<Daw>>,
}

impl KeyflowMidiAnalysis {
    pub fn new(daw: Daw) -> Self {
        Self {
            daw: Some(Arc::new(daw)),
        }
    }

    pub fn from_global_daw() -> Self {
        Self { daw: None }
    }

    fn daw(&self) -> Result<Daw, String> {
        self.daw
            .as_deref()
            .cloned()
            .or_else(|| Daw::try_get().cloned())
            .ok_or_else(|| "DAW not initialized".to_string())
    }

    async fn resolve_project(&self, request: &MidiChartRequest) -> Result<Project, String> {
        let daw = self.daw()?;
        match &request.project_guid {
            None => daw
                .current_project()
                .await
                .map_err(|e| format!("no current project: {e}")),
            Some(guid) => {
                let projects = daw
                    .projects()
                    .await
                    .map_err(|e| format!("project list failed: {e}"))?;
                projects
                    .into_iter()
                    .find(|p| p.guid() == guid)
                    .ok_or_else(|| format!("project '{guid}' not found"))
            }
        }
    }

    async fn find_track_handle(
        project: &Project,
        tag: Option<&str>,
    ) -> Result<daw::rpc::TrackHandle, String> {
        let needle = tag.map(|t| t.to_ascii_lowercase());
        let tracks = project
            .tracks()
            .all()
            .await
            .map_err(|e| format!("track list failed: {e}"))?;
        for info in &tracks {
            let matches = match &needle {
                Some(n) => info.name.to_ascii_lowercase().contains(n),
                None => true,
            };
            if matches {
                return project
                    .tracks()
                    .by_guid(&info.guid)
                    .await
                    .map_err(|e| format!("track handle failed: {e}"))?
                    .ok_or_else(|| format!("track '{}' disappeared mid-call", info.name));
            }
        }
        Err(match &needle {
            Some(n) => format!("no track matched tag '{n}'"),
            None => "project has no tracks".to_string(),
        })
    }

    async fn first_midi_take(
        track: &daw::rpc::TrackHandle,
    ) -> Result<(daw::rpc::TakeHandle, f64), String> {
        let items = track
            .items()
            .all()
            .await
            .map_err(|e| format!("items list failed: {e}"))?;
        for item in items {
            let item_handle = track
                .items()
                .by_guid(&item.guid)
                .await
                .map_err(|e| format!("item handle: {e}"))?
                .ok_or_else(|| "item missing".to_string())?;
            let take = match item_handle.takes().active().await {
                Ok(t) => t,
                Err(_) => continue,
            };
            let kind = take
                .source_type()
                .await
                .map_err(|e| format!("source_type: {e}"))?;
            if matches!(kind, daw::service::SourceType::Midi) {
                return Ok((take, item.position.as_seconds()));
            }
        }
        Err("track has no MIDI take".to_string())
    }

    fn to_keyflow_notes(notes: &[daw::service::MidiNote]) -> Vec<KeyflowMidiNote> {
        notes
            .iter()
            .map(|n| KeyflowMidiNote {
                pitch: n.pitch,
                velocity: n.velocity,
                channel: n.channel,
                start_ppq: n.start_ppq.round() as i64,
                end_ppq: (n.start_ppq + n.length_ppq).round() as i64,
            })
            .collect()
    }

    fn to_import_notes(notes: &[KeyflowMidiNote], item_start_tick: u32) -> Vec<ImportMidiNote> {
        notes
            .iter()
            .map(|note| {
                let abs_start = item_start_tick + (note.start_ppq.max(0) as u32);
                let abs_end = item_start_tick + (note.end_ppq.max(0) as u32);
                ImportMidiNote {
                    pitch: note.pitch,
                    velocity: note.velocity,
                    start_tick: abs_start,
                    duration_ticks: abs_end.saturating_sub(abs_start),
                    channel: note.channel,
                }
            })
            .collect()
    }

    async fn gather_markers(project: &Project) -> Vec<MarkerEvent> {
        let markers = match project.markers().all().await {
            Ok(m) => m,
            Err(e) => {
                warn!("marker list failed: {e}");
                return Vec::new();
            }
        };
        let tempo_map = project.tempo_map();
        let mut out = Vec::with_capacity(markers.len());
        for m in markers {
            let pos = m.position.time.map(|t| t.as_seconds()).unwrap_or(0.0);
            let tick = match tempo_map.time_to_musical(pos).await {
                Ok((measure, beat, fraction)) => {
                    let total_beats =
                        (measure.saturating_sub(1) as f64) * 4.0 + beat as f64 + fraction;
                    (total_beats * f64::from(KEYFLOW_PPQ)).round().max(0.0) as u32
                }
                Err(_) => 0,
            };
            out.push(MarkerEvent {
                tick,
                text: m.name,
                marker_type: MarkerType::Marker,
            });
        }
        out.sort_by_key(|e| e.tick);
        out
    }

    async fn time_to_tick(project: &Project, time_seconds: f64) -> u32 {
        let (measure, beat, fraction) =
            match project.tempo_map().time_to_musical(time_seconds).await {
                Ok(t) => t,
                Err(_) => return 0,
            };
        let total_beats = (measure.saturating_sub(1) as f64) * 4.0 + beat as f64 + fraction;
        (total_beats * f64::from(KEYFLOW_PPQ)).round().max(0.0) as u32
    }

    fn make_source_fingerprint(
        source_track_name: &str,
        import_notes: &[ImportMidiNote],
        markers: &[MarkerEvent],
    ) -> String {
        let mut hasher = DefaultHasher::new();
        source_track_name.hash(&mut hasher);
        import_notes.len().hash(&mut hasher);
        for note in import_notes {
            note.pitch.hash(&mut hasher);
            note.velocity.hash(&mut hasher);
            note.start_tick.hash(&mut hasher);
            note.duration_ticks.hash(&mut hasher);
            note.channel.hash(&mut hasher);
        }
        markers.len().hash(&mut hasher);
        for marker in markers {
            marker.tick.hash(&mut hasher);
            marker.text.hash(&mut hasher);
        }
        format!("{:x}", hasher.finish())
    }
}

impl MidiCharts for KeyflowMidiAnalysis {
    async fn source_fingerprint(&self, request: MidiChartRequest) -> Result<String, String> {
        let project = self.resolve_project(&request).await?;
        let track = Self::find_track_handle(&project, request.track_tag.as_deref()).await?;
        let track_name = track
            .info()
            .await
            .map_err(|e| format!("track info: {e}"))?
            .name;
        let (take, item_start_time) = Self::first_midi_take(&track).await?;
        let raw = take
            .midi()
            .notes()
            .await
            .map_err(|e| format!("midi notes: {e}"))?;
        if raw.is_empty() {
            return Err("no MIDI notes found".to_string());
        }
        let kf_notes = Self::to_keyflow_notes(&raw);
        let item_start_tick = Self::time_to_tick(&project, item_start_time).await;
        let import_notes = Self::to_import_notes(&kf_notes, item_start_tick);
        let markers = Self::gather_markers(&project).await;
        Ok(Self::make_source_fingerprint(
            &track_name,
            &import_notes,
            &markers,
        ))
    }

    async fn generate_chart_data(
        &self,
        request: MidiChartRequest,
    ) -> Result<MidiChartData, String> {
        let project = self.resolve_project(&request).await?;
        let track = Self::find_track_handle(&project, request.track_tag.as_deref()).await?;
        let info = track.info().await.map_err(|e| format!("track info: {e}"))?;
        let source_track_name = info.name.clone();
        let (take, item_start_time) = Self::first_midi_take(&track).await?;
        let raw = take
            .midi()
            .notes()
            .await
            .map_err(|e| format!("midi notes: {e}"))?;
        if raw.is_empty() {
            return Err("no MIDI notes found".to_string());
        }
        let kf_notes = Self::to_keyflow_notes(&raw);

        let item_start_tick = Self::time_to_tick(&project, item_start_time).await;
        let import_notes = Self::to_import_notes(&kf_notes, item_start_tick);
        let markers = Self::gather_markers(&project).await;
        let source_fingerprint =
            Self::make_source_fingerprint(&source_track_name, &import_notes, &markers);

        let midi_file = MidiFile::from_parts(
            KEYFLOW_PPQ,
            vec![MidiTrack {
                index: 0,
                name: Some(source_track_name.clone()),
                notes: import_notes.clone(),
                channel: None,
            }],
            vec![TempoEvent {
                tick: 0,
                microseconds_per_quarter: 500_000,
            }],
            vec![TimeSignatureEvent {
                tick: 0,
                numerator: 4,
                denominator: 4,
            }],
            markers,
            vec![Some(source_track_name.clone())],
            None,
        );
        let chart_text = generate_chart_text(&midi_file, &MidiChartConfig::default());

        let chords = detect_chords_from_midi_notes(&kf_notes, MIN_CHORD_DURATION_PPQ)
            .into_iter()
            .map(|chord| DetectedChord {
                symbol: chord.chord.to_string(),
                start_ppq: chord.start_ppq,
                end_ppq: chord.end_ppq,
                root_pitch: chord.root_pitch,
                velocity: chord.velocity,
            })
            .collect();

        Ok(MidiChartData {
            source_track_name,
            source_fingerprint,
            chart_text,
            chords,
        })
    }
}
