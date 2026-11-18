//! The session as an Architect document (feature `architect`).
//!
//! An Architect document is a Loro document of a `crdt::DocKind` ([`kind`]:
//! its id and schema stamp in the `meta` map); everything else
//! (the file, sync to the org, peer-to-peer links, presence, undo) is
//! Architect's, over that `LoroDoc`. This module is the session's shape
//! inside it:
//!
//! ```text
//! meta       : Map  { kind: "fts.session", schema: 1, name }
//! tempo      : Map  { map: <the whole tempo map, one value> }
//! rulers     : Map  "<index>" → name
//! markers    : Map  id → Map { at, name, color, ruler }
//! regions    : Map  id → Map { start, end, name, color, ruler }
//! lanes      : Map  id → Map { owner, kind, name, order, parent }
//! items      : Map  id → Map { owner, lane, kind, at, end, payload }
//!                    payload: a Map container when the payload is a map
//!                    (its keys merge one by one), else a value
//! extensions : Map  extension id → its data version
//! ext:<id>   : Map  "r:<name>" → a register's value,
//!                   "m:<name>" / "t:<name>" → true (a map / text entry)
//! ext:<id>:m:<name> : Map   a map entry's keys
//! ext:<id>:t:<name> : Text  a text entry
//!                    (ids and names percent-escaped: a root container's
//!                    name may not hold `/`, and `:` separates)
//! ```
//!
//! Times are `["s", seconds]` or `["qn", quarter notes]`: one value, so a
//! move that changes the time base cannot half-merge with another.
//!
//! **Why map and text entries are root containers.** Two peers creating
//! the same child container concurrently (the first cue of a new list, on
//! two machines) each make their own, and one wins whole: the other's
//! keys are lost. Root containers are named, not created, so two peers'
//! `ext:ignition:m:cues` are one map and both cues land.
//!
//! **What this does not do**: open, save or sync the document (that is
//! `crdt::DocFile`, `OrgDocs`, `SyncedDoc` over [`SessionDoc::doc`]), or
//! undo (`crdt::History` over the same doc). `tests/sync_e2e.rs` syncs two
//! peers through Architect's real path. See the design, "Architect".

use std::collections::BTreeMap;

use loro::{LoroDoc, LoroError, LoroMap, LoroText, LoroValue, UpdateOptions, ValueOrContainer};

use crate::ext::{Entry, ExtensionData};
use crate::id::{ExtensionId, ItemId, LaneId, MarkId};
use crate::session::Session;
use crate::time::TimeSignature;
use crate::timeline::{Item, Lane, Marker, Region, TempoMap, Time};
use crate::transport::Transport;
use crate::value::Value;

/// The kind id.
pub const KIND: &str = "fts.session";
/// The schema this build writes.
pub const SCHEMA: u32 = 1;

/// The session as an Architect document kind: register it in the app's
/// `crdt::DocKinds`, so `DocFile` and the org driver open, stamp and
/// migrate sessions like any other document. A `.session` file is one.
#[must_use]
pub fn kind() -> crdt::DocKind {
    crdt::DocKind::new(KIND, "Session")
        .extension("session")
        .schema(SCHEMA, Vec::new())
}

const META: &str = "meta";
const TEMPO: &str = "tempo";
const RULERS: &str = "rulers";
const MARKERS: &str = "markers";
const REGIONS: &str = "regions";
const LANES: &str = "lanes";
const ITEMS: &str = "items";
const EXTENSIONS: &str = "extensions";
/// Where the session's presence (its transport, and the clock its peers
/// share) is hosted: `host`, the node serving it (session#192). The same
/// root and key as Architect's `crdt::presence::presence_host`, which a
/// node's relay reads to route an extension's presence there.
const TRANSPORT: &str = "presence";

/// Writing the document failed.
#[derive(Debug)]
pub struct DocError(pub LoroError);

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session document: {}", self.0)
    }
}

impl std::error::Error for DocError {}

impl From<LoroError> for DocError {
    fn from(e: LoroError) -> Self {
        Self(e)
    }
}

pub type DocResult<T> = Result<T, DocError>;

/// A session's Loro document.
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
    /// A new document, stamped as [`kind`] at this build's schema.
    #[must_use]
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        kind().stamp(&doc);
        Self { doc }
    }

    /// A document that already exists (opened from a `DocFile`, joined
    /// through a link). Its kind is not checked here: `crdt::DocKinds`
    /// does that on open.
    #[must_use]
    pub const fn from_doc(doc: LoroDoc) -> Self {
        Self { doc }
    }

    /// The Loro document, for Architect's file, sync and undo.
    #[must_use]
    pub const fn doc(&self) -> &LoroDoc {
        &self.doc
    }

    /// The kind the document says it is (`crdt::kinds::kind_of`).
    #[must_use]
    pub fn kind(&self) -> Option<String> {
        crdt::kinds::kind_of(&self.doc)
    }

    /// Writes `session` into the document, recording only what differs
    /// (Loro skips a write of the value already there), and commits. One
    /// call is one transaction; wrap it in `crdt::History::transact` to
    /// make it one named undo step.
    ///
    /// # Errors
    ///
    /// Loro refused a write (a key holding a container of another type).
    pub fn write(&self, session: &Session) -> DocResult<()> {
        self.doc
            .get_map(META)
            .insert("name", session.name.as_str())?;
        self.doc
            .get_map(TEMPO)
            .insert("map", tempo_to_loro(&session.tempo))?;
        write_bar_one(&self.doc.get_map(TEMPO), session.bar_one)?;
        let rulers = self.doc.get_map(RULERS);
        let wanted: BTreeMap<String, &String> = session
            .rulers
            .iter()
            .map(|(i, name)| (i.to_string(), name))
            .collect();
        for (i, name) in &wanted {
            rulers.insert(i, name.as_str())?;
        }
        delete_absent(&rulers, |k| wanted.contains_key(k))?;
        write_records(&self.doc.get_map(MARKERS), &session.markers, write_marker)?;
        write_records(&self.doc.get_map(REGIONS), &session.regions, write_region)?;
        write_records(&self.doc.get_map(LANES), &session.lanes, write_lane)?;
        write_records(&self.doc.get_map(ITEMS), &session.items, write_item)?;
        let extensions = self.doc.get_map(EXTENSIONS);
        let ids: Vec<&ExtensionId> = session.extension_ids().collect();
        for id in &ids {
            if let Some(data) = session.ext(id) {
                self.write_ext(id, data)?;
            }
        }
        let gone: Vec<String> = extensions
            .keys()
            .map(|k| k.to_string())
            .filter(|k| !ids.iter().any(|id| id.as_str() == k))
            .collect();
        for id in gone {
            self.remove_ext(&ExtensionId(id))?;
        }
        self.doc.commit();
        Ok(())
    }

    /// The session the document holds now.
    #[must_use]
    pub fn read(&self) -> Session {
        let mut session = Session::new(
            value_of(&self.doc.get_map(META), "name")
                .and_then(|v| v.into_string().ok())
                .map(|s| s.to_string())
                .unwrap_or_default(),
        );
        if let Some(tempo) =
            value_of(&self.doc.get_map(TEMPO), "map").and_then(|v| tempo_from_loro(&v))
        {
            session.tempo = tempo;
        }
        session.bar_one = self.bar_one();
        self.doc.get_map(RULERS).for_each(|k, v| {
            if let (Ok(i), ValueOrContainer::Value(LoroValue::String(name))) = (k.parse::<u32>(), v)
            {
                session.rulers.insert(i, name.to_string());
            }
        });
        session.markers = read_records(&self.doc.get_map(MARKERS), read_marker);
        session.regions = read_records(&self.doc.get_map(REGIONS), read_region);
        session.lanes = read_records(&self.doc.get_map(LANES), read_lane);
        session.items = read_records(&self.doc.get_map(ITEMS), read_item);
        for id in self.doc.get_map(EXTENSIONS).keys() {
            let id = ExtensionId(id.to_string());
            let data = self.read_ext(&id);
            session.set_ext(id, data);
        }
        session
    }

    // ── one record at a time ────────────────────────────────────────────
    //
    // What an edit does (one cue moved, one region renamed), as opposed to
    // `write`'s whole-session reconcile. Each commits.

    /// Every lane.
    #[must_use]
    pub fn lanes(&self) -> BTreeMap<LaneId, Lane> {
        read_records(&self.doc.get_map(LANES), read_lane)
    }

    /// Every marker.
    #[must_use]
    pub fn markers(&self) -> BTreeMap<MarkId, Marker> {
        read_records(&self.doc.get_map(MARKERS), read_marker)
    }

    /// Every region.
    #[must_use]
    pub fn regions(&self) -> BTreeMap<MarkId, Region> {
        read_records(&self.doc.get_map(REGIONS), read_region)
    }

    /// Every item.
    #[must_use]
    pub fn items(&self) -> BTreeMap<ItemId, Item> {
        read_records(&self.doc.get_map(ITEMS), read_item)
    }

    /// The tempo map (the default one, before anyone set it).
    #[must_use]
    pub fn tempo(&self) -> TempoMap {
        value_of(&self.doc.get_map(TEMPO), "map")
            .and_then(|v| tempo_from_loro(&v))
            .unwrap_or_default()
    }

    /// The lane `id`.
    #[must_use]
    pub fn lane(&self, id: &LaneId) -> Option<Lane> {
        get_record(&self.doc.get_map(LANES), id.as_str(), read_lane)
    }

    /// Puts lane `id` (creating or updating it, field by field).
    ///
    /// # Errors
    ///
    /// Loro refused a write.
    pub fn put_lane(&self, id: &LaneId, lane: &Lane) -> DocResult<()> {
        self.put(LANES, id.as_str(), lane, write_lane)
    }

    /// Removes lane `id` (its items stay; the caller decides).
    ///
    /// # Errors
    ///
    /// Loro refused the delete.
    pub fn remove_lane(&self, id: &LaneId) -> DocResult<()> {
        self.remove(LANES, id.as_str())
    }

    /// The item `id`.
    #[must_use]
    pub fn item(&self, id: &ItemId) -> Option<Item> {
        get_record(&self.doc.get_map(ITEMS), id.as_str(), read_item)
    }

    /// Puts item `id` (creating or updating it; a map payload key by key).
    ///
    /// # Errors
    ///
    /// Loro refused a write.
    pub fn put_item(&self, id: &ItemId, item: &Item) -> DocResult<()> {
        self.put(ITEMS, id.as_str(), item, write_item)
    }

    /// Removes item `id`.
    ///
    /// # Errors
    ///
    /// Loro refused the delete.
    pub fn remove_item(&self, id: &ItemId) -> DocResult<()> {
        self.remove(ITEMS, id.as_str())
    }

    /// Puts marker `id`.
    ///
    /// # Errors
    ///
    /// Loro refused a write.
    pub fn put_marker(&self, id: &MarkId, marker: &Marker) -> DocResult<()> {
        self.put(MARKERS, id.as_str(), marker, write_marker)
    }

    /// Removes marker `id`.
    ///
    /// # Errors
    ///
    /// Loro refused the delete.
    pub fn remove_marker(&self, id: &MarkId) -> DocResult<()> {
        self.remove(MARKERS, id.as_str())
    }

    /// Puts region `id`.
    ///
    /// # Errors
    ///
    /// Loro refused a write.
    pub fn put_region(&self, id: &MarkId, region: &Region) -> DocResult<()> {
        self.put(REGIONS, id.as_str(), region, write_region)
    }

    /// Removes region `id`.
    ///
    /// # Errors
    ///
    /// Loro refused the delete.
    pub fn remove_region(&self, id: &MarkId) -> DocResult<()> {
        self.remove(REGIONS, id.as_str())
    }

    /// Sets the whole tempo map (one value: last writer wins).
    ///
    /// # Errors
    ///
    /// Loro refused the write.
    pub fn set_tempo(&self, tempo: &TempoMap) -> DocResult<()> {
        self.doc
            .get_map(TEMPO)
            .insert("map", tempo_to_loro(tempo))?;
        self.doc.commit();
        Ok(())
    }

    /// Where the song's bar 1 is ([`Session::bar_one`]); `None`: the
    /// session's start.
    #[must_use]
    pub fn bar_one(&self) -> Option<Time> {
        value_of(&self.doc.get_map(TEMPO), "bar_one").and_then(|v| time_from_loro(&v))
    }

    /// Sets where the song's bar 1 is (`None`: the session's start). One
    /// value, last writer wins, with the tempo map it belongs to.
    ///
    /// # Errors
    ///
    /// Loro refused the write.
    pub fn set_bar_one(&self, at: Option<Time>) -> DocResult<()> {
        write_bar_one(&self.doc.get_map(TEMPO), at)?;
        self.doc.commit();
        Ok(())
    }

    /// The node that hosts the session's transport: its presence and the
    /// clock every peer of the session shares (session#192), as the
    /// leader wrote it (an Architect node's iroh endpoint id). A follower
    /// reads it from the synced document and follows that node, through
    /// the org, with no address given. `None`: no leader has said.
    #[must_use]
    pub fn transport_host(&self) -> Option<String> {
        value_of(&self.doc.get_map(TRANSPORT), "host")
            .and_then(|v| v.into_string().ok())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Says which node hosts the session's transport (`None`: none now).
    /// The leader writes it when it starts leading; last writer wins.
    ///
    /// # Errors
    ///
    /// Loro refused the write.
    pub fn set_transport_host(&self, host: Option<&str>) -> DocResult<()> {
        let map = self.doc.get_map(TRANSPORT);
        match host {
            Some(host) => map.insert("host", host)?,
            None if map.get("host").is_some() => map.delete("host")?,
            None => return Ok(()),
        }
        self.doc.commit();
        Ok(())
    }

    fn put<R>(
        &self,
        root: &str,
        id: &str,
        record: &R,
        write: fn(&LoroMap, &R) -> DocResult<()>,
    ) -> DocResult<()> {
        let root = self.doc.get_map(root);
        write(&record_node(&root, id)?, record)?;
        self.doc.commit();
        Ok(())
    }

    fn remove(&self, root: &str, id: &str) -> DocResult<()> {
        let root = self.doc.get_map(root);
        if root.get(id).is_some() {
            root.delete(id)?;
            self.doc.commit();
        }
        Ok(())
    }

    /// Extension `id`'s namespace, to write in place: what an extension
    /// does for each edit (one cue added, one key set, a few characters
    /// typed), so concurrent edits merge as finely as its entries allow.
    ///
    /// # Errors
    ///
    /// Loro refused registering the extension.
    pub fn ext<'a>(&'a self, id: &'a ExtensionId) -> DocResult<ExtWriter<'a>> {
        let extensions = self.doc.get_map(EXTENSIONS);
        if extensions.get(id.as_str()).is_none() {
            extensions.insert(id.as_str(), 0_i64)?;
        }
        Ok(ExtWriter { doc: &self.doc, id })
    }

    /// What extension `id` keeps in the document.
    #[must_use]
    pub fn read_ext(&self, id: &ExtensionId) -> ExtensionData {
        let version = value_of(&self.doc.get_map(EXTENSIONS), id.as_str())
            .and_then(|v| v.into_i64().ok())
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let mut data = ExtensionData {
            version,
            entries: BTreeMap::new(),
        };
        let ns = self.doc.get_map(ns_name(id).as_str());
        ns.for_each(|key, v| {
            let ValueOrContainer::Value(v) = v else {
                return;
            };
            if let Some(name) = key.strip_prefix("r:") {
                data.entries
                    .insert(name.to_owned(), Entry::Register(value_from_loro(&v)));
            } else if let Some(name) = key.strip_prefix("m:") {
                let mut map = BTreeMap::new();
                self.doc
                    .get_map(map_name(id, name).as_str())
                    .for_each(|k, v| {
                        if let ValueOrContainer::Value(v) = v {
                            map.insert(k.to_owned(), value_from_loro(&v));
                        }
                    });
                data.entries.insert(name.to_owned(), Entry::Map(map));
            } else if let Some(name) = key.strip_prefix("t:") {
                let text = self.doc.get_text(text_name(id, name)).to_string();
                data.entries.insert(name.to_owned(), Entry::Text(text));
            }
        });
        data
    }

    fn write_ext(&self, id: &ExtensionId, data: &ExtensionData) -> DocResult<()> {
        let writer = self.ext(id)?;
        self.doc
            .get_map(EXTENSIONS)
            .insert(id.as_str(), i64::from(data.version))?;
        for (name, entry) in &data.entries {
            match entry {
                Entry::Register(v) => writer.set(name, v)?,
                Entry::Map(map) => {
                    let target = writer.map(name)?;
                    for (k, v) in map {
                        target.insert(k, value_to_loro(v))?;
                    }
                    delete_absent(&target, |k| map.contains_key(k))?;
                }
                Entry::Text(text) => writer.set_text(name, text)?,
            }
        }
        let ns = self.doc.get_map(ns_name(id).as_str());
        let stale: Vec<String> = ns
            .keys()
            .map(|k| k.to_string())
            .filter(|k| {
                k.get(2..)
                    .is_none_or(|name| !data.entries.contains_key(name))
            })
            .collect();
        for key in stale {
            if let Some(name) = key.get(2..) {
                writer.remove(name)?;
            }
        }
        Ok(())
    }

    /// Removes everything extension `id` keeps in the document. (Its lanes
    /// and items go with [`Session::remove_extension`] and a write.)
    fn remove_ext(&self, id: &ExtensionId) -> DocResult<()> {
        let writer = ExtWriter { doc: &self.doc, id };
        let names: Vec<String> = self
            .doc
            .get_map(ns_name(id).as_str())
            .keys()
            .filter_map(|k| k.get(2..).map(str::to_owned))
            .collect();
        for name in names {
            writer.remove(&name)?;
        }
        self.doc.get_map(EXTENSIONS).delete(id.as_str())?;
        Ok(())
    }
}

/// One extension's namespace in the document, written in place.
#[derive(Debug, Clone, Copy)]
pub struct ExtWriter<'a> {
    doc: &'a LoroDoc,
    id: &'a ExtensionId,
}

impl ExtWriter<'_> {
    /// Sets register `name` (last writer wins for the whole value).
    ///
    /// # Errors
    ///
    /// Loro refused the write.
    pub fn set(&self, name: &str, value: &Value) -> DocResult<()> {
        let ns = self.doc.get_map(ns_name(self.id).as_str());
        ns.insert(&format!("r:{name}"), value_to_loro(value))?;
        Ok(())
    }

    /// The map entry `name` (its keys merge one by one), declared if new.
    ///
    /// # Errors
    ///
    /// Loro refused declaring it.
    pub fn map(&self, name: &str) -> DocResult<LoroMap> {
        self.doc
            .get_map(ns_name(self.id).as_str())
            .insert(&format!("m:{name}"), true)?;
        Ok(self.doc.get_map(map_name(self.id, name).as_str()))
    }

    /// The text entry `name` (edits merge per character), declared if new.
    ///
    /// # Errors
    ///
    /// Loro refused declaring it.
    pub fn text(&self, name: &str) -> DocResult<LoroText> {
        self.doc
            .get_map(ns_name(self.id).as_str())
            .insert(&format!("t:{name}"), true)?;
        Ok(self.doc.get_text(text_name(self.id, name)))
    }

    /// Makes text `name` read `text`, as a diff: a collaborator's typing
    /// elsewhere in it survives.
    ///
    /// # Errors
    ///
    /// Loro refused the write.
    pub fn set_text(&self, name: &str, text: &str) -> DocResult<()> {
        let target = self.text(name)?;
        if target.to_string() != text {
            // A diff that takes too long falls back to a replace, which is
            // still a correct (if coarser) edit.
            if target.update(text, UpdateOptions::default()).is_err() {
                let len = target.len_unicode();
                target.delete(0, len)?;
                target.insert(0, text)?;
            }
        }
        Ok(())
    }

    /// Removes entry `name`, whatever its kind.
    ///
    /// # Errors
    ///
    /// Loro refused the delete.
    pub fn remove(&self, name: &str) -> DocResult<()> {
        let ns = self.doc.get_map(ns_name(self.id).as_str());
        for prefix in ["r:", "m:", "t:"] {
            let key = format!("{prefix}{name}");
            if ns.get(&key).is_some() {
                ns.delete(&key)?;
            }
        }
        let map = self.doc.get_map(map_name(self.id, name).as_str());
        map.clear()?;
        let text = self.doc.get_text(text_name(self.id, name));
        let len = text.len_unicode();
        if len > 0 {
            text.delete(0, len)?;
        }
        Ok(())
    }
}

/// A transport as Architect's presence carries a playhead.
///
/// That is `crdt::presence::PlayState`: `playing`, `position` (seconds),
/// `at` (milliseconds) and `rate`. Architect's presence stamps `at` in each
/// process's own clock; for a playhead others follow, stamp the transport
/// in the shared clock first (the design, "Playback sync").
#[must_use]
pub fn play_state(transport: &Transport) -> LoroValue {
    let mut m: BTreeMap<String, LoroValue> = BTreeMap::new();
    m.insert("playing".into(), LoroValue::Bool(transport.is_playing));
    m.insert(
        "position".into(),
        LoroValue::Double(transport.playhead_seconds),
    );
    m.insert(
        "at".into(),
        LoroValue::Double(transport.host_micros / 1000.0),
    );
    m.insert("rate".into(), LoroValue::Double(transport.playrate));
    LoroValue::from(m.into_iter().collect::<std::collections::HashMap<_, _>>())
}

/// The transport in a presence `PlayState` value.
#[must_use]
pub fn transport_of(play: &LoroValue) -> Option<Transport> {
    let LoroValue::Map(m) = play else {
        return None;
    };
    let f = |k: &str| match m.get(k) {
        Some(LoroValue::Double(d)) => Some(*d),
        _ => None,
    };
    let playing = match m.get("playing") {
        Some(LoroValue::Bool(b)) => *b,
        _ => return None,
    };
    Some(Transport {
        host_micros: f("at")? * 1000.0,
        playhead_seconds: f("position")?,
        playrate: f("rate").unwrap_or(1.0),
        is_playing: playing,
    })
}

// ── names ───────────────────────────────────────────────────────────────

fn ns_name(id: &ExtensionId) -> String {
    format!("ext:{}", escape(id.as_str()))
}

fn map_name(id: &ExtensionId, name: &str) -> String {
    format!("ext:{}:m:{}", escape(id.as_str()), escape(name))
}

fn text_name(id: &ExtensionId, name: &str) -> String {
    format!("ext:{}:t:{}", escape(id.as_str()), escape(name))
}

/// A root container's name may not hold `/` or NUL (Loro panics on one),
/// and `:` separates the parts of ours: escape all three, and `%`.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => out.push_str("%25"),
            '/' => out.push_str("%2F"),
            ':' => out.push_str("%3A"),
            '\0' => out.push_str("%00"),
            c => out.push(c),
        }
    }
    out
}

// ── values ──────────────────────────────────────────────────────────────

fn value_of(map: &LoroMap, key: &str) -> Option<LoroValue> {
    match map.get(key)? {
        ValueOrContainer::Value(v) => Some(v),
        ValueOrContainer::Container(_) => None,
    }
}

fn delete_absent(map: &LoroMap, keep: impl Fn(&str) -> bool) -> DocResult<()> {
    let gone: Vec<String> = map
        .keys()
        .map(|k| k.to_string())
        .filter(|k| !keep(k))
        .collect();
    for k in gone {
        map.delete(&k)?;
    }
    Ok(())
}

/// A [`Value`] as Loro's value.
#[must_use]
pub fn value_to_loro(v: &Value) -> LoroValue {
    match v {
        Value::Null => LoroValue::Null,
        Value::Bool(b) => LoroValue::Bool(*b),
        Value::Int(i) => LoroValue::I64(*i),
        Value::Float(f) => LoroValue::Double(*f),
        Value::Text(s) => LoroValue::from(s.as_str()),
        Value::Bytes(b) => LoroValue::from(b.clone()),
        Value::List(l) => LoroValue::from(l.iter().map(value_to_loro).collect::<Vec<_>>()),
        Value::Map(m) => LoroValue::from(
            m.iter()
                .map(|(k, v)| (k.clone(), value_to_loro(v)))
                .collect::<std::collections::HashMap<_, _>>(),
        ),
    }
}

/// Loro's value as a [`Value`] (a container reference reads as null).
#[must_use]
pub fn value_from_loro(v: &LoroValue) -> Value {
    match v {
        LoroValue::Null | LoroValue::Container(_) => Value::Null,
        LoroValue::Bool(b) => Value::Bool(*b),
        LoroValue::Double(d) => Value::Float(*d),
        LoroValue::I64(i) => Value::Int(*i),
        LoroValue::Binary(b) => Value::Bytes(b.to_vec()),
        LoroValue::String(s) => Value::Text(s.to_string()),
        LoroValue::List(l) => Value::List(l.iter().map(value_from_loro).collect()),
        LoroValue::Map(m) => Value::Map(
            m.iter()
                .map(|(k, v)| (k.clone(), value_from_loro(v)))
                .collect(),
        ),
    }
}

fn write_bar_one(tempo: &LoroMap, at: Option<Time>) -> DocResult<()> {
    match at {
        Some(at) => tempo.insert("bar_one", time_to_loro(at))?,
        None if tempo.get("bar_one").is_some() => tempo.delete("bar_one")?,
        None => {}
    }
    Ok(())
}

fn time_to_loro(t: Time) -> LoroValue {
    match t {
        Time::Seconds(s) => LoroValue::from(vec![LoroValue::from("s"), LoroValue::Double(s)]),
        Time::QuarterNotes(q) => LoroValue::from(vec![LoroValue::from("qn"), LoroValue::Double(q)]),
    }
}

fn time_from_loro(v: &LoroValue) -> Option<Time> {
    let LoroValue::List(l) = v else {
        return None;
    };
    let (Some(LoroValue::String(base)), Some(LoroValue::Double(at))) = (l.first(), l.get(1)) else {
        return None;
    };
    match base.as_str() {
        "s" => Some(Time::Seconds(*at)),
        "qn" => Some(Time::QuarterNotes(*at)),
        _ => None,
    }
}

fn opt<T: Into<LoroValue>>(v: Option<T>) -> LoroValue {
    v.map_or(LoroValue::Null, Into::into)
}

fn tempo_to_loro(tempo: &TempoMap) -> LoroValue {
    let points: Vec<LoroValue> = tempo
        .points()
        .iter()
        .map(|p| {
            let mut m = std::collections::HashMap::new();
            m.insert("at".to_owned(), LoroValue::Double(p.position_seconds()));
            m.insert("bpm".to_owned(), LoroValue::Double(p.bpm));
            m.insert(
                "num".to_owned(),
                opt(p.time_signature.map(|t| i64::from(t.numerator))),
            );
            m.insert(
                "den".to_owned(),
                opt(p.time_signature.map(|t| i64::from(t.denominator))),
            );
            m.insert("shape".to_owned(), opt(p.shape.map(i64::from)));
            m.insert("tension".to_owned(), opt(p.bezier_tension));
            m.insert("linear".to_owned(), opt(p.linear));
            LoroValue::from(m)
        })
        .collect();
    let mut m = std::collections::HashMap::new();
    m.insert("bpm".to_owned(), LoroValue::Double(tempo.default_bpm()));
    m.insert(
        "num".to_owned(),
        LoroValue::I64(i64::from(tempo.default_meter().numerator)),
    );
    m.insert(
        "den".to_owned(),
        LoroValue::I64(i64::from(tempo.default_meter().denominator)),
    );
    m.insert("points".to_owned(), LoroValue::from(points));
    LoroValue::from(m)
}

fn meter(num: Option<&LoroValue>, den: Option<&LoroValue>) -> Option<TimeSignature> {
    let (Some(LoroValue::I64(n)), Some(LoroValue::I64(d))) = (num, den) else {
        return None;
    };
    TimeSignature::try_new(u32::try_from(*n).ok()?, u32::try_from(*d).ok()?).ok()
}

fn f64_of(m: &loro::LoroMapValue, k: &str) -> Option<f64> {
    match m.get(k) {
        Some(LoroValue::Double(d)) => Some(*d),
        _ => None,
    }
}

fn tempo_from_loro(v: &LoroValue) -> Option<TempoMap> {
    let LoroValue::Map(m) = v else {
        return None;
    };
    let bpm = f64_of(m, "bpm")?;
    let default_meter = meter(m.get("num"), m.get("den")).unwrap_or_default();
    let mut points = Vec::new();
    if let Some(LoroValue::List(list)) = m.get("points") {
        for p in list.iter() {
            let LoroValue::Map(p) = p else { continue };
            let (Some(at), Some(point_bpm)) = (f64_of(p, "at"), f64_of(p, "bpm")) else {
                continue;
            };
            let mut point = TempoMap::point(at, point_bpm, meter(p.get("num"), p.get("den")));
            point.shape = match p.get("shape") {
                Some(LoroValue::I64(s)) => i32::try_from(*s).ok(),
                _ => None,
            };
            point.bezier_tension = f64_of(p, "tension");
            point.linear = match p.get("linear") {
                Some(LoroValue::Bool(b)) => Some(*b),
                _ => None,
            };
            points.push(point);
        }
    }
    Some(TempoMap::new(bpm, default_meter, points))
}

// ── records ─────────────────────────────────────────────────────────────

fn write_records<K: AsRef<str> + Ord, R>(
    root: &LoroMap,
    records: &BTreeMap<K, R>,
    write: fn(&LoroMap, &R) -> DocResult<()>,
) -> DocResult<()> {
    for (id, record) in records {
        write(&record_node(root, id.as_ref())?, record)?;
    }
    delete_absent(root, |k| records.keys().any(|id| id.as_ref() == k))
}

/// Record `id`'s map under `root`, made if new. Ids are minted by whoever
/// makes the record, so no two peers make the same one: a plain child
/// container is enough (but see the design, §10).
fn record_node(root: &LoroMap, id: &str) -> DocResult<LoroMap> {
    Ok(match root.get(id) {
        Some(ValueOrContainer::Container(loro::Container::Map(node))) => node,
        _ => root.insert_container(id, LoroMap::new())?,
    })
}

fn get_record<R>(root: &LoroMap, id: &str, read: fn(&LoroMap) -> Option<R>) -> Option<R> {
    match root.get(id)? {
        ValueOrContainer::Container(loro::Container::Map(node)) => read(&node),
        _ => None,
    }
}

fn read_records<K: From<String> + Ord, R>(
    root: &LoroMap,
    read: fn(&LoroMap) -> Option<R>,
) -> BTreeMap<K, R> {
    let mut out = BTreeMap::new();
    root.for_each(|id, v| {
        if let ValueOrContainer::Container(loro::Container::Map(node)) = v
            && let Some(record) = read(&node)
        {
            out.insert(K::from(id.to_owned()), record);
        }
    });
    out
}

fn color_of(node: &LoroMap) -> Option<u32> {
    value_of(node, "color")
        .and_then(|v| v.into_i64().ok())
        .and_then(|c| u32::try_from(c).ok())
}

fn ruler_of(node: &LoroMap) -> Option<u32> {
    value_of(node, "ruler")
        .and_then(|v| v.into_i64().ok())
        .and_then(|c| u32::try_from(c).ok())
}

fn string_of(node: &LoroMap, key: &str) -> Option<String> {
    value_of(node, key)
        .and_then(|v| v.into_string().ok())
        .map(|s| s.to_string())
}

fn time_of(node: &LoroMap, key: &str) -> Option<Time> {
    value_of(node, key).as_ref().and_then(time_from_loro)
}

fn write_marker(node: &LoroMap, m: &Marker) -> DocResult<()> {
    node.insert("at", time_to_loro(m.at))?;
    node.insert("name", m.name.as_str())?;
    node.insert("color", opt(m.color.map(i64::from)))?;
    node.insert("ruler", opt(m.ruler.map(i64::from)))?;
    Ok(())
}

fn read_marker(node: &LoroMap) -> Option<Marker> {
    Some(Marker {
        at: time_of(node, "at")?,
        name: string_of(node, "name").unwrap_or_default(),
        color: color_of(node),
        ruler: ruler_of(node),
    })
}

fn write_region(node: &LoroMap, r: &Region) -> DocResult<()> {
    node.insert("start", time_to_loro(r.start))?;
    node.insert("end", time_to_loro(r.end))?;
    node.insert("name", r.name.as_str())?;
    node.insert("color", opt(r.color.map(i64::from)))?;
    node.insert("ruler", opt(r.ruler.map(i64::from)))?;
    Ok(())
}

fn read_region(node: &LoroMap) -> Option<Region> {
    Some(Region {
        start: time_of(node, "start")?,
        end: time_of(node, "end")?,
        name: string_of(node, "name").unwrap_or_default(),
        color: color_of(node),
        ruler: ruler_of(node),
    })
}

fn write_lane(node: &LoroMap, l: &Lane) -> DocResult<()> {
    node.insert("owner", l.owner.as_str())?;
    node.insert("kind", l.kind.as_str())?;
    node.insert("name", l.name.as_str())?;
    node.insert("order", l.order)?;
    node.insert("parent", opt(l.parent.as_ref().map(LaneId::as_str)))?;
    Ok(())
}

fn read_lane(node: &LoroMap) -> Option<Lane> {
    Some(Lane {
        owner: ExtensionId(string_of(node, "owner")?),
        kind: string_of(node, "kind").unwrap_or_default(),
        name: string_of(node, "name").unwrap_or_default(),
        order: value_of(node, "order")
            .and_then(|v| v.into_double().ok())
            .unwrap_or(0.0),
        parent: string_of(node, "parent").map(LaneId),
    })
}

fn write_item(node: &LoroMap, i: &Item) -> DocResult<()> {
    node.insert("owner", i.owner.as_str())?;
    node.insert("lane", i.lane.as_str())?;
    node.insert("kind", i.kind.as_str())?;
    node.insert("at", time_to_loro(i.at))?;
    node.insert("end", i.end.map_or(LoroValue::Null, time_to_loro))?;
    match &i.payload {
        // A map payload is a container: its keys merge one by one (one
        // person changes a cue's fade while another changes its colour).
        Value::Map(m) => {
            let payload = match node.get("payload") {
                Some(ValueOrContainer::Container(loro::Container::Map(p))) => p,
                _ => node.insert_container("payload", LoroMap::new())?,
            };
            for (k, v) in m {
                payload.insert(k, value_to_loro(v))?;
            }
            delete_absent(&payload, |k| m.contains_key(k))?;
        }
        other => node.insert("payload", value_to_loro(other))?,
    }
    Ok(())
}

fn read_item(node: &LoroMap) -> Option<Item> {
    let payload = match node.get("payload") {
        Some(ValueOrContainer::Container(loro::Container::Map(p))) => {
            let mut m = BTreeMap::new();
            p.for_each(|k, v| {
                if let ValueOrContainer::Value(v) = v {
                    m.insert(k.to_owned(), value_from_loro(&v));
                }
            });
            Value::Map(m)
        }
        Some(ValueOrContainer::Value(v)) => value_from_loro(&v),
        _ => Value::Null,
    };
    Some(Item {
        owner: ExtensionId(string_of(node, "owner")?),
        lane: LaneId(string_of(node, "lane")?),
        kind: string_of(node, "kind").unwrap_or_default(),
        at: time_of(node, "at")?,
        end: time_of(node, "end"),
        payload,
    })
}

// Ids are strings in the document.
impl AsRef<str> for MarkId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for LaneId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for ItemId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
