//! The project, as one immutable snapshot.
//!
//! Everything the studio draws comes from here, fetched once and then
//! held behind an [`Arc`]. That is the first rule this rebuild is built
//! on: **a component never owns project data it could share**. Dioxus
//! hands a component its props by value on every render, so a
//! `Vec<Item>` prop of a real session's eight hundred takes is a deep
//! copy per render — which is how a UI ends up spending its frame budget
//! memcpy'ing a record it did not change.
//!
//! The snapshot is also *flat and pre-grouped*. Items arrive from the
//! facade as one list for the whole project; a lane wants only its own,
//! and looking that up by scanning is `O(tracks × items)` on every
//! render. Grouping once at fetch time makes drawing a lane an index.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use daw_proto::{Item, Track};

/// A song section — REAPER's regions, the coloured bands over the ruler.
#[derive(Clone, PartialEq, Debug)]
pub struct Section {
    /// REAPER's own region number — what addresses it for an edit.
    ///
    /// Not the index in this list, and not stable across a renumber:
    /// markers and regions are one list in REAPER wearing one set of
    /// numbers, and its renumber action reassigns them. The event
    /// stream reports that as `Renumbered` rather than as a deletion,
    /// so a window holding this can follow it.
    pub id: u32,
    pub start: f64,
    pub end: f64,
    pub name: String,
    /// Already resolved to CSS, so a render is a projection and not a
    /// colour conversion per band per frame.
    pub color: Option<String>,
    /// The ruler lane it sits on (REAPER 7.62+); 0 is the default lane.
    pub lane: u32,
}

/// A project marker — the numbered flags under the region lane.
#[derive(Clone, PartialEq, Debug)]
pub struct Marker {
    pub at: f64,
    pub name: String,
    pub color: Option<String>,
    /// REAPER's own marker number, which is what the flag is labelled
    /// with. Not the index in this list: markers can be renumbered.
    pub idx: u32,
    /// The ruler lane it sits on (REAPER 7.62+); 0 is the default lane.
    pub lane: u32,
}

/// Everything the window stands on, in one pass.
///
/// Deliberately NOT `Default`-constructed into the live signal and then
/// filled in field by field: a half-populated project is a state every
/// component would have to defend against. The signal holds
/// `Option<Arc<Project>>` instead, so "not open yet" is one branch at
/// the root rather than an empty vector everywhere.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Project {
    /// Project order, folders included — the TCP's own list before any
    /// collapse is applied.
    pub tracks: Vec<Track>,
    /// Items keyed by their track's guid, in time order within a track.
    pub items: HashMap<String, Vec<Item>>,
    pub sections: Vec<Section>,
    pub markers: Vec<Marker>,
    /// The project tempo. One number, for now: the ruler's bar lines
    /// assume a constant tempo, which is true of most sessions and
    /// wrong for some. A tempo map belongs here when the ruler grows
    /// one — the shape of this struct is what would change, not the
    /// components that read it.
    pub bpm: f64,
    /// End of the last item — how far the timeline has to reach.
    pub length_secs: f64,
    /// How many items the project holds, across every track. Kept
    /// because a count of a grouped map is a walk, and the readout
    /// wants it every render.
    pub item_count: usize,
    /// Each item's title — its active take's name — keyed by item guid.
    /// An item carries no name of its own; the take does, and the
    /// arrangement writes it on the item.
    pub names: HashMap<String, String>,
    /// Which items hold MIDI rather than audio, by item guid.
    ///
    /// A set and not a flag on `Item`, because `Item` is the daw's own
    /// type and MIDI-ness lives on the TAKE — an item can hold several,
    /// and which one is playing is the item's business. This is the
    /// active take's answer, which is the one being drawn.
    pub midi: HashSet<String>,
}

impl Project {
    /// What an item is called: its active take's name, else its label,
    /// else nothing.
    /// Does this item hold MIDI rather than audio?
    ///
    /// The active take's answer, which is the one being drawn.
    #[must_use]
    pub fn is_midi(&self, guid: &str) -> bool {
        self.midi.contains(guid)
    }

    pub fn title<'a>(&'a self, item: &'a Item) -> Option<&'a str> {
        self.names
            .get(&item.guid)
            .map(String::as_str)
            .filter(|n| !n.is_empty())
            .or(item.label.as_deref())
    }

    /// This track's items, or an empty slice. Never allocates: a lane
    /// renders every frame it is on screen.
    pub fn lane(&self, track_guid: &str) -> &[Item] {
        self.items.get(track_guid).map_or(&[], Vec::as_slice)
    }
}

/// Read the whole project from the live facade.
///
/// One pass, no waveforms. Peaks are the slow ninety percent of opening
/// a real session and none of the layout depends on them, so they stream
/// in afterwards ([`super::waveform`]) against a window that is already
/// standing. A spinner in front of an arrangement we could already draw
/// is a self-inflicted wait.
pub async fn fetch() -> Option<Project> {
    let daw = daw_control::Daw::try_get()?;
    let project = daw.current_project().await.ok()?;

    let tracks = project.tracks().all().await.ok()?;
    let all_items = project.items().all().await.ok()?;

    let mut length = 0.0f64;
    let item_count = all_items.len();
    let mut items: HashMap<String, Vec<Item>> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::with_capacity(item_count);
    let mut midi: HashSet<String> = HashSet::new();
    for item in all_items {
        length = length.max(item.position.as_seconds() + item.length.as_seconds());
        // The title is the active take's name — two calls per item,
        // in-process, once per open.
        if let Ok(Some(handle)) = project.items().by_guid(&item.guid).await {
            let take = handle.active_take();
            if let Ok(name) = take.name().await
                && !name.is_empty()
            {
                names.insert(item.guid.clone(), name);
            }
            // Whether it is MIDI, from the same take the name came
            // from. The call was already being paid for and the answer
            // thrown away — and without it an item drawn from its
            // content has no way to know WHICH content it has.
            if let Ok(info) = take.info().await
                && info.is_midi
            {
                midi.insert(item.guid.clone());
            }
        }
        items.entry(item.track_guid.clone()).or_default().push(item);
    }
    // Sorted, and sorted TOTALLY — position alone is not enough.
    //
    // Items on a lane can overlap, and a lane draws them in order, so
    // whichever comes last is the one you see where they cross. Ordering
    // by position alone leaves ties to be broken by whatever order the
    // facade happened to return, which is not stable between opens: the
    // same project drew a different picture on the second run, with one
    // take covering another that had been on top a moment before. The
    // item's index within its track is REAPER's own answer to "which of
    // these is on top", so that is the tiebreak, with the guid behind it
    // so the order is total no matter what.
    for lane in items.values_mut() {
        lane.sort_by(|a, b| {
            a.position
                .as_seconds()
                .partial_cmp(&b.position.as_seconds())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.index.cmp(&b.index))
                .then_with(|| a.guid.cmp(&b.guid))
        });
    }

    let sections = project
        .regions()
        .all()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| Section {
            id: r.id.unwrap_or_default(),
            start: r.time_range.start_seconds(),
            end: r.time_range.end_seconds(),
            name: r.name.clone(),
            color: r.color.map(|c| format!("#{c:06x}")),
            lane: r.lane.unwrap_or(0),
        })
        .collect();

    // Real markers only. REAPER stores a region as a marker pair and the
    // standalone loader keeps the two apart, but an unnamed marker
    // sitting on a region edge would be ruler noise either way.
    let markers = project
        .markers()
        .all()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|m| !m.name.is_empty())
        .enumerate()
        .map(|(i, m)| Marker {
            at: m.position.seconds().unwrap_or(0.0),
            name: m.name.clone(),
            color: m.color.map(|c| format!("#{c:06x}")),
            idx: m.id.unwrap_or(i as u32 + 1),
            lane: m.lane.unwrap_or(0),
        })
        .collect();

    let bpm = project.transport().get_tempo().await.unwrap_or(120.0);

    Some(Project {
        tracks,
        items,
        sections,
        markers,
        bpm,
        // A minute of empty ruler for a project with nothing in it, so
        // the timeline still has somewhere to put its bar numbers.
        length_secs: length.max(60.0),
        item_count,
        names,
        midi,
    })
}

/// Wait for the backend, then read the project.
///
/// The facade is stood up on a worker thread so the window does not have
/// to wait for it, which means it may not exist when the first render
/// runs. A single attempt would leave the panels empty for the life of
/// the window — this is what makes "open the window first" safe.
pub async fn fetch_when_ready() -> Arc<Project> {
    loop {
        if let Some(project) = fetch().await {
            return Arc::new(project);
        }
        futures_timer::Delay::new(std::time::Duration::from_millis(80)).await;
    }
}
