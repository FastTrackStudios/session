//! The engine side: read a [`SessionModel`] back from a daw project, and
//! apply [`Change`]s to one.
//!
//! Talks to whichever backend `daw_control` is connected to —
//! daw-standalone in-process for the Session app, REAPER through the
//! bridge. Nothing here knows about Loro.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use daw_control::Project;
use daw_proto::primitives::{Duration, PositionInSeconds};
use daw_proto::{FadeShape, TrackRef};

use crate::diff::{Change, ItemField, TrackField};
use crate::model::{
    EnvelopePointState, EnvelopeState, FxState, ItemState, LaneState, MarkerState, NoteState,
    RegionState, SendState, SessionModel, StretchPoint, TakeState, TempoPoint, TrackState,
};

/// Where an FX's state is kept: put its bytes, get its name back (a
/// SHA-256), and the bytes by name. The project's store on disk; a map in
/// memory for a session nobody saves.
pub trait Blobs: Send + Sync {
    fn put(&self, bytes: &[u8]) -> String;
    fn get(&self, name: &str) -> Option<Vec<u8>>;
}

/// [`Blobs`] in memory.
#[derive(Default)]
pub struct MemoryBlobs(std::sync::Mutex<HashMap<String, Vec<u8>>>);

impl Blobs for MemoryBlobs {
    fn put(&self, bytes: &[u8]) -> String {
        let name = blob_name(bytes);
        if let Ok(mut held) = self.0.lock() {
            held.entry(name.clone()).or_insert_with(|| bytes.to_vec());
        }
        name
    }
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.0.lock().ok()?.get(name).cloned()
    }
}

/// A blob's name: its SHA-256, lowercase hex — the store's and the
/// `.session` objects' naming.
#[must_use]
pub fn blob_name(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for b in sha2::Sha256::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Which engine FX and item each doc FX and item is.
///
/// An FX cannot be added with a chosen guid (REAPER's cannot, nor this
/// engine's), nor a MIDI item, so a project built from the doc has those
/// under new guids. The doc keeps the ids it first saw; this maps them to
/// whatever the engine calls the same thing now, both ways, for the life
/// of a bridge.
#[derive(Debug, Default, Clone)]
pub struct EngineIds {
    pub fx: IdMap,
    pub items: IdMap,
}

/// One kind's doc ids and engine guids, both ways.
#[derive(Debug, Default, Clone)]
pub struct IdMap {
    doc_to_engine: HashMap<String, String>,
    engine_to_doc: HashMap<String, String>,
}

impl IdMap {
    /// The doc's id for the engine's `guid` (its own, if never mapped).
    #[must_use]
    pub fn doc(&self, guid: &str) -> String {
        self.engine_to_doc
            .get(guid)
            .cloned()
            .unwrap_or_else(|| guid.to_owned())
    }

    /// The engine's guid for the doc's `id` (the id, if never mapped).
    #[must_use]
    pub fn engine(&self, id: &str) -> String {
        self.doc_to_engine
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_owned())
    }

    /// The doc's `id` is the engine's `guid`.
    pub fn bind(&mut self, id: &str, guid: &str) {
        self.doc_to_engine.insert(id.to_owned(), guid.to_owned());
        self.engine_to_doc.insert(guid.to_owned(), id.to_owned());
    }
}

/// Where the session's media lives on this machine.
///
/// The doc stores a take's source relative to the session folder, because
/// each peer keeps the session somewhere else; this turns one into the
/// other.
#[derive(Debug, Clone)]
pub struct MediaRoot(pub PathBuf);

impl MediaRoot {
    /// A path as the doc stores it: relative when it is under the root.
    #[must_use]
    pub fn to_doc(&self, path: &str) -> String {
        Path::new(path).strip_prefix(&self.0).map_or_else(
            |_| path.to_string(),
            |rel| rel.to_string_lossy().into_owned(),
        )
    }

    /// A doc path as this machine opens it.
    #[must_use]
    pub fn to_local(&self, path: &str) -> String {
        let p = Path::new(path);
        if p.is_absolute() {
            path.to_string()
        } else {
            self.0.join(p).to_string_lossy().into_owned()
        }
    }
}

/// Everything the engine holds that the doc shares. `chart` is left
/// empty: the chart is not engine state.
///
/// # Errors
/// When a daw call fails.
pub async fn read(
    project: &Project,
    media: &MediaRoot,
    blobs: &dyn Blobs,
    ids: &EngineIds,
) -> daw_control::Result<SessionModel> {
    let all = project.tracks().all().await?;
    let parents: HashMap<String, Option<String>> = all
        .iter()
        .map(|t| (t.guid.clone(), t.parent_guid.clone()))
        .collect();
    let mut tracks = Vec::with_capacity(all.len());
    for t in all {
        let mut state = TrackState {
            parent: parents.get(&t.guid).cloned().flatten(),
            name: t.name,
            color: t.color,
            volume: t.volume,
            pan: t.pan,
            muted: t.muted,
            soloed: t.soloed,
            phase_inverted: t.phase_inverted,
            parent_send: t.parent_send,
            visible_in_tcp: t.visible_in_tcp,
            visible_in_mixer: t.visible_in_mixer,
            lanes: LaneState {
                count: t.lane_count,
                names: t.lane_names,
                play_mask: t.lane_play_mask,
            },
            grouping: grouping_map(&t.grouping),
            guid: t.guid,
            ..TrackState::default()
        };
        if let Some(handle) = project.tracks().by_guid(&state.guid).await? {
            read_track_parts(&handle, &mut state, blobs, ids).await?;
        }
        tracks.push(state);
    }

    let items = read_items(project, media, ids).await?;

    let markers = project
        .markers()
        .all()
        .await?
        .into_iter()
        .filter_map(|m| {
            Some((
                m.guid?,
                MarkerState {
                    at: m.position.time.map_or(0.0, |t| t.as_seconds()),
                    name: m.name,
                    color: m.color,
                },
            ))
        })
        .collect();
    let regions = project
        .regions()
        .all()
        .await?
        .into_iter()
        .filter_map(|r| {
            Some((
                r.guid?,
                RegionState {
                    start: r.time_range.start_seconds(),
                    end: r.time_range.end_seconds(),
                    name: r.name,
                    color: r.color,
                },
            ))
        })
        .collect();
    let tempo = project
        .tempo_map()
        .points()
        .await?
        .into_iter()
        .map(|p| {
            let (beats_per_bar, beat_unit) = p
                .time_signature
                .map_or((4, 4), |s| (s.numerator, s.denominator));
            TempoPoint {
                at: p.position.time.map_or(0.0, |t| t.as_seconds()),
                bpm: p.bpm,
                beats_per_bar,
                beat_unit,
            }
        })
        .collect();

    Ok(SessionModel {
        tracks,
        items,
        markers,
        regions,
        tempo,
        chart: String::new(),
    })
}

/// Every item, keyed by its doc id, with its active take.
async fn read_items(
    project: &Project,
    media: &MediaRoot,
    ids: &EngineIds,
) -> daw_control::Result<BTreeMap<String, ItemState>> {
    let mut items = BTreeMap::new();
    for item in project.items().all().await? {
        let Some(handle) = project.items().by_guid(&item.guid).await? else {
            continue;
        };
        let key = ids.items.doc(&item.guid);
        let active = handle.active_take();
        let take = match active.info().await {
            Ok(take) => TakeState {
                stretch: active
                    .stretch_markers()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| StretchPoint {
                        at: m.position,
                        source: m.source_position,
                        slope: m.slope,
                    })
                    .collect(),
                notes: if take.is_midi {
                    active
                        .midi()
                        .notes()
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .map(|n| NoteState {
                            start: n.start_ppq,
                            length: n.length_ppq,
                            pitch: n.pitch,
                            velocity: n.velocity,
                            channel: n.channel,
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                midi: take.is_midi,
                name: take.name,
                source: take.source_file_path.map(|p| media.to_doc(&p)),
                start_offset: take.start_offset.as_seconds(),
                playrate: take.play_rate,
            },
            // An item with no take (a label-only KEY item, say).
            Err(_) => TakeState {
                playrate: 1.0,
                ..TakeState::default()
            },
        };
        items.insert(
            key,
            ItemState {
                track: item.track_guid,
                position: item.position.as_seconds(),
                length: item.length.as_seconds(),
                snap_offset: item.snap_offset.as_seconds(),
                muted: item.muted,
                locked: item.locked,
                volume: item.volume,
                fade_in: item.fade_in_length.as_seconds(),
                fade_out: item.fade_out_length.as_seconds(),
                fade_in_shape: fade_name(item.fade_in_shape).to_string(),
                fade_out_shape: fade_name(item.fade_out_shape).to_string(),
                label: item.label.unwrap_or_default(),
                color: item.color,
                take,
                fixed_lane: item.fixed_lane,
                loop_source: item.loop_source,
            },
        );
    }
    Ok(items)
}

/// Apply `changes` (from [`crate::diff`]) to the engine. `target` is the
/// model the changes lead to — structural steps are applied against its
/// whole track order.
///
/// Chart and tempo steps are skipped: the chart is not engine state, and
/// the tempo map belongs to the chart once one exists.
///
/// # Errors
/// On the first daw call that fails; the caller re-reads the engine and
/// reconciles from wherever it got to.
pub async fn apply(
    project: &Project,
    media: &MediaRoot,
    changes: &[Change],
    target: &SessionModel,
    blobs: &dyn Blobs,
    ids: &mut EngineIds,
) -> daw_control::Result<()> {
    let mut structure = false;
    // A track's FX, sends and envelopes wait until every track is there
    // and in place: a send names its destination, an FX's automation its
    // FX.
    let mut parts: Vec<(&TrackState, Vec<TrackField>)> = Vec::new();
    for change in changes {
        match change {
            Change::TrackAdded { track, .. } => {
                structure = true;
                let index = target.tracks.iter().position(|t| t.guid == track.guid);
                let at = index.and_then(|i| u32::try_from(i).ok());
                project
                    .tracks()
                    .add_with_guid(&track.guid, &track.name, at)
                    .await?;
                set_track_fields(project, track, ALL_TRACK_FIELDS).await?;
                parts.push((track, ALL_TRACK_FIELDS.to_vec()));
            }
            Change::TrackMoved { .. } => structure = true,
            Change::TrackChanged { track, fields } => {
                set_track_fields(project, track, fields).await?;
                parts.push((track, fields.clone()));
            }
            Change::TrackRemoved { guid } => {
                project
                    .tracks()
                    .remove(TrackRef::Guid(guid.clone()))
                    .await?;
            }
            Change::ItemAdded { guid, item } => add_item(project, media, guid, item, ids).await?,
            Change::ItemChanged { guid, item, fields } => {
                let at = ids.items.engine(guid);
                set_item_fields(project, media, &at, item, fields).await?;
            }
            Change::ItemRemoved { guid } => {
                if let Some(handle) = project.items().by_guid(&ids.items.engine(guid)).await? {
                    handle.delete().await?;
                }
            }
            Change::MarkerSet { guid, marker } => set_marker(project, guid, marker).await?,
            Change::MarkerRemoved { guid } => {
                if let Some(id) = marker_id(project, guid).await? {
                    project.markers().remove(id).await?;
                }
            }
            Change::RegionSet { guid, region } => set_region(project, guid, region).await?,
            Change::RegionRemoved { guid } => {
                if let Some(id) = region_id(project, guid).await? {
                    project.regions().remove(id).await?;
                }
            }
            Change::TempoChanged(_) | Change::ChartChanged(_) => {}
        }
    }
    if structure {
        apply_structure(project, &target.tracks).await?;
    }
    for (track, fields) in &parts {
        if fields.contains(&TrackField::Fx) {
            apply_fx(project, track, blobs, ids).await?;
        }
    }
    for (track, fields) in &parts {
        if fields.contains(&TrackField::Sends) {
            apply_sends(project, track).await?;
        }
        if fields.contains(&TrackField::Envelopes) {
            apply_envelopes(project, track, ids).await?;
        }
    }
    Ok(())
}

/// A new item, its take made the way it plays: a MIDI item (under a
/// guid of the engine's own, bound in `ids`) or an audio take on its
/// source; then every field set.
async fn add_item(
    project: &Project,
    media: &MediaRoot,
    guid: &str,
    item: &ItemState,
    ids: &mut EngineIds,
) -> daw_control::Result<()> {
    if item.take.midi {
        // A MIDI item is made with its MIDI take, under a
        // guid of the engine's own.
        let Some(track) = project.tracks().by_guid(&item.track).await? else {
            return Ok(());
        };
        if let Some(handle) = track
            .items()
            .create_midi_item_with_notes(item.position, item.position + item.length, Vec::new())
            .await?
        {
            let made = handle.info().await?.guid;
            ids.items.bind(guid, &made);
        }
    } else {
        let handle = project
            .items()
            .add_with_guid(
                &item.track,
                guid,
                PositionInSeconds::from_seconds(item.position),
                Duration::from_seconds(item.length),
            )
            .await?;
        // A new item has no take; the one it plays is added
        // here (take guids are each engine's own — the doc
        // keeps the active take's fields on the item).
        let take = handle.takes().add().await?;
        if let Some(source) = &item.take.source {
            take.set_source_file(&media.to_local(source)).await?;
        }
    }
    let at = ids.items.engine(guid);
    set_item_fields(project, media, &at, item, ALL_ITEM_FIELDS).await?;
    Ok(())
}

/// Put every track at its place in project order, and set the folder
/// depths that make the tree: a track's depth is how far the NEXT track's
/// nesting level differs from its own (REAPER's convention — 1 opens a
/// folder, -n closes n of them).
async fn apply_structure(project: &Project, tracks: &[TrackState]) -> daw_control::Result<()> {
    for (index, track) in tracks.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        project
            .tracks()
            .move_to(TrackRef::Guid(track.guid.clone()), index)
            .await?;
    }
    for (track, depth) in tracks.iter().zip(folder_depths(tracks)) {
        if let Some(handle) = project.tracks().by_guid(&track.guid).await? {
            handle.set_folder_depth(depth).await?;
        }
    }
    Ok(())
}

/// REAPER folder depths for tracks in project order, from their parents.
#[must_use]
pub fn folder_depths(tracks: &[TrackState]) -> Vec<i32> {
    let mut levels: HashMap<&str, i32> = HashMap::new();
    let level: Vec<i32> = tracks
        .iter()
        .map(|t| {
            let l = t
                .parent
                .as_deref()
                .and_then(|p| levels.get(p))
                .map_or(0, |p| p.saturating_add(1));
            levels.insert(t.guid.as_str(), l);
            l
        })
        .collect();
    level
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let next = level.get(i.saturating_add(1)).copied().unwrap_or(0);
            next.saturating_sub(*l)
        })
        .collect()
}

const ALL_TRACK_FIELDS: &[TrackField] = &[
    TrackField::Name,
    TrackField::Color,
    TrackField::Volume,
    TrackField::Pan,
    TrackField::Muted,
    TrackField::Soloed,
    TrackField::PhaseInverted,
    TrackField::ParentSend,
    TrackField::VisibleInTcp,
    TrackField::Lanes,
    TrackField::Grouping,
    TrackField::Fx,
    TrackField::Sends,
    TrackField::Envelopes,
];

async fn set_track_fields(
    project: &Project,
    track: &TrackState,
    fields: &[TrackField],
) -> daw_control::Result<()> {
    let Some(h) = project.tracks().by_guid(&track.guid).await? else {
        return Ok(());
    };
    for field in fields {
        match field {
            TrackField::Name => h.rename(&track.name).await?,
            TrackField::Color => h.set_color(track.color.unwrap_or(0)).await?,
            TrackField::Volume => h.set_volume(track.volume).await?,
            TrackField::Pan => h.set_pan(track.pan).await?,
            TrackField::Muted if track.muted => h.mute().await?,
            TrackField::Muted => h.unmute().await?,
            TrackField::Soloed if track.soloed => h.solo().await?,
            TrackField::Soloed => h.unsolo().await?,
            TrackField::PhaseInverted => h.set_phase_inverted(track.phase_inverted).await?,
            TrackField::ParentSend => h.set_parent_send(track.parent_send).await?,
            TrackField::VisibleInTcp | TrackField::VisibleInMixer => {
                h.set_visibility(track.visible_in_tcp, track.visible_in_mixer)
                    .await?;
            }
            TrackField::Lanes => {
                h.set_lane_count(track.lanes.count).await?;
                for (lane, name) in track.lanes.names.iter().enumerate() {
                    if let Ok(lane) = u32::try_from(lane) {
                        h.set_lane_name(lane, name).await?;
                    }
                }
                h.set_lane_play_mask(track.lanes.play_mask).await?;
            }
            TrackField::Grouping => apply_grouping(&h, track).await?,
            // After the structure: see `apply`.
            TrackField::Fx | TrackField::Sends | TrackField::Envelopes => {}
        }
    }
    Ok(())
}

const ALL_ITEM_FIELDS: &[ItemField] = &[
    ItemField::Muted,
    ItemField::Locked,
    ItemField::Volume,
    ItemField::FadeIn,
    ItemField::FadeOut,
    ItemField::Label,
    ItemField::Color,
    ItemField::Take,
    ItemField::FixedLane,
    ItemField::Loop,
    ItemField::Stretch,
    ItemField::Notes,
];

async fn set_item_fields(
    project: &Project,
    media: &MediaRoot,
    guid: &str,
    item: &ItemState,
    fields: &[ItemField],
) -> daw_control::Result<()> {
    let Some(h) = project.items().by_guid(guid).await? else {
        return Ok(());
    };
    for field in fields {
        match field {
            ItemField::Track => h.move_to_track(TrackRef::Guid(item.track.clone())).await?,
            ItemField::Position => {
                h.set_position(PositionInSeconds::from_seconds(item.position))
                    .await?;
            }
            ItemField::Length => h.set_length(Duration::from_seconds(item.length)).await?,
            // No setter on the handle yet; rides along with position.
            ItemField::SnapOffset => {}
            ItemField::Muted if item.muted => h.mute().await?,
            ItemField::Muted => h.unmute().await?,
            ItemField::Locked if item.locked => h.lock().await?,
            ItemField::Locked => h.unlock().await?,
            ItemField::Volume => h.set_volume(item.volume).await?,
            ItemField::FadeIn | ItemField::FadeInShape => {
                h.set_fade_in(
                    Duration::from_seconds(item.fade_in),
                    fade_shape(&item.fade_in_shape),
                )
                .await?;
            }
            ItemField::FadeOut | ItemField::FadeOutShape => {
                h.set_fade_out(
                    Duration::from_seconds(item.fade_out),
                    fade_shape(&item.fade_out_shape),
                )
                .await?;
            }
            ItemField::Label => h.set_label(&item.label).await?,
            ItemField::Color => h.set_color(item.color).await?,
            ItemField::Take => {
                let take = h.active_take();
                take.set_name(&item.take.name).await?;
                if let Some(source) = &item.take.source {
                    take.set_source_file(&media.to_local(source)).await?;
                }
                take.set_start_offset(Duration::from_seconds(item.take.start_offset))
                    .await?;
                take.set_play_rate(item.take.playrate).await?;
            }
            ItemField::FixedLane => {
                if let Some(lane) = item.fixed_lane {
                    h.set_fixed_lane(lane).await?;
                }
            }
            ItemField::Loop => h.set_loop_source(item.loop_source).await?,
            ItemField::Stretch => {
                h.active_take()
                    .set_stretch_markers(
                        item.take
                            .stretch
                            .iter()
                            .map(|m| daw_proto::StretchMarker {
                                position: m.at,
                                source_position: m.source,
                                slope: m.slope,
                            })
                            .collect(),
                    )
                    .await?;
            }
            ItemField::Notes => {
                let notes = item
                    .take
                    .notes
                    .iter()
                    .map(|n| daw_proto::MidiNoteCreate {
                        channel: n.channel,
                        pitch: n.pitch,
                        velocity: n.velocity,
                        start_ppq: n.start,
                        length_ppq: n.length,
                    })
                    .collect();
                // Cleared, then the notes in the take's own ticks — the
                // units `notes()` reads them in.
                let midi = h.active_take().midi();
                midi.write_take(
                    daw_proto::midi::MidiTakeContent::default(),
                    daw_proto::midi::WriteMode::Replace,
                )
                .await?;
                midi.add_notes_ppq(notes).await?;
            }
        }
    }
    Ok(())
}

async fn marker_id(project: &Project, guid: &str) -> daw_control::Result<Option<u32>> {
    Ok(project
        .markers()
        .all()
        .await?
        .into_iter()
        .find(|m| m.guid.as_deref() == Some(guid))
        .and_then(|m| m.id))
}

async fn region_id(project: &Project, guid: &str) -> daw_control::Result<Option<u32>> {
    Ok(project
        .regions()
        .all()
        .await?
        .into_iter()
        .find(|r| r.guid.as_deref() == Some(guid))
        .and_then(|r| r.id))
}

async fn set_marker(project: &Project, guid: &str, m: &MarkerState) -> daw_control::Result<()> {
    let markers = project.markers();
    let id = match marker_id(project, guid).await? {
        Some(id) => {
            markers.move_to(id, m.at).await?;
            markers.rename(id, &m.name).await?;
            id
        }
        None => markers.add_with_guid(guid, m.at, &m.name).await?,
    };
    if let Some(c) = m.color {
        markers.set_color(id, c).await?;
    }
    Ok(())
}

async fn set_region(project: &Project, guid: &str, r: &RegionState) -> daw_control::Result<()> {
    let regions = project.regions();
    let id = match region_id(project, guid).await? {
        Some(id) => {
            regions.set_bounds(id, r.start, r.end).await?;
            regions.rename(id, &r.name).await?;
            id
        }
        None => regions.add_with_guid(guid, r.start, r.end, &r.name).await?,
    };
    if let Some(c) = r.color {
        regions.set_color(id, c).await?;
    }
    Ok(())
}

const fn fade_name(shape: FadeShape) -> &'static str {
    match shape {
        FadeShape::Linear => "linear",
        FadeShape::FastStart => "fast_start",
        FadeShape::FastEnd => "fast_end",
        FadeShape::FastStartSteep => "fast_start_steep",
        FadeShape::FastEndSteep => "fast_end_steep",
        FadeShape::SlowStartEnd => "slow_start_end",
        FadeShape::SlowStartEndSteep => "slow_start_end_steep",
    }
}

fn fade_shape(name: &str) -> FadeShape {
    match name {
        "fast_start" => FadeShape::FastStart,
        "fast_end" => FadeShape::FastEnd,
        "fast_start_steep" => FadeShape::FastStartSteep,
        "fast_end_steep" => FadeShape::FastEndSteep,
        "slow_start_end" => FadeShape::SlowStartEnd,
        "slow_start_end_steep" => FadeShape::SlowStartEndSteep,
        _ => FadeShape::Linear,
    }
}

// ── a track's parts ─────────────────────────────────────────────────────

/// A track's group masks by family, the non-zero ones.
fn grouping_map(g: &daw_proto::track::TrackGrouping) -> BTreeMap<String, u128> {
    [
        ("volume_lead", g.volume_lead),
        ("volume_follow", g.volume_follow),
        ("pan_lead", g.pan_lead),
        ("pan_follow", g.pan_follow),
        ("mute_lead", g.mute_lead),
        ("mute_follow", g.mute_follow),
        ("solo_lead", g.solo_lead),
        ("solo_follow", g.solo_follow),
        ("recarm_lead", g.recarm_lead),
        ("recarm_follow", g.recarm_follow),
        ("polarity_lead", g.polarity_lead),
        ("polarity_follow", g.polarity_follow),
        ("automode_lead", g.automode_lead),
        ("automode_follow", g.automode_follow),
        ("width_lead", g.width_lead),
        ("width_follow", g.width_follow),
        ("vca_lead", g.vca_lead),
        ("vca_follow", g.vca_follow),
        ("media_edit_lead", g.media_edit_lead),
        ("media_edit_follow", g.media_edit_follow),
    ]
    .into_iter()
    .filter(|(_, mask)| *mask != 0)
    .map(|(family, mask)| (family.to_owned(), mask))
    .collect()
}

/// Each group family, by the name its masks are kept under.
const FAMILIES: [(&str, daw_proto::track::GroupFamily); 10] = [
    ("volume", daw_proto::track::GroupFamily::Volume),
    ("vca", daw_proto::track::GroupFamily::Vca),
    ("pan", daw_proto::track::GroupFamily::Pan),
    ("width", daw_proto::track::GroupFamily::Width),
    ("mute", daw_proto::track::GroupFamily::Mute),
    ("solo", daw_proto::track::GroupFamily::Solo),
    ("recarm", daw_proto::track::GroupFamily::RecArm),
    ("polarity", daw_proto::track::GroupFamily::Polarity),
    ("automode", daw_proto::track::GroupFamily::AutoMode),
    ("media_edit", daw_proto::track::GroupFamily::MediaEdit),
];

/// An envelope's key on its track, or `None` for one the doc does not
/// keep (a take's, play rate, a named one).
fn envelope_key(e: &daw_proto::automation::Envelope, ids: &EngineIds) -> Option<String> {
    use daw_proto::automation::EnvelopeType as T;
    Some(match e.envelope_type {
        T::Volume => "volume".to_owned(),
        T::VolumePrefx => "volume_prefx".to_owned(),
        T::Pan => "pan".to_owned(),
        T::PanPrefx => "pan_prefx".to_owned(),
        T::Width => "width".to_owned(),
        T::WidthPrefx => "width_prefx".to_owned(),
        T::Mute => "mute".to_owned(),
        T::FxParam => format!(
            "fx:{}:{}",
            ids.fx.doc(e.fx_guid.as_deref()?),
            e.param_index?
        ),
        _ => return None,
    })
}

/// The envelope a key names, on the engine (FX ids its own).
fn envelope_ref(key: &str, ids: &EngineIds) -> Option<daw_proto::automation::EnvelopeRef> {
    use daw_proto::automation::{EnvelopeRef as R, EnvelopeType as T};
    Some(match key {
        "volume" => R::Type(T::Volume),
        "volume_prefx" => R::Type(T::VolumePrefx),
        "pan" => R::Type(T::Pan),
        "pan_prefx" => R::Type(T::PanPrefx),
        "width" => R::Type(T::Width),
        "width_prefx" => R::Type(T::WidthPrefx),
        "mute" => R::Type(T::Mute),
        other => {
            let (fx, param) = other.strip_prefix("fx:")?.rsplit_once(':')?;
            R::FxParam {
                fx_guid: ids.fx.engine(fx),
                param_index: param.parse().ok()?,
            }
        }
    })
}

const fn mode_name(mode: daw_proto::primitives::AutomationMode) -> &'static str {
    use daw_proto::primitives::AutomationMode as M;
    match mode {
        M::TrimRead => "trim",
        M::Read => "read",
        M::Touch => "touch",
        M::Write => "write",
        M::Latch => "latch",
        M::LatchPreview => "latch_preview",
        M::Off => "off",
    }
}

fn mode_of(name: &str) -> daw_proto::primitives::AutomationMode {
    use daw_proto::primitives::AutomationMode as M;
    match name {
        "read" => M::Read,
        "touch" => M::Touch,
        "write" => M::Write,
        "latch" => M::Latch,
        "latch_preview" => M::LatchPreview,
        "off" => M::Off,
        _ => M::TrimRead,
    }
}

const fn shape_number(shape: daw_proto::automation::EnvelopeShape) -> u8 {
    use daw_proto::automation::EnvelopeShape as S;
    match shape {
        S::Linear => 0,
        S::Square => 1,
        S::SlowStartEnd => 2,
        S::FastStart => 3,
        S::FastEnd => 4,
        S::Bezier => 5,
    }
}

const fn shape_of(n: u8) -> daw_proto::automation::EnvelopeShape {
    use daw_proto::automation::EnvelopeShape as S;
    match n {
        1 => S::Square,
        2 => S::SlowStartEnd,
        3 => S::FastStart,
        4 => S::FastEnd,
        5 => S::Bezier,
        _ => S::Linear,
    }
}

const fn send_mode_name(mode: daw_proto::routing::SendMode) -> &'static str {
    use daw_proto::routing::SendMode as M;
    match mode {
        M::PreFx => "pre_fx",
        M::PostFx => "post_fx",
        M::PostFader => "post_fader",
    }
}

fn send_mode_of(name: &str) -> daw_proto::routing::SendMode {
    use daw_proto::routing::SendMode as M;
    match name {
        "pre_fx" => M::PreFx,
        "post_fx" => M::PostFx,
        _ => M::PostFader,
    }
}

/// A track's FX (their state into `blobs`), sends and envelopes.
async fn read_track_parts(
    handle: &daw_control::TrackHandle,
    state: &mut TrackState,
    blobs: &dyn Blobs,
    ids: &EngineIds,
) -> daw_control::Result<()> {
    let chain = handle.fx_chain();
    for fx in chain.all().await? {
        let saved = match chain.by_guid(&fx.guid).await? {
            Some(h) => h.state_chunk().await.ok().flatten(),
            None => None,
        };
        state.fx.push(FxState {
            guid: ids.fx.doc(&fx.guid),
            plugin: if fx.plugin_name.is_empty() {
                fx.name.clone()
            } else {
                fx.plugin_name.clone()
            },
            name: fx.name,
            enabled: fx.enabled,
            offline: fx.offline,
            state: saved.map(|bytes| blobs.put(&bytes)),
        });
    }
    for route in handle.sends().all().await? {
        let Some(dest) = route.dest_track_guid else {
            continue;
        };
        state.sends.push(SendState {
            dest,
            mode: send_mode_name(route.send_mode).to_owned(),
            volume: route.volume,
            pan: route.pan,
            muted: route.muted,
            from: route.source_channels.start_channel,
            to: route.dest_channels.start_channel,
        });
    }
    let envelopes = handle.envelopes();
    for e in envelopes.all().await? {
        let Some(key) = envelope_key(&e, ids) else {
            continue;
        };
        let Some(reference) = envelope_ref(&key, ids) else {
            continue;
        };
        let points = envelopes.of(reference).points().await?;
        if points.is_empty() {
            continue;
        }
        state.envelopes.insert(
            key,
            EnvelopeState {
                mode: mode_name(e.automation_mode).to_owned(),
                visible: e.visible,
                armed: e.armed,
                points: points
                    .iter()
                    .map(|p| EnvelopePointState {
                        at: p.time.as_seconds(),
                        value: p.value,
                        shape: shape_number(p.shape),
                        tension: p.tension,
                    })
                    .collect(),
            },
        );
    }
    Ok(())
}

/// Make the engine's chain on `track` the doc's: what is missing added
/// (with its state), what is gone removed, each one's switches set, all
/// in the doc's order. New FX are bound in `ids`.
async fn apply_fx(
    project: &Project,
    track: &TrackState,
    blobs: &dyn Blobs,
    ids: &mut EngineIds,
) -> daw_control::Result<()> {
    let Some(handle) = project.tracks().by_guid(&track.guid).await? else {
        return Ok(());
    };
    let chain = handle.fx_chain();
    let wanted: std::collections::HashSet<String> =
        track.fx.iter().map(|f| ids.fx.engine(&f.guid)).collect();
    for fx in chain.all().await? {
        if !wanted.contains(&fx.guid)
            && let Some(h) = chain.by_guid(&fx.guid).await?
        {
            h.remove().await?;
        }
    }
    for (index, fx) in track.fx.iter().enumerate() {
        let at = u32::try_from(index).unwrap_or(u32::MAX);
        let guid = ids.fx.engine(&fx.guid);
        let h = if let Some(h) = chain.by_guid(&guid).await? {
            h.move_to(at).await?;
            // Its state only if it changed: a plugin reloads on a load.
            if let Some(name) = &fx.state
                && h.state_chunk()
                    .await
                    .ok()
                    .flatten()
                    .map(|b| blob_name(&b))
                    .as_ref()
                    != Some(name)
                && let Some(bytes) = blobs.get(name)
            {
                h.set_state_chunk(bytes).await?;
            }
            h
        } else {
            let h = match chain.add_at(&fx.plugin, at).await {
                Ok(h) => h,
                Err(_) => chain.add_at(&fx.name, at).await?,
            };
            let info = h.info().await?;
            ids.fx.bind(&fx.guid, &info.guid);
            if let Some(bytes) = fx.state.as_deref().and_then(|name| blobs.get(name)) {
                h.set_state_chunk(bytes).await?;
            }
            h
        };
        if fx.enabled {
            h.enable().await?;
        } else {
            h.disable().await?;
        }
        h.set_offline(fx.offline).await?;
    }
    Ok(())
}

/// Make the engine's sends from `track` the doc's: every one removed and
/// the doc's added in order (sends have no identity of their own).
async fn apply_sends(project: &Project, track: &TrackState) -> daw_control::Result<()> {
    let Some(handle) = project.tracks().by_guid(&track.guid).await? else {
        return Ok(());
    };
    let sends = handle.sends();
    for _ in sends.all().await? {
        if let Some(first) = sends.by_index(0).await? {
            first.remove().await?;
        }
    }
    for send in &track.sends {
        let route = sends.add_to(&send.dest).await?;
        route.set_send_mode(send_mode_of(&send.mode)).await?;
        route.set_volume(send.volume).await?;
        route.set_pan(send.pan).await?;
        if send.muted {
            route.mute().await?;
        }
        if send.from != 0 {
            route.set_source_channels(send.from, 2).await?;
        }
        if send.to != 0 {
            route.set_dest_channels(send.to, 2).await?;
        }
    }
    Ok(())
}

/// Make the engine's envelopes on `track` the doc's: each one's points
/// replaced, its mode and switches set; one the doc no longer has
/// cleared.
async fn apply_envelopes(
    project: &Project,
    track: &TrackState,
    ids: &EngineIds,
) -> daw_control::Result<()> {
    let Some(handle) = project.tracks().by_guid(&track.guid).await? else {
        return Ok(());
    };
    let envelopes = handle.envelopes();
    for e in envelopes.all().await? {
        if let Some(key) = envelope_key(&e, ids)
            && !track.envelopes.contains_key(&key)
            && let Some(reference) = envelope_ref(&key, ids)
        {
            envelopes.of(reference).clear().await?;
        }
    }
    for (key, envelope) in &track.envelopes {
        let Some(reference) = envelope_ref(key, ids) else {
            continue;
        };
        let h = envelopes.of(reference);
        h.clear().await?;
        for p in &envelope.points {
            h.add_point(
                PositionInSeconds::from_seconds(p.at),
                p.value,
                shape_of(p.shape),
            )
            .await?;
        }
        h.set_automation_mode(mode_of(&envelope.mode)).await?;
        h.set_visible(envelope.visible).await?;
        h.set_armed(envelope.armed).await?;
    }
    Ok(())
}

/// Make the engine's group masks on `track` the doc's: each slot the
/// track is in joined (every family, both ways), then each family set to
/// its role — lead, follow, both, or out.
async fn apply_grouping(
    handle: &daw_control::TrackHandle,
    track: &TrackState,
) -> daw_control::Result<()> {
    let now = grouping_map(&handle.group_flags().await?);
    let mask = |map: &BTreeMap<String, u128>, family: &str, role: &str| {
        map.get(&format!("{family}_{role}")).copied().unwrap_or(0)
    };
    let slots: u128 = track
        .grouping
        .values()
        .chain(now.values())
        .fold(0, |all, m| all | m);
    for slot in 1..=128_u32 {
        let Some(bit) = 1_u128.checked_shl(slot.saturating_sub(1)) else {
            continue;
        };
        if slots & bit == 0 {
            continue;
        }
        let member = track.grouping.values().any(|m| m & bit != 0);
        handle.set_group_membership(slot, member).await?;
        if !member {
            continue;
        }
        for (family, which) in FAMILIES {
            let lead = mask(&track.grouping, family, "lead") & bit != 0;
            let follow = mask(&track.grouping, family, "follow") & bit != 0;
            let role = match (lead, follow) {
                (true, true) => continue,
                (true, false) => daw_proto::track::GroupRole::Lead,
                (false, true) => daw_proto::track::GroupRole::Follow,
                (false, false) => daw_proto::track::GroupRole::None,
            };
            handle.set_group_flags(slot, which, role).await?;
        }
    }
    Ok(())
}
