//! [`TemplateTarget`] over a live REAPER session — the in-process backend.
//!
//! The offline counterpart is [`super::dawfile`]; both are driven by
//! [`apply_buses`](super::apply_buses), so the bus tree this builds in a
//! running REAPER is the same one the file backend writes to an `.RPP`.
//!
//! Everything goes through the `daw::service` traits against the
//! `daw_reaper::Reaper` backend — never raw `reaper_low`/`reaper_medium` FFI.
//! These calls are synchronous and must run on REAPER's main thread, which is
//! where extension action handlers already are.

use daw::service::{Routing, Tracks};
use daw_proto::{FolderDepthChange, ProjectContext, ReorderTracksBehavior, TrackRef};
use daw_reaper::Reaper;

use super::TemplateTarget;

/// A live REAPER project the template can be applied to.
pub struct ReaperTarget {
    daw: Reaper,
    project: ProjectContext,
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

impl TemplateTarget for ReaperTarget {
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
        self.daw.set_color(
            self.project.clone(),
            TrackRef::Guid(id.clone()),
            color_u32,
        )
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

    fn gather_into_folder(
        &mut self,
        folder: &str,
        tracks: &[String],
    ) -> Result<Option<crate::apply::Gathered<String>>, Self::Error> {
        // Same rule as the file backend: a track carrying folder structure
        // cannot be pulled out without stranding what it held open.
        let all = self.daw.all(self.project.clone());
        let movable: Vec<String> = tracks
            .iter()
            .filter(|guid| {
                all.iter()
                    .find(|t| &&t.guid == guid)
                    .is_some_and(|t| t.folder_depth == 0)
            })
            .cloned()
            .collect();
        if movable.is_empty() {
            return Ok(None);
        }

        let folder_guid = self.daw.add(self.project.clone(), folder, None)?;
        let folder_index = u32::try_from(
            self
                .daw
                .all(self.project.clone())
                .iter()
                .position(|t| t.guid == folder_guid)
                .ok_or_else(|| {
                    daw_proto::DawError::NotFound(format!("track {folder_guid} vanished after add"))
                })?
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
