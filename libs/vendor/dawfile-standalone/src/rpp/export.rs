//! `.daw` → `.rpp`.
//!
//! Export is a **patch**, not a regeneration. The original project text is
//! re-parsed and only the values the document actually changed are written
//! over; every construct the schema never modelled is still in the tree and
//! goes out untouched. This is the whole reason the round trip can be
//! proven rather than asserted, and it is why the corpus test can assert
//! that exporting an unedited document is a no-op down to the byte.

mod sources;

use super::*;
use crate::document::{DawDocument, EnvelopeNode, ItemNode, TakeNode, TrackNode};
use crate::error::{DawError, DawResult};
use crate::objects::ObjectStore;
use daw_proto::automation::EnvelopeShape;
use daw_proto::item::FadeShape;
use dawfile_reaper::rpp_tree::RToken;

/// What an export changed.
///
/// Empty for an unedited document — which is the assertion the corpus test
/// leans on.
#[derive(Clone, Debug, Default)]
pub struct ExportReport {
    /// One line per rewritten token, in `<entity> <KEY>[n]: old → new`
    /// form. Useful in a failure message, and it is what a future
    /// `fts daw diff` would print.
    pub changes: Vec<String>,
}

impl ExportReport {
    /// Whether the export rewrote anything at all.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Export a document back to REAPER project text.
///
/// Requires the document's verbatim source object, because that is what is
/// being patched. A document with no provenance cannot be exported this way
/// — creating an `.rpp` from nothing is a different job (project scaffolding
/// already lives in `dawfile_reaper::scaffold`).
pub fn to_rpp(document: &DawDocument, store: &ObjectStore) -> DawResult<String> {
    Ok(to_rpp_patched(document, store)?.0)
}

/// Export, and report what was rewritten.
pub fn to_rpp_patched(
    document: &DawDocument,
    store: &ObjectStore,
) -> DawResult<(String, ExportReport)> {
    let provenance = document
        .provenance
        .as_ref()
        .ok_or_else(|| DawError::Rpp("document has no .rpp provenance to export against".into()))?;

    let source = store.get(&provenance.source)?;
    let text = String::from_utf8_lossy(source).into_owned();
    let mut root = dawfile_reaper::rpp_tree::read_rpp_chunk(&text)
        .map_err(|error| DawError::Rpp(error.to_string()))?;

    let mut report = ExportReport::default();
    patch_project(&mut root, document, &mut report);
    sources::write_missing_sources(&mut root, document, store, &mut report)?;

    let mut rendered =
        dawfile_reaper::rpp_tree::stringify_rpp_node(&RNodeTree::Chunk(root.clone()));
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok((rendered, report))
}

/// Walk the tree beside the document, matching **by id** at every level.
///
/// Nothing here indexes into a list to find its counterpart: a track is
/// found by its GUID, an item by its `IGUID`, an envelope by its chunk name
/// or `EGUID`. Reordering tracks in the editor therefore patches the right
/// chunks rather than scrambling them.
fn patch_project(root: &mut RChunk, document: &DawDocument, report: &mut ExportReport) {
    // A track chunk with no counterpart in the document was deleted in the
    // editor. Dropping the chunk is the *only* structural removal that is
    // safe to infer, and it has to happen — leaving it behind would mean an
    // export that quietly resurrects a deleted track.
    let mut removed = Vec::new();
    root.children.retain(|child| {
        let RNodeTree::Chunk(chunk) = child else {
            return true;
        };
        if chunk.name().as_deref() != Some("TRACK") {
            return true;
        }
        match track_guid(chunk) {
            Some(guid) if document.track_by_guid(&guid).is_none() => {
                removed.push(guid);
                false
            }
            _ => true,
        }
    });
    for guid in removed {
        report.changes.push(format!("track {guid}: removed"));
    }

    let mut present = Vec::new();
    let mut last_track_at = None;
    for (position, child) in root.children.iter().enumerate() {
        if let RNodeTree::Chunk(chunk) = child
            && chunk.name().as_deref() == Some("TRACK")
        {
            last_track_at = Some(position);
            if let Some(guid) = track_guid(chunk) {
                present.push(guid);
            }
        }
    }

    for position in 0..root.children.len() {
        let RNodeTree::Chunk(chunk) = &mut root.children[position] else {
            continue;
        };
        if chunk.name().as_deref() != Some("TRACK") {
            continue;
        }
        let Some(guid) = track_guid(chunk) else {
            continue;
        };
        let Some(track_node) = document.track_by_guid(&guid) else {
            continue;
        };
        patch_track(chunk, track_node, report);
    }

    // Tracks the editor added have no chunk yet. They go in after the last
    // existing track, in document order, so the arrangement reads the same
    // way in REAPER as it does in the editor.
    let mut insert_at = last_track_at
        .map(|position| position + 1)
        .unwrap_or(root.children.len());
    for track_node in &document.tracks {
        if present.iter().any(|guid| guid == track_node.id.as_str()) {
            continue;
        }
        report
            .changes
            .push(format!("track {}: added", track_node.id));
        root.children
            .insert(insert_at, RNodeTree::Chunk(build_track(track_node)));
        insert_at += 1;
    }
}

/// A track chunk's stable id: the header GUID, or `TRACKID` on projects old
/// enough not to carry one in the header.
fn track_guid(chunk: &RChunk) -> Option<String> {
    header_param(chunk, 1)
        .filter(|token| token.starts_with('{'))
        .or_else(|| child_node(chunk, "TRACKID").and_then(|node| param(node, 1)))
}

fn patch_track(chunk: &mut RChunk, node: &TrackNode, report: &mut ExportReport) {
    let label = format!("track {}", node.id);
    let track = &node.track;

    reconcile(
        chunk,
        "ITEM",
        |inner| child_node(inner, "IGUID").and_then(|line| param(line, 1)),
        &node.items,
        |item| item.id.as_str(),
        build_item,
        &label,
        "item",
        report,
    );
    reconcile_envelopes(chunk, &node.envelopes, &label, report);

    // Items and envelopes are patched by walking the children once and
    // dispatching, so a track with both keeps its original ordering.
    for child in &mut chunk.children {
        match child {
            RNodeTree::Node(line) => match key(line).as_str() {
                "NAME" => set_string(line, 1, &track.name, &label, "NAME", report),
                "VOLPAN" => {
                    set_number(line, 1, track.volume, &label, "VOLPAN", report);
                    set_number(line, 2, track.pan, &label, "VOLPAN", report);
                }
                "MUTESOLO" => {
                    set_bool(line, 1, track.muted, &label, "MUTESOLO", report);
                    set_bool(line, 2, track.soloed, &label, "MUTESOLO", report);
                }
                "IPHASE" => set_bool(line, 1, track.phase_inverted, &label, "IPHASE", report),
                "SEL" => set_bool(line, 1, track.selected, &label, "SEL", report),
                "PEAKCOL" => {
                    if let Some(color) = track.color {
                        // REAPER writes PEAKCOL as a *signed* 32-bit
                        // value; the document stores the same bits
                        // unsigned (see the importer's `as u32`).
                        // Writing the unsigned reading back would flag
                        // every coloured track as changed and flip the
                        // token's text form.
                        set_number(line, 1, color as i32 as f64, &label, "PEAKCOL", report);
                    }
                }
                "REC" => set_bool(line, 1, track.armed, &label, "REC", report),
                _ => {}
            },
            RNodeTree::Chunk(inner) => {
                let inner_name = inner.name().unwrap_or_default();
                if inner_name == "ITEM" {
                    let iguid = child_node(inner, "IGUID").and_then(|line| param(line, 1));
                    if let Some(iguid) = iguid
                        && let Some(item_node) = node
                            .items
                            .iter()
                            .find(|candidate| candidate.id.as_str() == iguid)
                    {
                        patch_item(inner, item_node, report);
                    }
                } else if is_envelope_chunk(&inner_name)
                    && let Some(envelope_node) = find_envelope(&node.envelopes, inner, &inner_name)
                {
                    patch_envelope(inner, envelope_node, &label, report);
                }
            }
        }
    }
}

fn patch_item(chunk: &mut RChunk, node: &ItemNode, report: &mut ExportReport) {
    let label = format!("item {}", node.id);
    let item = &node.item;
    reconcile_envelopes(chunk, &node.envelopes, &label, report);
    reconcile_takes(chunk, node, &label, report);
    // Takes are matched by GUID, so the walk collects them in file order and
    // pairs each with the document's take of the same id.
    // Resolve the whole run before writing NAME/SOFFS: GUID appears after
    // those fields, and empty/null take slots do not reliably match indices.
    let runs = take_runs(chunk);
    let mut current_take: Option<&TakeNode> = None;

    for (position, child) in chunk.children.iter_mut().enumerate() {
        if let Some((index, (_, _, guid))) = runs
            .iter()
            .enumerate()
            .find(|(_, (start, _, _))| *start == position)
        {
            current_take = match guid {
                Some(guid) => node.takes.iter().find(|take| take.id.as_str() == guid),
                None => node.takes.get(index),
            };
        }
        match child {
            RNodeTree::Node(line) => match key(line).as_str() {
                "POSITION" => set_number(
                    line,
                    1,
                    item.position.as_seconds(),
                    &label,
                    "POSITION",
                    report,
                ),
                "LENGTH" => set_number(line, 1, item.length.as_seconds(), &label, "LENGTH", report),
                "SNAPOFFS" => set_number(
                    line,
                    1,
                    item.snap_offset.as_seconds(),
                    &label,
                    "SNAPOFFS",
                    report,
                ),
                "MUTE" => set_bool(line, 1, item.muted, &label, "MUTE", report),
                "SEL" => set_bool(line, 1, item.selected, &label, "SEL", report),
                "LOCK" => set_bool(line, 1, item.locked, &label, "LOCK", report),
                "LOOP" => set_bool(line, 1, item.loop_source, &label, "LOOP", report),
                "FADEIN" => {
                    set_number(
                        line,
                        1,
                        fade_shape_code(item.fade_in_shape),
                        &label,
                        "FADEIN",
                        report,
                    );
                    set_number(
                        line,
                        2,
                        item.fade_in_length.as_seconds(),
                        &label,
                        "FADEIN",
                        report,
                    );
                }
                "FADEOUT" => {
                    set_number(
                        line,
                        1,
                        fade_shape_code(item.fade_out_shape),
                        &label,
                        "FADEOUT",
                        report,
                    );
                    set_number(
                        line,
                        2,
                        item.fade_out_length.as_seconds(),
                        &label,
                        "FADEOUT",
                        report,
                    );
                }
                "COLOR" => {
                    if let Some(color) = item.color {
                        set_number(line, 1, color as f64, &label, "COLOR", report);
                    }
                }

                // ── take-scoped keys ────────────────────────────────
                "TAKE" | "GUID" => {}
                "NAME" => {
                    if let Some(take) = current_take {
                        set_string(line, 1, &take.take.name, &label, "take NAME", report);
                    }
                }
                // Field 1 is the *item's* trim and field 3 the take's volume —
                // see the matching note in the importer. One line, two owners.
                "VOLPAN" => {
                    set_number(line, 1, item.volume, &label, "VOLPAN", report);
                    if let Some(take) = current_take {
                        set_number(line, 3, take.take.volume, &label, "take VOLPAN", report);
                    }
                }
                "SOFFS" => {
                    if let Some(take) = current_take {
                        set_number(
                            line,
                            1,
                            take.take.start_offset.as_seconds(),
                            &label,
                            "take SOFFS",
                            report,
                        );
                    }
                }
                "PLAYRATE" => {
                    if let Some(take) = current_take {
                        set_number(
                            line,
                            1,
                            take.take.play_rate,
                            &label,
                            "take PLAYRATE",
                            report,
                        );
                        set_bool(
                            line,
                            2,
                            take.take.preserve_pitch,
                            &label,
                            "take PLAYRATE",
                            report,
                        );
                        set_number(line, 3, take.take.pitch, &label, "take PLAYRATE", report);
                    }
                }
                "CHANMODE" => {
                    if let Some(take) = current_take {
                        set_number(
                            line,
                            1,
                            take.take.channel_mode as f64,
                            &label,
                            "take CHANMODE",
                            report,
                        );
                    }
                }
                _ => {}
            },
            RNodeTree::Chunk(inner) => {
                let inner_name = inner.name().unwrap_or_default();
                if is_envelope_chunk(&inner_name)
                    && let Some(envelope_node) = find_envelope(&node.envelopes, inner, &inner_name)
                {
                    patch_envelope(inner, envelope_node, &label, report);
                }
            }
        }
    }

    reconcile_stretch_markers(chunk, node, &label, report);
}

/// Rewrite each take's `SM` lines to match the document.
///
/// Stretch markers are a *list the edit owns*, like envelope points:
/// there is no honest way to patch one marker against another, so the
/// take's whole set is replaced whenever it differs — deleted when the
/// document has none, inserted (before the take's `<SOURCE`, where
/// REAPER writes them) when the file had none. A take whose markers
/// already match is left byte-for-byte alone.
fn reconcile_stretch_markers(
    chunk: &mut RChunk,
    node: &ItemNode,
    label: &str,
    report: &mut ExportReport,
) {
    let runs = take_runs(chunk);
    // Reverse order, so earlier runs' indices survive later edits.
    for (run_index, (start, end, guid)) in runs.iter().enumerate().rev() {
        let take_node = guid
            .as_ref()
            .and_then(|g| node.takes.iter().find(|t| t.id.as_str() == g))
            .or_else(|| node.takes.get(run_index));
        let want: Vec<[String; 3]> = take_node
            .map(|t| {
                t.stretch_markers
                    .iter()
                    .map(|m| {
                        [
                            format_f64(m.position),
                            format_f64(m.source_position),
                            format_f64(m.slope),
                        ]
                    })
                    .collect()
            })
            .unwrap_or_default();

        let existing: Vec<usize> = (*start..*end)
            .filter(|&i| matches!(&chunk.children[i], RNodeTree::Node(line) if key(line) == "SM"))
            .collect();
        let have: Vec<Vec<String>> = existing
            .iter()
            .map(|&i| match &chunk.children[i] {
                RNodeTree::Node(line) => (1..=3).filter_map(|n| param(line, n)).collect(),
                _ => unreachable!(),
            })
            .collect();
        let unchanged = have.len() == want.len()
            && have
                .iter()
                .zip(&want)
                .all(|(h, w)| h.len() == 3 && h.iter().zip(w.iter()).all(|(a, b)| a == b));
        if unchanged {
            continue;
        }

        // Where the new set goes: where the old one was, else just
        // before the run's first chunk child (the `<SOURCE`), else the
        // run's end.
        let insert_at = existing.first().copied().unwrap_or_else(|| {
            (*start..*end)
                .find(|&i| matches!(&chunk.children[i], RNodeTree::Chunk(_)))
                .unwrap_or(*end)
        });
        for &i in existing.iter().rev() {
            chunk.children.remove(i);
        }
        let insert_at = insert_at - existing.iter().filter(|&&i| i < insert_at).count();
        for (offset, m) in want.iter().enumerate() {
            chunk
                .children
                .insert(insert_at + offset, node_line(&["SM", &m[0], &m[1], &m[2]]));
        }
        report.changes.push(format!(
            "{label} take {}: SM {} → {}",
            guid.as_deref().unwrap_or("?"),
            have.len(),
            want.len()
        ));
    }
}

/// Find the document's envelope for a chunk — by `EGUID` if the file has
/// one, otherwise by the chunk name, which is unique within its owner.
fn find_envelope<'a>(
    envelopes: &'a [EnvelopeNode],
    chunk: &RChunk,
    chunk_name: &str,
) -> Option<&'a EnvelopeNode> {
    if let Some(guid) = child_node(chunk, "EGUID").and_then(|line| param(line, 1))
        && let Some(found) = envelopes
            .iter()
            .find(|candidate| candidate.id.as_str() == guid)
    {
        return Some(found);
    }
    envelopes
        .iter()
        .find(|candidate| candidate.envelope.name == chunk_name)
}

/// Patch an envelope's points.
///
/// Points are a list, not a set of addressable entities, so this is the one
/// place export rewrites structurally: if the point list differs from the
/// file's, the `PT` lines are replaced wholesale, keeping every other line
/// (`ACT`, `VIS`, `DEFSHAPE`, `<EXT>` blocks) exactly where it was.
fn patch_envelope(
    chunk: &mut RChunk,
    node: &EnvelopeNode,
    owner_label: &str,
    report: &mut ExportReport,
) {
    let existing: Vec<(f64, f64, i64)> = chunk
        .children
        .iter()
        .filter_map(|child| match child {
            RNodeTree::Node(line) if key(line) == "PT" => Some((
                param_f64(line, 1).unwrap_or(0.0),
                param_f64(line, 2).unwrap_or(0.0),
                param_i64(line, 3).unwrap_or(0),
            )),
            _ => None,
        })
        .collect();

    let wanted: Vec<(f64, f64, i64)> = node
        .points
        .iter()
        .map(|point| {
            (
                point.time.as_seconds(),
                point.value,
                envelope_shape_code(point.shape),
            )
        })
        .collect();

    if existing == wanted {
        return;
    }

    report.changes.push(format!(
        "{owner_label} envelope {} PT: {} points → {} points",
        node.id,
        existing.len(),
        wanted.len()
    ));

    // Rebuild in place: drop the old `PT` lines, then insert the new ones
    // where the first one was, so the envelope's other lines keep their
    // order and a diff stays local.
    let insert_at = chunk
        .children
        .iter()
        .position(|child| matches!(child, RNodeTree::Node(line) if key(line) == "PT"))
        .unwrap_or(chunk.children.len());
    chunk
        .children
        .retain(|child| !matches!(child, RNodeTree::Node(line) if key(line) == "PT"));

    for (offset, point) in node.points.iter().enumerate() {
        let line = RNode::from_tokens(vec![
            RToken::new("PT"),
            RToken::new(format_f64(point.time.as_seconds())),
            RToken::new(format_f64(point.value)),
            RToken::new(envelope_shape_code(point.shape).to_string()),
            RToken::new(if point.selected { "1" } else { "0" }),
            RToken::new("0"),
            RToken::new(format_f64(point.tension)),
        ]);
        chunk
            .children
            .insert(insert_at + offset, RNodeTree::Node(line));
    }
}

// ── structural reconciliation ──────────────────────────────────────────
//
// Patching alone would silently lose anything the editor *added* and
// silently resurrect anything it *deleted*. These functions close that gap:
// the tree's chunks and the document's nodes are matched by id, chunks with
// no node are dropped, and nodes with no chunk are built.

/// Drop chunks whose entity is gone; append chunks for entities that are new.
#[allow(clippy::too_many_arguments)]
fn reconcile<T>(
    parent: &mut RChunk,
    chunk_name: &str,
    id_of_chunk: impl Fn(&RChunk) -> Option<String>,
    nodes: &[T],
    id_of_node: impl Fn(&T) -> &str,
    build: impl Fn(&T) -> RChunk,
    owner_label: &str,
    kind: &str,
    report: &mut ExportReport,
) {
    let mut removed = Vec::new();
    parent.children.retain(|child| {
        let RNodeTree::Chunk(chunk) = child else {
            return true;
        };
        if chunk.name().as_deref() != Some(chunk_name) {
            return true;
        }
        match id_of_chunk(chunk) {
            Some(id) if !nodes.iter().any(|node| id_of_node(node) == id) => {
                removed.push(id);
                false
            }
            _ => true,
        }
    });
    for id in removed {
        report
            .changes
            .push(format!("{owner_label} {kind} {id}: removed"));
    }

    let present: Vec<String> = parent
        .children
        .iter()
        .filter_map(|child| match child {
            RNodeTree::Chunk(chunk) if chunk.name().as_deref() == Some(chunk_name) => {
                id_of_chunk(chunk)
            }
            _ => None,
        })
        .collect();

    for node in nodes {
        let id = id_of_node(node);
        if present.iter().any(|existing| existing == id) {
            continue;
        }
        report
            .changes
            .push(format!("{owner_label} {kind} {id}: added"));
        parent.children.push(RNodeTree::Chunk(build(node)));
    }
}

/// Envelopes are matched by `EGUID` when present and by chunk name otherwise
/// — the same rule [`find_envelope`] uses, so add/remove and patch never
/// disagree about which chunk is which.
fn reconcile_envelopes(
    parent: &mut RChunk,
    nodes: &[EnvelopeNode],
    owner_label: &str,
    report: &mut ExportReport,
) {
    let mut removed = Vec::new();
    parent.children.retain(|child| {
        let RNodeTree::Chunk(chunk) = child else {
            return true;
        };
        let name = chunk.name().unwrap_or_default();
        if !is_envelope_chunk(&name) {
            return true;
        }
        if find_envelope(nodes, chunk, &name).is_some() {
            return true;
        }
        removed.push(name);
        false
    });
    for name in removed {
        report
            .changes
            .push(format!("{owner_label} envelope {name}: removed"));
    }

    for node in nodes {
        let already = parent.children.iter().any(|child| match child {
            RNodeTree::Chunk(chunk) => {
                let name = chunk.name().unwrap_or_default();
                is_envelope_chunk(&name)
                    && find_envelope(std::slice::from_ref(node), chunk, &name).is_some()
            }
            _ => false,
        });
        if already {
            continue;
        }
        report
            .changes
            .push(format!("{owner_label} envelope {}: added", node.id));
        parent.children.push(RNodeTree::Chunk(build_envelope(node)));
    }
}

/// Takes are a flat run inside `<ITEM>` rather than chunks, so they cannot go
/// through [`reconcile`].
///
/// Pairing is by take GUID where the file has one. A run with no GUID — an
/// item written by an older REAPER, or a bare empty item — cannot be
/// identified, so it is paired positionally with the next unclaimed take
/// instead. Guessing there is safe in a way that guessing a *deletion* would
/// not be: the worst case is patching an unnamed take's values, not
/// destroying it.
fn reconcile_takes(chunk: &mut RChunk, node: &ItemNode, label: &str, report: &mut ExportReport) {
    let runs = take_runs(chunk);
    if runs.is_empty() {
        return;
    }

    let mut claimed: Vec<&str> = Vec::new();
    // `None` means "this run has no counterpart and should go".
    let mut pairing: Vec<Option<&str>> = Vec::with_capacity(runs.len());
    for (_, _, guid) in &runs {
        match guid {
            Some(guid) => {
                let found = node
                    .takes
                    .iter()
                    .find(|take| take.id.as_str() == guid)
                    .map(|take| take.id.as_str());
                if let Some(id) = found {
                    claimed.push(id);
                }
                pairing.push(found);
            }
            None => {
                let next = node
                    .takes
                    .iter()
                    .map(|take| take.id.as_str())
                    .find(|id| !claimed.contains(id));
                if let Some(id) = next {
                    claimed.push(id);
                }
                pairing.push(next);
            }
        }
    }

    // Removals go back-to-front so earlier ranges stay valid.
    let mut leading_run_removed = false;
    for (position, ((start, end, guid), paired)) in runs.iter().zip(&pairing).enumerate().rev() {
        if paired.is_some() {
            continue;
        }
        report.changes.push(format!(
            "{label} take {}: removed",
            guid.clone().unwrap_or_else(|| "<unnamed>".to_string())
        ));
        chunk.children.drain(*start..*end);
        if position == 0 {
            leading_run_removed = true;
        }
    }

    for take in &node.takes {
        if claimed.contains(&take.id.as_str()) {
            continue;
        }
        report
            .changes
            .push(format!("{label} take {}: added", take.id));
        chunk.children.push(if take.take.is_active {
            node_line(&["TAKE", "SEL"])
        } else {
            node_line(&["TAKE"])
        });
        for line in take_lines(take) {
            chunk.children.push(RNodeTree::Node(line));
        }
    }

    // REAPER writes the first take with no `TAKE` marker. If the run that
    // held that position was deleted, the marker that now leads has to go or
    // REAPER reads an empty take ahead of everything.
    if leading_run_removed
        && let Some(position) = chunk
            .children
            .iter()
            .position(|child| matches!(child, RNodeTree::Node(line) if key(line) == "TAKE"))
    {
        chunk.children.remove(position);
    }
}

/// The `[start, end)` child ranges of each take inside an `<ITEM>`, with the
/// take's GUID where it has one.
fn take_runs(chunk: &RChunk) -> Vec<(usize, usize, Option<String>)> {
    let mut boundaries = vec![0usize];
    for (position, child) in chunk.children.iter().enumerate() {
        if let RNodeTree::Node(line) = child
            && key(line) == "TAKE"
        {
            boundaries.push(position);
        }
    }
    boundaries.push(chunk.children.len());

    let mut runs = Vec::new();
    for window in boundaries.windows(2) {
        let (start, end) = (window[0], window[1]);
        if start >= end {
            continue;
        }
        let guid = chunk.children[start..end]
            .iter()
            .find_map(|child| match child {
                RNodeTree::Node(line) if key(line) == "GUID" => param(line, 1),
                _ => None,
            });
        runs.push((start, end, guid));
    }
    runs
}

// ── chunk builders ─────────────────────────────────────────────────────
//
// Minimal but well-formed: enough for REAPER to open the project and see the
// entity, and nothing invented beyond what the document actually says.

fn node_line(tokens: &[&str]) -> RNodeTree {
    RNodeTree::Node(RNode::from_tokens(
        tokens.iter().map(|token| RToken::new(*token)).collect(),
    ))
}

fn build_track(node: &TrackNode) -> RChunk {
    let track = &node.track;
    let mut chunk = RChunk::new(vec![RToken::new("TRACK"), RToken::new(node.id.as_str())]);
    chunk.children.push(node_line(&["NAME", &track.name]));
    if let Some(color) = track.color {
        chunk
            .children
            .push(node_line(&["PEAKCOL", &color.to_string()]));
    }
    chunk.children.push(node_line(&[
        "VOLPAN",
        &format_f64(track.volume),
        &format_f64(track.pan),
        "-1",
        "-1",
        "1",
    ]));
    chunk.children.push(node_line(&[
        "MUTESOLO",
        if track.muted { "1" } else { "0" },
        if track.soloed { "1" } else { "0" },
        "0",
    ]));
    chunk.children.push(node_line(&[
        "IPHASE",
        if track.phase_inverted { "1" } else { "0" },
    ]));
    chunk.children.push(node_line(&[
        "ISBUS",
        if track.is_folder { "1" } else { "0" },
        &track.folder_depth.to_string(),
    ]));
    chunk
        .children
        .push(node_line(&["SEL", if track.selected { "1" } else { "0" }]));
    chunk.children.push(node_line(&["NCHAN", "2"]));
    chunk
        .children
        .push(node_line(&["TRACKID", node.id.as_str()]));

    for envelope in &node.envelopes {
        chunk
            .children
            .push(RNodeTree::Chunk(build_envelope(envelope)));
    }
    for item in &node.items {
        chunk.children.push(RNodeTree::Chunk(build_item(item)));
    }
    chunk
}

fn build_item(node: &ItemNode) -> RChunk {
    let item = &node.item;
    let mut chunk = RChunk::new(vec![RToken::new("ITEM")]);
    chunk.children.push(node_line(&[
        "POSITION",
        &format_f64(item.position.as_seconds()),
    ]));
    chunk.children.push(node_line(&[
        "SNAPOFFS",
        &format_f64(item.snap_offset.as_seconds()),
    ]));
    chunk.children.push(node_line(&[
        "LENGTH",
        &format_f64(item.length.as_seconds()),
    ]));
    chunk.children.push(node_line(&[
        "LOOP",
        if item.loop_source { "1" } else { "0" },
    ]));
    chunk.children.push(node_line(&[
        "FADEIN",
        &format_f64(fade_shape_code(item.fade_in_shape)),
        &format_f64(item.fade_in_length.as_seconds()),
        "0",
    ]));
    chunk.children.push(node_line(&[
        "FADEOUT",
        &format_f64(fade_shape_code(item.fade_out_shape)),
        &format_f64(item.fade_out_length.as_seconds()),
        "0",
    ]));
    chunk.children.push(node_line(&[
        "MUTE",
        if item.muted { "1" } else { "0" },
        "0",
    ]));
    chunk
        .children
        .push(node_line(&["SEL", if item.selected { "1" } else { "0" }]));
    chunk.children.push(node_line(&["IGUID", node.id.as_str()]));

    for envelope in &node.envelopes {
        chunk
            .children
            .push(RNodeTree::Chunk(build_envelope(envelope)));
    }
    for (position, take) in node.takes.iter().enumerate() {
        if position > 0 {
            chunk.children.push(if take.take.is_active {
                node_line(&["TAKE", "SEL"])
            } else {
                node_line(&["TAKE"])
            });
        }
        for line in take_lines(take) {
            chunk.children.push(RNodeTree::Node(line));
        }
    }
    chunk
}

/// The flat run of lines one take contributes to an `<ITEM>`.
fn take_lines(node: &TakeNode) -> Vec<RNode> {
    let take = &node.take;
    let mut lines = vec![
        RNode::from_tokens(vec![RToken::new("NAME"), RToken::new(&take.name)]),
        // `VOLPAN <item trim> <take pan> <take volume> <take pan law>`. The
        // item's trim is written by the caller's item, so a freshly built
        // take leaves it at unity and puts its own volume in field 3.
        RNode::from_tokens(vec![
            RToken::new("VOLPAN"),
            RToken::new("1"),
            RToken::new("0"),
            RToken::new(format_f64(take.volume)),
            RToken::new("-1"),
        ]),
        RNode::from_tokens(vec![
            RToken::new("SOFFS"),
            RToken::new(format_f64(take.start_offset.as_seconds())),
        ]),
        RNode::from_tokens(vec![
            RToken::new("PLAYRATE"),
            RToken::new(format_f64(take.play_rate)),
            RToken::new(if take.preserve_pitch { "1" } else { "0" }),
            RToken::new(format_f64(take.pitch)),
            RToken::new("-1"),
            RToken::new("0"),
            RToken::new("0.0025"),
        ]),
        RNode::from_tokens(vec![
            RToken::new("CHANMODE"),
            RToken::new(take.channel_mode.to_string()),
        ]),
        RNode::from_tokens(vec![RToken::new("GUID"), RToken::new(node.id.as_str())]),
    ];
    for marker in &node.stretch_markers {
        lines.push(RNode::from_tokens(vec![
            RToken::new("SM"),
            RToken::new(format_f64(marker.position)),
            RToken::new(format_f64(marker.source_position)),
            RToken::new(format_f64(marker.slope)),
        ]));
    }
    lines
}

fn build_envelope(node: &EnvelopeNode) -> RChunk {
    // The envelope's REAPER chunk name was kept verbatim on import, so a
    // round-tripped envelope goes back under the name it came in with.
    let chunk_name = if node.envelope.name.is_empty() {
        "VOLENV2"
    } else {
        node.envelope.name.as_str()
    };
    let mut chunk = RChunk::new(vec![RToken::new(chunk_name)]);
    chunk.children.push(node_line(&["EGUID", node.id.as_str()]));
    chunk.children.push(node_line(&["ACT", "1", "-1"]));
    // `VIS`'s second field is the lane flag, not a constant: exporting
    // "0" here sent every laned envelope back overlaid on the waveform,
    // which is a silent loss on round trip.
    chunk.children.push(node_line(&[
        "VIS",
        if node.envelope.visible { "1" } else { "0" },
        if node.envelope.in_own_lane { "1" } else { "0" },
        "1",
    ]));
    chunk.children.push(node_line(&[
        "LANEHEIGHT",
        &node.envelope.lane_height.to_string(),
        "0",
    ]));
    chunk.children.push(node_line(&[
        "ARM",
        if node.envelope.armed { "1" } else { "0" },
    ]));
    chunk
        .children
        .push(node_line(&["DEFSHAPE", "0", "-1", "-1"]));
    for point in &node.points {
        chunk.children.push(node_line(&[
            "PT",
            &format_f64(point.time.as_seconds()),
            &format_f64(point.value),
            &envelope_shape_code(point.shape).to_string(),
            if point.selected { "1" } else { "0" },
            "0",
            &format_f64(point.tension),
        ]));
    }
    chunk
}

// ── token-level writers ────────────────────────────────────────────────
//
// Each one is a no-op when the value already matches. That is what keeps an
// unedited export byte-identical and an edited one a minimal diff.

fn set_number(
    line: &mut RNode,
    index: usize,
    value: f64,
    label: &str,
    key_name: &str,
    report: &mut ExportReport,
) {
    let current = param_f64(line, index);
    // Compare numerically, not textually: `1` and `1.0` are the same value
    // and rewriting one as the other would churn every file.
    if current.is_some_and(|current| current == value) {
        return;
    }
    let rendered = format_f64(value);
    report.changes.push(format!(
        "{label} {key_name}[{index}]: {} → {rendered}",
        param(line, index).unwrap_or_default()
    ));
    write_token(line, index, rendered);
}

fn set_bool(
    line: &mut RNode,
    index: usize,
    value: bool,
    label: &str,
    key_name: &str,
    report: &mut ExportReport,
) {
    if param_bool(line, index) == Some(value) {
        return;
    }
    let rendered = if value { "1" } else { "0" }.to_string();
    report.changes.push(format!(
        "{label} {key_name}[{index}]: {} → {rendered}",
        param(line, index).unwrap_or_default()
    ));
    write_token(line, index, rendered);
}

fn set_string(
    line: &mut RNode,
    index: usize,
    value: &str,
    label: &str,
    key_name: &str,
    report: &mut ExportReport,
) {
    if param(line, index).as_deref() == Some(value) {
        return;
    }
    report.changes.push(format!(
        "{label} {key_name}[{index}]: {} → {value}",
        param(line, index).unwrap_or_default()
    ));
    write_token(line, index, value.to_string());
}

/// Replace one token, leaving every other token on the line untouched.
///
/// Dropping `line` is deliberate: the tree's stringifier prefers `tokens`
/// when both are present, so leaving a stale raw line behind would be
/// harmless but confusing to anyone debugging the tree.
fn write_token(line: &mut RNode, index: usize, value: String) {
    let mut current: Vec<RToken> = tokens(line).into_iter().map(RToken::new).collect();
    if current.len() <= index {
        current.resize(index + 1, RToken::new("0"));
    }
    current[index] = RToken::new(value);
    line.tokens = Some(current);
    line.line = None;
}

fn fade_shape_code(shape: FadeShape) -> f64 {
    match shape {
        FadeShape::Linear => 0.0,
        FadeShape::FastStart => 1.0,
        FadeShape::FastEnd => 2.0,
        FadeShape::FastStartSteep => 3.0,
        FadeShape::FastEndSteep => 4.0,
        FadeShape::SlowStartEnd => 5.0,
        FadeShape::SlowStartEndSteep => 6.0,
    }
}

fn envelope_shape_code(shape: EnvelopeShape) -> i64 {
    match shape {
        EnvelopeShape::Linear => 0,
        EnvelopeShape::Square => 1,
        EnvelopeShape::SlowStartEnd => 2,
        EnvelopeShape::FastStart => 3,
        EnvelopeShape::FastEnd => 4,
        EnvelopeShape::Bezier => 5,
    }
}
