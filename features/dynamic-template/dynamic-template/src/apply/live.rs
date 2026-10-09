//! [`TemplateTarget`] over a live DAW — any backend behind the
//! `daw::service` traits: a running REAPER (`daw_reaper::Reaper`) or the
//! in-process `daw-standalone` engine.
//!
//! The offline counterparts are [`super::dawfile`] and [`super::chunk`]; all
//! of them are driven by [`organize`](super::organize), so the bus tree this
//! builds in a live session is the same one the file backends write to an
//! `.RPP`.
//!
//! Everything goes through the service traits — never raw
//! `reaper_low`/`reaper_medium` FFI. On REAPER these calls are synchronous
//! and must run on its main thread, which is where extension action
//! handlers already are.

use daw::service::{Routing, Tracks};
use daw_proto::{FolderDepthChange, ProjectContext, ReorderTracksBehavior, TrackRef};
#[cfg(feature = "reaper")]
use daw_reaper::Reaper;

use super::TemplateTarget;

/// A live project the template can be applied to, on whatever backend `D`
/// is.
pub struct DawTarget<D> {
    daw: D,
    project: ProjectContext,
}

/// A live REAPER project — the extension actions' target.
#[cfg(feature = "reaper")]
pub type ReaperTarget = DawTarget<Reaper>;

impl<D> DawTarget<D> {
    /// Target `project` on `daw`.
    pub const fn on(daw: D, project: ProjectContext) -> Self {
        Self { daw, project }
    }
}

#[cfg(feature = "reaper")]
impl ReaperTarget {
    /// Target the project REAPER currently has in front.
    #[must_use]
    pub const fn current() -> Self {
        Self::new(ProjectContext::Current)
    }

    /// Target a specific project.
    #[must_use]
    pub const fn new(project: ProjectContext) -> Self {
        Self {
            daw: Reaper,
            project,
        }
    }
}

impl<D: Tracks + Routing> TemplateTarget for DawTarget<D> {
    /// Track GUIDs — stable across the inserts and reorders that would
    /// invalidate an index.
    type TrackId = String;
    type Error = daw_proto::DawError;

    fn find_track(&self, name: &str) -> Option<String> {
        self.daw
            .all(self.project.clone())
            .into_iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(name.trim()))
            .map(|t| t.guid)
    }

    fn append_track(&mut self, name: &str) -> Result<String, Self::Error> {
        self.daw.add(self.project.clone(), name, None)
    }

    fn set_folder_depth(
        &mut self,
        id: &String,
        depth: FolderDepthChange,
    ) -> Result<(), Self::Error> {
        self.daw.set_folder_depth(
            self.project.clone(),
            TrackRef::Guid(id.clone()),
            depth.to_raw_value(),
        )
    }

    fn set_color(&mut self, id: &String, hex: &str) -> Result<(), Self::Error> {
        // An unparseable color leaves the track's own color alone rather than
        // failing the whole apply — the routing matters, the tint does not.
        let Ok(color) = color_palette::Color::from_hex_str(hex) else {
            return Ok(());
        };
        // REAPER color values are 24-bit RGB codes; the i32 is a signed
        // wrapper around unsigned bit patterns, so reinterpret rather than
        // convert.
        let color_u32 = color.to_reaper_native().cast_unsigned();
        self.daw
            .set_color(self.project.clone(), TrackRef::Guid(id.clone()), color_u32)
    }

    fn set_channel_count(&mut self, id: &String, channels: u32) -> Result<(), Self::Error> {
        self.daw
            .set_num_channels(self.project.clone(), TrackRef::Guid(id.clone()), channels)
    }

    fn has_send(&self, source: &String, dest: &String) -> bool {
        self.daw
            .sends(self.project.clone(), TrackRef::Guid(source.clone()))
            .iter()
            .any(|route| route.dest_track_guid.as_ref() == Some(dest))
    }

    fn add_send(&mut self, source: &String, dest: &String) -> Result<(), Self::Error> {
        self.daw.add_send(
            self.project.clone(),
            TrackRef::Guid(source.clone()),
            TrackRef::Guid(dest.clone()),
        );
        Ok(())
    }

    fn set_parent_send(&mut self, id: &String, enabled: bool) -> Result<(), Self::Error> {
        self.daw
            .set_parent_send_enabled(self.project.clone(), TrackRef::Guid(id.clone()), enabled)
    }

    fn folder_depths(&self) -> Vec<(String, String, i32)> {
        self.daw
            .all(self.project.clone())
            .into_iter()
            .map(|t| (t.guid, t.name, t.folder_depth))
            .collect()
    }

    /// The live form of the file backends' DI nesting: a "DI" track right
    /// after a plain sibling, in a group that opts in, becomes a muted child
    /// of it. (The file backends also collapse the new folder; the service
    /// surface has no collapse call, so a live nest stays expanded.)
    fn nest_secondary_mics(&mut self) {
        let config = crate::default_config();
        let entries = super::contextual_paths(self);
        let depths: std::collections::HashMap<String, i32> = self
            .folder_depths()
            .into_iter()
            .map(|(guid, _, depth)| (guid, depth))
            .collect();
        for pair in entries.windows(2) {
            let [main, di] = pair else { continue };
            if main.context != di.context {
                continue;
            }
            let is_di = |name: &str| name.trim().eq_ignore_ascii_case("di");
            if !is_di(&di.name) || is_di(&main.name) {
                continue;
            }
            let Some(leaf) = di.path.last() else { continue };
            if !super::find_group(&config, leaf).is_some_and(|g| g.nest_secondary_mics) {
                continue;
            }
            let main_depth = depths.get(&main.track).copied().unwrap_or(0);
            let di_depth = depths.get(&di.track).copied().unwrap_or(0);
            if main_depth != 0 || di_depth > 0 {
                continue;
            }
            let project = self.project.clone();
            let _ = self.daw.set_folder_depth(
                project.clone(),
                TrackRef::Guid(main.track.clone()),
                FolderDepthChange::FolderStart.to_raw_value(),
            );
            let _ = self.daw.set_folder_depth(
                project.clone(),
                TrackRef::Guid(di.track.clone()),
                di_depth.saturating_sub(1),
            );
            let _ = Tracks::set_muted(&self.daw, project, TrackRef::Guid(di.track.clone()), true);
        }
    }

    fn gather_into_folder(
        &mut self,
        folder: &str,
        tracks: &[String],
    ) -> Result<Option<crate::apply::Gathered<String>>, Self::Error> {
        // Same rule as the file backends: only a plain track at the top
        // level moves. One carrying folder structure would strand what it
        // held open, and one nested inside a folder belongs to that folder —
        // pulling it out reshapes a folder the user built.
        let all = self.daw.all(self.project.clone());
        let mut running = 0i32;
        let top_level_plain: std::collections::HashSet<&str> = all
            .iter()
            .filter_map(|t| {
                let at_top = running == 0;
                running = running.saturating_add(t.folder_depth);
                (at_top && t.folder_depth == 0).then_some(t.guid.as_str())
            })
            .collect();
        let movable: Vec<String> = tracks
            .iter()
            .filter(|guid| top_level_plain.contains(guid.as_str()))
            .cloned()
            .collect();
        if movable.is_empty() {
            return Ok(None);
        }

        let folder_guid = self.daw.add(self.project.clone(), folder, None)?;
        let folder_index = u32::try_from(
            self.daw
                .all(self.project.clone())
                .iter()
                .position(|t| t.guid == folder_guid)
                .ok_or_else(|| {
                    daw_proto::DawError::NotFound(format!("track {folder_guid} vanished after add"))
                })?,
        )
        .map_err(|_| daw_proto::DawError::NotFound("track index out of range".to_string()))?;

        self.daw.clear_selection(self.project.clone())?;
        for guid in &movable {
            self.daw
                .set_selected(self.project.clone(), TrackRef::Guid(guid.clone()), true)?;
        }
        // MakeChildOfPreviousTrack drops them inside the folder we just made,
        // and REAPER keeps every send intact across the move because it tracks
        // routing by pointer, not position.
        self.daw.reorder_selected(
            self.project.clone(),
            folder_index.saturating_add(1),
            ReorderTracksBehavior::MakeChildOfPreviousTrack,
        )?;
        self.daw.clear_selection(self.project.clone())?;

        self.daw.set_folder_depth(
            self.project.clone(),
            TrackRef::Guid(folder_guid.clone()),
            FolderDepthChange::FolderStart.to_raw_value(),
        )?;
        let skipped = tracks
            .iter()
            .filter(|g| !movable.contains(g))
            .cloned()
            .collect();
        Ok(Some(crate::apply::Gathered {
            folder: folder_guid,
            moved: movable,
            skipped,
        }))
    }
}

// ── Folder layout, live ─────────────────────────────────────────────────
//
// The bus pass (`organize`) wires a session into the mix; these give it
// the folder shape the template describes — Guide, Keyflow, Drums, Bass,
// Guitars … — which the file backends leave to REAPER's own user.

use daw::service::Items;

/// The kept block (see `DawTarget::kept_folders`) whose folder is named
/// `name`, taken out of `units` and left open — its last track no longer
/// closing its folder — so the group's other tracks can follow inside it.
fn open_kept(
    units: &mut Vec<Vec<(String, i32)>>,
    all: &[daw::service::Track],
    name: &str,
) -> Option<Vec<(String, i32)>> {
    let at = units.iter().position(|unit| {
        unit.first()
            .and_then(|(g, _)| all.iter().find(|t| &t.guid == g))
            .is_some_and(|t| t.name.trim().eq_ignore_ascii_case(name.trim()))
    })?;
    let mut unit = units.remove(at);
    if let Some(last) = unit.last_mut() {
        last.1 = last.1.saturating_add(1);
    }
    Some(unit)
}

/// The instrument groups in the order a session lays them out: the rhythm
/// section, then the harmony, then the voices.
const INSTRUMENT_ORDER: &[&str] = &[
    "Drums",
    "Percussion",
    "Bass",
    "Guitars",
    "Keys",
    "Synths",
    "Strings",
    "Orchestra",
    "Horns",
    "Harmonica",
    "Vocals",
    "Choir",
    "SFX",
];

/// What [`DawTarget::arrange_into_groups`] did.
#[derive(Debug, Default)]
pub struct Arranged {
    /// Wrapper folders removed (their tracks kept).
    pub unwrapped: Vec<String>,
    /// Group folders created.
    pub created: Vec<String>,
    /// Tracks placed inside a group.
    pub placed: usize,
}

impl<D: Tracks + Routing + Items> DawTarget<D> {
    fn has_items(&self, guid: &str) -> bool {
        !Items::get_items(
            &self.daw,
            self.project.clone(),
            TrackRef::Guid(guid.to_owned()),
        )
        .is_empty()
    }

    /// Put every content track into the group folder the template's
    /// grouping engine (`organize_into_tracks`) assigns it — Drums, Bass,
    /// Guitars / Electric, Keys, Guide … — creating the folders it needs,
    /// and take apart any folder that only wrapped a multitrack (no media
    /// of its own, not a bus or a group). Tracks keep their own names.
    ///
    /// Buses, the Keyflow folder and anything the engine does not place
    /// keep their order, after the groups. Idempotent: a second run finds
    /// every track already in its group.
    ///
    /// # Errors
    ///
    /// The grouping engine, or a backend call, failed.
    pub fn arrange_into_groups(&mut self) -> eyre::Result<Arranged> {
        use crate::OrganizeIntoTracks;

        let mut arranged = Arranged::default();
        let project = self.project.clone();
        // Reference material, read with each track's folders around it —
        // a rehearsal is recognised by its set and its container, which a
        // name alone does not show (`reclassify_rehearsal`). It is laid out
        // apart, below, not grouped by what its tracks are called.
        let mut rehearsal: Vec<String> = Vec::new();
        let mut bounces: Vec<String> = Vec::new();
        let mut classified: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut group_of: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // The guide's rendered prints, by name: kept in their own folder.
        let mut prints: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in super::classified_paths(self) {
            // A bus is a bus: its name ("MIX BUS") classifies as what it
            // carries — the mix, a reference — which it is not.
            if crate::buses::is_bus_name(&entry.name) {
                continue;
            }
            if let Some(group) = entry.path.first() {
                group_of.insert(entry.track.clone(), group.clone());
            }
            if entry.path.iter().any(|g| g == super::PRINTS_FOLDER) {
                prints.insert(entry.name.clone());
            }
            let reference = entry.path.first().is_some_and(|g| g == "Reference");
            if entry
                .path
                .iter()
                .map(String::as_str)
                .eq(super::REHEARSAL_PATH)
            {
                rehearsal.push(entry.track);
            } else if reference {
                bounces.push(entry.track);
            } else if !entry.path.is_empty() {
                classified.insert(entry.track);
            }
        }
        let all = Tracks::all(&self.daw, project.clone());
        // A session already built: a top-level folder whose tracks are what
        // its name and theirs say (an `Electric GTR` of amps, sides and
        // mics) is the engineer's structure, kept whole — moved as one
        // block, never taken apart for its tracks to be sorted by their
        // names alone, which reads `Main` and `DI` as nothing. Each is its
        // tracks, `(guid, depth change)`, in order.
        let mut units = self.kept_folders(&all, &classified, &rehearsal, &bounces);
        let in_unit: std::collections::HashSet<String> =
            units.iter().flatten().map(|(g, _)| g.clone()).collect();

        // Groups the template knows, so a folder named for one is kept.
        let config = crate::default_config();
        let is_known_folder = |name: &str| {
            let n = name.trim();
            crate::buses::is_bus_name(n)
                || super::find_group(&config, n).is_some()
                || n.eq_ignore_ascii_case("Keyflow")
                || n.eq_ignore_ascii_case(super::UNSORTED_FOLDER)
        };

        // 1. Unwrap: a folder with no media that is not a bus or a group
        //    only held a multitrack together — nor does a rehearsal's
        //    container, whatever it is called. Its children stay.
        for track in &all {
            let container = rehearsal.contains(&track.guid) || bounces.contains(&track.guid);
            if in_unit.contains(&track.guid) {
                continue;
            }
            if track.folder_depth > 0
                && (container || !is_known_folder(&track.name))
                && !self.has_items(&track.guid)
            {
                rehearsal.retain(|g| g != &track.guid);
                bounces.retain(|g| g != &track.guid);
                Tracks::remove(
                    &self.daw,
                    project.clone(),
                    TrackRef::Guid(track.guid.clone()),
                )?;
                arranged.unwrapped.push(track.name.clone());
            }
        }
        let all = Tracks::all(&self.daw, project.clone());

        // 2. Content: tracks with media, outside the bus / Keyflow tree.
        let keyflow_children: std::collections::HashSet<String> = {
            let mut inside = std::collections::HashSet::new();
            let mut depth: Option<i32> = None;
            let mut running = 0i32;
            for t in &all {
                if let Some(d) = depth {
                    if running > d {
                        inside.insert(t.guid.clone());
                    } else {
                        depth = None;
                    }
                }
                if t.name.trim().eq_ignore_ascii_case("Keyflow") && t.folder_depth > 0 {
                    depth = Some(running);
                }
                running += t.folder_depth;
            }
            inside
        };
        let content: Vec<&daw::service::Track> = all
            .iter()
            .filter(|t| {
                !crate::buses::is_bus_name(&t.name)
                    && !keyflow_children.contains(&t.guid)
                    && !rehearsal.contains(&t.guid)
                    && !bounces.contains(&t.guid)
                    && !in_unit.contains(&t.guid)
                    && self.has_items(&t.guid)
            })
            .collect();
        let names: Vec<String> = content.iter().map(|t| t.name.clone()).collect();
        let hierarchy = if names.is_empty() {
            crate::TrackHierarchy::default()
        } else {
            names
                .organize_into_tracks(&config, None)
                .map_err(|e| eyre::eyre!("grouping: {e}"))?
        };

        // 3. The desired layout: (guid, depth change) in order.
        let mut unused: Vec<(String, String)> = content
            .iter()
            .map(|t| (t.name.clone(), t.guid.clone()))
            .collect();
        let mut claim = |name: &str| -> Option<String> {
            let at = unused.iter().position(|(n, _)| n == name)?;
            Some(unused.remove(at).1)
        };
        let mut layout: Vec<(String, i32)> = Vec::new();
        // A kept folder is not a group folder to reuse.
        let mut folders_in_use: std::collections::HashSet<String> = in_unit.clone();
        // The folders open at this point of the layout, by name, and the
        // top-level ones laid out so far.
        let mut open: Vec<String> = Vec::new();
        let mut top_folders: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut prints_open = false;
        let is_print = |node: &daw_proto::TrackNode| {
            !node.is_folder && node.items.first().is_some_and(|n| prints.contains(n))
        };
        for (at, node) in hierarchy.tracks.iter().enumerate() {
            let depth = node.folder_depth_change.to_raw_value();
            if node.is_folder || node.items.is_empty() {
                // A group whose folder is already a kept block (the kit is
                // `Drums`, and a stray tom goes in Drums): the block, left
                // open, and the group's tracks after its own — one folder.
                let kept = (open.is_empty() && depth > 0)
                    .then(|| open_kept(&mut units, &all, &node.name))
                    .flatten();
                if let Some(unit) = kept {
                    layout.extend(unit);
                    top_folders.insert(node.name.to_lowercase());
                    open.push(node.name.clone());
                    continue;
                }
                let guid = self.group_folder(&node.name, &mut folders_in_use, &mut arranged)?;
                layout.push((guid, depth));
                if depth > 0 {
                    if open.is_empty() {
                        top_folders.insert(node.name.to_lowercase());
                    }
                    open.push(node.name.clone());
                }
            } else {
                // A leaf: the original track its item names. The engine
                // may give a leaf more than one item; the first names it.
                let Some(guid) = node.items.first().and_then(|n| claim(n)) else {
                    continue;
                };
                arranged.placed += 1;
                let mut depth = depth;
                // Alone at the top, it still goes in its instrument's folder:
                // a lone `Bass DI` is the Bass folder's, not a stray track.
                // (Not when the group already has its folder here: the
                // layout runs in order, and a second one is a duplicate.)
                let group = group_of
                    .get(&guid)
                    .filter(|g| g.as_str() != "Reference")
                    .filter(|g| !top_folders.contains(&g.to_lowercase()));
                if let (true, Some(group)) = (open.is_empty(), group) {
                    top_folders.insert(group.to_lowercase());
                    // Into the kept block of that name, if there is one.
                    if let Some(unit) = open_kept(&mut units, &all, group) {
                        layout.extend(unit);
                    } else {
                        let folder =
                            self.group_folder(group, &mut folders_in_use, &mut arranged)?;
                        layout.push((folder, 1));
                    }
                    depth = depth.saturating_sub(1);
                }
                // A print goes in PRINTS — even the only kind of guide track
                // there is, which the engine would leave bare in the Guide.
                if is_print(node) {
                    if !prints_open {
                        let folder = self.group_folder(
                            super::PRINTS_FOLDER,
                            &mut folders_in_use,
                            &mut arranged,
                        )?;
                        layout.push((folder, 1));
                        open.push(super::PRINTS_FOLDER.to_string());
                        prints_open = true;
                    }
                    // The last of a run closes it too.
                    let next = hierarchy.tracks.get(at.saturating_add(1));
                    if !next.is_some_and(&is_print) {
                        depth = depth.saturating_sub(1);
                        prints_open = false;
                    }
                }
                layout.push((guid, depth));
            }
            // Closes: the folders it ends.
            if depth < 0 {
                for _ in 0..depth.unsigned_abs() {
                    open.pop();
                }
            }
        }

        // The kept folders, whole, beside the groups.
        layout.extend(units.into_iter().flatten());

        // 4. The reference material, after the groups: REFERENCES, the
        //    rehearsal in REHEARSAL inside it, then the song's bounces.
        let references = self.references_layout(&rehearsal, &bounces, &mut arranged)?;
        layout.extend(references);

        // 5. Everything else keeps its order and depth, after them.
        let placed: std::collections::HashSet<&str> =
            layout.iter().map(|(g, _)| g.as_str()).collect();
        let rest: Vec<(String, i32)> = Tracks::all(&self.daw, project.clone())
            .into_iter()
            .filter(|t| !placed.contains(t.guid.as_str()))
            .map(|t| (t.guid, t.folder_depth))
            .collect();
        layout.extend(rest);

        self.apply_layout(&layout)?;
        Ok(arranged)
    }

    /// Hide the DI captures that sit beside the tracks meant to be used —
    /// an amp, a mic — in the track panel and the mixer. A DI is kept to
    /// reamp from, not to mix: `DI` and `DI Pedalboard` beside `Amp 1` and
    /// `Amp 2` are two lanes of noise between the guitar you play back.
    /// Only beside a sibling (same folder) that is not a capture and has
    /// audio, or nested under such a track; a DI alone — a bass recorded
    /// only DI — is the track, and stays. Returns the names hidden.
    /// Idempotent.
    ///
    /// # Errors
    ///
    /// A backend call failed.
    pub fn hide_captures(&mut self) -> eyre::Result<Vec<String>> {
        let project = self.project.clone();
        let all = Tracks::all(&self.daw, project.clone());
        // Each track's parent folder (by index), `None` at the top.
        let mut parents: Vec<Option<usize>> = Vec::with_capacity(all.len());
        let mut open: Vec<usize> = Vec::new();
        for (i, track) in all.iter().enumerate() {
            parents.push(open.last().copied());
            if track.folder_depth > 0 {
                open.push(i);
            } else if track.folder_depth < 0 {
                for _ in 0..track.folder_depth.unsigned_abs() {
                    open.pop();
                }
            }
        }
        let mut hidden = Vec::new();
        for (i, track) in all.iter().enumerate() {
            let Some(parent) = parents.get(i).copied().flatten() else {
                continue;
            };
            if track.folder_depth > 0 || !super::is_capture(&track.name) || !track.visible_in_tcp {
                continue;
            }
            let real_beside = all.iter().enumerate().any(|(j, sibling)| {
                j != i
                    && parents.get(j).copied().flatten() == Some(parent)
                    && sibling.folder_depth <= 0
                    && !super::is_capture(&sibling.name)
                    && self.has_items(&sibling.guid)
            });
            // Or nested under one: the bus pass nests a DI under its
            // primary (`nest_secondary_mics`), which makes the track it
            // stands beside its parent.
            let real_above = all
                .get(parent)
                .is_some_and(|p| !super::is_capture(&p.name) && self.has_items(&p.guid));
            if real_beside || real_above {
                Tracks::set_visibility(
                    &self.daw,
                    project.clone(),
                    TrackRef::Guid(track.guid.clone()),
                    false,
                    false,
                )?;
                hidden.push(track.name.clone());
            }
        }
        Ok(hidden)
    }

    /// Hide the buses from the track panel — the template's (`MIX BUS`
    /// and its tree), a session's own (`Guitar A BUS`), and a headphone
    /// mix. They are wiring: summed in the mixer, where they stay, and
    /// rows of nothing in an arrangement. Returns the names hidden.
    /// Idempotent.
    ///
    /// # Errors
    ///
    /// A backend call failed.
    pub fn hide_buses(&mut self) -> eyre::Result<Vec<String>> {
        let project = self.project.clone();
        let mut hidden = Vec::new();
        for track in Tracks::all(&self.daw, project.clone()) {
            if track.visible_in_tcp && super::is_bus_track(&track.name) {
                Tracks::set_visibility(
                    &self.daw,
                    project.clone(),
                    TrackRef::Guid(track.guid.clone()),
                    false,
                    track.visible_in_mixer,
                )?;
                hidden.push(track.name);
            }
        }
        Ok(hidden)
    }

    /// The folder `name` a group's tracks go in: one of the project's with
    /// no media and not already in use, or a new one.
    fn group_folder(
        &self,
        name: &str,
        in_use: &mut std::collections::HashSet<String>,
        arranged: &mut Arranged,
    ) -> eyre::Result<String> {
        let project = self.project.clone();
        let existing = Tracks::all(&self.daw, project.clone())
            .into_iter()
            .find(|t| {
                t.name.trim().eq_ignore_ascii_case(name.trim())
                    && !self.has_items(&t.guid)
                    && !in_use.contains(&t.guid)
            })
            .map(|t| t.guid);
        let guid = if let Some(g) = existing {
            g
        } else {
            arranged.created.push(name.to_string());
            Tracks::add(&self.daw, project, name, None)?
        };
        in_use.insert(guid.clone());
        Ok(guid)
    }

    /// The top-level folders kept whole by [`Self::arrange_into_groups`]:
    /// each a folder with no media of its own that classified (with its
    /// folders as context) and is not a bus, Keyflow, UNSORTED or reference
    /// material — with everything inside it, as `(guid, depth change)`.
    fn kept_folders(
        &self,
        all: &[daw::service::Track],
        classified: &std::collections::HashSet<String>,
        rehearsal: &[String],
        bounces: &[String],
    ) -> Vec<Vec<(String, i32)>> {
        let mut units = Vec::new();
        let mut running = 0i32;
        let mut i = 0;
        while let Some(track) = all.get(i) {
            let opens = running == 0 && track.folder_depth > 0;
            let name = track.name.trim();
            let keep = opens
                && classified.contains(&track.guid)
                && !rehearsal.contains(&track.guid)
                && !bounces.contains(&track.guid)
                && !crate::buses::is_bus_name(name)
                && !name.eq_ignore_ascii_case("Keyflow")
                && !name.eq_ignore_ascii_case(super::UNSORTED_FOLDER)
                && !name.eq_ignore_ascii_case(super::REFERENCES_FOLDER)
                && !self.has_items(&track.guid);
            if !keep {
                running = running.saturating_add(track.folder_depth);
                i = i.saturating_add(1);
                continue;
            }
            // The folder and everything until the depth comes back to the top.
            let mut unit = Vec::new();
            let mut depth = 0i32;
            while let Some(t) = all.get(i) {
                unit.push((t.guid.clone(), t.folder_depth));
                depth = depth.saturating_add(t.folder_depth);
                i = i.saturating_add(1);
                if depth <= 0 {
                    break;
                }
            }
            // Reference material inside it (a `DRUM REF` in the kit) is laid
            // out in REFERENCES, so it comes out — the track before it
            // taking over the folders it closed. A reference that is itself
            // a folder takes too much with it: the block is not kept.
            let reference = |g: &String| rehearsal.contains(g) || bounces.contains(g);
            if unit.iter().any(|(g, depth)| reference(g) && *depth > 0) {
                continue;
            }
            let mut kept: Vec<(String, i32)> = Vec::with_capacity(unit.len());
            for (guid, depth) in unit {
                if reference(&guid) {
                    if let Some(last) = kept.last_mut() {
                        last.1 = last.1.saturating_add(depth);
                    }
                } else {
                    kept.push((guid, depth));
                }
            }
            units.push(kept);
        }
        units
    }

    /// The REFERENCES folder's part of a layout: the folder (reused if the
    /// project has one), REHEARSAL inside it holding `rehearsal`, then
    /// `bounces` — each a `(guid, depth change)`. Empty when both are.
    fn references_layout(
        &self,
        rehearsal: &[String],
        bounces: &[String],
        arranged: &mut Arranged,
    ) -> eyre::Result<Vec<(String, i32)>> {
        if rehearsal.is_empty() && bounces.is_empty() {
            return Ok(Vec::new());
        }
        let mut layout = vec![(self.folder_named(super::REFERENCES_FOLDER, arranged)?, 1)];
        if !rehearsal.is_empty() {
            layout.push((self.folder_named(super::REHEARSAL_FOLDER, arranged)?, 1));
            layout.extend(rehearsal.iter().map(|g| (g.clone(), 0)));
            // The last of the rehearsal closes REHEARSAL — and REFERENCES
            // too, when there are no bounces after it.
            let closes = if bounces.is_empty() { -2 } else { -1 };
            if let Some(last) = layout.last_mut() {
                last.1 = closes;
            }
        }
        if !bounces.is_empty() {
            layout.extend(bounces.iter().map(|g| (g.clone(), 0)));
            if let Some(last) = layout.last_mut() {
                last.1 = -1;
            }
        }
        Ok(layout)
    }

    /// A folder track named `name` with no media of its own — the
    /// project's, or a new one.
    fn folder_named(&self, name: &str, arranged: &mut Arranged) -> eyre::Result<String> {
        let project = self.project.clone();
        let existing = Tracks::all(&self.daw, project.clone())
            .into_iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(name) && !self.has_items(&t.guid))
            .map(|t| t.guid);
        if let Some(guid) = existing {
            return Ok(guid);
        }
        arranged.created.push(name.to_string());
        Ok(Tracks::add(&self.daw, project, name, None)?)
    }

    /// Reorder the whole project to `layout` and set every depth from it.
    fn apply_layout(&mut self, layout: &[(String, i32)]) -> eyre::Result<()> {
        let project = self.project.clone();
        for (i, (guid, _)) in layout.iter().enumerate() {
            let current = Tracks::all(&self.daw, project.clone())
                .iter()
                .position(|t| &t.guid == guid);
            if current == Some(i) {
                continue;
            }
            Tracks::clear_selection(&self.daw, project.clone())?;
            Tracks::set_selected(
                &self.daw,
                project.clone(),
                TrackRef::Guid(guid.clone()),
                true,
            )?;
            Tracks::reorder_selected(
                &self.daw,
                project.clone(),
                u32::try_from(i).unwrap_or(u32::MAX),
                ReorderTracksBehavior::Normal,
            )?;
        }
        Tracks::clear_selection(&self.daw, project.clone())?;
        // The order is right; now say exactly where every folder opens
        // and closes, whatever the moves did to it on the way.
        for (guid, depth) in layout {
            Tracks::set_folder_depth(
                &self.daw,
                project.clone(),
                TrackRef::Guid(guid.clone()),
                *depth,
            )?;
        }
        Ok(())
    }

    /// Put the top level in the template's order: `first` (by folder
    /// name, in that order), then everything else as it stands, then
    /// `last`. Each top-level track moves with everything inside it.
    ///
    /// # Errors
    ///
    /// A backend call failed.
    pub fn order_top_level(&mut self, first: &[&str], last: &[&str]) -> eyre::Result<()> {
        let project = self.project.clone();
        let all = Tracks::all(&self.daw, project.clone());
        // Top-level blocks: a track at depth 0 and everything it opens.
        let mut blocks: Vec<Vec<(String, i32)>> = Vec::new();
        let mut running = 0i32;
        for t in &all {
            if running == 0 {
                blocks.push(Vec::new());
            }
            if let Some(block) = blocks.last_mut() {
                block.push((t.guid.clone(), t.folder_depth));
            }
            running += t.folder_depth;
        }
        let name_of = |block: &Vec<(String, i32)>| {
            all.iter()
                .find(|t| t.guid == block[0].0)
                .map(|t| t.name.trim().to_owned())
                .unwrap_or_default()
        };
        // The instruments between, by group in the template's order — an
        // `Electric GTR` and a `GTR E` are both the guitars'.
        let group_of: std::collections::HashMap<String, String> = super::classified_paths(self)
            .into_iter()
            .filter_map(|e| Some((e.track, e.path.first()?.clone())))
            .collect();
        let rank = |block: &Vec<(String, i32)>| {
            let name = name_of(block);
            if let Some(i) = first.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
                (0, i)
            } else if let Some(i) = last.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
                (2, i)
            } else {
                let group = block.first().and_then(|(g, _)| group_of.get(g));
                let at = group
                    .and_then(|g| {
                        INSTRUMENT_ORDER
                            .iter()
                            .position(|o| o.eq_ignore_ascii_case(g))
                    })
                    .unwrap_or(INSTRUMENT_ORDER.len());
                (1, at)
            }
        };
        let mut ordered = blocks.clone();
        ordered.sort_by_key(|b| rank(b)); // stable: the middle keeps its order
        if ordered == blocks {
            return Ok(());
        }
        let layout: Vec<(String, i32)> = ordered.into_iter().flatten().collect();
        self.apply_layout(&layout)
    }

    /// Hide the named folder track from the track panel (the mixer keeps
    /// it). Only the folder itself: a panel hides what a hidden folder
    /// holds, so unhiding this one track brings the whole tree back. For
    /// the MIX BUS — routing, not something to look at while playing.
    ///
    /// # Errors
    ///
    /// A backend call failed.
    pub fn hide_in_tcp(&mut self, folder: &str) -> eyre::Result<()> {
        let project = self.project.clone();
        if let Some(track) = Tracks::all(&self.daw, project.clone())
            .into_iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(folder))
        {
            Tracks::set_visibility(&self.daw, project, TrackRef::Guid(track.guid), false, true)?;
        }
        Ok(())
    }
}
