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
        !Items::get_items(&self.daw, self.project.clone(), TrackRef::Guid(guid.to_owned())).is_empty()
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
        let all = Tracks::all(&self.daw, project.clone());

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
        //    only held a multitrack together. Its children stay.
        for track in &all {
            if track.folder_depth > 0 && !is_known_folder(&track.name) && !self.has_items(&track.guid) {
                Tracks::remove(&self.daw, project.clone(), TrackRef::Guid(track.guid.clone()))?;
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
                    && self.has_items(&t.guid)
            })
            .collect();
        if content.is_empty() {
            return Ok(arranged);
        }
        let names: Vec<String> = content.iter().map(|t| t.name.clone()).collect();
        let hierarchy = names
            .clone()
            .organize_into_tracks(&config, None)
            .map_err(|e| eyre::eyre!("grouping: {e}"))?;

        // 3. The desired layout: (guid, depth change) in order.
        let mut unused: Vec<(String, String)> =
            content.iter().map(|t| (t.name.clone(), t.guid.clone())).collect();
        let mut claim = |name: &str| -> Option<String> {
            let at = unused.iter().position(|(n, _)| n == name)?;
            Some(unused.remove(at).1)
        };
        let mut layout: Vec<(String, i32)> = Vec::new();
        let mut folders_in_use: std::collections::HashSet<String> = std::collections::HashSet::new();
        for node in &hierarchy.tracks {
            let depth = node.folder_depth_change.to_raw_value();
            if node.is_folder || node.items.is_empty() {
                // Reuse a folder of this name, or make one.
                let existing = Tracks::all(&self.daw, project.clone())
                    .into_iter()
                    .find(|t| {
                        t.name.trim().eq_ignore_ascii_case(node.name.trim())
                            && !self.has_items(&t.guid)
                            && !folders_in_use.contains(&t.guid)
                    })
                    .map(|t| t.guid);
                let guid = match existing {
                    Some(g) => g,
                    None => {
                        arranged.created.push(node.name.clone());
                        Tracks::add(&self.daw, project.clone(), &node.name, None)?
                    }
                };
                folders_in_use.insert(guid.clone());
                layout.push((guid, depth));
            } else {
                // A leaf: the original track its item names. The engine
                // may give a leaf more than one item; the first names it.
                let Some(guid) = node.items.first().and_then(|n| claim(n)) else {
                    continue;
                };
                arranged.placed += 1;
                layout.push((guid, depth));
            }
        }

        // 4. Everything else keeps its order and depth, after the groups.
        let placed: std::collections::HashSet<&str> = layout.iter().map(|(g, _)| g.as_str()).collect();
        let rest: Vec<(String, i32)> = Tracks::all(&self.daw, project.clone())
            .into_iter()
            .filter(|t| !placed.contains(t.guid.as_str()))
            .map(|t| (t.guid, t.folder_depth))
            .collect();
        layout.extend(rest);

        self.apply_layout(&layout)?;
        Ok(arranged)
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
            Tracks::set_selected(&self.daw, project.clone(), TrackRef::Guid(guid.clone()), true)?;
            Tracks::reorder_selected(&self.daw, 
                project.clone(),
                u32::try_from(i).unwrap_or(u32::MAX),
                ReorderTracksBehavior::Normal,
            )?;
        }
        Tracks::clear_selection(&self.daw, project.clone())?;
        // The order is right; now say exactly where every folder opens
        // and closes, whatever the moves did to it on the way.
        for (guid, depth) in layout {
            Tracks::set_folder_depth(&self.daw, project.clone(), TrackRef::Guid(guid.clone()), *depth)?;
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
        let rank = |block: &Vec<(String, i32)>| {
            let name = name_of(block);
            if let Some(i) = first.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
                (0, i)
            } else if let Some(i) = last.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
                (2, i)
            } else {
                (1, 0)
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
