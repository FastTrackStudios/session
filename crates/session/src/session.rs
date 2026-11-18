//! The session: song structure, the timeline, and each extension's data.

use std::collections::BTreeMap;

use crate::ext::ExtensionData;
use crate::id::{ExtensionId, ItemId, LaneId, MarkId};
use crate::timeline::{Item, Lane, Marker, Region, TempoMap};

/// One session, as plain data.
///
/// This is what the Architect document holds (feature `architect`,
/// [`crate::doc::SessionDoc`]) and what a file round-trips through. What it
/// does not hold: any track, take, plugin, chord or cue as such. Those are
/// lanes, items and entries of the extensions that bring them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    /// Display name.
    pub name: String,
    /// Tempo and meter.
    pub tempo: TempoMap,
    /// The rulers markers and regions sit on, by index: their names ("Song",
    /// "Sections"). Index `0` is the default ruler.
    pub rulers: BTreeMap<u32, String>,
    pub markers: BTreeMap<MarkId, Marker>,
    pub regions: BTreeMap<MarkId, Region>,
    pub lanes: BTreeMap<LaneId, Lane>,
    pub items: BTreeMap<ItemId, Item>,
    extensions: BTreeMap<ExtensionId, ExtensionData>,
}

impl Session {
    /// An empty session at 120 BPM in 4/4.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// What extension `id` keeps here, if it keeps anything.
    ///
    /// Each extension reads and writes its own namespace by its own id;
    /// there is no call that lists another's entries. (Whether one
    /// extension may *ask* for another's is Architect's permission check
    /// on the caller, not the data model's.)
    #[must_use]
    pub fn ext(&self, id: &ExtensionId) -> Option<&ExtensionData> {
        self.extensions.get(id)
    }

    /// What extension `id` keeps here, made empty on first use.
    pub fn ext_mut(&mut self, id: &ExtensionId) -> &mut ExtensionData {
        self.extensions.entry(id.clone()).or_default()
    }

    /// Which extensions keep data here (not what they keep).
    pub fn extension_ids(&self) -> impl Iterator<Item = &ExtensionId> {
        self.extensions.keys()
    }

    /// Puts `data` as extension `id`'s whole namespace (what a load does).
    pub fn set_ext(&mut self, id: ExtensionId, data: ExtensionData) {
        self.extensions.insert(id, data);
    }

    /// Removes everything extension `id` brought: its data, its lanes and
    /// every item it owns or that sits on its lanes. Nothing else moves.
    pub fn remove_extension(&mut self, id: &ExtensionId) {
        self.extensions.remove(id);
        let gone: Vec<LaneId> = self
            .lanes
            .iter()
            .filter(|(_, lane)| &lane.owner == id)
            .map(|(lane_id, _)| lane_id.clone())
            .collect();
        for lane in &gone {
            self.lanes.remove(lane);
        }
        self.items
            .retain(|_, item| &item.owner != id && !gone.contains(&item.lane));
    }

    /// The lanes `owner` made, in order.
    #[must_use]
    pub fn lanes_of(&self, owner: &ExtensionId) -> Vec<(&LaneId, &Lane)> {
        let mut lanes: Vec<_> = self
            .lanes
            .iter()
            .filter(|(_, lane)| &lane.owner == owner)
            .collect();
        lanes.sort_by(|a, b| a.1.order.total_cmp(&b.1.order).then_with(|| a.0.cmp(b.0)));
        lanes
    }

    /// The items on `lane`, by start time (seconds, under the tempo map).
    #[must_use]
    pub fn items_on(&self, lane: &LaneId) -> Vec<(&ItemId, &Item)> {
        let mut items: Vec<_> = self
            .items
            .iter()
            .filter(|(_, item)| &item.lane == lane)
            .collect();
        items.sort_by(|a, b| {
            a.1.at
                .seconds(&self.tempo)
                .total_cmp(&b.1.at.seconds(&self.tempo))
                .then_with(|| a.0.cmp(b.0))
        });
        items
    }

    /// The items of `kind`, as `(seconds, item)`, by time: what a player
    /// builds a [`crate::sync::Schedule`] from (off the audio thread).
    #[must_use]
    pub fn timed(&self, kind: &str) -> Vec<(f64, &Item)> {
        let mut out: Vec<_> = self
            .items
            .values()
            .filter(|item| item.kind == kind)
            .map(|item| (item.at.seconds(&self.tempo), item))
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }
}
