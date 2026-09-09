//! `impl Regions for Standalone` — post-architect::rpc port.
//!
//! Backed by `ProjectState::regions: BTreeMap<u32, Region>` in the
//! existing in-memory state. The old async `StandaloneRegion` impl
//! (~500 LOC with parallel `RegionState` mock-data scaffolding +
//! event streams) retired in favor of operating directly on the
//! canonical state.

use daw_proto::Regions;
use daw_proto::region::{RegionEvent, RegionStreamEvent};
use daw_proto::{DawError, DawResult, ProjectContext, Region};

use crate::sync::Standalone;

fn resolve_project(daw: &Standalone, ctx: &ProjectContext) -> Option<String> {
    match ctx {
        ProjectContext::Project(guid) => Some(guid.clone()),
        ProjectContext::Current => {
            let state = daw.state.lock().ok()?;
            state.current_project_guid.clone()
        }
    }
}

fn publish_region_event(daw: &Standalone, project_guid: &str, event: RegionEvent) {
    let event = RegionStreamEvent {
        project_guid: project_guid.to_string(),
        event,
    };
    daw.bus_events
        .publish(daw_proto::event_bus::DawEvent::Region(event.clone()));
    daw.region_events.publish(event);
}

impl daw_proto::region::RegionsStreamSource for Standalone {
    fn events_hub(&self) -> &architect::PubSub<RegionStreamEvent> {
        &self.region_events
    }
}

impl Regions for Standalone {
    fn all(&self, project: ProjectContext) -> Vec<Region> {
        let Some(guid) = resolve_project(self, &project) else {
            return Vec::new();
        };
        self.with_project(&guid, |p| p.regions.values().cloned().collect())
            .unwrap_or_default()
    }

    fn get(&self, project: ProjectContext, id: u32) -> Option<Region> {
        let guid = resolve_project(self, &project)?;
        self.with_project(&guid, |p| p.regions.get(&id).cloned())
            .ok()
            .flatten()
    }

    fn count(&self, project: ProjectContext) -> u32 {
        let Some(guid) = resolve_project(self, &project) else {
            return 0;
        };
        self.with_project(&guid, |p| p.regions.len() as u32)
            .unwrap_or(0)
    }

    fn add(&self, project: ProjectContext, start: f64, end: f64, name: &str) -> DawResult<u32> {
        let guid = resolve_project(self, &project)
            .ok_or_else(|| DawError::not_found("Project", "current"))?;
        let (id, region) = self.with_project_mut(&guid, |p| {
            let id = p.next_region_id;
            p.next_region_id += 1;
            let mut region = Region::from_seconds(start, end, name.to_string());
            region.id = Some(id);
            p.regions.insert(id, region.clone());
            (id, region)
        })?;
        publish_region_event(self, &guid, RegionEvent::Added(region));
        Ok(id)
    }

    fn remove(&self, project: ProjectContext, id: u32) -> DawResult<()> {
        let guid = resolve_project(self, &project)
            .ok_or_else(|| DawError::not_found("Project", "current"))?;
        self.with_project_mut(&guid, |p| {
            p.regions
                .remove(&id)
                .map(|_| ())
                .ok_or_else(|| DawError::not_found("Region", &id.to_string()))
        })??;
        publish_region_event(self, &guid, RegionEvent::Removed(id));
        Ok(())
    }

    fn set_bounds(&self, project: ProjectContext, id: u32, start: f64, end: f64) -> DawResult<()> {
        let guid = resolve_project(self, &project)
            .ok_or_else(|| DawError::not_found("Project", "current"))?;
        let region = self.with_project_mut(&guid, |p| {
            let r = p
                .regions
                .get_mut(&id)
                .ok_or_else(|| DawError::not_found("Region", &id.to_string()))?;
            r.time_range = daw_proto::TimeRange::from_seconds(start, end);
            Ok::<_, DawError>(r.clone())
        })??;
        publish_region_event(self, &guid, RegionEvent::Changed(region));
        Ok(())
    }

    fn rename(&self, project: ProjectContext, id: u32, name: &str) -> DawResult<()> {
        let guid = resolve_project(self, &project)
            .ok_or_else(|| DawError::not_found("Project", "current"))?;
        let region = self.with_project_mut(&guid, |p| {
            let r = p
                .regions
                .get_mut(&id)
                .ok_or_else(|| DawError::not_found("Region", &id.to_string()))?;
            r.name = name.to_string();
            Ok::<_, DawError>(r.clone())
        })??;
        publish_region_event(self, &guid, RegionEvent::Changed(region));
        Ok(())
    }

    fn set_color(&self, project: ProjectContext, id: u32, color: u32) -> DawResult<()> {
        let guid = resolve_project(self, &project)
            .ok_or_else(|| DawError::not_found("Project", "current"))?;
        let region = self.with_project_mut(&guid, |p| {
            let r = p
                .regions
                .get_mut(&id)
                .ok_or_else(|| DawError::not_found("Region", &id.to_string()))?;
            r.color = if color == 0 { None } else { Some(color) };
            Ok::<_, DawError>(r.clone())
        })??;
        publish_region_event(self, &guid, RegionEvent::Changed(region));
        Ok(())
    }

    fn set_lane(&self, project: ProjectContext, id: u32, lane: Option<u32>) -> DawResult<()> {
        let guid = resolve_project(self, &project)
            .ok_or_else(|| DawError::not_found("Project", "current"))?;
        let region = self.with_project_mut(&guid, |p| {
            let r = p
                .regions
                .get_mut(&id)
                .ok_or_else(|| DawError::not_found("Region", &id.to_string()))?;
            r.lane = lane;
            Ok::<_, DawError>(r.clone())
        })??;
        publish_region_event(self, &guid, RegionEvent::Changed(region));
        Ok(())
    }
}
