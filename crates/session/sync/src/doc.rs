//! The session document: where each part of a [`SessionModel`] lives in
//! the Loro doc, and the two directions between them.
//!
//! Schema (version [`SCHEMA`]):
//!
//! ```text
//! meta   : Map   { schema: i64, tempo: String }
//! tracks : Tree  one node per track, folders as parents, sibling order
//!                by fractional index; each node's meta map holds `guid`
//!                and the track's fields
//! items  : Map   guid → Map { track, position, …, "take.source", … }
//! markers: Map   guid → Map { at, name, color }
//! regions: Map   guid → Map { start, end, name, color }
//! chart  : Text  the keyflow chart
//! fx     : Map   fx id → Map { track, index, plugin, name, enabled,
//!                offline, state }  (state: a blob's name in the store)
//! sends  : Map   "<track>#<n>" → Map { track, index, dest, mode, volume,
//!                pan, muted, from, to }
//! envelopes: Map "<track>|<what>" → Map { track, what, mode, visible,
//!                armed, points }  (points: [at, value, shape, tension]…)
//! ```
//!
//! Schema 2 adds `fx`, `sends` and `envelopes`, a track's fixed lanes and
//! groups (`lanes.*`, `group.*` on its node), and an item's fixed lane,
//! loop, stretch markers and MIDI notes (`take.stretch`, `take.notes`,
//! flat number lists). An FX, a send, an envelope is each its own map, so
//! one person bypassing a plugin while another automates a fader both
//! land.
//!
//! Every field is its own map entry, so two people changing different
//! things about the same item — one moves it, one turns it down — both
//! land. The same field changed twice concurrently is last-writer-wins,
//! which for a fader or a position is what anyone would expect.
//!
//! The tempo map is one value on purpose: a tempo map is only meaningful
//! whole, and merging two people's halves of one would be a tempo map
//! neither of them wrote.
//!
//! Writing is a *reconcile*: [`SessionDoc::write`] compares the model
//! with what the doc already holds and emits operations only for what
//! differs. Writing the same model twice records nothing.

use std::collections::{BTreeMap, HashMap, HashSet};

use loro::{
    ExportMode, LoroDoc, LoroEncodeError, LoroError, LoroMap, LoroResult, LoroText, LoroTree,
    LoroValue, TreeID, TreeParentId, UpdateOptions, ValueOrContainer, VersionVector,
};

use crate::model::{
    EnvelopePointState, EnvelopeState, FxState, ItemState, LaneState, MarkerState, NoteState,
    RegionState, SendState, SessionModel, StretchPoint, TakeState, TempoPoint, TrackState,
};

/// The schema version written into `meta.schema`.
pub const SCHEMA: i64 = 2;

const META: &str = "meta";
const TRACKS: &str = "tracks";
const ITEMS: &str = "items";
const MARKERS: &str = "markers";
const REGIONS: &str = "regions";
const CHART: &str = "chart";
const FX: &str = "fx";
const SENDS: &str = "sends";
const ENVELOPES: &str = "envelopes";

/// Origin tag for commits made from the local engine.
pub const ORIGIN_LOCAL: &str = "local";

/// One song's live session as a Loro document.
///
/// Cloning shares the document (Loro docs are handles), which is how the
/// sync driver and the bridge work on one replica.
#[derive(Debug, Clone)]
pub struct SessionDoc {
    doc: LoroDoc,
}

impl Default for SessionDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionDoc {
    /// An empty session.
    #[must_use]
    pub fn new() -> Self {
        Self::from_loro(LoroDoc::new())
    }

    /// Wrap an existing doc — one restored from a `.session` file or
    /// handed over by a sync driver.
    #[must_use]
    pub fn from_loro(doc: LoroDoc) -> Self {
        doc.get_tree(TRACKS).enable_fractional_index(0);
        Self { doc }
    }

    /// A replica with its own peer id, for a second participant.
    #[must_use]
    pub fn fork(&self) -> Self {
        Self::from_loro(self.doc.fork())
    }

    /// The underlying Loro doc (for sync drivers, persistence, history).
    #[must_use]
    pub const fn loro(&self) -> &LoroDoc {
        &self.doc
    }

    /// The whole history, for saving.
    ///
    /// # Errors
    /// When Loro cannot encode the document.
    pub fn snapshot(&self) -> Result<Vec<u8>, LoroEncodeError> {
        self.doc.export(ExportMode::Snapshot)
    }

    /// What this replica has that a peer at `since` does not.
    ///
    /// # Errors
    /// When Loro cannot encode the updates.
    pub fn updates_since(&self, since: &VersionVector) -> Result<Vec<u8>, LoroEncodeError> {
        self.doc.export(ExportMode::updates(since))
    }

    /// This replica's version, to ask a peer for what it is missing.
    #[must_use]
    pub fn version(&self) -> VersionVector {
        self.doc.oplog_vv()
    }

    /// Merge a snapshot or updates from elsewhere.
    ///
    /// # Errors
    /// When the bytes are not a Loro export this doc can import.
    pub fn import(&self, bytes: &[u8], origin: &str) -> Result<(), LoroError> {
        self.doc.import_with(bytes, origin).map(|_| ())
    }

    /// Reconcile the doc with `model`, committing under `origin`.
    ///
    /// # Errors
    /// When a Loro operation fails (a corrupt tree, say).
    pub fn write(&self, model: &SessionModel, origin: &str) -> LoroResult<()> {
        let meta = self.doc.get_map(META);
        put(&meta, "schema", LoroValue::I64(SCHEMA))?;
        put(&meta, "tempo", LoroValue::from(encode_tempo(&model.tempo)))?;
        self.write_tracks(&model.tracks)?;
        write_entities(&self.doc.get_map(ITEMS), &model.items, item_fields)?;
        write_entities(&self.doc.get_map(MARKERS), &model.markers, marker_fields)?;
        write_entities(&self.doc.get_map(REGIONS), &model.regions, region_fields)?;
        let (fx, sends, envelopes) = track_parts(&model.tracks);
        write_entities(&self.doc.get_map(FX), &fx, fx_fields)?;
        write_entities(&self.doc.get_map(SENDS), &sends, send_fields)?;
        write_entities(&self.doc.get_map(ENVELOPES), &envelopes, envelope_fields)?;
        write_text(&self.doc.get_text(CHART), &model.chart)?;
        self.doc.set_next_commit_origin(origin);
        self.doc.commit();
        Ok(())
    }

    /// Whether anything has been written or received yet — a joiner
    /// waits for this before letting its bridge at the doc.
    #[must_use]
    pub fn has_session(&self) -> bool {
        matches!(
            self.doc.get_map(META).get("schema"),
            Some(ValueOrContainer::Value(LoroValue::I64(_)))
        )
    }

    /// A stable position in the chart text (Unicode offset) that stays put
    /// while others type, encoded for presence.
    #[must_use]
    pub fn chart_cursor(&self, pos: usize) -> Option<Vec<u8>> {
        let text = self.doc.get_text(CHART);
        text.get_cursor(pos.min(text.len_unicode()), loro::cursor::Side::Left)
            .map(|c| c.encode())
    }

    /// Where an encoded chart cursor (from [`Self::chart_cursor`]) is now.
    #[must_use]
    pub fn resolve_chart_cursor(&self, bytes: &[u8]) -> Option<usize> {
        let cursor = loro::cursor::Cursor::decode(bytes).ok()?;
        self.doc.get_cursor_pos(&cursor).ok().map(|p| p.current.pos)
    }

    /// The model the doc currently holds.
    #[must_use]
    pub fn read(&self) -> SessionModel {
        let meta = map_value(&self.doc.get_map(META).get_deep_value());
        let tempo = meta
            .get("tempo")
            .and_then(as_string)
            .map(|s| decode_tempo(&s))
            .unwrap_or_default();
        let mut tracks = self.read_tracks();
        attach_parts(
            &mut tracks,
            &read_entities(&self.doc.get_map(FX), read_fx),
            &read_entities(&self.doc.get_map(SENDS), read_send),
            &read_entities(&self.doc.get_map(ENVELOPES), read_envelope),
        );
        SessionModel {
            tracks,
            items: read_entities(&self.doc.get_map(ITEMS), read_item),
            markers: read_entities(&self.doc.get_map(MARKERS), read_marker),
            regions: read_entities(&self.doc.get_map(REGIONS), read_region),
            tempo,
            chart: self.doc.get_text(CHART).to_string(),
        }
    }

    // ── tracks ──────────────────────────────────────────────────────

    /// Each live track node by guid, plus any second node for a guid
    /// already seen.
    ///
    /// A duplicate happens when two replicas each create a node for the
    /// same track before they have synced — two people opening their own
    /// copy of one `.session` and joining. The canonical node is the
    /// smallest `TreeID`, the same answer on every replica, so each one
    /// deletes the same extras and they converge.
    fn track_nodes(tree: &LoroTree) -> (HashMap<String, TreeID>, Vec<TreeID>) {
        let mut all: Vec<(String, TreeID)> = tree
            .get_nodes(false)
            .into_iter()
            .filter_map(|node| {
                let meta = tree.get_meta(node.id).ok()?;
                let guid = map_value(&meta.get_deep_value())
                    .get("guid")
                    .and_then(as_string)?;
                Some((guid, node.id))
            })
            .collect();
        all.sort_by_key(|(_, id)| (id.peer, id.counter));
        let mut nodes = HashMap::new();
        let mut extras = Vec::new();
        for (guid, id) in all {
            if let std::collections::hash_map::Entry::Vacant(slot) = nodes.entry(guid) {
                slot.insert(id);
            } else {
                extras.push(id);
            }
        }
        (nodes, extras)
    }

    fn write_tracks(&self, tracks: &[TrackState]) -> LoroResult<()> {
        let tree = self.doc.get_tree(TRACKS);
        let (mut nodes, extras) = Self::track_nodes(&tree);
        for id in extras {
            if tree.contains(id) && !tree.is_node_deleted(&id)? {
                tree.delete(id)?;
            }
        }
        // How many of each folder's children have been placed so far:
        // the index the next one belongs at.
        let mut placed: HashMap<Option<String>, usize> = HashMap::new();

        for track in tracks {
            // A folder the model does not hold leaves the track at the
            // top level rather than losing it.
            let parent = track
                .parent
                .as_ref()
                .and_then(|guid| nodes.get(guid))
                .map_or(TreeParentId::Root, |id| TreeParentId::Node(*id));
            let slot = placed.entry(track.parent.clone()).or_insert(0);
            let index = *slot;
            *slot = slot.saturating_add(1);

            let id = if let Some(id) = nodes.get(&track.guid).copied() {
                let here = tree.parent(id) == Some(parent)
                    && tree
                        .children(parent)
                        .and_then(|c| c.iter().position(|n| *n == id))
                        == Some(index);
                if !here {
                    tree.mov_to(id, parent, index)?;
                }
                id
            } else {
                let id = tree.create_at(parent, index)?;
                nodes.insert(track.guid.clone(), id);
                id
            };
            sync_fields(&tree.get_meta(id)?, &track_fields(track))?;
        }

        let keep: HashSet<&str> = tracks.iter().map(|t| t.guid.as_str()).collect();
        for (guid, id) in &nodes {
            if !keep.contains(guid.as_str()) && tree.contains(*id) && !tree.is_node_deleted(id)? {
                tree.delete(*id)?;
            }
        }
        Ok(())
    }

    fn read_tracks(&self) -> Vec<TrackState> {
        fn walk(
            tree: &LoroTree,
            canonical: &HashMap<String, TreeID>,
            parent: TreeParentId,
            folder: Option<&str>,
            out: &mut Vec<TrackState>,
        ) {
            for id in tree.children(parent).unwrap_or_default() {
                let Ok(meta) = tree.get_meta(id) else {
                    continue;
                };
                let fields = map_value(&meta.get_deep_value());
                let Some(track) = read_track(&fields, folder) else {
                    continue;
                };
                if canonical.get(&track.guid) != Some(&id) {
                    continue; // a duplicate node; see `track_nodes`
                }
                let guid = track.guid.clone();
                out.push(track);
                walk(tree, canonical, TreeParentId::Node(id), Some(&guid), out);
            }
        }
        let tree = self.doc.get_tree(TRACKS);
        let (canonical, _) = Self::track_nodes(&tree);
        let mut out = Vec::new();
        walk(&tree, &canonical, TreeParentId::Root, None, &mut out);
        out
    }
}

// ── generic entity maps ─────────────────────────────────────────────────

type Fields = Vec<(&'static str, LoroValue)>;

fn write_entities<T>(
    root: &LoroMap,
    entities: &BTreeMap<String, T>,
    fields: fn(&T) -> Fields,
) -> LoroResult<()> {
    for (guid, entity) in entities {
        // Mergeable: two peers creating the same entity concurrently get
        // ONE map, not two containers racing for the key.
        let map = root.ensure_mergeable_map(guid)?;
        sync_fields(&map, &fields(entity))?;
    }
    let stale: Vec<String> = root
        .keys()
        .map(|k| k.to_string())
        .filter(|k| !entities.contains_key(k))
        .collect();
    for key in stale {
        root.delete(&key)?;
    }
    Ok(())
}

fn read_entities<T>(
    root: &LoroMap,
    read: fn(&HashMap<String, LoroValue>) -> T,
) -> BTreeMap<String, T> {
    match root.get_deep_value() {
        LoroValue::Map(entities) => entities
            .iter()
            .filter_map(|(guid, value)| match value {
                LoroValue::Map(_) => Some((guid.clone(), read(&map_value(value)))),
                _ => None,
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// Set each field that differs from what the map holds.
fn sync_fields(map: &LoroMap, fields: &[(&'static str, LoroValue)]) -> LoroResult<()> {
    for (key, value) in fields {
        put(map, key, value.clone())?;
    }
    Ok(())
}

fn put(map: &LoroMap, key: &str, value: LoroValue) -> LoroResult<()> {
    if let Some(ValueOrContainer::Value(current)) = map.get(key)
        && current == value
    {
        return Ok(());
    }
    map.insert(key, value)
}

fn write_text(text: &LoroText, value: &str) -> LoroResult<()> {
    if text.to_string() == value {
        return Ok(());
    }
    text.update(value, UpdateOptions::default())
        .map_err(|e| LoroError::Unknown(format!("chart diff: {e:?}").into_boxed_str()))
}

fn map_value(value: &LoroValue) -> HashMap<String, LoroValue> {
    match value {
        LoroValue::Map(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => HashMap::new(),
    }
}

// ── field codecs ────────────────────────────────────────────────────────

fn color(c: Option<u32>) -> LoroValue {
    c.map_or(LoroValue::Null, |c| LoroValue::I64(i64::from(c)))
}

fn opt_str(s: Option<&str>) -> LoroValue {
    s.map_or(LoroValue::Null, LoroValue::from)
}

fn as_f64(v: &LoroValue) -> Option<f64> {
    match v {
        LoroValue::Double(d) => Some(*d),
        LoroValue::I64(i) => i32::try_from(*i).ok().map(f64::from),
        _ => None,
    }
}

const fn as_bool(v: &LoroValue) -> Option<bool> {
    match v {
        LoroValue::Bool(b) => Some(*b),
        _ => None,
    }
}

fn as_string(v: &LoroValue) -> Option<String> {
    match v {
        LoroValue::String(s) => Some(s.to_string()),
        _ => None,
    }
}

fn as_color(v: &LoroValue) -> Option<u32> {
    match v {
        LoroValue::I64(i) => u32::try_from(*i).ok(),
        _ => None,
    }
}

struct Get<'a>(&'a HashMap<String, LoroValue>);

impl Get<'_> {
    fn f64(&self, key: &str, default: f64) -> f64 {
        self.0.get(key).and_then(as_f64).unwrap_or(default)
    }
    fn bool(&self, key: &str, default: bool) -> bool {
        self.0.get(key).and_then(as_bool).unwrap_or(default)
    }
    fn string(&self, key: &str) -> String {
        self.0.get(key).and_then(as_string).unwrap_or_default()
    }
    fn opt_string(&self, key: &str) -> Option<String> {
        self.0.get(key).and_then(as_string)
    }
    fn color(&self, key: &str) -> Option<u32> {
        self.0.get(key).and_then(as_color)
    }
    fn i64(&self, key: &str, default: i64) -> i64 {
        match self.0.get(key) {
            Some(LoroValue::I64(i)) => *i,
            _ => default,
        }
    }
}

fn track_fields(t: &TrackState) -> Fields {
    vec![
        ("guid", LoroValue::from(t.guid.as_str())),
        ("name", LoroValue::from(t.name.as_str())),
        ("color", color(t.color)),
        ("volume", LoroValue::Double(t.volume)),
        ("pan", LoroValue::Double(t.pan)),
        ("muted", LoroValue::Bool(t.muted)),
        ("soloed", LoroValue::Bool(t.soloed)),
        ("phase_inverted", LoroValue::Bool(t.phase_inverted)),
        ("parent_send", LoroValue::Bool(t.parent_send)),
        ("visible_in_tcp", LoroValue::Bool(t.visible_in_tcp)),
        ("visible_in_mixer", LoroValue::Bool(t.visible_in_mixer)),
        ("lanes.count", LoroValue::I64(i64::from(t.lanes.count))),
        (
            "lanes.names",
            LoroValue::from(
                t.lanes
                    .names
                    .iter()
                    .map(|n| LoroValue::from(n.as_str()))
                    .collect::<Vec<_>>(),
            ),
        ),
        // The bits as they are: lane 63 playing is a negative number.
        (
            "lanes.play",
            LoroValue::I64(i64::from_ne_bytes(t.lanes.play_mask.to_ne_bytes())),
        ),
        ("grouping", grouping_value(&t.grouping)),
    ]
}

/// A track's group masks, as `family → hex`.
fn grouping_value(grouping: &BTreeMap<String, u128>) -> LoroValue {
    let map: HashMap<String, LoroValue> = grouping
        .iter()
        .filter(|(_, mask)| **mask != 0)
        .map(|(family, mask)| (family.clone(), LoroValue::from(format!("{mask:x}"))))
        .collect();
    LoroValue::from(map)
}

fn read_grouping(v: Option<&LoroValue>) -> BTreeMap<String, u128> {
    match v {
        Some(LoroValue::Map(map)) => map
            .iter()
            .filter_map(|(family, hex)| {
                let mask = u128::from_str_radix(&as_string(hex)?, 16).ok()?;
                Some((family.clone(), mask))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn strings(v: Option<&LoroValue>) -> Vec<String> {
    match v {
        Some(LoroValue::List(list)) => list.iter().filter_map(as_string).collect(),
        _ => Vec::new(),
    }
}

/// A flat list of numbers.
fn numbers(v: Option<&LoroValue>) -> Vec<f64> {
    match v {
        Some(LoroValue::List(list)) => list.iter().filter_map(as_f64).collect(),
        _ => Vec::new(),
    }
}

fn number_list(values: impl IntoIterator<Item = f64>) -> LoroValue {
    LoroValue::from(
        values
            .into_iter()
            .map(LoroValue::Double)
            .collect::<Vec<_>>(),
    )
}

fn read_track(fields: &HashMap<String, LoroValue>, folder: Option<&str>) -> Option<TrackState> {
    let g = Get(fields);
    let guid = g.opt_string("guid")?;
    Some(TrackState {
        guid,
        parent: folder.map(str::to_string),
        name: g.string("name"),
        color: g.color("color"),
        volume: g.f64("volume", 1.0),
        pan: g.f64("pan", 0.0),
        muted: g.bool("muted", false),
        soloed: g.bool("soloed", false),
        phase_inverted: g.bool("phase_inverted", false),
        parent_send: g.bool("parent_send", true),
        visible_in_tcp: g.bool("visible_in_tcp", true),
        visible_in_mixer: g.bool("visible_in_mixer", true),
        lanes: LaneState {
            count: u32::try_from(g.i64("lanes.count", 0)).unwrap_or(0),
            names: strings(fields.get("lanes.names")),
            play_mask: u64::from_ne_bytes(g.i64("lanes.play", 0).to_ne_bytes()),
        },
        grouping: read_grouping(fields.get("grouping")),
        fx: Vec::new(),
        sends: Vec::new(),
        envelopes: BTreeMap::new(),
    })
}

fn item_fields(i: &ItemState) -> Fields {
    vec![
        ("track", LoroValue::from(i.track.as_str())),
        ("position", LoroValue::Double(i.position)),
        ("length", LoroValue::Double(i.length)),
        ("snap_offset", LoroValue::Double(i.snap_offset)),
        ("muted", LoroValue::Bool(i.muted)),
        ("locked", LoroValue::Bool(i.locked)),
        ("volume", LoroValue::Double(i.volume)),
        ("fade_in", LoroValue::Double(i.fade_in)),
        ("fade_out", LoroValue::Double(i.fade_out)),
        ("fade_in_shape", LoroValue::from(i.fade_in_shape.as_str())),
        ("fade_out_shape", LoroValue::from(i.fade_out_shape.as_str())),
        ("label", LoroValue::from(i.label.as_str())),
        ("color", color(i.color)),
        ("take.name", LoroValue::from(i.take.name.as_str())),
        ("take.source", opt_str(i.take.source.as_deref())),
        ("take.start_offset", LoroValue::Double(i.take.start_offset)),
        ("take.playrate", LoroValue::Double(i.take.playrate)),
        ("take.midi", LoroValue::Bool(i.take.midi)),
        (
            "fixed_lane",
            i.fixed_lane
                .map_or(LoroValue::Null, |l| LoroValue::I64(i64::from(l))),
        ),
        ("loop", LoroValue::Bool(i.loop_source)),
        (
            "take.stretch",
            number_list(
                i.take
                    .stretch
                    .iter()
                    .flat_map(|m| [m.at, m.source, m.slope]),
            ),
        ),
        (
            "take.notes",
            number_list(i.take.notes.iter().flat_map(|n| {
                [
                    n.start,
                    n.length,
                    f64::from(n.pitch),
                    f64::from(n.velocity),
                    f64::from(n.channel),
                ]
            })),
        ),
    ]
}

fn read_item(fields: &HashMap<String, LoroValue>) -> ItemState {
    let g = Get(fields);
    ItemState {
        track: g.string("track"),
        position: g.f64("position", 0.0),
        length: g.f64("length", 0.0),
        snap_offset: g.f64("snap_offset", 0.0),
        muted: g.bool("muted", false),
        locked: g.bool("locked", false),
        volume: g.f64("volume", 1.0),
        fade_in: g.f64("fade_in", 0.0),
        fade_out: g.f64("fade_out", 0.0),
        fade_in_shape: g.string("fade_in_shape"),
        fade_out_shape: g.string("fade_out_shape"),
        label: g.string("label"),
        color: g.color("color"),
        take: TakeState {
            name: g.string("take.name"),
            source: g.opt_string("take.source"),
            start_offset: g.f64("take.start_offset", 0.0),
            playrate: g.f64("take.playrate", 1.0),
            midi: g.bool("take.midi", false),
            stretch: numbers(fields.get("take.stretch"))
                .chunks_exact(3)
                .filter_map(|m| match *m {
                    [at, source, slope] => Some(StretchPoint { at, source, slope }),
                    _ => None,
                })
                .collect(),
            notes: numbers(fields.get("take.notes"))
                .chunks_exact(5)
                .filter_map(|n| match *n {
                    [start, length, pitch, velocity, channel] => Some(NoteState {
                        start,
                        length,
                        pitch: byte(pitch),
                        velocity: byte(velocity),
                        channel: byte(channel),
                    }),
                    _ => None,
                })
                .collect(),
        },
        fixed_lane: match fields.get("fixed_lane") {
            Some(LoroValue::I64(l)) => u32::try_from(*l).ok(),
            _ => None,
        },
        loop_source: g.bool("loop", false),
    }
}

/// A MIDI byte stored as a number.
fn byte(v: f64) -> u8 {
    num_traits::ToPrimitive::to_u8(&v.round().clamp(0.0, 127.0)).unwrap_or(0)
}

// ── a track's parts: its FX, sends, envelopes ──────────────────────────

/// One FX as the doc keeps it, with the track it is on and its place.
#[derive(Debug, Clone, PartialEq)]
struct FxEntry {
    track: String,
    index: usize,
    fx: FxState,
}

#[derive(Debug, Clone, PartialEq)]
struct SendEntry {
    track: String,
    index: usize,
    send: SendState,
}

#[derive(Debug, Clone, PartialEq)]
struct EnvelopeEntry {
    track: String,
    what: String,
    envelope: EnvelopeState,
}

type Parts = (
    BTreeMap<String, FxEntry>,
    BTreeMap<String, SendEntry>,
    BTreeMap<String, EnvelopeEntry>,
);

/// Every track's FX, sends and envelopes, each keyed as the doc keeps it.
fn track_parts(tracks: &[TrackState]) -> Parts {
    let mut fx = BTreeMap::new();
    let mut sends = BTreeMap::new();
    let mut envelopes = BTreeMap::new();
    for t in tracks {
        for (index, f) in t.fx.iter().enumerate() {
            fx.insert(
                f.guid.clone(),
                FxEntry {
                    track: t.guid.clone(),
                    index,
                    fx: f.clone(),
                },
            );
        }
        for (index, send) in t.sends.iter().enumerate() {
            sends.insert(
                format!("{}#{index}", t.guid),
                SendEntry {
                    track: t.guid.clone(),
                    index,
                    send: send.clone(),
                },
            );
        }
        for (what, envelope) in &t.envelopes {
            envelopes.insert(
                format!("{}|{what}", t.guid),
                EnvelopeEntry {
                    track: t.guid.clone(),
                    what: what.clone(),
                    envelope: envelope.clone(),
                },
            );
        }
    }
    (fx, sends, envelopes)
}

/// Each part back onto its track, in order.
fn attach_parts(
    tracks: &mut [TrackState],
    fx: &BTreeMap<String, FxEntry>,
    sends: &BTreeMap<String, SendEntry>,
    envelopes: &BTreeMap<String, EnvelopeEntry>,
) {
    let by_track: HashMap<String, usize> = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.guid.clone(), i))
        .collect();
    let mut chains: HashMap<usize, Vec<FxEntry>> = HashMap::new();
    for (id, entry) in fx {
        if let Some(&i) = by_track.get(entry.track.as_str()) {
            // The FX's id is the key it is kept under.
            let mut entry = entry.clone();
            entry.fx.guid.clone_from(id);
            chains.entry(i).or_default().push(entry);
        }
    }
    let mut routes: HashMap<usize, Vec<&SendEntry>> = HashMap::new();
    for entry in sends.values() {
        if let Some(&i) = by_track.get(entry.track.as_str()) {
            routes.entry(i).or_default().push(entry);
        }
    }
    for (i, mut chain) in chains {
        // By place, then id: two FX given one place at once (two people
        // adding at the same time) keep one order on every replica.
        chain.sort_by(|a, b| a.index.cmp(&b.index).then(a.fx.guid.cmp(&b.fx.guid)));
        if let Some(t) = tracks.get_mut(i) {
            t.fx = chain.into_iter().map(|e| e.fx).collect();
        }
    }
    for (i, mut route) in routes {
        route.sort_by_key(|e| e.index);
        if let Some(t) = tracks.get_mut(i) {
            t.sends = route.into_iter().map(|e| e.send.clone()).collect();
        }
    }
    for entry in envelopes.values() {
        if let Some(t) = by_track
            .get(entry.track.as_str())
            .and_then(|&i| tracks.get_mut(i))
        {
            t.envelopes
                .insert(entry.what.clone(), entry.envelope.clone());
        }
    }
}

fn fx_fields(e: &FxEntry) -> Fields {
    vec![
        ("track", LoroValue::from(e.track.as_str())),
        ("index", LoroValue::I64(i64::try_from(e.index).unwrap_or(0))),
        ("plugin", LoroValue::from(e.fx.plugin.as_str())),
        ("name", LoroValue::from(e.fx.name.as_str())),
        ("enabled", LoroValue::Bool(e.fx.enabled)),
        ("offline", LoroValue::Bool(e.fx.offline)),
        ("state", opt_str(e.fx.state.as_deref())),
    ]
}

fn read_fx(fields: &HashMap<String, LoroValue>) -> FxEntry {
    let g = Get(fields);
    FxEntry {
        track: g.string("track"),
        index: usize::try_from(g.i64("index", 0)).unwrap_or(0),
        fx: FxState {
            guid: String::new(),
            plugin: g.string("plugin"),
            name: g.string("name"),
            enabled: g.bool("enabled", true),
            offline: g.bool("offline", false),
            state: g.opt_string("state"),
        },
    }
}

fn send_fields(e: &SendEntry) -> Fields {
    vec![
        ("track", LoroValue::from(e.track.as_str())),
        ("index", LoroValue::I64(i64::try_from(e.index).unwrap_or(0))),
        ("dest", LoroValue::from(e.send.dest.as_str())),
        ("mode", LoroValue::from(e.send.mode.as_str())),
        ("volume", LoroValue::Double(e.send.volume)),
        ("pan", LoroValue::Double(e.send.pan)),
        ("muted", LoroValue::Bool(e.send.muted)),
        ("from", LoroValue::I64(i64::from(e.send.from))),
        ("to", LoroValue::I64(i64::from(e.send.to))),
    ]
}

fn read_send(fields: &HashMap<String, LoroValue>) -> SendEntry {
    let g = Get(fields);
    SendEntry {
        track: g.string("track"),
        index: usize::try_from(g.i64("index", 0)).unwrap_or(0),
        send: SendState {
            dest: g.string("dest"),
            mode: g.string("mode"),
            volume: g.f64("volume", 1.0),
            pan: g.f64("pan", 0.0),
            muted: g.bool("muted", false),
            from: u32::try_from(g.i64("from", 0)).unwrap_or(0),
            to: u32::try_from(g.i64("to", 0)).unwrap_or(0),
        },
    }
}

fn envelope_fields(e: &EnvelopeEntry) -> Fields {
    vec![
        ("track", LoroValue::from(e.track.as_str())),
        ("what", LoroValue::from(e.what.as_str())),
        ("mode", LoroValue::from(e.envelope.mode.as_str())),
        ("visible", LoroValue::Bool(e.envelope.visible)),
        ("armed", LoroValue::Bool(e.envelope.armed)),
        (
            "points",
            number_list(
                e.envelope
                    .points
                    .iter()
                    .flat_map(|p| [p.at, p.value, f64::from(p.shape), p.tension]),
            ),
        ),
    ]
}

fn read_envelope(fields: &HashMap<String, LoroValue>) -> EnvelopeEntry {
    let g = Get(fields);
    EnvelopeEntry {
        track: g.string("track"),
        what: g.string("what"),
        envelope: EnvelopeState {
            mode: g.string("mode"),
            visible: g.bool("visible", false),
            armed: g.bool("armed", false),
            points: numbers(fields.get("points"))
                .chunks_exact(4)
                .filter_map(|p| match *p {
                    [at, value, shape, tension] => Some(EnvelopePointState {
                        at,
                        value,
                        shape: num_traits::ToPrimitive::to_u8(&shape).unwrap_or(0),
                        tension,
                    }),
                    _ => None,
                })
                .collect(),
        },
    }
}

fn marker_fields(m: &MarkerState) -> Fields {
    vec![
        ("at", LoroValue::Double(m.at)),
        ("name", LoroValue::from(m.name.as_str())),
        ("color", color(m.color)),
    ]
}

fn read_marker(fields: &HashMap<String, LoroValue>) -> MarkerState {
    let g = Get(fields);
    MarkerState {
        at: g.f64("at", 0.0),
        name: g.string("name"),
        color: g.color("color"),
    }
}

fn region_fields(r: &RegionState) -> Fields {
    vec![
        ("start", LoroValue::Double(r.start)),
        ("end", LoroValue::Double(r.end)),
        ("name", LoroValue::from(r.name.as_str())),
        ("color", color(r.color)),
    ]
}

fn read_region(fields: &HashMap<String, LoroValue>) -> RegionState {
    let g = Get(fields);
    RegionState {
        start: g.f64("start", 0.0),
        end: g.f64("end", 0.0),
        name: g.string("name"),
        color: g.color("color"),
    }
}

/// `at:bpm:beats/unit` per point, space-separated. `f64`'s `Display` is
/// the shortest string that reads back to the same number, so a tempo
/// map survives the round trip exactly.
fn encode_tempo(points: &[TempoPoint]) -> String {
    points
        .iter()
        .map(|p| format!("{}:{}:{}/{}", p.at, p.bpm, p.beats_per_bar, p.beat_unit))
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_tempo(s: &str) -> Vec<TempoPoint> {
    s.split_whitespace()
        .filter_map(|point| {
            let mut parts = point.split(':');
            let at = parts.next()?.parse().ok()?;
            let bpm = parts.next()?.parse().ok()?;
            let (beats, unit) = parts.next()?.split_once('/')?;
            Some(TempoPoint {
                at,
                bpm,
                beats_per_bar: beats.parse().ok()?,
                beat_unit: unit.parse().ok()?,
            })
        })
        .collect()
}
