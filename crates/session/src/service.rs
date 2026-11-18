//! What an extension may do to the session it runs in (feature `service`).
//!
//! A runtime extension does not hold the document: its host does, and
//! answers the extension's requests. Every host (Studio, Keyflow's app,
//! Ignition's) answers them with this one function, [`handle`], so the
//! rules are the same everywhere:
//!
//! - **Who is asking is the host's word, never the request's.** The host
//!   passes the caller's extension id (as Architect names the caller of a
//!   service call) and what it was granted.
//! - **An extension reads and writes its own namespace only.** There is no
//!   request that names another extension's entries.
//! - **It edits only what it owns**: lanes and items whose `owner` is it.
//!   It may put an item on another extension's lane (Ignition's blackout
//!   on the vocal track), and it may not move, change or remove another's
//!   item or lane.
//! - **Everyone reads the structure**: the tempo map, markers, regions,
//!   and every lane and item's kind and time, which is what a view that
//!   follows the timeline draws. Another extension's item **payload** comes
//!   back as null: it is that extension's data.
//! - **The song's structure is written only with a grant**
//!   ([`Grants::structure`]): Keyflow scaffolding sections into regions,
//!   say. Without it, tempo, markers and regions are read-only.
//!
//! Requests and responses are plain data (Facet), so a host carries them
//! over whatever it serves: an `#[architect::rpc]` service it `[provides]`
//! as `fts.session`, a WIT import, a web view's `call`. This module has no
//! transport yet: the `#[architect::rpc]` wrapper is the next step (the
//! design, §9), now that `architect` resolves here with its workspace-hack
//! patched out (§2.4). One request is one Loro transaction; a host wraps [`handle`] in
//! `crdt::History::transact` to make it one named undo step, made as the
//! extension.

use std::collections::BTreeMap;

use crate::doc::{DocError, SessionDoc};
use crate::ext::ExtensionData;
use crate::id::{ExtensionId, ItemId, LaneId, MarkId};
use crate::timeline::{Item, Lane, Marker, Region, TempoMap};
use crate::value::Value;

/// What the host granted the caller beyond its own namespace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, facet::Facet)]
pub struct Grants {
    /// May write the tempo map, markers and regions.
    pub structure: bool,
}

/// What an extension asks of the session.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
#[repr(u8)]
pub enum Request {
    /// Its whole namespace.
    ReadData,
    /// Sets register `name`.
    Set { name: String, value: Value },
    /// Sets `key` of map entry `name` (declaring the map if new).
    MapSet {
        name: String,
        key: String,
        value: Value,
    },
    /// Removes `key` of map entry `name`.
    MapRemove { name: String, key: String },
    /// Replaces `delete` characters at `at` in text entry `name` with
    /// `insert` (positions in Unicode scalars), declaring it if new.
    TextSplice {
        name: String,
        at: u32,
        delete: u32,
        insert: String,
    },
    /// Removes entry `name`, whatever its kind.
    Remove { name: String },
    /// Every lane, and every item with other extensions' payloads nulled.
    ReadTimeline,
    /// Puts a lane it owns.
    PutLane { id: LaneId, lane: Lane },
    /// Removes a lane it owns, and the items on it it owns. Another
    /// extension's items on it stay, orphaned: a concurrent put of an item
    /// and removal of its lane ends the same way whatever this does, so
    /// readers skip items whose lane is gone.
    RemoveLane { id: LaneId },
    /// Puts an item it owns, on any lane that exists.
    PutItem { id: ItemId, item: Item },
    /// Removes an item it owns.
    RemoveItem { id: ItemId },
    /// Sets the tempo map (needs [`Grants::structure`]).
    SetTempo { tempo: TempoMapData },
    /// Puts a marker (needs [`Grants::structure`]).
    PutMarker { id: MarkId, marker: Marker },
    /// Puts a region (needs [`Grants::structure`]).
    PutRegion { id: MarkId, region: Region },
    /// Removes a marker or region (needs [`Grants::structure`]).
    RemoveMark { id: MarkId },
}

/// A tempo map as plain data, for the wire.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
pub struct TempoMapData {
    pub default_bpm: f64,
    pub numerator: u32,
    pub denominator: u32,
    /// The changes, any order; each holds until the next.
    pub points: Vec<TempoChange>,
}

/// A meter, for the wire (no tuples in wire types: vox's schema exchange
/// refuses them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
pub struct Meter {
    pub numerator: u32,
    pub denominator: u32,
}

/// One change in a [`TempoMapData`].
#[derive(Debug, Clone, PartialEq, facet::Facet)]
pub struct TempoChange {
    pub seconds: f64,
    pub bpm: f64,
    /// The new meter, when it changes here.
    pub meter: Option<Meter>,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
#[repr(u8)]
pub enum Response {
    Done,
    Data(ExtensionData),
    Timeline {
        lanes: BTreeMap<LaneId, Lane>,
        items: BTreeMap<ItemId, Item>,
    },
}

/// Why a request was not done.
#[derive(Debug)]
pub enum ServiceError {
    /// Outside what the caller may do; says what and why.
    Refused(String),
    /// Allowed, and the document refused it.
    Failed(DocError),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(why) => write!(f, "refused: {why}"),
            Self::Failed(e) => write!(f, "failed: {e}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<DocError> for ServiceError {
    fn from(e: DocError) -> Self {
        Self::Failed(e)
    }
}

impl From<loro::LoroError> for ServiceError {
    fn from(e: loro::LoroError) -> Self {
        Self::Failed(DocError(e))
    }
}

fn refuse<T>(why: impl Into<String>) -> Result<T, ServiceError> {
    Err(ServiceError::Refused(why.into()))
}

/// Answers `request` from extension `caller`, granted `grants`, against
/// `doc`.
///
/// # Errors
///
/// [`ServiceError::Refused`] when the request is outside what the caller
/// may do (nothing is written); [`ServiceError::Failed`] when Loro refused
/// a write.
pub fn handle(
    doc: &SessionDoc,
    caller: &ExtensionId,
    grants: Grants,
    request: Request,
) -> Result<Response, ServiceError> {
    match request {
        Request::ReadTimeline
        | Request::PutLane { .. }
        | Request::RemoveLane { .. }
        | Request::PutItem { .. }
        | Request::RemoveItem { .. } => timeline(doc, caller, request),
        Request::SetTempo { .. }
        | Request::PutMarker { .. }
        | Request::PutRegion { .. }
        | Request::RemoveMark { .. } => {
            structure(caller, grants)?;
            song_structure(doc, request)
        }
        Request::ReadData
        | Request::Set { .. }
        | Request::MapSet { .. }
        | Request::MapRemove { .. }
        | Request::TextSplice { .. }
        | Request::Remove { .. } => namespace(doc, caller, request),
    }
}

/// Requests on the caller's own namespace.
fn namespace(
    doc: &SessionDoc,
    caller: &ExtensionId,
    request: Request,
) -> Result<Response, ServiceError> {
    match request {
        Request::ReadData => Ok(Response::Data(doc.read_ext(caller))),
        Request::Set { name, value } => {
            doc.ext(caller)?.set(&name, &value)?;
            Ok(done(doc))
        }
        Request::MapSet { name, key, value } => {
            doc.ext(caller)?
                .map(&name)?
                .insert(&key, crate::doc::value_to_loro(&value))?;
            Ok(done(doc))
        }
        Request::MapRemove { name, key } => {
            let map = doc.ext(caller)?.map(&name)?;
            if map.get(&key).is_some() {
                map.delete(&key)?;
            }
            Ok(done(doc))
        }
        Request::TextSplice {
            name,
            at,
            delete,
            insert,
        } => {
            let text = doc.ext(caller)?.text(&name)?;
            let len = text.len_unicode();
            let at = usize::try_from(at).unwrap_or(usize::MAX);
            let delete = usize::try_from(delete).unwrap_or(usize::MAX);
            if at > len || delete > len.saturating_sub(at) {
                return refuse(format!(
                    "text `{name}` is {len} characters; cannot replace {delete} at {at}"
                ));
            }
            if delete > 0 {
                text.delete(at, delete)?;
            }
            if !insert.is_empty() {
                text.insert(at, &insert)?;
            }
            Ok(done(doc))
        }
        Request::Remove { name } => {
            doc.ext(caller)?.remove(&name)?;
            Ok(done(doc))
        }
        _ => refuse("not a namespace request"),
    }
}

/// Requests on lanes and items.
fn timeline(
    doc: &SessionDoc,
    caller: &ExtensionId,
    request: Request,
) -> Result<Response, ServiceError> {
    match request {
        Request::ReadTimeline => {
            let mut items = doc.items();
            for item in items.values_mut() {
                if &item.owner != caller {
                    item.payload = Value::Null;
                }
            }
            Ok(Response::Timeline {
                lanes: doc.lanes(),
                items,
            })
        }
        Request::PutLane { id, lane } => {
            if &lane.owner != caller {
                return refuse(format!(
                    "a lane `{caller}` puts must be its own, not `{}`'s",
                    lane.owner
                ));
            }
            if let Some(held) = doc.lane(&id)
                && &held.owner != caller
            {
                return refuse(format!("lane `{id}` is `{}`'s", held.owner));
            }
            doc.put_lane(&id, &lane)?;
            Ok(Response::Done)
        }
        Request::RemoveLane { id } => {
            let Some(held) = doc.lane(&id) else {
                return Ok(Response::Done);
            };
            if &held.owner != caller {
                return refuse(format!("lane `{id}` is `{}`'s", held.owner));
            }
            for (item_id, item) in doc.items() {
                if item.lane == id && &item.owner == caller {
                    doc.remove_item(&item_id)?;
                }
            }
            doc.remove_lane(&id)?;
            Ok(Response::Done)
        }
        Request::PutItem { id, item } => {
            if &item.owner != caller {
                return refuse(format!(
                    "an item `{caller}` puts must be its own, not `{}`'s",
                    item.owner
                ));
            }
            if let Some(held) = doc.item(&id)
                && &held.owner != caller
            {
                return refuse(format!("item `{id}` is `{}`'s", held.owner));
            }
            if doc.lane(&item.lane).is_none() {
                return refuse(format!("no lane `{}`", item.lane));
            }
            doc.put_item(&id, &item)?;
            Ok(Response::Done)
        }
        Request::RemoveItem { id } => {
            match doc.item(&id) {
                Some(held) if &held.owner != caller => {
                    return refuse(format!("item `{id}` is `{}`'s", held.owner));
                }
                Some(_) => doc.remove_item(&id)?,
                None => {}
            }
            Ok(Response::Done)
        }
        _ => refuse("not a timeline request"),
    }
}

/// Requests on the song's structure; the grant is checked by [`handle`].
fn song_structure(doc: &SessionDoc, request: Request) -> Result<Response, ServiceError> {
    match request {
        Request::SetTempo { tempo } => {
            doc.set_tempo(&tempo_map(&tempo)?)?;
            Ok(Response::Done)
        }
        Request::PutMarker { id, marker } => {
            doc.put_marker(&id, &marker)?;
            Ok(Response::Done)
        }
        Request::PutRegion { id, region } => {
            doc.put_region(&id, &region)?;
            Ok(Response::Done)
        }
        Request::RemoveMark { id } => {
            doc.remove_marker(&id)?;
            doc.remove_region(&id)?;
            Ok(Response::Done)
        }
        _ => refuse("not a structure request"),
    }
}

fn done(doc: &SessionDoc) -> Response {
    doc.doc().commit();
    Response::Done
}

fn structure(caller: &ExtensionId, grants: Grants) -> Result<(), ServiceError> {
    if grants.structure {
        Ok(())
    } else {
        refuse(format!(
            "`{caller}` may not change the song's structure (tempo, markers, regions)"
        ))
    }
}

fn meter(numerator: u32, denominator: u32) -> Result<crate::time::TimeSignature, ServiceError> {
    crate::time::TimeSignature::try_new(numerator, denominator).or_else(refuse)
}

fn tempo_map(data: &TempoMapData) -> Result<TempoMap, ServiceError> {
    let bpm_ok = |bpm: f64| bpm.is_finite() && bpm > 0.0;
    if !bpm_ok(data.default_bpm) {
        return refuse(format!("tempo {} BPM", data.default_bpm));
    }
    let mut points = Vec::with_capacity(data.points.len());
    for change in &data.points {
        if !bpm_ok(change.bpm) || !change.seconds.is_finite() {
            return refuse(format!(
                "tempo change {} BPM at {} s",
                change.bpm, change.seconds
            ));
        }
        let meter = match change.meter {
            Some(m) => Some(meter(m.numerator, m.denominator)?),
            None => None,
        };
        points.push(TempoMap::point(change.seconds, change.bpm, meter));
    }
    Ok(TempoMap::new(
        data.default_bpm,
        meter(data.numerator, data.denominator)?,
        points,
    ))
}
