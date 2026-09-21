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
use daw_reaper::Reaper;

use super::TemplateTarget;

/// A live project the template can be applied to, on whatever backend `D`
/// is.
pub struct DawTarget<D> {
    daw: D,
    project: ProjectContext,
}

/// A live REAPER project — the extension actions' target.
pub type ReaperTarget = DawTarget<Reaper>;

impl<D> DawTarget<D> {
    /// Target `project` on `daw`.
    pub const fn on(daw: D, project: ProjectContext) -> Self {
        Self { daw, project }
    }
}

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
