//! What changed between two models, as the steps an engine takes to get
//! from one to the other.
//!
//! The doc side never needs this — [`crate::SessionDoc::write`] reconciles
//! field by field on its own. This is the other direction: a remote edit
//! arrives, the doc now reads differently from what the engine last
//! showed, and the engine has to be told exactly what moved.
//!
//! Steps come out in an order an engine can apply blindly: tracks are
//! created before anything is put on them, moved before they are edited,
//! and removed only after every item on them has gone.

use std::collections::{HashMap, HashSet};

use crate::model::{ItemState, MarkerState, RegionState, SessionModel, TempoPoint, TrackState};

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A new track, at `index` among its folder's children.
    TrackAdded {
        track: TrackState,
        index: usize,
    },
    /// A track now sits in a different folder, or at a different place
    /// among its siblings.
    TrackMoved {
        guid: String,
        parent: Option<String>,
        index: usize,
    },
    TrackChanged {
        track: TrackState,
        fields: Vec<TrackField>,
    },
    TrackRemoved {
        guid: String,
    },
    ItemAdded {
        guid: String,
        item: ItemState,
    },
    ItemChanged {
        guid: String,
        item: ItemState,
        fields: Vec<ItemField>,
    },
    ItemRemoved {
        guid: String,
    },
    MarkerSet {
        guid: String,
        marker: MarkerState,
    },
    MarkerRemoved {
        guid: String,
    },
    RegionSet {
        guid: String,
        region: RegionState,
    },
    RegionRemoved {
        guid: String,
    },
    TempoChanged(Vec<TempoPoint>),
    ChartChanged(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackField {
    Name,
    Color,
    Volume,
    Pan,
    Muted,
    Soloed,
    PhaseInverted,
    ParentSend,
    VisibleInTcp,
    VisibleInMixer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemField {
    Track,
    Position,
    Length,
    SnapOffset,
    Muted,
    Locked,
    Volume,
    FadeIn,
    FadeOut,
    FadeInShape,
    FadeOutShape,
    Label,
    Color,
    Take,
}

/// The steps from `old` to `new`.
#[must_use]
pub fn diff(old: &SessionModel, new: &SessionModel) -> Vec<Change> {
    let mut out = Vec::new();
    let old_tracks: HashMap<&str, &TrackState> =
        old.tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let old_slots = sibling_slots(&old.tracks);
    let new_slots = sibling_slots(&new.tracks);

    for track in &new.tracks {
        let index = new_slots
            .get(track.guid.as_str())
            .copied()
            .unwrap_or_default();
        match old_tracks.get(track.guid.as_str()) {
            None => out.push(Change::TrackAdded {
                track: track.clone(),
                index,
            }),
            Some(before) => {
                if before.parent != track.parent
                    || old_slots.get(track.guid.as_str()) != Some(&index)
                {
                    out.push(Change::TrackMoved {
                        guid: track.guid.clone(),
                        parent: track.parent.clone(),
                        index,
                    });
                }
                let fields = track_fields(before, track);
                if !fields.is_empty() {
                    out.push(Change::TrackChanged {
                        track: track.clone(),
                        fields,
                    });
                }
            }
        }
    }

    for (guid, item) in &new.items {
        match old.items.get(guid) {
            None => out.push(Change::ItemAdded {
                guid: guid.clone(),
                item: item.clone(),
            }),
            Some(before) => {
                let fields = item_fields(before, item);
                if !fields.is_empty() {
                    out.push(Change::ItemChanged {
                        guid: guid.clone(),
                        item: item.clone(),
                        fields,
                    });
                }
            }
        }
    }
    for guid in old.items.keys().filter(|g| !new.items.contains_key(*g)) {
        out.push(Change::ItemRemoved { guid: guid.clone() });
    }

    for (guid, marker) in &new.markers {
        if old.markers.get(guid) != Some(marker) {
            out.push(Change::MarkerSet {
                guid: guid.clone(),
                marker: marker.clone(),
            });
        }
    }
    for guid in old.markers.keys().filter(|g| !new.markers.contains_key(*g)) {
        out.push(Change::MarkerRemoved { guid: guid.clone() });
    }
    for (guid, region) in &new.regions {
        if old.regions.get(guid) != Some(region) {
            out.push(Change::RegionSet {
                guid: guid.clone(),
                region: region.clone(),
            });
        }
    }
    for guid in old.regions.keys().filter(|g| !new.regions.contains_key(*g)) {
        out.push(Change::RegionRemoved { guid: guid.clone() });
    }

    if old.tempo != new.tempo {
        out.push(Change::TempoChanged(new.tempo.clone()));
    }
    if old.chart != new.chart {
        out.push(Change::ChartChanged(new.chart.clone()));
    }

    let kept: HashSet<&str> = new.tracks.iter().map(|t| t.guid.as_str()).collect();
    // Deepest first, so a folder goes after what it held.
    for track in old
        .tracks
        .iter()
        .rev()
        .filter(|t| !kept.contains(t.guid.as_str()))
    {
        out.push(Change::TrackRemoved {
            guid: track.guid.clone(),
        });
    }
    out
}

/// Each track's index among its folder's children.
fn sibling_slots(tracks: &[TrackState]) -> HashMap<&str, usize> {
    let mut counts: HashMap<Option<&str>, usize> = HashMap::new();
    tracks
        .iter()
        .map(|t| {
            let slot = counts.entry(t.parent.as_deref()).or_insert(0);
            let index = *slot;
            *slot = slot.saturating_add(1);
            (t.guid.as_str(), index)
        })
        .collect()
}

#[allow(clippy::float_cmp)] // exact: a value the engine reported, read back
fn track_fields(a: &TrackState, b: &TrackState) -> Vec<TrackField> {
    let checks = [
        (a.name != b.name, TrackField::Name),
        (a.color != b.color, TrackField::Color),
        (a.volume != b.volume, TrackField::Volume),
        (a.pan != b.pan, TrackField::Pan),
        (a.muted != b.muted, TrackField::Muted),
        (a.soloed != b.soloed, TrackField::Soloed),
        (
            a.phase_inverted != b.phase_inverted,
            TrackField::PhaseInverted,
        ),
        (a.parent_send != b.parent_send, TrackField::ParentSend),
        (
            a.visible_in_tcp != b.visible_in_tcp,
            TrackField::VisibleInTcp,
        ),
        (
            a.visible_in_mixer != b.visible_in_mixer,
            TrackField::VisibleInMixer,
        ),
    ];
    checks
        .into_iter()
        .filter(|(changed, _)| *changed)
        .map(|(_, f)| f)
        .collect()
}

#[allow(clippy::float_cmp)] // exact: a value the engine reported, read back
fn item_fields(a: &ItemState, b: &ItemState) -> Vec<ItemField> {
    let checks = [
        (a.track != b.track, ItemField::Track),
        (a.position != b.position, ItemField::Position),
        (a.length != b.length, ItemField::Length),
        (a.snap_offset != b.snap_offset, ItemField::SnapOffset),
        (a.muted != b.muted, ItemField::Muted),
        (a.locked != b.locked, ItemField::Locked),
        (a.volume != b.volume, ItemField::Volume),
        (a.fade_in != b.fade_in, ItemField::FadeIn),
        (a.fade_out != b.fade_out, ItemField::FadeOut),
        (a.fade_in_shape != b.fade_in_shape, ItemField::FadeInShape),
        (
            a.fade_out_shape != b.fade_out_shape,
            ItemField::FadeOutShape,
        ),
        (a.label != b.label, ItemField::Label),
        (a.color != b.color, ItemField::Color),
        (a.take != b.take, ItemField::Take),
    ];
    checks
        .into_iter()
        .filter(|(changed, _)| *changed)
        .map(|(_, f)| f)
        .collect()
}
