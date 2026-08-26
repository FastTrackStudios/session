//! DawModule implementation for dynamic-template.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use daw::module::{ActionDef, DawModule, ModuleContext};
use daw::service::{ExtState, ProjectContext, Tracks};
use daw_reaper::track::{
    add_track_on_main_thread, set_folder_depth_on_main_thread, set_tcp_height_on_main_thread,
    set_visibility_on_main_thread,
};

use crate::{
    default_config, monarchy_sort, track_schema, ItemMetadata, OrganizeIntoTracks, Structure,
};
/// Every action this module declares, in one list.
///
/// The four `#[architect::actions]` traits below are the single source of
/// truth for ids, names and descriptions; REAPER registration (`actions()`)
/// and the command-name dispatch in `handle_action` both read from here, so
/// an action cannot be registered without a handler or vice versa.
fn architect_metas() -> Vec<&'static architect::action::ActionMeta> {
    DynamicTemplateActionsActions::all()
        .iter()
        .chain(VisibilityManagerActionsActions::all())
        .chain(CreateGroupActionsActions::all())
        .chain(ToggleGroupActionsActions::all())
        .collect()
}

struct State {
    group_cache: HashMap<String, Vec<String>>,
}

struct CreateTemplateSpec {
    command_suffix: &'static str,
    folders: &'static [&'static str],
    tracks: &'static [&'static str],
}

const CREATE_STATE_SECTION: &str = "FTSDYNAMICTEMPLATE";
const CREATE_STATE_KEY_PREFIX: &str = "create.track.";

static STATE: std::sync::OnceLock<Arc<Mutex<State>>> = std::sync::OnceLock::new();

fn state() -> Arc<Mutex<State>> {
    STATE
        .get_or_init(|| {
            Arc::new(Mutex::new(State {
                group_cache: HashMap::new(),
            }))
        })
        .clone()
}

pub struct DynamicTemplateModule;

impl DawModule for DynamicTemplateModule {
    fn name(&self) -> &str {
        "dynamic-template"
    }

    fn display_name(&self) -> &str {
        "Dynamic Template"
    }

    fn actions(&self) -> Vec<ActionDef> {
        architect_metas()
            .into_iter()
            .map(|m| {
                ActionDef::new(m.id.to_string(), m.display_name.to_string(), move || {
                    dispatch(m.id)
                })
            })
            .collect()
    }

    fn init(&self, _ctx: &ModuleContext) {
        tracing::info!("[dynamic-template] runtime initialized");
    }

    fn subscribe(&self, _ctx: &ModuleContext) {
        // Auto-colour's initial pass and its reactive re-application now
        // live in `session::color`, which owns the enable/disable state
        // and persists what it applied. Nothing to hook here.
    }
}

fn dispatch(command_name: &str) {
    let state = state();
    tracing::info!("[dynamic-template] dispatching action {command_name}");
    if let Err(err) = handle_action(command_name, &state) {
        tracing::warn!("[dynamic-template] action failed for {command_name}: {err}");
    }
}

/// Compatibility shim for the `FTS_SESSION_*` aliases of this module's
/// actions.
///
/// These are a *second* name for actions this module already registers as
/// `FTS_DYNAMIC_TEMPLATE_*`. They exist only because committed FTS config
/// still binds the old names: `reaper-input`'s `tracks.styx` /
/// `mode-organize.styx` keybindings and `fts-icons`' `tracks.toml`
/// toolbar assignments. Retiring the aliases means repointing those files
/// at the real ids *and* re-running `fts-icons build --install`, so it is
/// a deliberate, sequenced change — not a refactor.
///
/// Every alias with no committed binding has already been deleted (the
/// visibility toggles, show-all/hide-all, the visibility profiles,
/// rebuild-cache, organize-session). What remains is exactly what config
/// still points at.
///
/// Returns `false` for anything it doesn't recognise, so the caller can
/// fall through.
pub fn dispatch_session_command(command_name: &str) -> bool {
    let mapped = match command_name {
        "FTS_SESSION_ORGANIZE_EVERYTHING" => "FTS_DYNAMIC_TEMPLATE_SORT_ALL".to_string(),
        "FTS_SESSION_ORGANIZE_SELECTED_TRACKS" => "FTS_DYNAMIC_TEMPLATE_SORT_SELECTED".to_string(),
        command_name => {
            if let Some(suffix) = command_name.strip_prefix("FTS_SESSION_CREATE_NEW_") {
                let suffix = match suffix {
                    "ELECTRONIC_DRUMS" => "ELECTRONIC_KIT",
                    "SYNTH_BASS" => "BASS_SYNTH",
                    suffix => suffix,
                };
                format!("FTS_DYNAMIC_TEMPLATE_CREATE_NEW_{suffix}")
            } else {
                return false;
            }
        }
    };
    dispatch(&mapped);
    true
}

/// Actions that are declared and registered but have no working handler.
///
/// `import_and_sort` needs to apply a track hierarchy to the project, which
/// the current DAW facade no longer exposes — the same gap `sort_tracks`
/// warns about. It stays declared so its REAPER command id keeps its slot in
/// user keymaps, and it is listed here so `every_registered_action_has_a_
/// dispatch_arm` stays honest instead of being loosened into uselessness.
const UNIMPLEMENTED: &[&str] = &["FTS_DYNAMIC_TEMPLATE_IMPORT_AND_SORT"];

/// Whether `handle_action` recognises `id` — the match arms below, minus the
/// REAPER calls, so a test can walk every registered id without a project.
#[cfg_attr(not(test), allow(dead_code))]
fn is_dispatchable(id: &str) -> bool {
    const KNOWN: &[&str] = &[
        "FTS_DYNAMIC_TEMPLATE_SORT_SELECTED",
        "FTS_DYNAMIC_TEMPLATE_SORT_ALL",
        "FTS_DYNAMIC_TEMPLATE_LOG_STATUS",
        "FTS_DYNAMIC_TEMPLATE_LOG_GROUPS",
        "FTS_DYNAMIC_TEMPLATE_ORGANIZE_DEMO",
        "FTS_VISIBILITY_MANAGER_SHOW_ALL",
        "FTS_VISIBILITY_MANAGER_HIDE_ALL",
        "FTS_VISIBILITY_MANAGER_REBUILD_CACHE",
    ];
    const PREFIXES: &[&str] = &[
        "FTS_VISIBILITY_MANAGER_TOGGLE_",
        "FTS_VISIBILITY_MANAGER_PROFILE_",
        "FTS_VISIBILITY_MANAGER_MODE_",
        "FTS_DYNAMIC_TEMPLATE_CREATE_NEW_",
    ];

    KNOWN.contains(&id) || UNIMPLEMENTED.contains(&id) || PREFIXES.iter().any(|p| id.starts_with(p))
}

fn handle_action(command_name: &str, state: &Arc<Mutex<State>>) -> eyre::Result<()> {
    let sort_selected = SORT_SELECTED.id;
    let sort_all = SORT_ALL.id;
    let log_status = LOG_STATUS.id;
    let log_groups = LOG_GROUPS.id;
    let show_all_cmd = SHOW_ALL.id;
    let hide_all_cmd = HIDE_ALL.id;
    let rebuild_cache_cmd = REBUILD_CACHE.id;
    let vis_toggle_prefix = "FTS_VISIBILITY_MANAGER_TOGGLE_";
    let vis_profile_prefix = "FTS_VISIBILITY_MANAGER_PROFILE_";
    let vis_mode_prefix = "FTS_VISIBILITY_MANAGER_MODE_";
    let create_prefix = "FTS_DYNAMIC_TEMPLATE_CREATE_NEW_";

    match command_name {
        n if n == sort_selected => sort_tracks(true)?,
        n if n == sort_all => sort_tracks(false)?,
        n if n == log_status => log_status_action(state),
        n if n == log_groups => log_groups_action(),
        n if n == ORGANIZE_DEMO.id => organize_demo_action()?,
        n if n == show_all_cmd => show_all_tracks()?,
        n if n == hide_all_cmd => hide_all_group_tracks(state)?,
        n if n == rebuild_cache_cmd => {
            let cache = rebuild_group_cache()?;
            tracing::info!(
                "[dynamic-template] rebuilt visibility cache for {} groups",
                cache.len()
            );
            state.lock().unwrap().group_cache = cache;
        }
        cmd if cmd.starts_with(vis_toggle_prefix) => {
            let group = cmd.strip_prefix(vis_toggle_prefix).unwrap();
            toggle_group_visibility(state, group)?;
        }
        cmd if cmd.starts_with(vis_profile_prefix) => {
            let profile = cmd.strip_prefix(vis_profile_prefix).unwrap();
            apply_visibility_profile(profile)?;
        }
        cmd if cmd.starts_with(vis_mode_prefix) => {
            let slug = cmd.strip_prefix(vis_mode_prefix).unwrap().to_lowercase();
            apply_mode_visibility(&slug)?;
        }
        cmd if cmd.starts_with(create_prefix) => {
            let suffix = cmd.strip_prefix(create_prefix).unwrap();
            create_template_group(suffix)?;
        }
        n if UNIMPLEMENTED.contains(&n) => {
            tracing::warn!("[dynamic-template] {command_name} is declared but not implemented yet")
        }
        _ => tracing::debug!("[dynamic-template] unhandled action: {command_name}"),
    }
    Ok(())
}

fn project() -> ProjectContext {
    ProjectContext::Current
}

fn selected_or_all_tracks(selected_only: bool) -> Vec<daw::service::Track> {
    if selected_only {
        daw_reaper::Reaper.selected(project())
    } else {
        daw_reaper::Reaper.all(project())
    }
}

fn sort_tracks(selected_only: bool) -> eyre::Result<()> {
    let source = selected_or_all_tracks(selected_only);
    if source.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = source.iter().map(|t| t.name.clone()).collect();
    let config = default_config();
    let hierarchy = names.organize_into_tracks(&config, None)?;
    tracing::warn!(
        "[dynamic-template] sort skipped for {} tracks; current DAW facade no longer exposes hierarchy apply",
        hierarchy.tracks.len()
    );
    Ok(())
}

fn show_all_tracks() -> eyre::Result<()> {
    for track in daw_reaper::Reaper.all(project()) {
        set_track_visibility(&track.guid, true)?;
        set_track_height(&track.guid, 0)?;
    }
    Ok(())
}

fn hide_all_group_tracks(state: &Arc<Mutex<State>>) -> eyre::Result<()> {
    let cache = ensure_group_cache(state)?;
    let target_names: HashSet<String> = cache.values().flatten().cloned().collect();
    set_named_tracks_visible(&target_names, false)?;
    tracing::info!(
        "[dynamic-template] hid {} classified group tracks",
        target_names.len()
    );
    Ok(())
}

fn toggle_group_visibility(state: &Arc<Mutex<State>>, group_name: &str) -> eyre::Result<()> {
    let cache = ensure_group_cache(state)?;
    let key = normalize_key(group_name);
    let Some(names) = cache.get(&key) else {
        tracing::info!("[dynamic-template] no tracks matched visibility group {group_name}");
        return Ok(());
    };
    let target_names: HashSet<String> = names.iter().cloned().collect();
    let infos = daw_reaper::Reaper.all(project());
    let should_show = !infos
        .iter()
        .filter(|track| target_names.contains(&track.name))
        .any(|track| track.visible_in_tcp || track.visible_in_mixer);

    set_named_tracks_visible(&target_names, should_show)?;
    tracing::info!(
        "[dynamic-template] {} {} tracks for visibility group {group_name}",
        if should_show { "showed" } else { "hid" },
        target_names.len()
    );
    Ok(())
}

fn set_named_tracks_visible(target_names: &HashSet<String>, visible: bool) -> eyre::Result<()> {
    for track in daw_reaper::Reaper.all(project()) {
        if !target_names.contains(&track.name) {
            continue;
        }
        set_track_visibility(&track.guid, visible)?;
    }
    Ok(())
}

fn set_track_visibility(guid: &str, visible: bool) -> eyre::Result<()> {
    set_visibility_on_main_thread(guid, visible, visible)
        .map_err(|err| eyre::eyre!("failed to set visibility for track {guid}: {err}"))
}

/// Apply a session mode's rule-based, per-surface visibility to the live
/// session. Resolves the mode's rules against the current track list (taxonomy
/// + folder role) and applies arrange/mixer visibility plus folder-collapse.
///
/// No-op (logged) for modes without a rule set. Fired by the
/// `FTS_VISIBILITY_MANAGER_MODE_<SLUG>` actions on a mode switch.
fn apply_mode_visibility(slug: &str) -> eyre::Result<()> {
    use crate::visibility_rules::{self, TrackInput};

    let Some(mode) = visibility_rules::mode_visibility_for(slug) else {
        tracing::info!("[dynamic-template] no visibility rules for mode '{slug}' — left as-is");
        return Ok(());
    };

    let config = default_config();
    let tracks: Vec<TrackInput> = daw_reaper::Reaper
        .all(project())
        .into_iter()
        .map(|t| TrackInput {
            guid: t.guid,
            name: t.name,
            index: t.index,
            is_folder: t.is_folder,
        })
        .collect();

    let plans = visibility_rules::resolve(&tracks, &config, &mode);
    let mut fold_pending = 0usize;
    for plan in &plans {
        set_visibility_on_main_thread(&plan.guid, plan.arrange_show, plan.mixer_show)
            .map_err(|err| eyre::eyre!("set visibility for {}: {err}", plan.guid))?;
        // Folder-collapse (arrange `I_FOLDERCOMPACT` + mixer `BUSCOMP`) is planned
        // here but its application is pending `daw_reaper::track::
        // set_folder_compact_on_main_thread`, which lives in the local daw
        // checkout and isn't yet published to the git dep this builds against.
        // Re-enable once that primitive lands. See visibility_rules::TrackPlan.
        if plan.arrange_fold.is_some() || plan.mixer_fold.is_some() {
            fold_pending += 1;
        }
    }

    tracing::info!(
        "[dynamic-template] applied '{slug}' mode visibility to {} tracks ({fold_pending} folder-collapse(s) planned, pending daw-reaper primitive)",
        plans.len()
    );
    Ok(())
}

fn set_track_height(guid: &str, height_pixels: u32) -> eyre::Result<()> {
    set_tcp_height_on_main_thread(guid, height_pixels)
        .map_err(|err| eyre::eyre!("failed to set height for track {guid}: {err}"))
}

fn apply_visibility_profile(profile: &str) -> eyre::Result<()> {
    let infos = daw_reaper::Reaper.all(project());
    let visible_count = infos
        .iter()
        .filter(|track| profile_matches_track(profile, &track.name))
        .count();
    let focused_height = profile_track_height(visible_count);

    for track in infos {
        let visible = profile_matches_track(profile, &track.name);
        set_track_visibility(&track.guid, visible)?;
        set_track_height(&track.guid, if visible { focused_height } else { 0 })?;
    }

    tracing::info!(
        "[dynamic-template] applied visibility profile {profile}: {} visible tracks",
        visible_count
    );
    Ok(())
}

fn profile_matches_track(profile: &str, track_name: &str) -> bool {
    let classification = track_schema::classify_track(track_name);
    match profile {
        "DRUM_EDITING" => classification
            .visibility_groups
            .iter()
            .any(|group| normalize_key(group) == "drums"),
        "MIDI_EDITING" => classification.visibility_groups.iter().any(|group| {
            matches!(
                normalize_key(group).as_str(),
                "drums" | "percussion" | "keys" | "synths" | "orchestra" | "strings" | "horns"
            )
        }),
        _ => false,
    }
}

fn profile_track_height(visible_count: usize) -> u32 {
    match visible_count {
        0 => 0,
        1..=4 => 180,
        5..=8 => 128,
        9..=16 => 92,
        _ => 64,
    }
}

fn ensure_group_cache(state: &Arc<Mutex<State>>) -> eyre::Result<HashMap<String, Vec<String>>> {
    let existing = state.lock().unwrap().group_cache.clone();
    if !existing.is_empty() {
        return Ok(existing);
    }
    let cache = rebuild_group_cache()?;
    state.lock().unwrap().group_cache = cache.clone();
    Ok(cache)
}

fn rebuild_group_cache() -> eyre::Result<HashMap<String, Vec<String>>> {
    let names: Vec<String> = daw_reaper::Reaper
        .all(project())
        .into_iter()
        .map(|t| t.name)
        .collect();
    let structure = monarchy_sort(names, &default_config())?;
    let mut cache = HashMap::new();
    collect_group_cache(&structure, &mut Vec::new(), &mut cache);
    for names in cache.values_mut() {
        names.sort();
        names.dedup();
    }
    Ok(cache)
}

fn collect_group_cache(
    structure: &Structure<ItemMetadata>,
    path: &mut Vec<String>,
    cache: &mut HashMap<String, Vec<String>>,
) {
    let pushed = !structure.name.is_empty() && structure.name != "root";
    if pushed {
        path.push(structure.name.clone());
    }

    for item in &structure.items {
        for group in path.iter() {
            cache
                .entry(normalize_key(group))
                .or_default()
                .push(item.original.clone());
        }
        if !path.is_empty() {
            cache
                .entry(normalize_key(&path.join("_")))
                .or_default()
                .push(item.original.clone());
        }
    }

    for child in &structure.children {
        collect_group_cache(child, path, cache);
    }

    if pushed {
        path.pop();
    }
}

fn log_status_action(state: &Arc<Mutex<State>>) {
    let locked = state.lock().unwrap();
    tracing::info!(
        "[dynamic-template] status: cached_groups={}",
        locked.group_cache.len()
    );
}

fn log_groups_action() {
    let groups = [
        "Drums",
        "Percussion",
        "Bass",
        "Guitars",
        "Keys",
        "Synths",
        "Horns",
        "Harmonica",
        "Strings",
        "Vocals",
        "Choir",
        "Orchestra",
        "SFX",
        "Guide",
        "Reference",
        "Stem Split",
    ];
    tracing::info!(
        "[dynamic-template] configured groups: {}",
        groups.join(", ")
    );
}

/// Run the organizer over a built-in set of track names and log the shape it
/// produces.
///
/// A dev action: it touches no project, so it answers "is the organizer
/// behaving?" from inside REAPER without needing a session to sacrifice.
fn organize_demo_action() -> eyre::Result<()> {
    const SAMPLE: &[&str] = &[
        "Kick In",
        "Kick Out",
        "Snare Top",
        "Snare Btm",
        "OH L",
        "OH R",
        "Bass DI",
        "Bass Amp",
        "Gtr L",
        "Gtr R",
        "Lead Vox",
        "BGV 1",
        "BGV 2",
    ];

    let names: Vec<String> = SAMPLE.iter().map(|n| n.to_string()).collect();
    let hierarchy = names.organize_into_tracks(&default_config(), None)?;

    tracing::info!(
        "[dynamic-template] demo: {} names organized into {} tracks",
        SAMPLE.len(),
        hierarchy.tracks.len(),
    );
    for track in &hierarchy.tracks {
        tracing::info!("[dynamic-template] demo track: {}", track.name);
    }
    Ok(())
}

fn create_template_group(command_suffix: &str) -> eyre::Result<()> {
    let Some(spec) = create_template_specs()
        .iter()
        .find(|spec| spec.command_suffix == command_suffix)
    else {
        tracing::warn!(
            "[dynamic-template] unknown create-template action suffix: {command_suffix}"
        );
        return Ok(());
    };

    let command_suffix = spec.command_suffix;
    let folders = spec.folders;
    let tracks = spec.tracks;
    daw_reaper::main_thread::run(move || {
        let project_tracks = current_project_tracks();
        let existing: HashSet<String> = project_tracks
            .iter()
            .map(|track| track.name.clone())
            .collect();
        let reaper = daw_reaper::Reaper;
        let project = ProjectContext::Current;

        if !is_drum_create_action(command_suffix) {
            let root = folders[0];
            let insert_index = insertion_index_for_top_level_group(&project_tracks, root);
            let suffix = next_version_suffix(&existing, root);
            if let Some(guid) =
                add_track_on_main_thread(&with_suffix(root, &suffix), Some(insert_index))
            {
                save_created_track_state(&reaper, project, &guid, command_suffix, root);
                tracing::info!(
                    "[dynamic-template] created top-level template group {} at index {}",
                    root,
                    insert_index
                );
            }
            return;
        }

        let plan = plan_create_insertion(&project_tracks, folders);
        let suffix = next_version_suffix(&existing, plan.version_root);
        let mut created = 0usize;
        let mut insert_index = plan.insert_index;

        tracing::info!(
            "[dynamic-template] create plan group={} root={} insert_index={} folders_to_create={} closing_depth={} existing_tracks={}",
            folders.last().unwrap_or(&command_suffix),
            plan.version_root,
            plan.insert_index,
            plan.folders_to_create.join("/"),
            plan.closing_depth,
            project_tracks.len()
        );

        if let Some(adjustment) = plan.previous_folder_close_adjustment {
            if let Err(err) =
                set_folder_depth_on_main_thread(&adjustment.guid, adjustment.new_depth)
            {
                tracing::warn!(
                    "[dynamic-template] failed to prepare parent folder insertion: {err}"
                );
            }
        }

        if let (true, Some(root_index)) = (plan.collapsed_root, plan.root_index) {
            if promote_collapsed_template_group(
                &reaper,
                project.clone(),
                &project_tracks,
                root_index,
            ) {
                insert_index += 1;
                created += 1;
            }
        }

        for folder in plan.folders_to_create {
            if let Some(guid) =
                add_track_on_main_thread(&with_suffix(folder, &suffix), Some(insert_index))
            {
                save_created_track_state(&reaper, project.clone(), &guid, command_suffix, folder);
                if let Err(err) = set_folder_depth_on_main_thread(&guid, 1) {
                    tracing::warn!(
                        "[dynamic-template] failed to set folder depth for {folder}: {err}"
                    );
                }
                insert_index += 1;
                created += 1;
            }
        }

        let leaf_tracks: Vec<&str> = if tracks.is_empty() {
            vec!["Main"]
        } else {
            tracks.to_vec()
        };
        for (index, track) in leaf_tracks.iter().copied().enumerate() {
            if let Some(guid) =
                add_track_on_main_thread(&with_suffix(track, &suffix), Some(insert_index))
            {
                save_created_track_state(&reaper, project.clone(), &guid, command_suffix, track);
                if index == leaf_tracks.len() - 1 {
                    let depth = -plan.closing_depth;
                    if let Err(err) = set_folder_depth_on_main_thread(&guid, depth) {
                        tracing::warn!(
                            "[dynamic-template] failed to close folder depth for {track}: {err}"
                        );
                    }
                }
                insert_index += 1;
                created += 1;
            }
        }
        tracing::info!(
            "[dynamic-template] created template group {} at index {} with {} tracks",
            folders.last().unwrap_or(&command_suffix),
            plan.insert_index,
            created
        );
    });
    Ok(())
}

struct CreateInsertionPlan {
    insert_index: u32,
    folders_to_create: &'static [&'static str],
    version_root: &'static str,
    closing_depth: i32,
    previous_folder_close_adjustment: Option<FolderCloseAdjustment>,
    collapsed_root: bool,
    root_index: Option<usize>,
}

struct FolderCloseAdjustment {
    guid: String,
    new_depth: i32,
}

fn is_drum_create_action(command_suffix: &str) -> bool {
    matches!(command_suffix, "DRUMS" | "DRUM_KIT" | "ELECTRONIC_KIT")
}

fn current_project_tracks() -> Vec<daw::service::Track> {
    let Some(daw) = daw::main_thread_daw() else {
        return Vec::new();
    };
    daw.track_list()
}

fn plan_create_insertion(
    tracks: &[daw::service::Track],
    folders: &'static [&'static str],
) -> CreateInsertionPlan {
    let root = folders[0];
    if folders.len() > 1 {
        if let Some(parent) = find_top_level_folder(tracks, root) {
            let (insert_index, previous_folder_close_adjustment) =
                insertion_point_inside_folder(tracks, parent);
            return CreateInsertionPlan {
                insert_index,
                folders_to_create: &folders[1..],
                version_root: folders[1],
                closing_depth: folders.len() as i32,
                previous_folder_close_adjustment,
                collapsed_root: is_collapsed_template_root(tracks, parent),
                root_index: Some(parent),
            };
        }
    }

    let collapse_subtype_into_root = folders.len() > 1;
    CreateInsertionPlan {
        insert_index: insertion_index_for_top_level_group(tracks, root),
        folders_to_create: if collapse_subtype_into_root {
            &folders[..1]
        } else {
            folders
        },
        version_root: root,
        closing_depth: if collapse_subtype_into_root {
            1
        } else {
            folders.len() as i32
        },
        previous_folder_close_adjustment: None,
        collapsed_root: false,
        root_index: None,
    }
}

fn promote_collapsed_template_group(
    reaper: &daw_reaper::Reaper,
    project: ProjectContext,
    tracks: &[daw::service::Track],
    root_index: usize,
) -> bool {
    let Some(root) = tracks.get(root_index) else {
        return false;
    };
    let Some(kind_name) =
        created_track_kind(reaper, project.clone(), &root.guid).and_then(create_kind_display_name)
    else {
        return false;
    };
    let end = folder_end_exclusive(tracks, root_index);
    if end <= root_index + 1 {
        return false;
    }
    let Some(last_child) = tracks.get(end - 1) else {
        return false;
    };
    let mut promoted = false;
    if let Some(guid) = add_track_on_main_thread(kind_name, Some(root.index + 1)) {
        save_created_track_state(
            reaper,
            project,
            &guid,
            &kind_name_to_suffix(kind_name),
            kind_name,
        );
        if let Err(err) = set_folder_depth_on_main_thread(&guid, 1) {
            tracing::warn!("[dynamic-template] failed to promote collapsed group: {err}");
        }
        promoted = true;
    }
    if let Err(err) = set_folder_depth_on_main_thread(&last_child.guid, last_child.folder_depth - 1)
    {
        tracing::warn!("[dynamic-template] failed to close promoted collapsed group: {err}");
    }
    promoted
}

fn is_collapsed_template_root(tracks: &[daw::service::Track], root_index: usize) -> bool {
    let Some(root) = tracks.get(root_index) else {
        return false;
    };
    if root.folder_depth <= 0 {
        return false;
    }
    let end = folder_end_exclusive(tracks, root_index);
    tracks[root_index + 1..end]
        .iter()
        .filter(|track| track.parent_guid.as_deref() == Some(&root.guid))
        .all(|track| track.folder_depth <= 0)
}

fn created_track_kind(
    reaper: &daw_reaper::Reaper,
    project: ProjectContext,
    guid: &str,
) -> Option<String> {
    let key = format!("{CREATE_STATE_KEY_PREFIX}{guid}");
    let state = ExtState::get_project(reaper, project, CREATE_STATE_SECTION, &key)?;
    state
        .lines()
        .find_map(|line| line.strip_prefix("kind=").map(str::to_string))
}

fn create_kind_display_name(kind: String) -> Option<&'static str> {
    create_template_specs()
        .iter()
        .find(|spec| spec.command_suffix == kind)
        .and_then(|spec| spec.folders.last().copied())
}

fn kind_name_to_suffix(kind_name: &str) -> String {
    normalize_key(kind_name).to_ascii_uppercase()
}

fn save_created_track_state(
    reaper: &daw_reaper::Reaper,
    project: ProjectContext,
    guid: &str,
    command_suffix: &str,
    role: &str,
) {
    let key = format!("{CREATE_STATE_KEY_PREFIX}{guid}");
    let value = format!("kind={command_suffix}\nrole={role}");
    if let Err(err) = ExtState::set_project(reaper, project, CREATE_STATE_SECTION, &key, &value) {
        tracing::warn!("[dynamic-template] failed to save create state for {guid}: {err}");
    }
}

fn find_top_level_folder(tracks: &[daw::service::Track], group_name: &str) -> Option<usize> {
    let key = normalize_key(group_name);
    tracks.iter().position(|track| {
        track.parent_guid.is_none()
            && track.folder_depth > 0
            && normalize_key(&base_track_name(&track.name)) == key
    })
}

fn insertion_point_inside_folder(
    tracks: &[daw::service::Track],
    folder_index: usize,
) -> (u32, Option<FolderCloseAdjustment>) {
    let end = folder_end_exclusive(tracks, folder_index);
    if end <= folder_index + 1 {
        return (tracks[folder_index].index + 1, None);
    }
    let previous = &tracks[end - 1];
    let adjustment = (previous.folder_depth < 0).then(|| FolderCloseAdjustment {
        guid: previous.guid.clone(),
        new_depth: previous.folder_depth + 1,
    });
    (track_insert_index_at(tracks, end), adjustment)
}

fn insertion_index_for_top_level_group(tracks: &[daw::service::Track], group_name: &str) -> u32 {
    let Some(target_order) = default_group_order(group_name) else {
        return track_insert_index_at(tracks, tracks.len());
    };

    let mut fallback = track_insert_index_at(tracks, tracks.len());
    for (index, track) in tracks.iter().enumerate() {
        if track.parent_guid.is_some() {
            continue;
        }
        let Some(order) = default_group_order(&base_track_name(&track.name)) else {
            continue;
        };
        if order > target_order {
            return track.index;
        }
        if order <= target_order {
            fallback = track_insert_index_at(tracks, folder_end_exclusive(tracks, index));
        }
    }
    fallback
}

fn track_insert_index_at(tracks: &[daw::service::Track], position: usize) -> u32 {
    tracks
        .get(position)
        .map(|track| track.index)
        .unwrap_or(tracks.len() as u32)
}

fn folder_end_exclusive(tracks: &[daw::service::Track], folder_index: usize) -> usize {
    if tracks
        .get(folder_index)
        .is_none_or(|track| track.folder_depth <= 0)
    {
        return folder_index + 1;
    }

    let mut depth = 0i32;
    for (index, track) in tracks.iter().enumerate().skip(folder_index) {
        depth += track.folder_depth;
        if index > folder_index && depth <= 0 {
            return index + 1;
        }
    }
    tracks.len()
}

fn default_group_order(group_name: &str) -> Option<usize> {
    const GROUPS: &[&str] = &[
        "Drums",
        "Percussion",
        "Bass",
        "Guitars",
        "Keys",
        "Synths",
        "Horns",
        "Harmonica",
        "Strings",
        "Vocals",
        "Choir",
        "Orchestra",
        "SFX",
        "Guide",
        "Reference",
        "Stem Split",
    ];
    let key = normalize_key(group_name);
    GROUPS.iter().position(|group| normalize_key(group) == key)
}

fn base_track_name(name: &str) -> String {
    let Some((prefix, suffix)) = name.rsplit_once(' ') else {
        return name.to_string();
    };
    if suffix.chars().all(|ch| ch.is_ascii_digit()) {
        prefix.to_string()
    } else {
        name.to_string()
    }
}

fn next_version_suffix(existing: &HashSet<String>, root_name: &str) -> String {
    if !existing.contains(root_name) {
        return String::new();
    }
    for index in 2.. {
        let suffix = format!(" {index}");
        if !existing.contains(&format!("{root_name}{suffix}")) {
            return suffix;
        }
    }
    unreachable!()
}

fn with_suffix(name: &str, suffix: &str) -> String {
    format!("{name}{suffix}")
}

fn normalize_key(value: &str) -> String {
    let mut key = String::new();
    let mut last_was_sep = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep && !key.is_empty() {
            key.push('_');
            last_was_sep = true;
        }
    }
    while key.ends_with('_') {
        key.pop();
    }
    key
}

fn create_template_specs() -> &'static [CreateTemplateSpec] {
    &[
        CreateTemplateSpec {
            command_suffix: "DRUMS",
            folders: &["Drums"],
            tracks: &["Kick", "Snare", "Toms", "Hi-Hat", "Overheads", "Room"],
        },
        CreateTemplateSpec {
            command_suffix: "DRUM_KIT",
            folders: &["Drums", "Drum Kit"],
            tracks: &["Kick", "Snare", "Toms", "Hi-Hat", "Overheads", "Room"],
        },
        CreateTemplateSpec {
            command_suffix: "ELECTRONIC_KIT",
            folders: &["Drums", "Electronic Kit"],
            tracks: &["Kick", "Snare", "Clap", "Hats", "Perc"],
        },
        CreateTemplateSpec {
            command_suffix: "PERCUSSION",
            folders: &["Percussion"],
            tracks: &["Shaker", "Tambourine", "Conga", "Perc Loop"],
        },
        CreateTemplateSpec {
            command_suffix: "BASS",
            folders: &["Bass"],
            tracks: &["Bass"],
        },
        CreateTemplateSpec {
            command_suffix: "BASS_GUITAR",
            folders: &["Bass", "Bass Guitar"],
            tracks: &["DI", "Amp"],
        },
        CreateTemplateSpec {
            command_suffix: "BASS_SYNTH",
            folders: &["Bass", "Bass Synth"],
            tracks: &["Bass Synth"],
        },
        CreateTemplateSpec {
            command_suffix: "UPRIGHT_BASS",
            folders: &["Bass", "Upright Bass"],
            tracks: &["Upright Bass"],
        },
        CreateTemplateSpec {
            command_suffix: "GUITARS",
            folders: &["Guitars"],
            tracks: &["Electric Guitar", "Acoustic Guitar"],
        },
        CreateTemplateSpec {
            command_suffix: "ELECTRIC_GUITAR",
            folders: &["Guitars", "Electric Guitar"],
            tracks: &["DI", "Amp", "Lead"],
        },
        CreateTemplateSpec {
            command_suffix: "ACOUSTIC_GUITAR",
            folders: &["Guitars", "Acoustic Guitar"],
            tracks: &["Acoustic Guitar"],
        },
        CreateTemplateSpec {
            command_suffix: "KEYS",
            folders: &["Keys"],
            tracks: &["Piano", "Organ", "Electric Keys"],
        },
        CreateTemplateSpec {
            command_suffix: "PIANO",
            folders: &["Keys", "Piano"],
            tracks: &["Piano"],
        },
        CreateTemplateSpec {
            command_suffix: "ORGAN",
            folders: &["Keys", "Organ"],
            tracks: &["Organ"],
        },
        CreateTemplateSpec {
            command_suffix: "ELECTRIC_KEYS",
            folders: &["Keys", "Electric Keys"],
            tracks: &["Electric Keys"],
        },
        CreateTemplateSpec {
            command_suffix: "SYNTHS",
            folders: &["Synths"],
            tracks: &["Lead", "Pad", "Arp", "FX"],
        },
        CreateTemplateSpec {
            command_suffix: "SYNTH_LEAD",
            folders: &["Synths", "Lead"],
            tracks: &["Synth Lead"],
        },
        CreateTemplateSpec {
            command_suffix: "SYNTH_PAD",
            folders: &["Synths", "Pad"],
            tracks: &["Synth Pad"],
        },
        CreateTemplateSpec {
            command_suffix: "SYNTH_ARP",
            folders: &["Synths", "Arp"],
            tracks: &["Synth Arp"],
        },
        CreateTemplateSpec {
            command_suffix: "HORNS",
            folders: &["Horns"],
            tracks: &["Trumpet", "Trombone", "Saxophone"],
        },
        CreateTemplateSpec {
            command_suffix: "TRUMPET",
            folders: &["Horns", "Trumpet"],
            tracks: &["Trumpet"],
        },
        CreateTemplateSpec {
            command_suffix: "TROMBONE",
            folders: &["Horns", "Trombone"],
            tracks: &["Trombone"],
        },
        CreateTemplateSpec {
            command_suffix: "SAXOPHONE",
            folders: &["Horns", "Saxophone"],
            tracks: &["Saxophone"],
        },
        CreateTemplateSpec {
            command_suffix: "HARMONICA",
            folders: &["Harmonica"],
            tracks: &["Harmonica"],
        },
        CreateTemplateSpec {
            command_suffix: "STRINGS",
            folders: &["Strings"],
            tracks: &["Violin", "Viola", "Cello", "Bass"],
        },
        CreateTemplateSpec {
            command_suffix: "VOCALS",
            folders: &["Vocals"],
            tracks: &["Lead Vocal", "Background Vocal", "Harmony"],
        },
        CreateTemplateSpec {
            command_suffix: "LEAD_VOCALS",
            folders: &["Vocals", "Lead Vocals"],
            tracks: &["Lead Vocal"],
        },
        CreateTemplateSpec {
            command_suffix: "BACKGROUND_VOCALS",
            folders: &["Vocals", "Background Vocals"],
            tracks: &["Background Vocal"],
        },
        CreateTemplateSpec {
            command_suffix: "CHOIR",
            folders: &["Choir"],
            tracks: &["Soprano", "Alto", "Tenor", "Bass"],
        },
        CreateTemplateSpec {
            command_suffix: "ORCHESTRA",
            folders: &["Orchestra"],
            tracks: &["Strings", "Brass", "Woodwinds", "Percussion"],
        },
        CreateTemplateSpec {
            command_suffix: "SFX",
            folders: &["SFX"],
            tracks: &["SFX"],
        },
        CreateTemplateSpec {
            command_suffix: "GUIDE",
            folders: &["Guide"],
            tracks: &["Guide"],
        },
        CreateTemplateSpec {
            command_suffix: "REFERENCE",
            folders: &["Reference"],
            tracks: &["Reference"],
        },
        CreateTemplateSpec {
            command_suffix: "STEM_SPLIT",
            folders: &["Stem Split"],
            tracks: &["Vocal", "Drums", "Bass", "Other"],
        },
    ]
}

/// Export the module.
pub fn module() -> Box<dyn DawModule> {
    Box::new(DynamicTemplateModule)
}

// ── architect::actions — declarative layer over the actions above ──────────
//
// Additive: `DawModule::actions()` above (and the old `actions_proto`-based
// `dynamic_template_actions`/`visibility_manager_actions`
// definitions) are untouched and still do the real REAPER registration. This
// gives the same ~26 actions real `ActionMeta` (description/category/group)
// through the new architect primitive, forwarding to the exact same handler
// functions.
//
// The 16 per-group `TOGGLE_*` and 34 `CREATE_NEW_*` actions (below, after
// VisibilityManagerActions) looked data-driven at a glance (their handlers,
// `toggle_group_visibility(state, group)` / `create_template_group(suffix)`,
// take a runtime string) but turned out to be a fixed, fully enumerable set —
// each already has its own static name/description/category/group declared
// in the `actions_proto::define_actions!` blocks above. That's a better fit
// for plain `#[architect::actions]` methods (matching every other action in
// this file) than architect's newer `DynamicActionMeta` (for genuinely
// unbounded-at-compile-time families) — so they're declared as
// CreateGroupActions / ToggleGroupActions below, each method forwarding its
// own fixed suffix/group string to the shared handler.

struct DynamicTemplateActionsImpl;

#[architect::actions(namespace = "FTS_DYNAMIC_TEMPLATE")]
trait DynamicTemplateActions {
    #[action(
        description = "Organize selected items into a hierarchical track template",
        category = "General"
    )]
    fn sort_selected(&self);

    #[action(
        description = "Organize all project items into a hierarchical track template",
        category = "General"
    )]
    fn sort_all(&self);

    #[action(
        description = "Import audio files and organize them into a hierarchical track template",
        category = "General"
    )]
    fn import_and_sort(&self);

    #[action(
        description = "Run organizer on a built-in sample input set",
        category = "Dev",
        group = "Dev"
    )]
    fn organize_demo(&self);

    #[action(
        description = "Log dynamic-template runtime status",
        category = "Dev",
        group = "Dev"
    )]
    fn log_status(&self);

    #[action(
        description = "Log configured dynamic-template group names",
        category = "Dev",
        group = "Dev"
    )]
    fn log_groups(&self);
}

impl DynamicTemplateActions for DynamicTemplateActionsImpl {
    fn sort_selected(&self) {
        dispatch(SORT_SELECTED.id);
    }
    fn sort_all(&self) {
        dispatch(SORT_ALL.id);
    }
    fn import_and_sort(&self) {
        dispatch(IMPORT_AND_SORT.id);
    }
    fn organize_demo(&self) {
        dispatch(ORGANIZE_DEMO.id);
    }
    fn log_status(&self) {
        log_status_action(&state());
    }
    fn log_groups(&self) {
        log_groups_action();
    }
}

struct VisibilityManagerActionsImpl;

#[architect::actions(namespace = "FTS_VISIBILITY_MANAGER")]
trait VisibilityManagerActions {
    #[action(description = "Show all tracks (reset visibility)", category = "View")]
    fn show_all(&self);

    #[action(description = "Hide all group tracks", category = "View")]
    fn hide_all(&self);

    #[action(
        description = "Show and size drum tracks for editing, hiding unrelated tracks",
        category = "View"
    )]
    fn profile_drum_editing(&self);

    #[action(
        description = "Show and size MIDI-oriented template groups for editing",
        category = "View"
    )]
    fn profile_midi_editing(&self);

    #[action(
        description = "Rebuild the track-to-group classification cache",
        category = "Dev",
        group = "Dev"
    )]
    fn rebuild_cache(&self);

    #[action(
        description = "Apply the Organize mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_organize(&self);
    #[action(
        description = "Apply the Write mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_write(&self);
    #[action(
        description = "Apply the Produce mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_produce(&self);
    #[action(
        description = "Apply the Record mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_record(&self);
    #[action(
        description = "Apply the Edit mode visibility rules (mixer shows collapsed buses, arrange shows one audio track per instrument)",
        category = "View",
        group = "Modes"
    )]
    fn mode_edit(&self);
    #[action(
        description = "Apply the Mix mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_mix(&self);
    #[action(
        description = "Apply the Master mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_master(&self);
    #[action(
        description = "Apply the Live mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_live(&self);
    #[action(
        description = "Apply the Video mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_video(&self);
    #[action(
        description = "Apply the Scoring mode visibility rules",
        category = "View",
        group = "Modes"
    )]
    fn mode_scoring(&self);
}

impl VisibilityManagerActions for VisibilityManagerActionsImpl {
    fn show_all(&self) {
        show_all_tracks().ok();
    }
    fn hide_all(&self) {
        hide_all_group_tracks(&state()).ok();
    }
    fn profile_drum_editing(&self) {
        apply_visibility_profile("DRUM_EDITING").ok();
    }
    fn profile_midi_editing(&self) {
        apply_visibility_profile("MIDI_EDITING").ok();
    }
    fn rebuild_cache(&self) {
        if let Ok(cache) = rebuild_group_cache() {
            state().lock().unwrap().group_cache = cache;
        }
    }
    fn mode_organize(&self) {
        apply_mode_visibility("organize").ok();
    }
    fn mode_write(&self) {
        apply_mode_visibility("write").ok();
    }
    fn mode_produce(&self) {
        apply_mode_visibility("produce").ok();
    }
    fn mode_record(&self) {
        apply_mode_visibility("record").ok();
    }
    fn mode_edit(&self) {
        apply_mode_visibility("edit").ok();
    }
    fn mode_mix(&self) {
        apply_mode_visibility("mix").ok();
    }
    fn mode_master(&self) {
        apply_mode_visibility("master").ok();
    }
    fn mode_live(&self) {
        apply_mode_visibility("live").ok();
    }
    fn mode_video(&self) {
        apply_mode_visibility("video").ok();
    }
    fn mode_scoring(&self) {
        apply_mode_visibility("scoring").ok();
    }
}

struct CreateGroupActionsImpl;

#[architect::actions(namespace = "FTS_DYNAMIC_TEMPLATE")]
trait CreateGroupActions {
    #[action(
        description = "Create a new drums template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_drums(&self);

    #[action(
        description = "Create a new drum kit template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_drum_kit(&self);

    #[action(
        description = "Create a new electronic kit template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_electronic_kit(&self);

    #[action(
        description = "Create a new percussion template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_percussion(&self);

    #[action(
        description = "Create a new bass template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_bass(&self);

    #[action(
        description = "Create a new bass guitar template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_bass_guitar(&self);

    #[action(
        description = "Create a new bass synth template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_bass_synth(&self);

    #[action(
        description = "Create a new upright bass template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_upright_bass(&self);

    #[action(
        description = "Create a new guitars template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_guitars(&self);

    #[action(
        description = "Create a new electric guitar template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_electric_guitar(&self);

    #[action(
        description = "Create a new acoustic guitar template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_acoustic_guitar(&self);

    #[action(
        description = "Create a new keys template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_keys(&self);

    #[action(
        description = "Create a new piano template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_piano(&self);

    #[action(
        description = "Create a new organ template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_organ(&self);

    #[action(
        description = "Create a new electric keys template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_electric_keys(&self);

    #[action(
        description = "Create a new synths template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_synths(&self);

    #[action(
        description = "Create a new synth lead template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_synth_lead(&self);

    #[action(
        description = "Create a new synth pad template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_synth_pad(&self);

    #[action(
        description = "Create a new synth arp template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_synth_arp(&self);

    #[action(
        description = "Create a new horns template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_horns(&self);

    #[action(
        description = "Create a new trumpet template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_trumpet(&self);

    #[action(
        description = "Create a new trombone template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_trombone(&self);

    #[action(
        description = "Create a new saxophone template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_saxophone(&self);

    #[action(
        description = "Create a new harmonica template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_harmonica(&self);

    #[action(
        description = "Create a new strings template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_strings(&self);

    #[action(
        description = "Create a new vocals template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_vocals(&self);

    #[action(
        description = "Create a new lead vocals template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_lead_vocals(&self);

    #[action(
        description = "Create a new background vocals template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_background_vocals(&self);

    #[action(
        description = "Create a new choir template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_choir(&self);

    #[action(
        description = "Create a new orchestra template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_orchestra(&self);

    #[action(
        description = "Create a new SFX template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_sfx(&self);

    #[action(
        description = "Create a new guide template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_guide(&self);

    #[action(
        description = "Create a new reference template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_reference(&self);

    #[action(
        description = "Create a new stem split template group",
        category = "General",
        group = "Create"
    )]
    fn create_new_stem_split(&self);
}

impl CreateGroupActions for CreateGroupActionsImpl {
    fn create_new_drums(&self) {
        create_template_group("DRUMS").ok();
    }
    fn create_new_drum_kit(&self) {
        create_template_group("DRUM_KIT").ok();
    }
    fn create_new_electronic_kit(&self) {
        create_template_group("ELECTRONIC_KIT").ok();
    }
    fn create_new_percussion(&self) {
        create_template_group("PERCUSSION").ok();
    }
    fn create_new_bass(&self) {
        create_template_group("BASS").ok();
    }
    fn create_new_bass_guitar(&self) {
        create_template_group("BASS_GUITAR").ok();
    }
    fn create_new_bass_synth(&self) {
        create_template_group("BASS_SYNTH").ok();
    }
    fn create_new_upright_bass(&self) {
        create_template_group("UPRIGHT_BASS").ok();
    }
    fn create_new_guitars(&self) {
        create_template_group("GUITARS").ok();
    }
    fn create_new_electric_guitar(&self) {
        create_template_group("ELECTRIC_GUITAR").ok();
    }
    fn create_new_acoustic_guitar(&self) {
        create_template_group("ACOUSTIC_GUITAR").ok();
    }
    fn create_new_keys(&self) {
        create_template_group("KEYS").ok();
    }
    fn create_new_piano(&self) {
        create_template_group("PIANO").ok();
    }
    fn create_new_organ(&self) {
        create_template_group("ORGAN").ok();
    }
    fn create_new_electric_keys(&self) {
        create_template_group("ELECTRIC_KEYS").ok();
    }
    fn create_new_synths(&self) {
        create_template_group("SYNTHS").ok();
    }
    fn create_new_synth_lead(&self) {
        create_template_group("SYNTH_LEAD").ok();
    }
    fn create_new_synth_pad(&self) {
        create_template_group("SYNTH_PAD").ok();
    }
    fn create_new_synth_arp(&self) {
        create_template_group("SYNTH_ARP").ok();
    }
    fn create_new_horns(&self) {
        create_template_group("HORNS").ok();
    }
    fn create_new_trumpet(&self) {
        create_template_group("TRUMPET").ok();
    }
    fn create_new_trombone(&self) {
        create_template_group("TROMBONE").ok();
    }
    fn create_new_saxophone(&self) {
        create_template_group("SAXOPHONE").ok();
    }
    fn create_new_harmonica(&self) {
        create_template_group("HARMONICA").ok();
    }
    fn create_new_strings(&self) {
        create_template_group("STRINGS").ok();
    }
    fn create_new_vocals(&self) {
        create_template_group("VOCALS").ok();
    }
    fn create_new_lead_vocals(&self) {
        create_template_group("LEAD_VOCALS").ok();
    }
    fn create_new_background_vocals(&self) {
        create_template_group("BACKGROUND_VOCALS").ok();
    }
    fn create_new_choir(&self) {
        create_template_group("CHOIR").ok();
    }
    fn create_new_orchestra(&self) {
        create_template_group("ORCHESTRA").ok();
    }
    fn create_new_sfx(&self) {
        create_template_group("SFX").ok();
    }
    fn create_new_guide(&self) {
        create_template_group("GUIDE").ok();
    }
    fn create_new_reference(&self) {
        create_template_group("REFERENCE").ok();
    }
    fn create_new_stem_split(&self) {
        create_template_group("STEM_SPLIT").ok();
    }
}

struct ToggleGroupActionsImpl;

#[architect::actions(namespace = "FTS_VISIBILITY_MANAGER")]
trait ToggleGroupActions {
    #[action(
        description = "Toggle visibility of all Drums tracks",
        category = "View"
    )]
    fn toggle_drums(&self);

    #[action(
        description = "Toggle visibility of all Percussion tracks",
        category = "View"
    )]
    fn toggle_percussion(&self);

    #[action(
        description = "Toggle visibility of all Bass tracks",
        category = "View"
    )]
    fn toggle_bass(&self);

    #[action(
        description = "Toggle visibility of all Guitars tracks",
        category = "View"
    )]
    fn toggle_guitars(&self);

    #[action(
        description = "Toggle visibility of all Keys tracks",
        category = "View"
    )]
    fn toggle_keys(&self);

    #[action(
        description = "Toggle visibility of all Synths tracks",
        category = "View"
    )]
    fn toggle_synths(&self);

    #[action(
        description = "Toggle visibility of all Horns tracks",
        category = "View"
    )]
    fn toggle_horns(&self);

    #[action(
        description = "Toggle visibility of all Harmonica tracks",
        category = "View"
    )]
    fn toggle_harmonica(&self);

    #[action(
        description = "Toggle visibility of all Strings tracks",
        category = "View"
    )]
    fn toggle_strings(&self);

    #[action(
        description = "Toggle visibility of all Vocals tracks",
        category = "View"
    )]
    fn toggle_vocals(&self);

    #[action(
        description = "Toggle visibility of all Choir tracks",
        category = "View"
    )]
    fn toggle_choir(&self);

    #[action(
        description = "Toggle visibility of all Orchestra tracks",
        category = "View"
    )]
    fn toggle_orchestra(&self);

    #[action(description = "Toggle visibility of all SFX tracks", category = "View")]
    fn toggle_sfx(&self);

    #[action(
        description = "Toggle visibility of all Guide tracks",
        category = "View"
    )]
    fn toggle_guide(&self);

    #[action(
        description = "Toggle visibility of all Reference tracks",
        category = "View"
    )]
    fn toggle_reference(&self);

    #[action(
        description = "Toggle visibility of all Stem Split tracks",
        category = "View"
    )]
    fn toggle_stem_split(&self);
}

impl ToggleGroupActions for ToggleGroupActionsImpl {
    fn toggle_drums(&self) {
        toggle_group_visibility(&state(), "DRUMS").ok();
    }
    fn toggle_percussion(&self) {
        toggle_group_visibility(&state(), "PERCUSSION").ok();
    }
    fn toggle_bass(&self) {
        toggle_group_visibility(&state(), "BASS").ok();
    }
    fn toggle_guitars(&self) {
        toggle_group_visibility(&state(), "GUITARS").ok();
    }
    fn toggle_keys(&self) {
        toggle_group_visibility(&state(), "KEYS").ok();
    }
    fn toggle_synths(&self) {
        toggle_group_visibility(&state(), "SYNTHS").ok();
    }
    fn toggle_horns(&self) {
        toggle_group_visibility(&state(), "HORNS").ok();
    }
    fn toggle_harmonica(&self) {
        toggle_group_visibility(&state(), "HARMONICA").ok();
    }
    fn toggle_strings(&self) {
        toggle_group_visibility(&state(), "STRINGS").ok();
    }
    fn toggle_vocals(&self) {
        toggle_group_visibility(&state(), "VOCALS").ok();
    }
    fn toggle_choir(&self) {
        toggle_group_visibility(&state(), "CHOIR").ok();
    }
    fn toggle_orchestra(&self) {
        toggle_group_visibility(&state(), "ORCHESTRA").ok();
    }
    fn toggle_sfx(&self) {
        toggle_group_visibility(&state(), "SFX").ok();
    }
    fn toggle_guide(&self) {
        toggle_group_visibility(&state(), "GUIDE").ok();
    }
    fn toggle_reference(&self) {
        toggle_group_visibility(&state(), "REFERENCE").ok();
    }
    fn toggle_stem_split(&self) {
        toggle_group_visibility(&state(), "STEM_SPLIT").ok();
    }
}

/// Register every architect-declared action in this module against `backend`.
pub fn register_architect_actions<B: architect::action::ActionBackend>(backend: &B) {
    register_dynamic_template_actions(backend, Arc::new(DynamicTemplateActionsImpl));
    register_visibility_manager_actions(backend, Arc::new(VisibilityManagerActionsImpl));
    register_create_group_actions(backend, Arc::new(CreateGroupActionsImpl));
    register_toggle_group_actions(backend, Arc::new(ToggleGroupActionsImpl));
}

#[cfg(test)]
mod architect_actions_id_tests {
    use super::*;

    /// Every action REAPER can invoke reaches a handler.
    ///
    /// `actions()` registers straight from `architect_metas()`, so a
    /// command id exists in REAPER's action list purely because a trait
    /// method declares it — with nothing forcing `handle_action` to know
    /// what to do when it fires. This walks the same list through the
    /// dispatch match, so a new method with no arm is a failing test
    /// rather than a menu entry that silently does nothing.
    #[test]
    fn every_registered_action_has_a_dispatch_arm() {
        let unhandled: Vec<&str> = architect_metas()
            .into_iter()
            .map(|m| m.id)
            .filter(|id| !is_dispatchable(id))
            .collect();

        assert!(
            unhandled.is_empty(),
            "{} registered ids reach no handler:\n  {}",
            unhandled.len(),
            unhandled.join("\n  "),
        );
    }

    #[test]
    fn create_group_ids_match_existing_reaper_command_convention() {
        let ids: Vec<&str> = CreateGroupActionsActions::all()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids.len(), 34, "expected all 34 CREATE_NEW_* ids");
        assert!(ids.contains(&"FTS_DYNAMIC_TEMPLATE_CREATE_NEW_DRUMS"));
        assert!(ids.contains(&"FTS_DYNAMIC_TEMPLATE_CREATE_NEW_STEM_SPLIT"));
        assert!(ids.contains(&"FTS_DYNAMIC_TEMPLATE_CREATE_NEW_BASS_SYNTH"));
        assert!(ids.contains(&"FTS_DYNAMIC_TEMPLATE_CREATE_NEW_ELECTRONIC_KIT"));
    }

    #[test]
    fn toggle_group_ids_match_existing_reaper_command_convention() {
        let ids: Vec<&str> = ToggleGroupActionsActions::all()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids.len(), 16, "expected all 16 TOGGLE_* ids");
        assert!(ids.contains(&"FTS_VISIBILITY_MANAGER_TOGGLE_DRUMS"));
        assert!(ids.contains(&"FTS_VISIBILITY_MANAGER_TOGGLE_STEM_SPLIT"));
    }
}
