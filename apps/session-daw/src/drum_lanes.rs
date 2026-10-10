//! The kit's lanes: the comp it was played from and the edit made of it.
//!
//! Every drum track alike, because the kit is one recording: its comping
//! lane — the comp that plays — named `COMP`, and a lane `EDIT` holding a
//! copy of it, where the edits land. Either can be played, the whole kit
//! at once, to weigh the edit against the comp it was made from; the
//! Edit mode's phases do that by themselves (Comp plays `COMP`, Edit
//! plays `EDIT` — [`follow_phase`]).

use daw::service::{ItemRef, Items as _, ProjectContext, TrackRef, Tracks};

/// The comp's lane name.
pub const COMP: &str = "COMP";
/// The edit's lane name.
pub const EDIT: &str = "EDIT";

/// The kit's tracks: every track under the top-level Drums folder, but
/// none in its Process (the mix, not the recording).
#[must_use]
pub fn kit_tracks(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
) -> Vec<daw_proto::Track> {
    let tracks = Tracks::all(daw, project.clone());
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    tracks
        .iter()
        .filter(|t| !t.is_folder)
        .filter(|t| {
            let path = crate::gain_stage::path_of(t, &by_guid);
            path.first()
                .is_some_and(|p| p.trim().eq_ignore_ascii_case("drums"))
                && !path
                    .iter()
                    .any(|p| p.trim().eq_ignore_ascii_case("process"))
        })
        .cloned()
        .collect()
}

/// A lane by name on `track`.
#[must_use]
pub fn lane_named(track: &daw_proto::Track, name: &str) -> Option<u32> {
    track
        .lane_names
        .iter()
        .position(|n| n.trim().eq_ignore_ascii_case(name.trim()))
        .and_then(|i| u32::try_from(i).ok())
        .filter(|&i| i < track.lane_count)
}

/// Set the kit up for editing; what it did — or, `dry`, would — track by
/// track.
///
/// Each track's comping lane (the comping lane, else the lowest playing,
/// else the first) named `COMP`, a new lane `EDIT` after the last with a
/// copy of it, and `EDIT` playing. Appended, so the takes keep the lane
/// numbers REAPER's comp areas point at; a track already set up is left.
///
/// # Errors
///
/// The engine refusing one of the writes, named by the track's path.
pub fn set_up(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    dry: bool,
) -> Result<Vec<String>, String> {
    let all = Tracks::all(daw, project.clone());
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        all.iter().map(|t| (t.guid.as_str(), t)).collect();
    let mut said = Vec::new();
    for track in kit_tracks(daw, project) {
        let path = crate::gain_stage::path_of(&track, &by_guid).join("/");
        let at = TrackRef::Guid(track.guid.clone());
        if lane_named(&track, EDIT).is_some() {
            said.push(format!("{path}: already set up"));
            continue;
        }
        let items = daw.get_items(project.clone(), at.clone());
        if items.is_empty() {
            continue;
        }
        let comp = daw
            .comping(project.clone(), at.clone())
            .ok()
            .and_then(|c| c.comp_lane)
            .or_else(|| (track.lane_play_mask != 0).then(|| track.lane_play_mask.trailing_zeros()))
            .unwrap_or(0);
        let (count, edit) = if track.lane_count == 0 {
            (2, 1)
        } else {
            (track.lane_count.saturating_add(1), track.lane_count)
        };
        let on_comp: Vec<&daw_proto::Item> = items
            .iter()
            .filter(|i| i.fixed_lane.unwrap_or(0) == comp)
            .collect();
        said.push(format!(
            "{path}: lane {comp} → {COMP}, lane {edit} → {EDIT} ({} items copied), {EDIT} plays",
            on_comp.len()
        ));
        if dry {
            continue;
        }
        let fail = |e: daw_proto::DawError| format!("{path}: {e}");
        daw.set_lane_count(project.clone(), at.clone(), count)
            .map_err(fail)?;
        daw.set_lane_name(project.clone(), at.clone(), comp, COMP)
            .map_err(fail)?;
        daw.set_lane_name(project.clone(), at.clone(), edit, EDIT)
            .map_err(fail)?;
        for item in on_comp {
            let copy = daw
                .duplicate_item(project.clone(), ItemRef::Guid(item.guid.clone()))
                .ok_or_else(|| format!("{path}: an item would not copy"))?;
            daw.set_fixed_lane(project.clone(), ItemRef::Guid(copy), edit)
                .map_err(fail)?;
        }
        daw.set_lane_play_mask(project.clone(), at, 1_u64.checked_shl(edit).unwrap_or(0))
            .map_err(fail)?;
    }
    if !dry && !said.is_empty() {
        // The window in the Edit phase, where EDIT plays.
        session::mode_phases::choose(session::modes::Mode::Edit, "edit");
        crate::studio::request_resync();
        crate::actions::list().changed("drums.lanes", "the kit's lanes were set up");
    }
    Ok(said)
}

/// Play the lane named `name` on every drum track that has one; how many
/// do.
///
/// # Errors
///
/// No drum track having a lane of that name, or the engine refusing.
pub fn play(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    name: &str,
) -> Result<u32, String> {
    let mut played = 0_u32;
    for track in kit_tracks(daw, project) {
        let Some(lane) = lane_named(&track, name) else {
            continue;
        };
        let mask = 1_u64.checked_shl(lane).unwrap_or(0);
        if track.lane_play_mask != mask {
            daw.set_lane_play_mask(project.clone(), TrackRef::Guid(track.guid.clone()), mask)
                .map_err(|e| e.to_string())?;
        }
        played = played.saturating_add(1);
    }
    if played == 0 {
        return Err(format!(
            "no drum track has a lane named {name} — DRUMS_EDIT_LANES makes {COMP} and {EDIT}"
        ));
    }
    crate::studio::request_resync();
    crate::actions::list().changed("drums.lanes", "the kit plays another lane");
    Ok(played)
}

/// The lane an Edit-mode phase plays, by its slug.
#[must_use]
pub fn lane_of_phase(slug: &str) -> Option<&'static str> {
    match slug {
        "comp" => Some(COMP),
        "edit" => Some(EDIT),
        _ => None,
    }
}

/// The phase choice last followed, by its version.
static FOLLOWED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// In Edit mode, the kit plays its phase's lane.
///
/// Called each tick of the window's loop with the mode it is in, it acts
/// only when the phase was chosen again (from the bottom bar, the rails,
/// a model). A kit with no such lane is left as it is.
pub fn follow_phase(mode: session::modes::Mode) {
    if mode != session::modes::Mode::Edit {
        return;
    }
    let version = session::mode_phases::version();
    if FOLLOWED.swap(version, std::sync::atomic::Ordering::Relaxed) == version {
        return;
    }
    let Some(lane) = session::mode_phases::current(mode).and_then(|p| lane_of_phase(p.slug)) else {
        return;
    };
    let (Some(daw), Some(song)) = (crate::open::local_engine(), crate::open::current_song()) else {
        return;
    };
    if let Err(why) = play(daw, &ProjectContext::Project(song), lane) {
        tracing::debug!(drums.lane = lane, why, "the phase's lane was not played");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_edit_phase_plays_its_lane() {
        assert_eq!(lane_of_phase("comp"), Some(COMP));
        assert_eq!(lane_of_phase("edit"), Some(EDIT));
        assert_eq!(lane_of_phase("balance"), None);
    }

    #[test]
    fn a_lane_is_found_by_name_and_only_while_it_exists() {
        let track = daw_proto::Track {
            lane_count: 3,
            lane_names: vec!["comp".into(), "1".into(), "EDIT".into(), "ghost".into()],
            ..daw_proto::Track::default()
        };
        assert_eq!(lane_named(&track, "COMP"), Some(0));
        assert_eq!(lane_named(&track, " edit "), Some(2));
        assert_eq!(lane_named(&track, "ghost"), None, "past the lane count");
    }
}
