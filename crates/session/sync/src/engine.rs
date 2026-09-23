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
    ItemState, MarkerState, RegionState, SessionModel, TakeState, TempoPoint, TrackState,
};

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
        Path::new(path)
            .strip_prefix(&self.0)
            .map_or_else(|_| path.to_string(), |rel| rel.to_string_lossy().into_owned())
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
pub async fn read(project: &Project, media: &MediaRoot) -> daw_control::Result<SessionModel> {
    let tracks = project.tracks().all().await?;
    let parents: HashMap<String, Option<String>> =
        tracks.iter().map(|t| (t.guid.clone(), t.parent_guid.clone())).collect();
    let tracks = tracks
        .into_iter()
        .map(|t| TrackState {
            parent: parents.get(&t.guid).cloned().flatten(),
            guid: t.guid,
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
        })
        .collect();

    let mut items = BTreeMap::new();
    for item in project.items().all().await? {
        let Some(handle) = project.items().by_guid(&item.guid).await? else {
            continue;
        };
        let take = match handle.active_take().info().await {
            Ok(take) => TakeState {
                name: take.name,
                source: take.source_file_path.map(|p| media.to_doc(&p)),
                start_offset: take.start_offset.as_seconds(),
                playrate: take.play_rate,
            },
            // An item with no take (a label-only KEY item, say).
            Err(_) => TakeState { playrate: 1.0, ..TakeState::default() },
        };
        items.insert(
            item.guid.clone(),
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
            },
        );
    }

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
            let (beats_per_bar, beat_unit) =
                p.time_signature.map_or((4, 4), |s| (s.numerator, s.denominator));
            TempoPoint {
                at: p.position.time.map_or(0.0, |t| t.as_seconds()),
                bpm: p.bpm,
                beats_per_bar,
                beat_unit,
            }
        })
        .collect();

    Ok(SessionModel { tracks, items, markers, regions, tempo, chart: String::new() })
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
) -> daw_control::Result<()> {
    let mut structure = false;
    for change in changes {
        match change {
            Change::TrackAdded { track, .. } => {
                structure = true;
                let index = target.tracks.iter().position(|t| t.guid == track.guid);
                let at = index.and_then(|i| u32::try_from(i).ok());
                project.tracks().add_with_guid(&track.guid, &track.name, at).await?;
                set_track_fields(project, track, ALL_TRACK_FIELDS).await?;
            }
            Change::TrackMoved { .. } => structure = true,
            Change::TrackChanged { track, fields } => {
                set_track_fields(project, track, fields).await?;
            }
            Change::TrackRemoved { guid } => {
                project.tracks().remove(TrackRef::Guid(guid.clone())).await?;
            }
            Change::ItemAdded { guid, item } => {
                let handle = project
                    .items()
                    .add_with_guid(
                        &item.track,
                        guid,
                        PositionInSeconds::from_seconds(item.position),
                        Duration::from_seconds(item.length),
                    )
                    .await?;
                // A new item has no take; the one it plays is added here
                // (take guids are each engine's own — the doc keeps the
                // active take's fields on the item).
                let take = handle.takes().add().await?;
                if let Some(source) = &item.take.source {
                    take.set_source_file(&media.to_local(source)).await?;
                }
                set_item_fields(project, media, guid, item, ALL_ITEM_FIELDS).await?;
            }
            Change::ItemChanged { guid, item, fields } => {
                set_item_fields(project, media, guid, item, fields).await?;
            }
            Change::ItemRemoved { guid } => {
                if let Some(handle) = project.items().by_guid(guid).await? {
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
    Ok(())
}

/// Put every track at its place in project order, and set the folder
/// depths that make the tree: a track's depth is how far the NEXT track's
/// nesting level differs from its own (REAPER's convention — 1 opens a
/// folder, -n closes n of them).
async fn apply_structure(project: &Project, tracks: &[TrackState]) -> daw_control::Result<()> {
    for (index, track) in tracks.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else { break };
        project.tracks().move_to(TrackRef::Guid(track.guid.clone()), index).await?;
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
                h.set_visibility(track.visible_in_tcp, track.visible_in_mixer).await?;
            }
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
                h.set_position(PositionInSeconds::from_seconds(item.position)).await?;
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
                h.set_fade_in(Duration::from_seconds(item.fade_in), fade_shape(&item.fade_in_shape))
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
                take.set_start_offset(Duration::from_seconds(item.take.start_offset)).await?;
                take.set_play_rate(item.take.playrate).await?;
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
