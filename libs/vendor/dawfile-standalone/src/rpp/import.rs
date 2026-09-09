//! `.rpp` → `.daw`.

use super::*;
use crate::document::{
    DawDocument, EnvelopeNode, ItemNode, MarkerNode, Provenance, SourceFormat, SourceRef, TakeNode,
    TrackNode,
};
use crate::error::{DawError, DawResult};
use crate::id::EntityId;
use crate::objects::ObjectStore;
use daw_proto::automation::{Envelope, EnvelopePoint, EnvelopeShape};
use daw_proto::item::{FadeShape, Item, SourceType, Take};
use daw_proto::marker::Marker;
use daw_proto::primitives::{AutomationMode, Duration, Position, PositionInSeconds, TimeSignature};
use daw_proto::stretch_marker::StretchMarker;
use daw_proto::tempo_map::TempoPoint;
use daw_proto::track::Track;
use std::collections::BTreeMap;

/// What an import saw that it did not model.
///
/// Unmodelled is **not** dropped — the verbatim source object holds every
/// byte, and export patches that source rather than regenerating it. The
/// report exists so a construct the schema never anticipated shows up as a
/// number in a test rather than as silence.
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    /// `context/KEY` → how many times it appeared. Contexts are `project`,
    /// `track`, `item`, `take`, `envelope`, `source`.
    pub unmodelled: BTreeMap<String, usize>,
    /// Chunks carried whole as opaque objects (FX chains, MIDI sources).
    pub opaque_objects: usize,
}

impl ImportReport {
    fn note(&mut self, context: &str, key: &str) {
        if key.is_empty() {
            return;
        }
        *self
            .unmodelled
            .entry(format!("{context}/{key}"))
            .or_insert(0) += 1;
    }

    /// The distinct unmodelled keys, most frequent first. What a human reads
    /// when deciding whether the schema should grow.
    pub fn ranked(&self) -> Vec<(&str, usize)> {
        let mut ranked: Vec<(&str, usize)> = self
            .unmodelled
            .iter()
            .map(|(key, count)| (key.as_str(), *count))
            .collect();
        ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
        ranked
    }
}

/// Import REAPER project text into a `.daw` document.
///
/// The original bytes go into `store` verbatim, and the returned document
/// points at them through [`Provenance`]. That is what makes the round trip
/// provable rather than asserted.
pub fn from_rpp(
    text: &str,
    name: impl Into<String>,
    store: &mut ObjectStore,
) -> DawResult<(DawDocument, ImportReport)> {
    let root = dawfile_reaper::rpp_tree::read_rpp_chunk(text)
        .map_err(|error| DawError::Rpp(error.to_string()))?;

    let name = name.into();
    let mut report = ImportReport::default();
    let mut document = DawDocument::new(name.clone());
    document.provenance = Some(Provenance {
        format: SourceFormat::Rpp,
        source: store.put(text.as_bytes().to_vec()),
        original_name: Some(name),
    });

    read_project(&root, &mut document, &mut report, store);
    document.reindex();
    Ok((document, report))
}

/// Project-level nodes, plus the track list.
fn read_project(
    root: &RChunk,
    document: &mut DawDocument,
    report: &mut ImportReport,
    store: &mut ObjectStore,
) {
    // A folder's children are not nested in the file: REAPER writes a flat
    // track list and encodes depth in `ISBUS`. The stack turns that back
    // into parent ids, so the document references a parent *by id* and
    // never by "the track above".
    let mut folder_stack: Vec<EntityId> = Vec::new();
    let mut default_time_signature = TimeSignature::new(4, 4);

    for child in &root.children {
        match child {
            RNodeTree::Node(node) => match key(node).as_str() {
                "SAMPLERATE" => document.sample_rate = param_i64(node, 1).map(|rate| rate as u32),
                "TEMPO" => {
                    if let (Some(numerator), Some(denominator)) =
                        (param_i64(node, 2), param_i64(node, 3))
                        && numerator > 0
                        && denominator > 0
                    {
                        default_time_signature =
                            TimeSignature::new(numerator as u32, denominator as u32);
                    }
                }
                "MARKER" => {
                    if let Some(marker) = read_marker(node) {
                        merge_marker(document, marker);
                    }
                }
                other => report.note("project", other),
            },
            RNodeTree::Chunk(chunk) => {
                let chunk_name = chunk.name().unwrap_or_default();
                match chunk_name.as_str() {
                    "TEMPOENVEX" => {
                        document.tempo_map = read_tempo_map(chunk, default_time_signature)
                    }
                    "TRACK" => {
                        let track_node =
                            read_track(chunk, &mut folder_stack, report, store, document);
                        document.tracks.push(track_node);
                    }
                    other => report.note("project", other),
                }
            }
        }
    }
}

/// `MARKER id position name flags color locked ? guid lane`.
///
/// Note field 4 is **flags**, not a region bit — markers and regions are the
/// same record in REAPER, and a region is recognised by a *second* `MARKER`
/// line sharing the first's id and carrying the end position. Reading field 4
/// as "is region" happens to work on projects where every region is flagged,
/// and silently mis-classifies everything else.
fn read_marker(node: &RNode) -> Option<(i64, MarkerNode)> {
    let id = param_i64(node, 1)?;
    let position = param_f64(node, 2)?;
    let name = param(node, 3).unwrap_or_default();
    let color = param_i64(node, 5).and_then(|raw| (raw != 0).then_some(raw as u32));
    let guid = param(node, 8).filter(|token| token.starts_with('{'));
    // Field 9 is the ruler lane on v7.62+. Older projects put other things
    // here, so anything outside a plausible lane range is left unread rather
    // than turned into a lane the project does not have.
    let lane = param_i64(node, 9).and_then(|lane| (0..256).contains(&lane).then_some(lane as u32));

    let entity_id = guid
        .clone()
        .map(EntityId::adopt)
        .unwrap_or_else(|| EntityId::derived(&EntityId::adopt("markers"), &format!("{id}:{name}")));

    Some((
        id,
        MarkerNode {
            id: entity_id,
            marker: Marker {
                id: Some(id as u32),
                position: Position::from_time(PositionInSeconds::from_seconds(position)),
                name,
                color,
                guid,
                lane,
            },
            region_end_seconds: None,
        },
    ))
}

/// Fold a `MARKER` line into the document, joining a region's two lines.
fn merge_marker(document: &mut DawDocument, (id, node): (i64, MarkerNode)) {
    if let Some(existing) = document
        .markers
        .iter_mut()
        .find(|existing| existing.marker.id == Some(id as u32))
    {
        // A second line with an id already seen is the closing half of a
        // region. It carries the end position and an empty name, so the
        // opening half keeps its identity and its name.
        existing.region_end_seconds = Some(node.marker.position_seconds());
        return;
    }
    document.markers.push(node);
}

/// `<TEMPOENVEX>` `PT position bpm shape [sig] [selected] [...]`.
fn read_tempo_map(chunk: &RChunk, default_signature: TimeSignature) -> Vec<TempoPoint> {
    let mut points = Vec::new();
    let mut running_signature = default_signature;

    for child in &chunk.children {
        let RNodeTree::Node(node) = child else {
            continue;
        };
        if key(node) != "PT" {
            continue;
        }
        let (Some(position), Some(bpm)) = (param_f64(node, 1), param_f64(node, 2)) else {
            continue;
        };

        // Field 4 packs the time signature as `numerator | denominator << 16`
        // when a signature change happens at this point, and is absent or 0
        // otherwise.
        let packed = param_i64(node, 4).unwrap_or(0);
        let mut signature = None;
        if packed > 0 {
            let numerator = (packed & 0xFFFF) as u32;
            let denominator = ((packed >> 16) & 0xFFFF) as u32;
            if numerator > 0 && denominator > 0 {
                running_signature = TimeSignature::new(numerator, denominator);
                signature = Some(running_signature);
            }
        }
        if points.is_empty() && signature.is_none() {
            // The project's `TEMPO` line carries the signature in force at
            // the start of the timeline; the first tempo point only repeats
            // it when it changes. Without this the map would claim the
            // project has no time signature at all until the first change.
            signature = Some(running_signature);
        }

        points.push(TempoPoint {
            position: Position::from_time(PositionInSeconds::from_seconds(position)),
            bpm,
            time_signature: signature,
            shape: param_i64(node, 3).map(|shape| shape as i32),
            bezier_tension: param_f64(node, 7),
            selected: param_bool(node, 5),
            linear: param_i64(node, 3).map(|shape| shape == 0),
        });
    }
    points
}

/// One `<TRACK>` chunk.
fn read_track(
    chunk: &RChunk,
    folder_stack: &mut Vec<EntityId>,
    report: &mut ImportReport,
    store: &mut ObjectStore,
    document: &DawDocument,
) -> TrackNode {
    // The chunk header carries the GUID on modern projects; `TRACKID` is the
    // fallback, and a derived id the last resort so an ancient project still
    // gets *stable* ids rather than positional ones.
    let guid = header_param(chunk, 1)
        .filter(|token| token.starts_with('{'))
        .or_else(|| child_node(chunk, "TRACKID").and_then(|node| param(node, 1)))
        .unwrap_or_else(|| {
            EntityId::derived(&document.id, &format!("track:{}", document.tracks.len())).to_string()
        });
    let id = EntityId::adopt(guid.clone());

    let mut track = Track::new(guid, 0, String::new());
    let mut envelopes = Vec::new();
    let mut items = Vec::new();
    let mut fx_chain = None;
    let mut input_fx_chain = None;
    let mut folder_state = 0i64;
    let mut folder_depth_change = 0i64;

    for child in &chunk.children {
        match child {
            RNodeTree::Node(node) => match key(node).as_str() {
                "NAME" => track.name = param(node, 1).unwrap_or_default(),
                "PEAKCOL" => track.color = param_i64(node, 1).map(|raw| raw as u32),
                "VOLPAN" => {
                    track.volume = param_f64(node, 1).unwrap_or(1.0);
                    track.pan = param_f64(node, 2).unwrap_or(0.0);
                }
                "MUTESOLO" => {
                    track.muted = param_bool(node, 1).unwrap_or(false);
                    track.soloed = param_i64(node, 2).unwrap_or(0) != 0;
                }
                "IPHASE" => track.phase_inverted = param_bool(node, 1).unwrap_or(false),
                "AUTOMODE" => {
                    track.automation_mode = automation_mode(param_i64(node, 1).unwrap_or(0))
                }
                "SEL" => track.selected = param_bool(node, 1).unwrap_or(false),
                "REC" => {
                    track.armed = param_bool(node, 1).unwrap_or(false);
                    track.input_monitor = input_monitor(param_i64(node, 3).unwrap_or(0));
                }
                "ISBUS" => {
                    folder_state = param_i64(node, 1).unwrap_or(0);
                    folder_depth_change = param_i64(node, 2).unwrap_or(0);
                }
                "SHOWINMIX" => track.visible_in_mixer = param_bool(node, 1).unwrap_or(true),
                "TRACKHEIGHT" => {
                    // Height 0 with the "hidden" flag set is REAPER's way of
                    // saying the track is not in the TCP.
                    track.visible_in_tcp = param_i64(node, 2).unwrap_or(0) & 2 == 0;
                }
                "FIXEDLANES" => {
                    track.lane_count = param_i64(node, 1).unwrap_or(0).max(0) as u32;
                    track.lane_play_mask = param_i64(node, 2).unwrap_or(0) as u64;
                }
                "LANENAME" => {
                    let names = tokens(node);
                    track.lane_names = names.into_iter().skip(2).collect();
                }
                "NCHAN" | "TRACKID" | "BEAT" | "PERF" => {}
                other => report.note("track", other),
            },
            RNodeTree::Chunk(inner) => {
                let inner_name = inner.name().unwrap_or_default();
                match inner_name.as_str() {
                    "FXCHAIN" => {
                        fx_chain = Some(store.put(stringify(inner).into_bytes()));
                        report.opaque_objects += 1;
                        track.fx_count = count_fx(inner);
                    }
                    "FXCHAIN_REC" => {
                        input_fx_chain = Some(store.put(stringify(inner).into_bytes()));
                        report.opaque_objects += 1;
                        track.input_fx_count = count_fx(inner);
                    }
                    "ITEM" => items.push(read_item(inner, &id, report, store)),
                    other if is_envelope_chunk(other) => {
                        envelopes.push(read_envelope(inner, &id, report))
                    }
                    other => report.note("track", other),
                }
            }
        }
    }

    // `ISBUS <state> <depth-change>`: state 1 opens a folder, and the depth
    // change closes as many as it is negative.
    let parent = folder_stack.last().cloned();
    track.is_folder = folder_state == 1;
    track.folder_depth = folder_depth_change as i32;
    if folder_state == 1 {
        folder_stack.push(id.clone());
    } else if folder_depth_change < 0 {
        for _ in 0..(-folder_depth_change) {
            folder_stack.pop();
        }
    }

    TrackNode {
        id,
        track,
        parent,
        envelopes,
        items,
        fx_chain,
        input_fx_chain,
    }
}

/// One `<ITEM>` chunk, including its flat run of takes.
fn read_item(
    chunk: &RChunk,
    track_id: &EntityId,
    report: &mut ImportReport,
    store: &mut ObjectStore,
) -> ItemNode {
    let mut item = Item::default();
    let mut envelopes = Vec::new();
    let mut takes: Vec<TakeNode> = Vec::new();
    // REAPER writes the first take's fields inline with no `TAKE` marker;
    // every later take is introduced by one. `pending` is the take being
    // filled in.
    let mut pending = PendingTake::default();
    let mut selected_take = 0usize;

    let iguid = child_node(chunk, "IGUID")
        .and_then(|node| param(node, 1))
        .unwrap_or_else(|| EntityId::new().to_string());
    let id = EntityId::adopt(iguid.clone());
    item.guid = iguid;
    item.track_guid = track_id.to_string();

    for child in &chunk.children {
        match child {
            RNodeTree::Node(node) => match key(node).as_str() {
                "POSITION" => {
                    item.position =
                        PositionInSeconds::from_seconds(param_f64(node, 1).unwrap_or(0.0))
                }
                "LENGTH" => item.length = Duration::from_seconds(param_f64(node, 1).unwrap_or(0.0)),
                "SNAPOFFS" => {
                    item.snap_offset = Duration::from_seconds(param_f64(node, 1).unwrap_or(0.0))
                }
                "LOOP" => item.loop_source = param_bool(node, 1).unwrap_or(false),
                "MUTE" => item.muted = param_bool(node, 1).unwrap_or(false),
                "SEL" => item.selected = param_bool(node, 1).unwrap_or(false),
                "LOCK" => item.locked = param_bool(node, 1).unwrap_or(false),
                "COLOR" => item.color = param_i64(node, 1).map(|raw| raw as u32),
                "GROUP" => item.group_id = param_i64(node, 1).map(|raw| raw as u32),
                "NOTES" => item.label = param(node, 1),
                "LANE" => item.fixed_lane = param_i64(node, 1).map(|lane| lane as u32),
                "FADEIN" => {
                    item.fade_in_shape = fade_shape(param_i64(node, 1).unwrap_or(0));
                    item.fade_in_length = Duration::from_seconds(param_f64(node, 2).unwrap_or(0.0));
                }
                "FADEOUT" => {
                    item.fade_out_shape = fade_shape(param_i64(node, 1).unwrap_or(0));
                    item.fade_out_length =
                        Duration::from_seconds(param_f64(node, 2).unwrap_or(0.0));
                }
                "BEAT" => {
                    item.beat_attach_mode = beat_attach_mode(param_i64(node, 1).unwrap_or(-1))
                }
                "SM" => {
                    if let (Some(position), Some(source_position)) =
                        (param_f64(node, 1), param_f64(node, 2))
                    {
                        pending.stretch_markers.push(StretchMarker {
                            position,
                            source_position,
                            slope: param_f64(node, 3).unwrap_or(0.0),
                        });
                    }
                }

                // ── take-scoped keys ────────────────────────────────
                "TAKE" => {
                    if param(node, 1).as_deref() == Some("SEL") {
                        selected_take = takes.len() + 1;
                    }
                    takes.push(pending.finish(&id, takes.len()));
                    pending = PendingTake::default();
                }
                "NAME" => pending.name = param(node, 1).unwrap_or_default(),
                "GUID" => pending.guid = param(node, 1),
                // `VOLPAN <item trim> <take pan> <take volume> <take pan law>`
                // — the line sits in the take's run but its first field is the
                // *item's* trim, not the take's volume. Reading field 1 as take
                // volume is invisible on a default project (both are 1.0) and
                // wrong on every project anyone has actually mixed.
                "VOLPAN" => {
                    item.volume = param_f64(node, 1).unwrap_or(1.0);
                    pending.volume = param_f64(node, 3).unwrap_or(1.0);
                }
                "SOFFS" => pending.start_offset = param_f64(node, 1).unwrap_or(0.0),
                "PLAYRATE" => {
                    pending.play_rate = param_f64(node, 1).unwrap_or(1.0);
                    pending.preserve_pitch = param_bool(node, 2).unwrap_or(true);
                    pending.pitch = param_f64(node, 3).unwrap_or(0.0);
                }
                "CHANMODE" => pending.channel_mode = param_i64(node, 1).unwrap_or(0) as u32,
                "TAKECOLOR" => pending.color = param_i64(node, 1).map(|raw| raw as u32),
                // `IGUID` is read ahead of this loop (it is the item's id, so it
                // has to be known before anything else is built); the rest are
                // REAPER bookkeeping the editor has no use for.
                "IGUID" | "IID" | "ALLTAKES" | "YPOS" | "RECPASS" => {}
                other => report.note("item", other),
            },
            RNodeTree::Chunk(inner) => {
                let inner_name = inner.name().unwrap_or_default();
                match inner_name.as_str() {
                    "SOURCE" => pending.source = Some(read_source(inner, store, report)),
                    other if is_envelope_chunk(other) => {
                        envelopes.push(read_envelope(inner, &id, report))
                    }
                    other => report.note("item", other),
                }
            }
        }
    }
    takes.push(pending.finish(&id, takes.len()));

    for (position, take_node) in takes.iter_mut().enumerate() {
        take_node.take.is_active = position == selected_take;
    }
    item.take_count = takes.len() as u32;
    item.active_take_index = selected_take as u32;

    ItemNode {
        id,
        item,
        takes,
        envelopes,
    }
}

/// A take under construction, before its `TAKE` delimiter (or the end of the
/// item) closes it.
#[derive(Default)]
struct PendingTake {
    guid: Option<String>,
    name: String,
    volume: f64,
    start_offset: f64,
    play_rate: f64,
    pitch: f64,
    preserve_pitch: bool,
    channel_mode: u32,
    color: Option<u32>,
    source: Option<(SourceRef, SourceType)>,
    stretch_markers: Vec<StretchMarker>,
}

impl PendingTake {
    fn finish(self, item_id: &EntityId, index: usize) -> TakeNode {
        let id = self
            .guid
            .clone()
            .map(EntityId::adopt)
            .unwrap_or_else(|| EntityId::derived(item_id, &format!("take/{index}")));
        let (source, source_type) = self.source.unwrap_or((SourceRef::Empty, SourceType::Empty));

        let take = Take {
            guid: id.to_string(),
            item_guid: item_id.to_string(),
            index: 0,
            is_active: false,
            name: self.name,
            color: self.color,
            // A default-constructed `PendingTake` has 0.0 where REAPER's
            // implicit defaults are 1.0, so unwrap them here rather than
            // writing a hand-rolled Default for the sake of two fields.
            volume: if self.volume == 0.0 { 1.0 } else { self.volume },
            play_rate: if self.play_rate == 0.0 {
                1.0
            } else {
                self.play_rate
            },
            pitch: self.pitch,
            preserve_pitch: self.preserve_pitch,
            channel_mode: self.channel_mode,
            start_offset: Duration::from_seconds(self.start_offset),
            source_type,
            source_file_path: match &source {
                SourceRef::File { path, .. } => Some(path.clone()),
                _ => None,
            },
            source_length: None,
            source_sample_rate: None,
            source_channels: None,
            is_midi: source_type == SourceType::Midi,
            midi_note_count: None,
        };

        TakeNode {
            id,
            take,
            source,
            stretch_markers: self.stretch_markers,
            envelopes: Vec::new(),
        }
    }
}

/// A `<SOURCE KIND>` chunk.
///
/// A file source is modelled; anything else — MIDI events above all — is
/// carried as an immutable object. Opaque is not lost: the chunk round-trips
/// byte for byte, and modelling MIDI events is #162's job, not this
/// ticket's.
fn read_source(
    chunk: &RChunk,
    store: &mut ObjectStore,
    report: &mut ImportReport,
) -> (SourceRef, SourceType) {
    let kind = header_param(chunk, 1).unwrap_or_default();
    if let Some(file_node) = child_node(chunk, "FILE")
        && let Some(path) = param(file_node, 1)
        && !matches!(kind.as_str(), "MIDI" | "MIDIPOOL")
    {
        let source_type = match kind.as_str() {
            "WAVE" | "MP3" | "FLAC" | "VORBIS" | "OGG" | "WAVPACK" | "AIFF" => SourceType::Audio,
            "VIDEO" | "AVI" | "MOV" => SourceType::Video,
            "" => SourceType::Unknown,
            _ => SourceType::Audio,
        };
        return (SourceRef::File { path, kind }, source_type);
    }

    report.opaque_objects += 1;
    let source_type = match kind.as_str() {
        "MIDI" | "MIDIPOOL" => SourceType::Midi,
        _ => SourceType::Unknown,
    };
    (
        SourceRef::Object {
            object: store.put(stringify(chunk).into_bytes()),
            kind,
        },
        source_type,
    )
}

/// One envelope chunk and its `PT` points.
fn read_envelope(chunk: &RChunk, owner: &EntityId, report: &mut ImportReport) -> EnvelopeNode {
    let chunk_name = chunk.name().unwrap_or_default();
    let mut points = Vec::new();
    let mut visible = true;
    let mut armed = false;
    // `VIS`'s second field and `LANEHEIGHT`'s first — where REAPER keeps
    // an envelope's lane. Read here rather than dropped, because a
    // parameter envelope that comes back overlaid on the waveform is not
    // the project the user saved.
    let mut in_own_lane = false;
    let mut lane_height = 0u32;
    let mut guid = None;
    let mut param_index = None;

    for child in &chunk.children {
        let RNodeTree::Node(node) = child else {
            report.note("envelope", &chunk.name().unwrap_or_default());
            continue;
        };
        match key(node).as_str() {
            "EGUID" => guid = param(node, 1),
            "VIS" => {
                visible = param_bool(node, 1).unwrap_or(true);
                in_own_lane = param_bool(node, 2).unwrap_or(false);
            }
            "ARM" => armed = param_bool(node, 1).unwrap_or(false),
            "LANEHEIGHT" => {
                lane_height = param_i64(node, 1).unwrap_or(0).max(0) as u32;
            }
            "PARAMBASE" | "PARMENVNAME" => param_index = param_i64(node, 1).map(|raw| raw as u32),
            "PT" => {
                if let (Some(time), Some(value)) = (param_f64(node, 1), param_f64(node, 2)) {
                    points.push(EnvelopePoint {
                        index: points.len() as u32,
                        time: PositionInSeconds::from_seconds(time),
                        value,
                        shape: envelope_shape(param_i64(node, 3).unwrap_or(0)),
                        tension: param_f64(node, 6).unwrap_or(0.0),
                        selected: param_bool(node, 4).unwrap_or(false),
                    });
                }
            }
            "ACT" | "DEFSHAPE" | "VOLTYPE" => {}
            other => report.note("envelope", other),
        }
    }

    let id = guid
        .clone()
        .filter(|token| token.starts_with('{'))
        .map(EntityId::adopt)
        .unwrap_or_else(|| EntityId::derived(owner, &chunk_name));

    EnvelopeNode {
        id,
        envelope: Envelope {
            track_guid: owner.to_string(),
            envelope_type: envelope_type_for(&chunk_name),
            // The chunk name is kept verbatim so a `Custom` envelope can be
            // written back exactly, and so promoting one to its own
            // `EnvelopeType` variant later needs no migration.
            name: chunk_name,
            fx_guid: None,
            param_index,
            visible,
            armed,
            automation_mode: AutomationMode::TrimRead,
            in_own_lane,
            lane_height,
            // Automation items are not modelled by the `.daw` document
            // yet — the count stays honest at 0 rather than guessing.
            automation_item_count: 0,
            point_count: points.len() as u32,
        },
        points,
    }
}

fn count_fx(chunk: &RChunk) -> u32 {
    chunk
        .children
        .iter()
        .filter(|child| {
            matches!(child, RNodeTree::Chunk(inner)
                if matches!(inner.name().as_deref(), Some("VST" | "AU" | "CLAP" | "JS" | "LV2")))
        })
        .count() as u32
}

fn stringify(chunk: &RChunk) -> String {
    dawfile_reaper::rpp_tree::stringify_rpp_node(&RNodeTree::Chunk(chunk.clone()))
}

fn automation_mode(raw: i64) -> AutomationMode {
    match raw {
        1 => AutomationMode::Read,
        2 => AutomationMode::Touch,
        3 => AutomationMode::Write,
        4 => AutomationMode::Latch,
        5 => AutomationMode::LatchPreview,
        _ => AutomationMode::TrimRead,
    }
}

fn input_monitor(raw: i64) -> daw_proto::track::InputMonitoringMode {
    use daw_proto::track::InputMonitoringMode;
    match raw {
        1 => InputMonitoringMode::Normal,
        2 => InputMonitoringMode::NotWhenPlaying,
        _ => InputMonitoringMode::Off,
    }
}

fn fade_shape(raw: i64) -> FadeShape {
    match raw {
        1 => FadeShape::FastStart,
        2 => FadeShape::FastEnd,
        3 => FadeShape::FastStartSteep,
        4 => FadeShape::FastEndSteep,
        5 => FadeShape::SlowStartEnd,
        6 => FadeShape::SlowStartEndSteep,
        _ => FadeShape::Linear,
    }
}

fn envelope_shape(raw: i64) -> EnvelopeShape {
    match raw {
        1 => EnvelopeShape::Square,
        2 => EnvelopeShape::SlowStartEnd,
        3 => EnvelopeShape::FastStart,
        4 => EnvelopeShape::FastEnd,
        5 => EnvelopeShape::Bezier,
        _ => EnvelopeShape::Linear,
    }
}

/// `BEAT <mode>`: REAPER's `-1` means "follow the project default", `0`
/// pins to time, `1` moves position and length with tempo, `2` moves only
/// the position.
fn beat_attach_mode(raw: i64) -> daw_proto::primitives::BeatAttachMode {
    use daw_proto::primitives::BeatAttachMode;
    match raw {
        1 => BeatAttachMode::Beats,
        2 => BeatAttachMode::BeatsPositionOnly,
        _ => BeatAttachMode::Time,
    }
}
