use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use daw::service::{ExtState, ProjectContext, Projects, TrackEvent, TrackRef, Tracks};
use tokio::sync::broadcast::error::RecvError;

static STATE: OnceLock<Arc<AutoColorState>> = OnceLock::new();
static TIMER_REGISTERED: AtomicBool = AtomicBool::new(false);

const EXT_STATE_SECTION: &str = "FastTrackStudio.Session.AutoColor";
const EXT_STATE_ENABLED_KEY: &str = "enabled";
const PROJECT_STATE_SECTION: &str = "FTSAUTOCOLOR";
const PROJECT_STATE_APPLIED_COLORS_KEY: &str = "tracks";
const LEGACY_PROJECT_STATE_SECTION: &str = "FastTrackStudio.Session.AutoColor";
const LEGACY_PROJECT_STATE_APPLIED_COLORS_KEY: &str = "applied_colors";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoColorAction {
    ColorAll,
    ColorSelected,
    Toggle,
    ClearAll,
    ClearSelected,
}

struct AutoColorState {
    enabled: AtomicBool,
    subscribed: AtomicBool,
    pending_current: AtomicBool,
    pending_projects: Mutex<HashSet<String>>,
    applied_colors: Mutex<HashMap<String, u32>>,
    loaded_projects: Mutex<HashSet<String>>,
}

impl AutoColorState {
    fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            subscribed: AtomicBool::new(false),
            pending_current: AtomicBool::new(false),
            pending_projects: Mutex::new(HashSet::new()),
            applied_colors: Mutex::new(HashMap::new()),
            loaded_projects: Mutex::new(HashSet::new()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AutoColorRule {
    color: u32,
    matcher: fn(&str) -> bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AutoColorDecision {
    pub color: Option<u32>,
}

pub fn init(ctx: &daw::module::ModuleContext) {
    let _ = ctx;
    let _ = STATE.set(Arc::new(AutoColorState::new()));
    register_timer();
}

pub fn action_for_id(action_id: &str) -> Option<AutoColorAction> {
    let normalized = action_id
        .trim()
        .to_lowercase()
        .strip_prefix("fts.session.")
        .unwrap_or(action_id.trim())
        .to_string();
    match normalized.as_str() {
        "auto_color_color_all" => Some(AutoColorAction::ColorAll),
        "auto_color_color_selected" => Some(AutoColorAction::ColorSelected),
        "auto_color_toggle" => Some(AutoColorAction::Toggle),
        "auto_color_clear_all" => Some(AutoColorAction::ClearAll),
        "auto_color_clear_selected" => Some(AutoColorAction::ClearSelected),
        _ => None,
    }
}

pub fn dispatch(action: AutoColorAction) {
    if let Err(err) = run_action(action) {
        tracing::error!(?action, ?err, "[session] Auto-color action failed");
    }
}

pub fn subscribe(ctx: &daw::module::ModuleContext) {
    let _ = STATE.set(Arc::new(AutoColorState::new()));
    register_timer();

    match load_enabled() {
        Ok(enabled) => set_enabled(enabled),
        Err(err) => {
            tracing::warn!(?err, "[session] Failed to load auto-color enabled state");
        }
    }

    ctx.spawn(async {
        if let Err(err) = ensure_reactive_updates() {
            tracing::warn!(?err, "[session] Failed to start auto-color subscription");
        }

        if state()
            .map(|state| state.enabled.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
            schedule_current_project_apply();
        }
    });
}

fn run_action(action: AutoColorAction) -> eyre::Result<()> {
    match action {
        AutoColorAction::ColorAll => {
            set_enabled(true);
            save_enabled(true)?;
            ensure_reactive_updates()?;
            apply_to_current_project(false, true)
        }
        AutoColorAction::ColorSelected => apply_to_current_project(true, true),
        AutoColorAction::Toggle => {
            let enabled = !state()?.enabled.load(Ordering::Relaxed);
            set_enabled(enabled);
            save_enabled(enabled)?;
            if enabled {
                ensure_reactive_updates()?;
                schedule_current_project_apply();
            }
            Ok(())
        }
        AutoColorAction::ClearAll => {
            set_enabled(false);
            save_enabled(false)?;
            clear_current_project(false)
        }
        AutoColorAction::ClearSelected => clear_current_project(true),
    }
}

fn ensure_reactive_updates() -> eyre::Result<()> {
    let state = state()?;
    if state.subscribed.swap(true, Ordering::AcqRel) {
        return Ok(());
    }

    let mut rx = daw::reaper::event_hub().subscribe_tracks();

    moire::task::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if should_schedule(&event.event) {
                        schedule_project_apply(event.project_guid);
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "[session] Auto-color sync stream lagged");
                    schedule_current_project_apply();
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    Ok(())
}

fn should_schedule(event: &TrackEvent) -> bool {
    matches!(
        event,
        TrackEvent::Added(_)
            | TrackEvent::Removed(_)
            | TrackEvent::Renamed { .. }
            | TrackEvent::Moved { .. }
    )
}

fn schedule_current_project_apply() {
    let Ok(state) = state() else {
        return;
    };
    if !state.enabled.load(Ordering::Relaxed) {
        return;
    }
    state.pending_current.store(true, Ordering::Relaxed);
}

fn schedule_project_apply(project_guid: String) {
    let Ok(state) = state().cloned() else {
        return;
    };
    if !state.enabled.load(Ordering::Relaxed) {
        return;
    }

    if let Ok(mut pending) = state.pending_projects.lock() {
        pending.insert(project_guid);
    }
}

fn register_timer() {
    if TIMER_REGISTERED.swap(true, Ordering::AcqRel) {
        return;
    }
    daw::register_timer(auto_color_timer);
}

fn auto_color_timer() {
    let Ok(state) = state().cloned() else {
        return;
    };
    if !state.enabled.load(Ordering::Relaxed) {
        return;
    }

    if state.pending_current.swap(false, Ordering::Relaxed)
        && let Err(err) = apply_to_current_project(false, false)
    {
        tracing::warn!(?err, "[session] Startup auto-color pass failed");
    }

    let pending_projects = match state.pending_projects.lock() {
        Ok(mut pending) => pending.drain().collect::<Vec<_>>(),
        Err(err) => {
            tracing::warn!(?err, "[session] Auto-color pending project lock poisoned");
            Vec::new()
        }
    };

    for project_guid in pending_projects {
        if let Err(err) = apply_to_project_guid(&project_guid, false, false) {
            tracing::warn!(?err, project_guid, "[session] Reactive auto-color failed");
        }
    }
}

fn apply_to_current_project(selected_only: bool, force: bool) -> eyre::Result<()> {
    apply_to_project(ProjectContext::Current, selected_only, force)
}

fn apply_to_project_guid(project_guid: &str, selected_only: bool, force: bool) -> eyre::Result<()> {
    apply_to_project(
        ProjectContext::Project(project_guid.to_string()),
        selected_only,
        force,
    )
}

fn apply_to_project(project: ProjectContext, selected_only: bool, force: bool) -> eyre::Result<()> {
    let state = state()?.clone();
    let project_key = project_key(&project);
    load_applied_colors_for_project(&project, &project_key)?;
    let all = daw::reaper::Reaper.all(project.clone());
    let selected: HashSet<String> = if selected_only {
        daw::reaper::Reaper
            .selected(project.clone())
            .into_iter()
            .map(|track| track.guid)
            .collect()
    } else {
        HashSet::new()
    };

    let decisions = decide_colors(&all);
    let mut changed = 0usize;
    let mut applied_colors = state
        .applied_colors
        .lock()
        .map_err(|err| eyre::eyre!("Auto-color applied color lock poisoned: {err}"))?;
    let live_guids: HashSet<&str> = all.iter().map(|track| track.guid.as_str()).collect();
    applied_colors.retain(|key, _| {
        let Some((key_project, guid)) = key.split_once(':') else {
            return false;
        };
        key_project != project_key || live_guids.contains(guid)
    });

    for track in &all {
        if selected_only && !selected.contains(&track.guid) {
            continue;
        }
        let key = auto_color_key(&project_key, &track.guid);
        let current = track.color.unwrap_or(0);
        let previous = applied_colors.get(&key).copied().unwrap_or(0);
        let desired = decisions
            .get(&track.guid)
            .and_then(|decision| decision.color);

        if let Some(desired) = desired {
            if current != desired && (force || current == previous) {
                daw::reaper::Reaper.set_color(
                    project.clone(),
                    TrackRef::Guid(track.guid.clone()),
                    desired,
                )?;
                changed += 1;
            }
            applied_colors.insert(key, desired);
        } else if previous != 0 {
            if current == previous {
                daw::reaper::Reaper.set_color(
                    project.clone(),
                    TrackRef::Guid(track.guid.clone()),
                    0,
                )?;
                changed += 1;
            }
            applied_colors.remove(&key);
        }
    }

    save_applied_colors_for_project(&project, &project_key, &applied_colors)?;

    tracing::debug!(
        changed,
        selected_only,
        force,
        "[session] Auto-color pass complete"
    );
    Ok(())
}

fn clear_current_project(selected_only: bool) -> eyre::Result<()> {
    let project = ProjectContext::Current;
    let project_key = project_key(&project);
    let selected: HashSet<String> = if selected_only {
        daw::reaper::Reaper
            .selected(project.clone())
            .into_iter()
            .map(|track| track.guid)
            .collect()
    } else {
        HashSet::new()
    };

    for track in daw::reaper::Reaper.all(project.clone()) {
        if selected_only && !selected.contains(&track.guid) {
            continue;
        }
        remove_applied_color(&project_key, &track.guid);
        if track.color.is_some() {
            daw::reaper::Reaper.set_color(project.clone(), TrackRef::Guid(track.guid), 0)?;
        }
    }
    save_current_applied_colors_for_project(&project, &project_key)?;
    Ok(())
}

fn state() -> eyre::Result<&'static Arc<AutoColorState>> {
    STATE
        .get()
        .ok_or_else(|| eyre::eyre!("Auto-color state not initialized"))
}

fn set_enabled(enabled: bool) {
    if let Some(state) = STATE.get() {
        state.enabled.store(enabled, Ordering::Relaxed);
    }
}

fn project_key(project: &ProjectContext) -> String {
    match project {
        ProjectContext::Current => daw::reaper::Reaper
            .current()
            .map(|project| project.guid)
            .unwrap_or_else(|| "current".to_string()),
        ProjectContext::Project(guid) => guid.clone(),
    }
}

fn auto_color_key(project_key: &str, track_guid: &str) -> String {
    format!("{project_key}:{track_guid}")
}

fn remove_applied_color(project_key: &str, track_guid: &str) {
    let Some(state) = STATE.get() else {
        return;
    };
    let key = auto_color_key(project_key, track_guid);
    if let Ok(mut applied_colors) = state.applied_colors.lock() {
        applied_colors.remove(&key);
    }
}

fn load_applied_colors_for_project(
    project: &ProjectContext,
    project_key: &str,
) -> eyre::Result<()> {
    let state = state()?.clone();
    {
        let mut loaded = state
            .loaded_projects
            .lock()
            .map_err(|err| eyre::eyre!("Auto-color loaded project lock poisoned: {err}"))?;
        if !loaded.insert(project_key.to_string()) {
            return Ok(());
        }
    }

    let Some(serialized) = ExtState::get_project(
        &daw::reaper::Reaper,
        project.clone(),
        PROJECT_STATE_SECTION,
        PROJECT_STATE_APPLIED_COLORS_KEY,
    )
    .or_else(|| {
        ExtState::get_project(
            &daw::reaper::Reaper,
            project.clone(),
            LEGACY_PROJECT_STATE_SECTION,
            LEGACY_PROJECT_STATE_APPLIED_COLORS_KEY,
        )
    }) else {
        return Ok(());
    };

    let mut applied_colors = state
        .applied_colors
        .lock()
        .map_err(|err| eyre::eyre!("Auto-color applied color lock poisoned: {err}"))?;
    for line in serialized.lines() {
        let Some((guid, color)) = line.split_once('=') else {
            continue;
        };
        let color = color.trim().trim_start_matches("0x");
        let Ok(color) = u32::from_str_radix(color, 16) else {
            continue;
        };
        if color != 0 {
            applied_colors.insert(auto_color_key(project_key, guid.trim()), color);
        }
    }
    Ok(())
}

fn save_current_applied_colors_for_project(
    project: &ProjectContext,
    project_key: &str,
) -> eyre::Result<()> {
    let state = state()?.clone();
    let applied_colors = state
        .applied_colors
        .lock()
        .map_err(|err| eyre::eyre!("Auto-color applied color lock poisoned: {err}"))?;
    save_applied_colors_for_project(project, project_key, &applied_colors)
}

fn save_applied_colors_for_project(
    project: &ProjectContext,
    project_key: &str,
    applied_colors: &HashMap<String, u32>,
) -> eyre::Result<()> {
    let prefix = format!("{project_key}:");
    let mut entries = applied_colors
        .iter()
        .filter_map(|(key, color)| {
            let guid = key.strip_prefix(&prefix)?;
            Some((guid, *color))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let serialized = entries
        .into_iter()
        .map(|(guid, color)| format!("{guid}=0x{color:06X}"))
        .collect::<Vec<_>>()
        .join("\n");

    if serialized.is_empty() {
        ExtState::delete_project(
            &daw::reaper::Reaper,
            project.clone(),
            PROJECT_STATE_SECTION,
            PROJECT_STATE_APPLIED_COLORS_KEY,
        )?;
    } else {
        ExtState::set_project(
            &daw::reaper::Reaper,
            project.clone(),
            PROJECT_STATE_SECTION,
            PROJECT_STATE_APPLIED_COLORS_KEY,
            &serialized,
        )?;
    }
    Ok(())
}

fn load_enabled() -> eyre::Result<bool> {
    let Some(value) = ExtState::get(
        &daw::reaper::Reaper,
        EXT_STATE_SECTION,
        EXT_STATE_ENABLED_KEY,
    ) else {
        return Ok(true);
    };
    Ok(!matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    ))
}

fn save_enabled(enabled: bool) -> eyre::Result<()> {
    ExtState::set(
        &daw::reaper::Reaper,
        EXT_STATE_SECTION,
        EXT_STATE_ENABLED_KEY,
        if enabled { "true" } else { "false" },
        true,
    )?;
    Ok(())
}

pub(crate) fn decide_colors(tracks: &[daw::service::Track]) -> HashMap<String, AutoColorDecision> {
    let by_guid: HashMap<&str, &daw::service::Track> = tracks
        .iter()
        .map(|track| (track.guid.as_str(), track))
        .collect();
    let explicit: HashMap<&str, u32> = tracks
        .iter()
        .filter_map(|track| {
            rule_color_for_name(&track.name).map(|color| (track.guid.as_str(), color))
        })
        .collect();

    tracks
        .iter()
        .map(|track| {
            let color = explicit
                .get(track.guid.as_str())
                .copied()
                .or_else(|| inherited_parent_color(track, &by_guid, &explicit));
            (track.guid.clone(), AutoColorDecision { color })
        })
        .collect()
}

fn inherited_parent_color(
    track: &daw::service::Track,
    by_guid: &HashMap<&str, &daw::service::Track>,
    explicit: &HashMap<&str, u32>,
) -> Option<u32> {
    let mut current = track.parent_guid.as_deref();
    while let Some(parent_guid) = current {
        if let Some(color) = explicit.get(parent_guid) {
            return Some(*color);
        }
        current = by_guid
            .get(parent_guid)
            .and_then(|parent| parent.parent_guid.as_deref());
    }
    None
}

fn rule_color_for_name(name: &str) -> Option<u32> {
    let normalized = normalize_name(name);
    AUTO_COLOR_RULES
        .iter()
        .find(|rule| (rule.matcher)(&normalized))
        .map(|rule| rule.color)
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
}

const AUTO_COLOR_RULES: &[AutoColorRule] = &[
    AutoColorRule {
        color: 0x7C3AED,
        matcher: has_tracks_folder,
    },
    AutoColorRule {
        color: 0x14B8A6,
        matcher: has_reference,
    },
    AutoColorRule {
        color: 0xF59E0B,
        matcher: has_guide,
    },
    AutoColorRule {
        color: 0xEF4444,
        matcher: has_drums,
    },
    AutoColorRule {
        color: 0xF97316,
        matcher: has_percussion,
    },
    AutoColorRule {
        color: 0x0EA5E9,
        matcher: has_bass,
    },
    AutoColorRule {
        color: 0x22C55E,
        matcher: has_guitar,
    },
    AutoColorRule {
        color: 0xA855F7,
        matcher: has_keys,
    },
    AutoColorRule {
        color: 0x06B6D4,
        matcher: has_synth,
    },
    AutoColorRule {
        color: 0xEC4899,
        matcher: has_vocal,
    },
    AutoColorRule {
        color: 0x84CC16,
        matcher: has_strings,
    },
    AutoColorRule {
        color: 0xEAB308,
        matcher: has_horns,
    },
    AutoColorRule {
        color: 0x64748B,
        matcher: has_fx,
    },
];

fn has_word(name: &str, word: &str) -> bool {
    name.split_whitespace().any(|part| part == word)
}

fn has_any_word(name: &str, words: &[&str]) -> bool {
    words.iter().any(|word| has_word(name, word))
}

fn has_tracks_folder(name: &str) -> bool {
    name.trim() == "tracks"
}

fn has_reference(name: &str) -> bool {
    has_any_word(name, &["reference", "refs", "ref", "mix"])
}

fn has_guide(name: &str) -> bool {
    has_any_word(name, &["guide", "click", "count", "loop"])
}

fn has_drums(name: &str) -> bool {
    has_any_word(
        name,
        &["drums", "drum", "kit", "kick", "snare", "tom", "oh"],
    )
}

fn has_percussion(name: &str) -> bool {
    has_any_word(name, &["perc", "percussion", "shaker", "tamb", "conga"])
}

fn has_bass(name: &str) -> bool {
    has_word(name, "bass")
}

fn has_guitar(name: &str) -> bool {
    has_any_word(name, &["guitar", "gtr", "egtr", "agtr"])
}

fn has_keys(name: &str) -> bool {
    has_any_word(name, &["keys", "key", "piano", "organ", "rhodes", "wurli"])
}

fn has_synth(name: &str) -> bool {
    has_any_word(name, &["synth", "pad", "lead", "arp"])
}

fn has_vocal(name: &str) -> bool {
    has_any_word(name, &["vocal", "vocals", "vox", "bgv", "bgvs", "choir"])
}

fn has_strings(name: &str) -> bool {
    has_any_word(name, &["string", "strings", "violin", "cello", "viola"])
}

fn has_horns(name: &str) -> bool {
    has_any_word(name, &["horn", "horns", "trumpet", "sax", "trombone"])
}

fn has_fx(name: &str) -> bool {
    has_any_word(name, &["fx", "sfx", "effect", "effects"])
}

#[cfg(test)]
mod tests {
    use daw::service::Track;

    use super::*;

    fn track(guid: &str, index: u32, name: &str, parent_guid: Option<&str>) -> Track {
        let mut track = Track::new(guid.to_string(), index, name.to_string());
        track.parent_guid = parent_guid.map(str::to_string);
        track
    }

    #[test]
    fn first_matching_rule_wins() {
        assert_eq!(rule_color_for_name("Guide Bass"), Some(0xF59E0B));
    }

    #[test]
    fn child_inherits_nearest_colored_parent() {
        let tracks = vec![
            track("folder", 0, "Guitars", None),
            track("child", 1, "57", Some("folder")),
        ];

        let decisions = decide_colors(&tracks);

        assert_eq!(decisions["folder"].color, Some(0x22C55E));
        assert_eq!(decisions["child"].color, Some(0x22C55E));
    }

    #[test]
    fn explicit_child_color_overrides_parent() {
        let tracks = vec![
            track("folder", 0, "Guitars", None),
            track("child", 1, "Lead Vocal", Some("folder")),
        ];

        let decisions = decide_colors(&tracks);

        assert_eq!(decisions["child"].color, Some(0xEC4899));
    }
}
