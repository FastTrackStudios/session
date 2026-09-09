//! `impl Tracks for Standalone` — post-architect::rpc port.
//!
//! Backed by `ProjectState::tracks: Vec<Track>` in the existing
//! in-memory state. Master track is synthesized on demand. The old
//! 400+-line async `StandaloneTrack` impl with parallel `TrackState`
//! storage was retired in favor of operating directly on the
//! canonical state — fewer places for state to drift.

use daw_proto::Tracks;
use daw_proto::track::{ReorderTracksBehavior, TrackEvent, TrackStreamEvent};
use daw_proto::{DawError, DawResult, ProjectContext, RecordInput, Track, TrackRef};
use uuid::Uuid;

use crate::sync::{Standalone, TrackExt};

fn resolve_project(daw: &Standalone, ctx: &ProjectContext) -> Option<String> {
    match ctx {
        ProjectContext::Project(guid) => Some(guid.clone()),
        ProjectContext::Current => {
            let state = daw.state.lock().ok()?;
            state.current_project_guid.clone()
        }
    }
}

fn find_track_index(tracks: &[Track], r: &TrackRef) -> Option<usize> {
    match r {
        TrackRef::Guid(guid) => tracks.iter().position(|t| t.guid == *guid),
        TrackRef::Index(idx) => {
            let i = *idx as usize;
            if i < tracks.len() { Some(i) } else { None }
        }
        TrackRef::Master => None,
    }
}

/// Tracks ganged to a control gesture on `origin`: every other track
/// whose FOLLOW mask for the parameter intersects the origin's LEAD
/// mask (REAPER's grouping matrix — touching a lead's control moves
/// the followers' controls).
fn group_followers(
    tracks: &[Track],
    origin: usize,
    lead_of: impl Fn(&daw_proto::track::TrackGrouping) -> u64,
    follow_of: impl Fn(&daw_proto::track::TrackGrouping) -> u64,
) -> Vec<usize> {
    let lead_mask = lead_of(&tracks[origin].grouping);
    if lead_mask == 0 {
        return Vec::new();
    }
    tracks
        .iter()
        .enumerate()
        .filter(|(j, t)| *j != origin && follow_of(&t.grouping) & lead_mask != 0)
        .map(|(j, _)| j)
        .collect()
}

fn not_found_proj() -> DawError {
    DawError::not_found("Project", "context")
}

fn not_found_track() -> DawError {
    DawError::not_found("Track", "")
}

fn reconcile_track_structure(tracks: &mut [Track]) {
    let mut folder_stack: Vec<String> = Vec::new();
    for (index, track) in tracks.iter_mut().enumerate() {
        track.index = index as u32;
        track.parent_guid = folder_stack.last().cloned();
        track.is_folder = track.folder_depth > 0;

        if track.folder_depth > 0 {
            folder_stack.push(track.guid.clone());
        } else if track.folder_depth < 0 {
            for _ in 0..track.folder_depth.unsigned_abs() {
                folder_stack.pop();
            }
        }
    }
}

fn selected_reorder_insert_index(tracks: &[Track], index: u32) -> usize {
    let target = (index as usize).min(tracks.len());
    let selected_before_target = tracks
        .iter()
        .take(target)
        .filter(|track| track.selected)
        .count();
    target.saturating_sub(selected_before_target)
}

fn moved_events(before: &[Track], after: &[Track]) -> Vec<TrackEvent> {
    let old_indices = before
        .iter()
        .map(|track| (track.guid.as_str(), track.index))
        .collect::<std::collections::HashMap<_, _>>();

    after
        .iter()
        .filter_map(|track| {
            let old_index = old_indices.get(track.guid.as_str()).copied()?;
            (old_index != track.index).then(|| TrackEvent::Moved {
                guid: track.guid.clone(),
                old_index,
                new_index: track.index,
            })
        })
        .collect()
}

fn publish_track_events(daw: &Standalone, project_guid: &str, events: Vec<TrackEvent>) {
    for event in events {
        let event = TrackStreamEvent {
            project_guid: project_guid.to_string(),
            event,
        };
        daw.bus_events
            .publish(daw_proto::event_bus::DawEvent::Track(event.clone()));
        daw.track_events.publish(event);
    }
}

impl daw_proto::track::TracksStreamSource for Standalone {
    fn events_hub(&self) -> &architect::PubSub<TrackStreamEvent> {
        &self.track_events
    }
}

impl Tracks for Standalone {
    fn all(&self, project: ProjectContext) -> Vec<Track> {
        let Some(guid) = resolve_project(self, &project) else {
            return Vec::new();
        };
        self.with_project(&guid, |p| p.tracks.clone())
            .unwrap_or_default()
    }

    fn get(&self, project: ProjectContext, track: TrackRef) -> Option<Track> {
        let guid = resolve_project(self, &project)?;
        self.with_project(&guid, |p| {
            find_track_index(&p.tracks, &track).map(|i| p.tracks[i].clone())
        })
        .ok()
        .flatten()
    }

    fn count(&self, project: ProjectContext) -> u32 {
        let Some(guid) = resolve_project(self, &project) else {
            return 0;
        };
        self.with_project(&guid, |p| p.tracks.len() as u32)
            .unwrap_or(0)
    }

    fn selected(&self, project: ProjectContext) -> Vec<Track> {
        let Some(guid) = resolve_project(self, &project) else {
            return Vec::new();
        };
        self.with_project(&guid, |p| {
            p.tracks.iter().filter(|t| t.selected).cloned().collect()
        })
        .unwrap_or_default()
    }

    fn master(&self, project: ProjectContext) -> Option<Track> {
        // Standalone synthesizes a master track on demand — there's no
        // persistent master row in `ProjectState::tracks`.
        let _ = project;
        Some(Track {
            guid: "master".to_string(),
            index: 0,
            name: "MASTER".to_string(),
            ..Default::default()
        })
    }

    fn set_automation_mode(
        &self,
        project: ProjectContext,
        track: TrackRef,
        mode: daw_proto::primitives::AutomationMode,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let event = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            p.tracks[i].automation_mode = mode;
            let track_guid = p.tracks[i].guid.clone();
            // The track mode IS the envelopes' mode — propagate so the
            // existing touch/latch/write machinery records (or stops
            // recording) immediately. REAPER's I_AUTOMODE semantics.
            for (key, data) in p.envelopes.iter_mut() {
                if key.0 == track_guid {
                    data.automation_mode = mode;
                }
            }
            Ok::<_, DawError>(TrackEvent::AutomationModeChanged {
                guid: track_guid,
                mode,
            })
        })??;
        publish_track_events(self, &guid, vec![event]);
        Ok(())
    }

    fn set_input_monitor(
        &self,
        project: ProjectContext,
        track: TrackRef,
        monitor: daw_proto::track::InputMonitoringMode,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let event = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            p.tracks[i].input_monitor = monitor;
            Ok::<_, DawError>(TrackEvent::InputMonitorChanged {
                guid: p.tracks[i].guid.clone(),
                monitor,
            })
        })??;
        publish_track_events(self, &guid, vec![event]);
        Ok(())
    }

    fn set_phase_inverted(
        &self,
        project: ProjectContext,
        track: TrackRef,
        inverted: bool,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let mut events = Vec::new();
            // Polarity group: gang the flip.
            for j in group_followers(&p.tracks, i, |g| g.polarity_lead, |g| g.polarity_follow) {
                p.tracks[j].phase_inverted = inverted;
                events.push(TrackEvent::PhaseInvertedChanged {
                    guid: p.tracks[j].guid.clone(),
                    inverted,
                });
            }
            p.tracks[i].phase_inverted = inverted;
            events.push(TrackEvent::PhaseInvertedChanged {
                guid: p.tracks[i].guid.clone(),
                inverted,
            });
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_muted(&self, project: ProjectContext, track: TrackRef, muted: bool) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let mut events = Vec::new();
            // Mute group: the gesture gangs to followers.
            for j in group_followers(&p.tracks, i, |g| g.mute_lead, |g| g.mute_follow) {
                p.tracks[j].muted = muted;
                events.push(TrackEvent::MuteChanged {
                    guid: p.tracks[j].guid.clone(),
                    muted,
                });
            }
            p.tracks[i].muted = muted;
            events.push(TrackEvent::MuteChanged {
                guid: p.tracks[i].guid.clone(),
                muted,
            });
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_soloed(&self, project: ProjectContext, track: TrackRef, soloed: bool) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let mut events = Vec::new();
            for j in group_followers(&p.tracks, i, |g| g.solo_lead, |g| g.solo_follow) {
                p.tracks[j].soloed = soloed;
                events.push(TrackEvent::SoloChanged {
                    guid: p.tracks[j].guid.clone(),
                    soloed,
                });
            }
            p.tracks[i].soloed = soloed;
            events.push(TrackEvent::SoloChanged {
                guid: p.tracks[i].guid.clone(),
                soloed,
            });
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_solo_exclusive(&self, project: ProjectContext, track: TrackRef) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let target = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let target_guid = p.tracks[target].guid.clone();
            let mut events = Vec::new();
            for t in p.tracks.iter_mut() {
                let next = t.guid == target_guid;
                if t.soloed != next {
                    t.soloed = next;
                    events.push(TrackEvent::SoloChanged {
                        guid: t.guid.clone(),
                        soloed: next,
                    });
                }
            }
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn clear_all_solo(&self, project: ProjectContext) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let mut events = Vec::new();
            for t in p.tracks.iter_mut() {
                if t.soloed {
                    t.soloed = false;
                    events.push(TrackEvent::SoloChanged {
                        guid: t.guid.clone(),
                        soloed: false,
                    });
                }
            }
            events
        })?;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_armed(&self, project: ProjectContext, track: TrackRef, armed: bool) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let mut events = Vec::new();
            // Record-arm group: arming the lead arms every follower
            // (the showcase session's "BAND RECORD VCA" arms the band).
            for j in group_followers(&p.tracks, i, |g| g.recarm_lead, |g| g.recarm_follow) {
                p.tracks[j].armed = armed;
                events.push(TrackEvent::ArmChanged {
                    guid: p.tracks[j].guid.clone(),
                    armed,
                });
            }
            p.tracks[i].armed = armed;
            events.push(TrackEvent::ArmChanged {
                guid: p.tracks[i].guid.clone(),
                armed,
            });
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_volume(&self, project: ProjectContext, track: TrackRef, volume: f64) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let mut events = Vec::new();
            // Volume group: gestures gang RELATIVELY (a dB offset =
            // a linear ratio), matching REAPER's fader linking.
            let old = p.tracks[i].volume;
            let ratio = if old.abs() > 1e-12 { volume / old } else { 0.0 };
            for j in group_followers(&p.tracks, i, |g| g.volume_lead, |g| g.volume_follow) {
                let next = if ratio > 0.0 {
                    p.tracks[j].volume * ratio
                } else {
                    volume
                };
                p.tracks[j].volume = next;
                events.push(TrackEvent::VolumeChanged {
                    guid: p.tracks[j].guid.clone(),
                    volume: next,
                });
            }
            p.tracks[i].volume = volume;
            events.push(TrackEvent::VolumeChanged {
                guid: p.tracks[i].guid.clone(),
                volume,
            });
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_pan(&self, project: ProjectContext, track: TrackRef, pan: f64) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let pan = pan.clamp(-1.0, 1.0);
            let mut events = Vec::new();
            // Pan group: gang the gesture as an additive offset.
            let delta = pan - p.tracks[i].pan;
            for j in group_followers(&p.tracks, i, |g| g.pan_lead, |g| g.pan_follow) {
                let next = (p.tracks[j].pan + delta).clamp(-1.0, 1.0);
                p.tracks[j].pan = next;
                events.push(TrackEvent::PanChanged {
                    guid: p.tracks[j].guid.clone(),
                    pan: next,
                });
            }
            p.tracks[i].pan = pan;
            events.push(TrackEvent::PanChanged {
                guid: p.tracks[i].guid.clone(),
                pan,
            });
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    // Track-group slots are a REAPER concept; standalone has no equivalent,
    // so these are no-ops (naming/membership) and "no free slot" (query).
    fn set_group_name(&self, _project: ProjectContext, _slot: u32, _name: &str) -> DawResult<()> {
        Ok(())
    }

    fn first_free_group_slot(
        &self,
        _project: ProjectContext,
        _band_start: u32,
        _band_end: u32,
    ) -> Option<u32> {
        None
    }

    fn set_group_membership(
        &self,
        _project: ProjectContext,
        _track: TrackRef,
        _slot: u32,
        _member: bool,
    ) -> DawResult<()> {
        Ok(())
    }

    fn set_selected(
        &self,
        project: ProjectContext,
        track: TrackRef,
        selected: bool,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let event = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let track_guid = p.tracks[i].guid.clone();
            p.tracks[i].selected = selected;
            Ok::<_, DawError>(TrackEvent::SelectionChanged {
                guid: track_guid,
                selected,
            })
        })??;
        publish_track_events(self, &guid, vec![event]);
        Ok(())
    }

    fn select_exclusive(&self, project: ProjectContext, track: TrackRef) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let target = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let target_guid = p.tracks[target].guid.clone();
            let mut events = Vec::new();
            for t in p.tracks.iter_mut() {
                let next = t.guid == target_guid;
                if t.selected != next {
                    t.selected = next;
                    events.push(TrackEvent::SelectionChanged {
                        guid: t.guid.clone(),
                        selected: next,
                    });
                }
            }
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn clear_selection(&self, project: ProjectContext) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let mut events = Vec::new();
            for t in p.tracks.iter_mut() {
                if t.selected {
                    t.selected = false;
                    events.push(TrackEvent::SelectionChanged {
                        guid: t.guid.clone(),
                        selected: false,
                    });
                }
            }
            events
        })?;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn mute_all(&self, project: ProjectContext) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let mut events = Vec::new();
            for t in p.tracks.iter_mut() {
                if !t.muted {
                    t.muted = true;
                    events.push(TrackEvent::MuteChanged {
                        guid: t.guid.clone(),
                        muted: true,
                    });
                }
            }
            events
        })?;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn unmute_all(&self, project: ProjectContext) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let mut events = Vec::new();
            for t in p.tracks.iter_mut() {
                if t.muted {
                    t.muted = false;
                    events.push(TrackEvent::MuteChanged {
                        guid: t.guid.clone(),
                        muted: false,
                    });
                }
            }
            events
        })?;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn add(&self, project: ProjectContext, name: &str, at_index: Option<u32>) -> DawResult<String> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let (new_guid, events) = self.with_project_mut(&guid, |p| {
            let before = p.tracks.clone();
            let new_guid = Uuid::new_v4().to_string();
            let pos = at_index
                .map(|i| (i as usize).min(p.tracks.len()))
                .unwrap_or(p.tracks.len());
            let track = Track {
                guid: new_guid.clone(),
                index: pos as u32,
                name: name.to_string(),
                ..Default::default()
            };
            p.tracks.insert(pos, track);
            reconcile_track_structure(&mut p.tracks);
            let added = p.tracks[pos].clone();
            let mut events = vec![TrackEvent::Added(added)];
            events.extend(moved_events(&before, &p.tracks));
            (new_guid, events)
        })?;
        publish_track_events(self, &guid, events);
        Ok(new_guid)
    }

    fn remove(&self, project: ProjectContext, track: TrackRef) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let before = p.tracks.clone();
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let removed = p.tracks[i].guid.clone();
            p.tracks.remove(i);
            p.track_ext.remove(&removed);
            p.track_ext_state
                .retain(|(track_guid, _, _), _| track_guid != &removed);
            p.sends.remove(&removed);
            p.receives.remove(&removed);
            p.hw_outputs.remove(&removed);
            for sends in p.sends.values_mut() {
                sends.retain(|route| route.dest_track_guid.as_deref() != Some(removed.as_str()));
                for (idx, route) in sends.iter_mut().enumerate() {
                    route.index = idx as u32;
                }
            }
            for receives in p.receives.values_mut() {
                receives.retain(|route| route.source_track_guid != removed);
                for (idx, route) in receives.iter_mut().enumerate() {
                    route.index = idx as u32;
                }
            }
            reconcile_track_structure(&mut p.tracks);
            let mut events = vec![TrackEvent::Removed(removed)];
            events.extend(moved_events(&before, &p.tracks));
            Ok::<_, DawError>(events)
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn remove_all(&self, project: ProjectContext) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let events = p
                .tracks
                .iter()
                .map(|track| TrackEvent::Removed(track.guid.clone()))
                .collect::<Vec<_>>();
            p.tracks.clear();
            p.track_ext.clear();
            p.track_ext_state.clear();
            p.sends.clear();
            p.receives.clear();
            p.hw_outputs.clear();
            events
        })?;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn rename(&self, project: ProjectContext, track: TrackRef, name: &str) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let event = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let track_guid = p.tracks[i].guid.clone();
            p.tracks[i].name = name.to_string();
            Ok::<_, DawError>(TrackEvent::Renamed {
                guid: track_guid,
                name: name.to_string(),
            })
        })??;
        publish_track_events(self, &guid, vec![event]);
        Ok(())
    }

    fn set_color(&self, project: ProjectContext, track: TrackRef, color: u32) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let event = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let track_guid = p.tracks[i].guid.clone();
            let color = if color == 0 { None } else { Some(color) };
            p.tracks[i].color = color;
            Ok::<_, DawError>(TrackEvent::ColorChanged {
                guid: track_guid,
                color,
            })
        })??;
        publish_track_events(self, &guid, vec![event]);
        Ok(())
    }

    fn set_folder_depth(
        &self,
        project: ProjectContext,
        track: TrackRef,
        folder_depth: i32,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let before = p.tracks.clone();
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            p.tracks[i].folder_depth = folder_depth;
            reconcile_track_structure(&mut p.tracks);
            Ok::<_, DawError>(moved_events(&before, &p.tracks))
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_num_channels(
        &self,
        project: ProjectContext,
        track: TrackRef,
        num_channels: u32,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        // REAPER caps tracks at 128 channels. Allow any count 1..=128 —
        // standalone doesn't enforce stereo pairing.
        let n = num_channels.clamp(1, 128);
        self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let track_guid = p.tracks[i].guid.clone();
            p.track_ext
                .entry(track_guid)
                .or_insert_with(TrackExt::default)
                .num_channels = n;
            Ok::<(), DawError>(())
        })?
    }

    fn set_record_input(
        &self,
        project: ProjectContext,
        track: TrackRef,
        input: RecordInput,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let track_guid = p.tracks[i].guid.clone();
            // Mirrored onto the stored `Track` as well as the side map:
            // a strip reads `Track::record_input` from the bulk read, and
            // the two must not disagree.
            p.tracks[i].record_input = input;
            p.track_ext
                .entry(track_guid)
                .or_insert_with(TrackExt::default)
                .record_input = input;
            Ok::<(), DawError>(())
        })?
    }

    fn reorder_selected(
        &self,
        project: ProjectContext,
        index: u32,
        behavior: ReorderTracksBehavior,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let before = p.tracks.clone();
            if !p.tracks.iter().any(|track| track.selected) {
                return Ok::<_, DawError>(Vec::new());
            }

            let insert_at = selected_reorder_insert_index(&p.tracks, index);
            let mut selected = Vec::new();
            let mut remaining = Vec::with_capacity(p.tracks.len());
            for track in p.tracks.drain(..) {
                if track.selected {
                    selected.push(track);
                } else {
                    remaining.push(track);
                }
            }

            let insert_at = insert_at.min(remaining.len());
            if matches!(
                behavior,
                ReorderTracksBehavior::MakeChildOfPreviousTrack
                    | ReorderTracksBehavior::ExtendFolder
            ) && insert_at > 0
            {
                remaining[insert_at - 1].folder_depth =
                    remaining[insert_at - 1].folder_depth.max(1);
            }
            remaining.splice(insert_at..insert_at, selected);
            p.tracks = remaining;
            reconcile_track_structure(&mut p.tracks);
            Ok::<_, DawError>(moved_events(&before, &p.tracks))
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_visibility(
        &self,
        project: ProjectContext,
        track: TrackRef,
        visible_in_tcp: bool,
        visible_in_mixer: bool,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        let events = self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let track_guid = p.tracks[i].guid.clone();
            p.tracks[i].visible_in_tcp = visible_in_tcp;
            p.tracks[i].visible_in_mixer = visible_in_mixer;
            Ok::<_, DawError>(vec![
                TrackEvent::TcpVisibilityChanged {
                    guid: track_guid.clone(),
                    visible: visible_in_tcp,
                },
                TrackEvent::MixerVisibilityChanged {
                    guid: track_guid,
                    visible: visible_in_mixer,
                },
            ])
        })??;
        publish_track_events(self, &guid, events);
        Ok(())
    }

    fn set_tcp_height(
        &self,
        project: ProjectContext,
        track: TrackRef,
        height_pixels: u32,
    ) -> DawResult<()> {
        let guid = resolve_project(self, &project).ok_or_else(not_found_proj)?;
        self.with_project_mut(&guid, |p| {
            let i = find_track_index(&p.tracks, &track).ok_or_else(not_found_track)?;
            let track_guid = p.tracks[i].guid.clone();
            p.track_ext
                .entry(track_guid)
                .or_insert_with(TrackExt::default)
                .tcp_height_pixels = height_pixels;
            Ok::<(), DawError>(())
        })?
    }
}

// ── Inherent helpers for reading extended track state ────────────────
//
// These don't go through the `Tracks` proto trait (it has no
// `get_num_channels` / `get_record_input` methods today) — routing,
// mixer code, and tests can call them directly on `Standalone`.

impl Standalone {
    /// Channel count for a track. Returns 2 (stereo default) if the
    /// track exists but no override has been set; `None` if the track
    /// can't be resolved.
    pub fn track_num_channels(&self, project: &ProjectContext, track: &TrackRef) -> Option<u32> {
        let guid = resolve_project(self, project)?;
        self.with_project(&guid, |p| {
            let i = find_track_index(&p.tracks, track)?;
            let g = &p.tracks[i].guid;
            Some(p.track_ext.get(g).map(|e| e.num_channels).unwrap_or(2))
        })
        .ok()
        .flatten()
    }

    /// Record input for a track. Returns `RecordInput::None` if unset;
    /// `None` if the track can't be resolved.
    pub fn track_record_input(
        &self,
        project: &ProjectContext,
        track: &TrackRef,
    ) -> Option<RecordInput> {
        let guid = resolve_project(self, project)?;
        self.with_project(&guid, |p| {
            let i = find_track_index(&p.tracks, track)?;
            let g = &p.tracks[i].guid;
            Some(
                p.track_ext
                    .get(g)
                    .map(|e| e.record_input)
                    .unwrap_or(RecordInput::None),
            )
        })
        .ok()
        .flatten()
    }

    /// TCP height override for a track. Returns `0` for host/default
    /// height, or `None` if the track can't be resolved.
    pub fn track_tcp_height(&self, project: &ProjectContext, track: &TrackRef) -> Option<u32> {
        let guid = resolve_project(self, project)?;
        self.with_project(&guid, |p| {
            let i = find_track_index(&p.tracks, track)?;
            let g = &p.tracks[i].guid;
            Some(p.track_ext.get(g).map(|e| e.tcp_height_pixels).unwrap_or(0))
        })
        .ok()
        .flatten()
    }
}
