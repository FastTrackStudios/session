//! Track-group manager actions.
//!
//! Registered under the `fts.session.*` namespace via the `session_actions`
//! `define_actions!` block in [`crate::lib`], dispatched from
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
