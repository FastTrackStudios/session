use std::collections::HashSet;

use daw::service::{
    DawError, DawResult, ItemRef, Items, ProjectContext, Projects, Track, TrackRef, Tracks,
    TracksExt,
};
use dynamic_template::track_schema::{self, TrackDimension};

#[derive(Debug, Clone)]
pub struct TrackShape {
    pub name: String,
    pub children: Vec<TrackShape>,
}

pub fn init(_ctx: &daw::module::ModuleContext) {}

/// Session's track-tree editor for one DAW backend. This is what callers
/// actually use: `TrackManager::new(daw).add_channel()`, never
/// `daw.add_channel()` — adding a dynamic-template channel/multi-mic/
/// arrangement isn't a generic DAW capability, it's session business logic
/// layered on top of one (`daw::service::Tracks`/`Items`/`Projects`), so it
/// isn't blanket-impl'd onto every backend the way `daw::service::TracksExt`
/// (generic selection/lookup plumbing — `selected_scope`, `select`,
/// `children_of`, `get_track`, ...) is. `TrackManager<D>` wraps whichever
/// `D` for the duration it's used; it always acts on
/// `ProjectContext::Current` (matches how a REAPER named command works —
/// there's no "target a background project" for a user-triggered action).
///
/// Production wraps `daw::reaper::Reaper`; tests wrap
/// `daw_standalone::sync::Standalone` to drive the tree logic headless,
/// with no live REAPER process — same trait impl either way.
pub struct TrackManager<D> {
    daw: D,
}

impl<D> TrackManager<D> {
    pub fn new(daw: D) -> Self {
        Self { daw }
    }
}

/// Derefs to the wrapped backend so `Tracks`/`Items`/`Projects`/
/// `TracksExt` methods are callable directly as `self.method(...)` instead
/// of `self.daw.method(...)` everywhere below — `TrackManager`'s own
/// methods (`append_child`, `add_named_scope`, ...) still resolve to
/// themselves first, Rust only reaches through `Deref` for names `Self`
/// doesn't otherwise have.
impl<D> std::ops::Deref for TrackManager<D> {
    type Target = D;
    fn deref(&self) -> &D {
        &self.daw
    }
}

/// The seven REAPER-facing actions, declared with no bodies — `TrackManager<D>`
/// is the (only) implementor, below. `#[architect::actions(namespace =
/// "TRACK_MANAGER")]` turns each `#[action(...)]` method directly into a
/// REAPER named command (`register_track_manager_actions_actions` —
/// macro-generated — wires each one through `architect::action::
/// ActionBackend`) — no hand-written action-id enum, `action_for_id`
/// string-matcher, or dispatch bridge. This trait declares only its own
/// identity ("Track Manager") and knows nothing about being nested under
/// Session or FTS — that nesting is composed at registration time by
/// wrapping the backend in `architect::action::ScopedActionBackend` once
/// per level (see `register_actions` below), not by this trait naming its
/// ancestors. Each method returns `daw::service::DawResult<()>`; a failure
/// reaches the user as a REAPER message box (see `show_action_error` in
/// `daw-reaper`'s `ActionBackend` impl) instead of being silently logged.
#[architect::actions(namespace = "TRACK_MANAGER")]
pub trait TrackManagerActions {
    #[action(description = "Add the next dynamic-template channel to the selected track scope")]
    fn add_channel(&self) -> DawResult<()>;

    #[action(description = "Add the next dynamic-template layer to the selected track scope")]
    fn add_layer(&self) -> DawResult<()>;

    #[action(
        description = "Add the next dynamic-template multi-mic track to the selected track scope"
    )]
    fn add_multi_mic(&self) -> DawResult<()>;

    #[action(description = "Add a performer folder to the selected track scope")]
    fn add_performer(&self) -> DawResult<()>;

    #[action(
        description = "Add the next dynamic-template arrangement to the selected instrument scope"
    )]
    fn add_arrangement(&self) -> DawResult<()>;

    #[action(
        description = "Reorganize selected tracks with performer as the top metadata dimension"
    )]
    fn reorganize_selected_by_performer(&self) -> DawResult<()>;

    #[action(
        description = "Reorganize selected tracks with arrangement as the top metadata dimension"
    )]
    fn reorganize_selected_by_arrangement(&self) -> DawResult<()>;
}

impl<D: Tracks + Items + Projects> TrackManagerActions for TrackManager<D> {
    fn add_channel(&self) -> DawResult<()> {
        let project = ProjectContext::Current;
        let label = "Session Track Manager - Add Channel";
        self.begin_undo_block(project.clone(), label);
        let result = (|| -> DawResult<()> {
            let scope_info = self.selected_scope()?;
            let context = scope_context(&scope_info);
            let direct_children = self.children_of(&scope_info.guid);
            let child_names: HashSet<_> = direct_children
                .iter()
                .map(|track| track.name.as_str())
                .collect();

            let existing_channel = direct_children
                .iter()
                .find(|track| track_dimension(&track.name, &context) == TrackDimension::Channel);
            let channel_child_shape = existing_channel
                .map(|track| self.child_shapes(&track.guid))
                .unwrap_or_default();
            let initial_channels = ["L".to_string(), "R".to_string()];

            if !initial_channels
                .iter()
                .any(|name| child_names.contains(name.as_str()))
            {
                let direct_multi_mic_names: Vec<String> = direct_children
                    .iter()
                    .filter(|track| {
                        track_dimension(&track.name, &context) == TrackDimension::MultiMic
                    })
                    .map(|track| track.name.clone())
                    .collect();

                if !direct_multi_mic_names.is_empty() {
                    return self.convert_direct_multi_mics_to_channels(
                        &scope_info,
                        &direct_multi_mic_names,
                        &initial_channels,
                    );
                }

                self.set_folder_depth(project.clone(), TrackRef::Guid(scope_info.guid.clone()), 1)?;
                let l = self.add(
                    project.clone(),
                    &initial_channels[0],
                    Some(scope_info.index + 1),
                )?;
                let r = self.add(
                    project.clone(),
                    &initial_channels[1],
                    Some(scope_info.index + 2),
                )?;
                self.set_folder_depth(project.clone(), TrackRef::Guid(l.clone()), 0)?;
                self.set_folder_depth(project.clone(), TrackRef::Guid(r), -1)?;
                return self.move_items(&scope_info.guid, &l);
            }

            let next = track_schema::next_configured_value(
                TrackDimension::Channel,
                &context,
                child_names.iter().copied(),
            )
            .ok_or_else(|| DawError::not_found("configured channel name", &scope_info.name))?;
            if channel_child_shape.is_empty() {
                self.append_child(&scope_info.guid, &next, false)
            } else {
                self.append_shape(
                    &scope_info.guid,
                    &[TrackShape {
                        name: next,
                        children: channel_child_shape,
                    }],
                )
            }
        })();
        self.end_undo_block(project, label, None);
        result
    }

    fn add_layer(&self) -> DawResult<()> {
        let project = ProjectContext::Current;
        let label = "Session Track Manager - Add Layer";
        self.begin_undo_block(project.clone(), label);
        let result = self.add_named_scope("DBL", true);
        self.end_undo_block(project, label, None);
        result
    }

    fn add_multi_mic(&self) -> DawResult<()> {
        let project = ProjectContext::Current;
        let label = "Session Track Manager - Add Multi-Mic";
        self.begin_undo_block(project.clone(), label);
        let result = (|| -> DawResult<()> {
            let scope_info = self.selected_scope()?;
            let context = scope_context(&scope_info);
            let child_names: HashSet<_> = self
                .children_of(&scope_info.guid)
                .iter()
                .map(|track| track.name.clone())
                .collect();
            let next = track_schema::next_configured_value(
                TrackDimension::MultiMic,
                &context,
                child_names.iter().map(String::as_str),
            )
            .ok_or_else(|| DawError::not_found("configured multi-mic name", &scope_info.name))?;
            self.append_child(&scope_info.guid, &next, false)
        })();
        self.end_undo_block(project, label, None);
        result
    }

    fn add_performer(&self) -> DawResult<()> {
        let project = ProjectContext::Current;
        let label = "Session Track Manager - Add Performer";
        self.begin_undo_block(project.clone(), label);
        let result = self.add_named_scope("New Performer", true);
        self.end_undo_block(project, label, None);
        result
    }

    fn add_arrangement(&self) -> DawResult<()> {
        let project = ProjectContext::Current;
        let label = "Session Track Manager - Add Arrangement";
        self.begin_undo_block(project.clone(), label);
        let result = (|| -> DawResult<()> {
            let selected_info = self.selected_scope()?;
            let scope_info = self.instrument_scope(selected_info);
            self.append_child(&scope_info.guid, "<ArrangementDescriptor>", false)
        })();
        self.end_undo_block(project, label, None);
        result
    }

    fn reorganize_selected_by_performer(&self) -> DawResult<()> {
        self.reorganize_selected_by("Performer")
    }

    fn reorganize_selected_by_arrangement(&self) -> DawResult<()> {
        self.reorganize_selected_by("Arrangement")
    }
}

impl<D: Tracks + Items + Projects> TrackManager<D> {
    fn add_named_scope(&self, name: &str, inherit_channels: bool) -> DawResult<()> {
        let scope_info = self.selected_scope()?;
        let context = scope_context(&scope_info);

        let inherited_shape = if inherit_channels {
            let children = self.child_shapes(&scope_info.guid);
            let channel_shape: Vec<_> = children
                .iter()
                .filter(|track| track_dimension(&track.name, &context) == TrackDimension::Channel)
                .cloned()
                .collect();
            if channel_shape.is_empty() {
                children
                    .into_iter()
                    .filter(|track| {
                        track_dimension(&track.name, &context) == TrackDimension::MultiMic
                    })
                    .collect()
            } else {
                channel_shape
            }
        } else {
            Vec::new()
        };

        if inherited_shape.is_empty() {
            return self.append_child(&scope_info.guid, name, false);
        }

        self.append_shape(
            &scope_info.guid,
            &[TrackShape {
                name: name.to_string(),
                children: inherited_shape,
            }],
        )
    }

    /// `TrackShape`s (session's own tree-building type) for every direct
    /// child of `guid`, recursively.
    fn child_shapes(&self, guid: &str) -> Vec<TrackShape> {
        self.children_of(guid)
            .into_iter()
            .map(|track| TrackShape {
                name: track.name.clone(),
                children: self.child_shapes(&track.guid),
            })
            .collect()
    }

    fn append_child(&self, parent_guid: &str, name: &str, as_folder: bool) -> DawResult<()> {
        let project = ProjectContext::Current;
        let insertion_index = self.prepare_append(parent_guid)?;
        self.set_folder_depth(project.clone(), TrackRef::Guid(parent_guid.to_string()), 1)?;
        let child = self.add(project.clone(), name, Some(insertion_index))?;
        self.set_folder_depth(
            project,
            TrackRef::Guid(child),
            if as_folder { 1 } else { -1 },
        )
    }

    fn append_shape(&self, parent_guid: &str, shape: &[TrackShape]) -> DawResult<()> {
        let project = ProjectContext::Current;
        let insertion_index = self.prepare_append(parent_guid)?;
        self.set_folder_depth(project.clone(), TrackRef::Guid(parent_guid.to_string()), 1)?;

        let mut flattened = Vec::new();
        flatten_shape(shape, &mut flattened);
        if let Some(last) = flattened.last_mut() {
            last.1 -= 1;
        }

        for (offset, (name, folder_depth)) in flattened.into_iter().enumerate() {
            let track = self.add(project.clone(), &name, Some(insertion_index + offset as u32))?;
            self.set_folder_depth(project.clone(), TrackRef::Guid(track), folder_depth)?;
        }
        Ok(())
    }

    /// The index to insert a new last child of `parent_guid` at. If the
    /// track immediately before that index is a direct child of
    /// `parent_guid` that currently closes the folder (`folder_depth <
    /// 0`), reopens it (`folder_depth = 0`) first so the new sibling
    /// takes over closing it instead — computed once, before that
    /// reopen, since reopening changes what a fresh subtree-end walk
    /// would see.
    fn prepare_append(&self, parent_guid: &str) -> DawResult<u32> {
        let insertion_index = self
            .subtree_end_index(parent_guid)
            .unwrap_or(self.get_track(parent_guid)?.index + 1);
        let previous_index = insertion_index.saturating_sub(1);
        if let Some(previous) =
            Tracks::get(&self.daw, ProjectContext::Current, TrackRef::Index(previous_index))
            && previous.parent_guid.as_deref() == Some(parent_guid)
            && previous.folder_depth < 0
        {
            self.set_folder_depth(ProjectContext::Current, TrackRef::Guid(previous.guid), 0)?;
        }
        Ok(insertion_index)
    }

    fn convert_direct_multi_mics_to_channels(
        &self,
        scope_info: &Track,
        multi_mic_names: &[String],
        channel_names: &[String],
    ) -> DawResult<()> {
        if channel_names.len() < 2 {
            return Err(DawError::operation_failed(format!(
                "dynamic-template config does not define enough channel values for {}",
                scope_info.name
            )));
        }
        let project = ProjectContext::Current;
        let context = scope_context(scope_info);
        let direct_multi_mics: Vec<Track> = self
            .children_of(&scope_info.guid)
            .into_iter()
            .filter(|track| track_dimension(&track.name, &context) == TrackDimension::MultiMic)
            .collect();
        let Some(first_multi_mic) = direct_multi_mics.first() else {
            return Ok(());
        };

        self.set_folder_depth(project.clone(), TrackRef::Guid(scope_info.guid.clone()), 1)?;

        let l = self.add(
            project.clone(),
            &channel_names[0],
            Some(first_multi_mic.index),
        )?;
        self.set_folder_depth(project.clone(), TrackRef::Guid(l), 1)?;
        for (index, track) in direct_multi_mics.iter().enumerate() {
            self.set_folder_depth(
                project.clone(),
                TrackRef::Guid(track.guid.clone()),
                if index + 1 == direct_multi_mics.len() {
                    -1
                } else {
                    0
                },
            )?;
        }

        self.append_shape(
            &scope_info.guid,
            &[TrackShape {
                name: channel_names[1].clone(),
                children: multi_mic_names
                    .iter()
                    .map(|name| TrackShape {
                        name: name.clone(),
                        children: Vec::new(),
                    })
                    .collect(),
            }],
        )
    }

    /// Walk up from `track` while its ancestors are still classified as
    /// part of an instrument (guitar/keys/drums/...), stopping at the
    /// outermost one — that's the scope a new arrangement variant nests
    /// under, not whatever leaf track happened to be selected.
    fn instrument_scope(&self, track: Track) -> Track {
        let mut current = track;
        while let Some(parent_guid) = current.parent_guid.clone() {
            let Ok(parent) = self.get_track(&parent_guid) else {
                break;
            };
            let parent_context = scope_context(&parent);
            if track_dimension(&current.name, &parent_context) != TrackDimension::Other {
                current = parent;
                continue;
            }
            if let Some(grandparent_guid) = parent.parent_guid.clone()
                && let Ok(grandparent) = self.get_track(&grandparent_guid)
                && track_dimension(&parent.name, &scope_context(&grandparent))
                    != TrackDimension::Other
            {
                current = parent;
                continue;
            }
            break;
        }
        current
    }

    fn reorganize_selected_by(&self, field_name: &str) -> DawResult<()> {
        let _scope = self.selected_scope()?;
        tracing::warn!(
            "[session] Reorganize selected by {field_name} is registered; hierarchy rewrite policy is pending"
        );
        Ok(())
    }
}

fn flatten_shape(shape: &[TrackShape], output: &mut Vec<(String, i32)>) {
    for node in shape {
        let index = output.len();
        output.push((
            node.name.clone(),
            if node.children.is_empty() { 0 } else { 1 },
        ));
        if !node.children.is_empty() {
            flatten_shape(&node.children, output);
            if let Some(last) = output.last_mut() {
                last.1 -= 1;
            }
        }
        if node.children.is_empty() && output.len() == index {
            unreachable!("shape flattening must emit the current node");
        }
    }
}

fn track_dimension(name: &str, context: &[String]) -> TrackDimension {
    track_schema::classify_track_dimension(name, context)
}

fn scope_context(scope: &Track) -> Vec<String> {
    vec![scope.name.clone()]
}

/// Registers all seven track-manager actions with `backend`, dispatching
/// through a `TrackManager` wrapping `daw`, nested one level under
/// "Session" (see `TrackManagerActions`'s docs — the trait itself only
/// knows its own name).
pub fn register_actions<B, D>(backend: &B, daw: D)
where
    B: ::architect::action::ActionBackend + Clone,
    D: Tracks + Items + Projects + Send + Sync + 'static,
{
    let session =
        ::architect::action::ScopedActionBackend::new(backend.clone(), "SESSION", "Session");
    register_track_manager_actions_actions(&session, std::sync::Arc::new(TrackManager::new(daw)));
}
