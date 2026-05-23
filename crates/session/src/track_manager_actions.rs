use std::collections::HashSet;

use daw::service::{ItemRef, Items, ProjectContext, Projects, Track, TrackRef, Tracks};
use dynamic_template::track_schema::{self, TrackDimension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackManagerAction {
    AddChannel,
    AddLayer,
    AddMultiMic,
    AddPerformer,
    AddArrangement,
    ReorganizeSelectedByPerformer,
    ReorganizeSelectedByArrangement,
}

#[derive(Debug, Clone)]
struct TrackShape {
    name: String,
    children: Vec<TrackShape>,
}

pub fn init(_ctx: &daw::module::ModuleContext) {}

pub fn action_for_id(action_id: &str) -> Option<TrackManagerAction> {
    let normalized = action_id
        .trim()
        .to_lowercase()
        .strip_prefix("fts.session.")
        .unwrap_or(action_id.trim())
        .to_string();
    match normalized.as_str() {
        "track_manager_add_channel" => Some(TrackManagerAction::AddChannel),
        "track_manager_add_layer" => Some(TrackManagerAction::AddLayer),
        "track_manager_add_multi_mic" => Some(TrackManagerAction::AddMultiMic),
        "track_manager_add_performer" => Some(TrackManagerAction::AddPerformer),
        "track_manager_add_arrangement" => Some(TrackManagerAction::AddArrangement),
        "track_manager_reorganize_selected_by_performer" => {
            Some(TrackManagerAction::ReorganizeSelectedByPerformer)
        }
        "track_manager_reorganize_selected_by_arrangement" => {
            Some(TrackManagerAction::ReorganizeSelectedByArrangement)
        }
        _ => None,
    }
}

pub fn dispatch(action: TrackManagerAction) {
    if let Err(err) = run_action(action) {
        tracing::error!(?action, ?err, "[session] Track manager action failed");
    }
}

fn run_action(action: TrackManagerAction) -> eyre::Result<()> {
    match action {
        TrackManagerAction::AddChannel => {
            run_track_edit("Session Track Manager - Add Channel", add_channel)
        }
        TrackManagerAction::AddLayer => run_track_edit("Session Track Manager - Add Layer", || {
            add_named_scope("DBL", true)
        }),
        TrackManagerAction::AddMultiMic => {
            run_track_edit("Session Track Manager - Add Multi-Mic", || {
                add_next_multi_mic()
            })
        }
        TrackManagerAction::AddPerformer => {
            run_track_edit("Session Track Manager - Add Performer", || {
                add_named_scope("New Performer", true)
            })
        }
        TrackManagerAction::AddArrangement => {
            run_track_edit("Session Track Manager - Add Arrangement", add_arrangement)
        }
        TrackManagerAction::ReorganizeSelectedByPerformer => reorganize_selected_by("Performer"),
        TrackManagerAction::ReorganizeSelectedByArrangement => {
            reorganize_selected_by("Arrangement")
        }
    }
}

fn run_track_edit<F>(label: &str, edit: F) -> eyre::Result<()>
where
    F: FnOnce() -> eyre::Result<()>,
{
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let selected_guids: Vec<String> = reaper
        .selected(project.clone())
        .into_iter()
        .map(|track| track.guid)
        .collect();

    if selected_guids.is_empty() {
        eyre::bail!("Select a track before running a Session Track Manager action");
    }

    reaper.begin_undo_block(project.clone(), label);
    let edit_result = edit();
    let restore_result = restore_track_selection(&selected_guids);
    reaper.end_undo_block(project, label, None);

    edit_result?;
    restore_result?;
    Ok(())
}

fn restore_track_selection(guids: &[String]) -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    reaper.clear_selection(project.clone())?;
    for guid in guids {
        Tracks::set_selected(&reaper, project.clone(), TrackRef::Guid(guid.clone()), true)?;
    }
    Ok(())
}

fn selected_scope() -> eyre::Result<Track> {
    let reaper = daw::reaper::Reaper;
    let mut selected = reaper.selected(ProjectContext::Current);
    selected
        .drain(..)
        .next()
        .ok_or_else(|| eyre::eyre!("Select a track before running a Session Track Manager action"))
}

fn add_channel() -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let scope_info = selected_scope()?;
    let all = reaper.all(project.clone());
    let direct_children = direct_children(&all, &scope_info.guid);
    let context = scope_context(&scope_info);
    let child_names: HashSet<_> = direct_children
        .iter()
        .map(|track| track.name.as_str())
        .collect();

    let existing_channel = direct_children
        .iter()
        .find(|track| track_dimension(&track.name, &context) == TrackDimension::Channel)
        .copied();
    let channel_child_shape = existing_channel
        .map(|track| child_shapes(&all, &track.guid))
        .unwrap_or_default();
    let initial_channels = ["L".to_string(), "R".to_string()];

    if !initial_channels
        .iter()
        .any(|name| child_names.contains(name.as_str()))
    {
        let direct_multi_mic_names: Vec<String> = direct_children
            .iter()
            .filter(|track| track_dimension(&track.name, &context) == TrackDimension::MultiMic)
            .map(|track| track.name.clone())
            .collect();

        if !direct_multi_mic_names.is_empty() {
            convert_direct_multi_mics_to_channels(
                &scope_info,
                &direct_multi_mic_names,
                &initial_channels,
            )?;
            return Ok(());
        }

        reaper.set_folder_depth(project.clone(), TrackRef::Guid(scope_info.guid.clone()), 1)?;
        let l = reaper.add(
            project.clone(),
            &initial_channels[0],
            Some(scope_info.index + 1),
        )?;
        let r = reaper.add(
            project.clone(),
            &initial_channels[1],
            Some(scope_info.index + 2),
        )?;
        reaper.set_folder_depth(project.clone(), TrackRef::Guid(l.clone()), 0)?;
        reaper.set_folder_depth(project.clone(), TrackRef::Guid(r), -1)?;
        move_items(&scope_info.guid, &l)?;
        return Ok(());
    }

    let next = track_schema::next_configured_value(
        TrackDimension::Channel,
        &context,
        child_names.iter().copied(),
    )
    .ok_or_else(|| eyre::eyre!("No configured channel names remain for {}", scope_info.name))?;
    if channel_child_shape.is_empty() {
        append_child(&scope_info.guid, &next, false)?;
    } else {
        append_shape(
            &scope_info.guid,
            &[TrackShape {
                name: next,
                children: channel_child_shape,
            }],
        )?;
    }
    Ok(())
}

fn add_next_multi_mic() -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let scope_info = selected_scope()?;
    let all = reaper.all(project);
    let context = scope_context(&scope_info);
    let child_names: HashSet<_> = direct_children(&all, &scope_info.guid)
        .iter()
        .map(|track| track.name.as_str())
        .collect();
    let next = track_schema::next_configured_value(
        TrackDimension::MultiMic,
        &context,
        child_names.iter().copied(),
    )
    .ok_or_else(|| {
        eyre::eyre!(
            "No configured multi-mic names remain for {}",
            scope_info.name
        )
    })?;
    append_child(&scope_info.guid, &next, false)
}

fn add_arrangement() -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let selected_info = selected_scope()?;
    let all = reaper.all(project);
    let scope_info = instrument_scope(&all, &selected_info)
        .cloned()
        .unwrap_or(selected_info);

    append_child(&scope_info.guid, "<ArrangementDescriptor>", false)
}

fn add_named_scope(name: &str, inherit_channels: bool) -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let scope_info = selected_scope()?;
    let all = reaper.all(project);
    let context = scope_context(&scope_info);

    let inherited_shape = if inherit_channels {
        let children = child_shapes(&all, &scope_info.guid);
        let channel_shape: Vec<_> = children
            .iter()
            .filter(|track| track_dimension(&track.name, &context) == TrackDimension::Channel)
            .cloned()
            .collect();
        if channel_shape.is_empty() {
            children
                .into_iter()
                .filter(|track| track_dimension(&track.name, &context) == TrackDimension::MultiMic)
                .collect()
        } else {
            channel_shape
        }
    } else {
        Vec::new()
    };

    if inherited_shape.is_empty() {
        append_child(&scope_info.guid, name, false)?;
        return Ok(());
    }

    append_shape(
        &scope_info.guid,
        &[TrackShape {
            name: name.to_string(),
            children: inherited_shape,
        }],
    )?;
    Ok(())
}

fn append_child(parent_guid: &str, name: &str, as_folder: bool) -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let all = reaper.all(project.clone());
    let parent = all
        .iter()
        .find(|track| track.guid == parent_guid)
        .ok_or_else(|| eyre::eyre!("Selected track no longer exists"))?;
    let insertion_index = subtree_end_index(&all, parent_guid).unwrap_or(parent.index + 1);
    let previous_index = insertion_index.saturating_sub(1);
    if let Some(previous) = all.iter().find(|track| track.index == previous_index)
        && previous.parent_guid.as_deref() == Some(parent_guid)
        && previous.folder_depth < 0
    {
        reaper.set_folder_depth(project.clone(), TrackRef::Guid(previous.guid.clone()), 0)?;
    }
    reaper.set_folder_depth(project.clone(), TrackRef::Guid(parent_guid.to_string()), 1)?;
    let child = reaper.add(project.clone(), name, Some(insertion_index))?;
    reaper.set_folder_depth(
        project,
        TrackRef::Guid(child),
        if as_folder { 1 } else { -1 },
    )?;
    Ok(())
}

fn append_shape(parent_guid: &str, shape: &[TrackShape]) -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let all = reaper.all(project.clone());
    let parent = all
        .iter()
        .find(|track| track.guid == parent_guid)
        .ok_or_else(|| eyre::eyre!("Selected track no longer exists"))?;
    let insertion_index = subtree_end_index(&all, parent_guid).unwrap_or(parent.index + 1);
    let previous_index = insertion_index.saturating_sub(1);
    if let Some(previous) = all.iter().find(|track| track.index == previous_index)
        && previous.parent_guid.as_deref() == Some(parent_guid)
        && previous.folder_depth < 0
    {
        reaper.set_folder_depth(project.clone(), TrackRef::Guid(previous.guid.clone()), 0)?;
    }
    reaper.set_folder_depth(project.clone(), TrackRef::Guid(parent_guid.to_string()), 1)?;

    let mut flattened = Vec::new();
    flatten_shape(shape, &mut flattened);
    if let Some(last) = flattened.last_mut() {
        last.1 -= 1;
    }

    for (offset, (name, folder_depth)) in flattened.into_iter().enumerate() {
        let track = reaper.add(
            project.clone(),
            &name,
            Some(insertion_index + offset as u32),
        )?;
        reaper.set_folder_depth(project.clone(), TrackRef::Guid(track), folder_depth)?;
    }
    Ok(())
}

fn convert_direct_multi_mics_to_channels(
    scope_info: &Track,
    multi_mic_names: &[String],
    channel_names: &[String],
) -> eyre::Result<()> {
    if channel_names.len() < 2 {
        eyre::bail!(
            "Dynamic-template config does not define enough channel values for {}",
            scope_info.name
        );
    }
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let all = reaper.all(project.clone());
    let context = scope_context(scope_info);
    let direct_multi_mics = direct_children(&all, &scope_info.guid)
        .into_iter()
        .filter(|track| track_dimension(&track.name, &context) == TrackDimension::MultiMic)
        .collect::<Vec<_>>();
    let Some(first_multi_mic) = direct_multi_mics.first() else {
        return Ok(());
    };

    reaper.set_folder_depth(project.clone(), TrackRef::Guid(scope_info.guid.clone()), 1)?;

    let l = reaper.add(
        project.clone(),
        &channel_names[0],
        Some(first_multi_mic.index),
    )?;
    reaper.set_folder_depth(project.clone(), TrackRef::Guid(l), 1)?;
    for (index, track) in direct_multi_mics.iter().enumerate() {
        reaper.set_folder_depth(
            project.clone(),
            TrackRef::Guid(track.guid.clone()),
            if index + 1 == direct_multi_mics.len() {
                -1
            } else {
                0
            },
        )?;
    }

    append_shape(
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

fn move_items(from_guid: &str, to_guid: &str) -> eyre::Result<()> {
    let reaper = daw::reaper::Reaper;
    let project = ProjectContext::Current;
    let target = TrackRef::Guid(to_guid.to_string());
    for item in reaper.get_items(project.clone(), TrackRef::Guid(from_guid.to_string())) {
        reaper.move_to_track(project.clone(), ItemRef::Guid(item.guid), target.clone())?;
    }
    Ok(())
}

fn reorganize_selected_by(field_name: &str) -> eyre::Result<()> {
    let _scope = selected_scope()?;
    tracing::warn!(
        "[session] Reorganize selected by {field_name} is registered; hierarchy rewrite policy is pending"
    );
    Ok(())
}

fn direct_children<'a>(
    tracks: &'a [daw::service::Track],
    parent_guid: &str,
) -> Vec<&'a daw::service::Track> {
    tracks
        .iter()
        .filter(|track| track.parent_guid.as_deref() == Some(parent_guid))
        .collect()
}

fn child_shapes(tracks: &[daw::service::Track], parent_guid: &str) -> Vec<TrackShape> {
    direct_children(tracks, parent_guid)
        .into_iter()
        .map(|track| TrackShape {
            name: track.name.clone(),
            children: child_shapes(tracks, &track.guid),
        })
        .collect()
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

fn scope_context(scope: &daw::service::Track) -> Vec<String> {
    vec![scope.name.clone()]
}

fn instrument_scope<'a>(
    tracks: &'a [daw::service::Track],
    selected: &'a daw::service::Track,
) -> Option<&'a daw::service::Track> {
    let mut current = selected;
    while let Some(parent) = parent_track(tracks, current) {
        if track_dimension(&current.name, &scope_context(parent)) != TrackDimension::Other {
            current = parent;
            continue;
        }
        if let Some(grandparent) = parent_track(tracks, parent)
            && track_dimension(&parent.name, &scope_context(grandparent)) != TrackDimension::Other
        {
            current = parent;
            continue;
        }
        break;
    }
    Some(current)
}

fn parent_track<'a>(
    tracks: &'a [daw::service::Track],
    track: &daw::service::Track,
) -> Option<&'a daw::service::Track> {
    let parent_guid = track.parent_guid.as_deref()?;
    tracks
        .iter()
        .find(|candidate| candidate.guid == parent_guid)
}

fn subtree_end_index(tracks: &[daw::service::Track], parent_guid: &str) -> Option<u32> {
    let parent = tracks.iter().find(|track| track.guid == parent_guid)?;
    let mut depth = 0;
    for track in tracks.iter().filter(|track| track.index >= parent.index) {
        depth += track.folder_depth;
        if track.index > parent.index && depth <= 0 {
            return Some(track.index + 1);
        }
    }
    Some(parent.index + 1)
}
