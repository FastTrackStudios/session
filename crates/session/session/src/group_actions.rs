//! Track-group manager actions.
//!
//! Registered under the `fts.session.*` namespace via the `session_actions`
//! `define_actions!` block in `crate::lib`, dispatched from
//! `daw_module`'s `action_for_id` chain. All work runs on REAPER's main
//! thread (the action-callback context), delegating to [`crate::group_manager`].

use crate::group_manager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupAction {
    /// Write the instrument-category partition into the 128 group names.
    ApplyNaming,
    /// Assign the selected tracks to the next free slot in a category band.
    AssignSelected(&'static str),
}

pub fn action_for_id(action_id: &str) -> Option<GroupAction> {
    let slug = action_id
        .trim()
        .to_lowercase()
        .strip_prefix("fts.session.")
        .map(str::to_string)
        .unwrap_or_else(|| action_id.to_lowercase());
    let action = match slug.as_str() {
        "group_apply_naming" => GroupAction::ApplyNaming,
        "group_assign_drums" => GroupAction::AssignSelected("Drums"),
        "group_assign_bass" => GroupAction::AssignSelected("Bass"),
        "group_assign_electric_gtr" => GroupAction::AssignSelected("Electric Gtr"),
        "group_assign_acoustic_gtr" => GroupAction::AssignSelected("Acoustic Gtr"),
        "group_assign_keys" => GroupAction::AssignSelected("Keys"),
        "group_assign_synths" => GroupAction::AssignSelected("Synths"),
        "group_assign_lead_vocal" => GroupAction::AssignSelected("Lead Vocal"),
        "group_assign_background_vox" => GroupAction::AssignSelected("Background Vox"),
        _ => return None,
    };
    Some(action)
}

pub fn dispatch(action: GroupAction) {
    match action {
        GroupAction::ApplyNaming => {
            group_manager::apply_group_naming();
        }
        GroupAction::AssignSelected(category) => {
            group_manager::assign_selected_to_category(category);
        }
    }
}

// ── architect::actions declaration ──────────────────────────────────────
//
// `GroupAction` / `action_for_id` / `dispatch` above stay put — still the
// live path `daw_module.rs`'s dispatch chain calls into. Additive
// declarative layer only, mirroring `setlist_actions`'s migration.

/// Bridges the nine track-group-manager actions onto
/// `#[architect::actions]`. Every method forwards to the existing
/// synchronous `dispatch` — no behavior change, just a declarative front
/// door with real metadata.
pub struct GroupActionsImpl;

#[architect::actions(namespace = "FTS_SESSION")]
pub trait GroupActions {
    #[action(
        description = "Name the project's 128 track groups by the FTS instrument partition (Drums 1-10, Bass 11-20, Electric Gtr 21-40, Acoustic Gtr 41-60, Keys 61-70, Synths 71-80, Lead Vocal 81-100, Background Vox 101-120, Spare 121-128).",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_apply_naming(&self);
    #[action(
        description = "Add the selected tracks to the next free Drums group slot as a mutual group (all flag families).",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_drums(&self);
    #[action(
        description = "Add the selected tracks to the next free Bass group slot as a mutual group.",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_bass(&self);
    #[action(
        description = "Add the selected tracks to the next free Electric Gtr group slot as a mutual group.",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_electric_gtr(&self);
    #[action(
        description = "Add the selected tracks to the next free Acoustic Gtr group slot as a mutual group.",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_acoustic_gtr(&self);
    #[action(
        description = "Add the selected tracks to the next free Keys group slot as a mutual group.",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_keys(&self);
    #[action(
        description = "Add the selected tracks to the next free Synths group slot as a mutual group.",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_synths(&self);
    #[action(
        description = "Add the selected tracks to the next free Lead Vocal group slot as a mutual group.",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_lead_vocal(&self);
    #[action(
        description = "Add the selected tracks to the next free Background Vox group slot as a mutual group.",
        category = "Tracks",
        group = "Track Groups"
    )]
    fn group_assign_background_vox(&self);
}

impl GroupActions for GroupActionsImpl {
    fn group_apply_naming(&self) {
        dispatch(GroupAction::ApplyNaming);
    }
    fn group_assign_drums(&self) {
        dispatch(GroupAction::AssignSelected("Drums"));
    }
    fn group_assign_bass(&self) {
        dispatch(GroupAction::AssignSelected("Bass"));
    }
    fn group_assign_electric_gtr(&self) {
        dispatch(GroupAction::AssignSelected("Electric Gtr"));
    }
    fn group_assign_acoustic_gtr(&self) {
        dispatch(GroupAction::AssignSelected("Acoustic Gtr"));
    }
    fn group_assign_keys(&self) {
        dispatch(GroupAction::AssignSelected("Keys"));
    }
    fn group_assign_synths(&self) {
        dispatch(GroupAction::AssignSelected("Synths"));
    }
    fn group_assign_lead_vocal(&self) {
        dispatch(GroupAction::AssignSelected("Lead Vocal"));
    }
    fn group_assign_background_vox(&self) {
        dispatch(GroupAction::AssignSelected("Background Vox"));
    }
}

/// Registers all nine track-group-manager actions with `backend`.
pub fn register_actions<B>(backend: &B)
where
    B: ::architect::action::ActionBackend + ?Sized,
{
    register_group_actions(backend, std::sync::Arc::new(GroupActionsImpl));
}
