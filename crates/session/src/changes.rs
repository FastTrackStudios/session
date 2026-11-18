//! What changed in a session, by kind: what an engine recompiles its plan
//! for, and what a view reads again.
//!
//! Two faces, one vocabulary ([`Change`]):
//!
//! - **In process**, [`SessionDoc::subscribe_changes`]: every commit and
//!   every import, as a [`ChangeSet`] (and whether it came from another
//!   replica). The engine holding the document follows this.
//! - **Over the service**, [`changes_since`] (`Request::ReadChanges`):
//!   what changed since a version the caller was handed last time, so a
//!   view or an engine in another process asks cheaply at any rate and
//!   reads again only what moved. An unknown or missing version answers
//!   `whole`: read everything.
//!
//! A change names what moved, never its value: the reader asks for the
//! value through the same reads as always, so a change says nothing the
//! caller could not read anyway.

use std::collections::BTreeSet;

use loro::event::{Diff, DiffEvent};
use loro::{ContainerID, Frontiers, Index, LoroDoc};

use crate::doc::SessionDoc;
use crate::id::{ExtensionId, ItemId, LaneId};

/// One thing in a session that changed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, facet::Facet)]
#[repr(u8)]
pub enum Change {
    /// The session's name.
    Name,
    /// The tempo map (and its meters).
    Tempo,
    /// The rulers (the song's numbered divisions).
    Rulers,
    /// The markers.
    Markers,
    /// The regions.
    Regions,
    /// A lane: made, changed or removed.
    Lane(LaneId),
    /// An item: made, moved, changed or removed.
    Item(ItemId),
    /// An extension's own data.
    Extension(ExtensionId),
}

/// What one commit or import changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    /// Each change once, in order.
    pub changes: Vec<Change>,
    /// Made on another replica (an import), not here.
    pub remote: bool,
}

/// The root containers of a session (`crate::doc`'s layout).
fn root_change(root: &str) -> Option<Change> {
    match root {
        "meta" => Some(Change::Name),
        "tempo" => Some(Change::Tempo),
        "rulers" => Some(Change::Rulers),
        "markers" => Some(Change::Markers),
        "regions" => Some(Change::Regions),
        _ => None,
    }
}

/// The change a key of root `root` names: a lane's or an item's id, an
/// extension's id in the registry.
fn keyed_change(root: &str, key: &str) -> Option<Change> {
    match root {
        "lanes" => Some(Change::Lane(LaneId::from(key))),
        "items" => Some(Change::Item(ItemId::from(key))),
        "extensions" => Some(Change::Extension(ExtensionId::from(key))),
        _ => root_change(root).or_else(|| extension_of(root).map(Change::Extension)),
    }
}

/// The extension a namespace container belongs to: `ext:<id>`,
/// `ext:<id>:m:<name>`, `ext:<id>:t:<name>` (the id escaped).
fn extension_of(root: &str) -> Option<ExtensionId> {
    let rest = root.strip_prefix("ext:")?;
    let escaped = rest.split(':').next()?;
    Some(ExtensionId::from(unescape(escaped).as_str()))
}

/// `crate::doc`'s escape, undone.
fn unescape(s: &str) -> String {
    s.replace("%2F", "/")
        .replace("%3A", ":")
        .replace("%00", "\0")
        .replace("%25", "%")
}

fn root_name(id: &ContainerID) -> Option<&str> {
    match id {
        ContainerID::Root { name, .. } => Some(name.as_str()),
        ContainerID::Normal { .. } => None,
    }
}

fn key_of(index: &Index) -> Option<&str> {
    match index {
        Index::Key(key) => Some(key.as_str()),
        Index::Seq(_) | Index::Node(_) => None,
    }
}

/// The changes one container's diff makes. `path` runs from a root
/// container down to `target` (each step the container and its index in
/// its parent): a root alone, or a root and the key under it that the
/// change belongs to (an item's id, an extension map's key).
fn changes_of(
    target: &ContainerID,
    path: &[(ContainerID, Index)],
    diff: &Diff<'_>,
    out: &mut BTreeSet<Change>,
) {
    let root = path
        .first()
        .and_then(|(id, _)| root_name(id))
        .or_else(|| root_name(target));
    let Some(root) = root else {
        return;
    };
    // Below a root: the key under the root names what changed.
    if let Some(key) = path.get(1).and_then(|(_, index)| key_of(index)) {
        if let Some(change) = keyed_change(root, key) {
            out.insert(change);
        }
        return;
    }
    // The root itself: a keyed root says which keys; another, itself.
    match diff {
        Diff::Map(delta) if matches!(root, "lanes" | "items" | "extensions") => {
            for key in delta.updated.keys() {
                if let Some(change) = keyed_change(root, key) {
                    out.insert(change);
                }
            }
        }
        _ => {
            if let Some(change) =
                root_change(root).or_else(|| extension_of(root).map(Change::Extension))
            {
                out.insert(change);
            }
        }
    }
}

fn of_event(event: &DiffEvent<'_>) -> Vec<Change> {
    let mut out = BTreeSet::new();
    for container in &event.events {
        changes_of(container.target, container.path, &container.diff, &mut out);
    }
    out.into_iter().collect()
}

/// What changed in `doc` between versions `from` and `to`.
fn between(doc: &LoroDoc, from: &Frontiers, to: &Frontiers) -> Option<Vec<Change>> {
    let batch = doc.diff(from, to).ok()?;
    let mut out = BTreeSet::new();
    for (target, diff) in batch.iter() {
        let path = doc.get_path_to_container(target).unwrap_or_default();
        changes_of(target, &path, diff, &mut out);
    }
    Some(out.into_iter().collect())
}

/// A version of a session, as the service hands it out: opaque to the
/// caller, given back as `since`.
#[must_use]
pub fn version_of(doc: &SessionDoc) -> String {
    hex(&doc.doc().oplog_frontiers().encode())
}

/// What changed since `since` (a [`version_of`]), and the version now.
/// `None` for the changes when `since` is missing or not this session's:
/// read everything.
#[must_use]
pub fn changes_since(doc: &SessionDoc, since: Option<&str>) -> (String, Option<Vec<Change>>) {
    let loro = doc.doc();
    let now = loro.oplog_frontiers();
    let version = hex(&now.encode());
    let changes = since
        .and_then(unhex)
        .and_then(|bytes| Frontiers::decode(&bytes).ok())
        .and_then(|from| between(loro, &from, &now));
    (version, changes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len().saturating_mul(2)),
        |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        },
    )
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|p| u8::from_str_radix(p, 16).ok())
        })
        .collect()
}

impl SessionDoc {
    /// Calls `tell` with what every commit and every import changed,
    /// for as long as the subscription is kept. Off the audio thread: an
    /// engine recompiles its plan here.
    ///
    /// Never hold a strong handle to the document inside `tell` (Loro
    /// calls it under its subscriber lock); send what changed on.
    #[must_use = "dropping the subscription stops it"]
    pub fn subscribe_changes(
        &self,
        tell: impl Fn(ChangeSet) + Send + Sync + 'static,
    ) -> loro::Subscription {
        self.doc().subscribe_root(std::sync::Arc::new(move |event| {
            let found = of_event(&event);
            if found.is_empty() {
                return;
            }
            tell(ChangeSet {
                changes: found,
                remote: matches!(event.triggered_by, loro::EventTriggerKind::Import),
            });
        }))
    }
}
