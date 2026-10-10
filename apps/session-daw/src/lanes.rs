//! A track's fixed lanes, as the panel steps through them.
//!
//! REAPER 7 keeps a track's takes, its comp and its edit as lanes; one
//! plays. The panel's lane line names the playing lane and steps to the
//! lane before or after it — among the lanes worth stepping to: those
//! holding items or named for what they are (`COMP`, `EDIT`), never the
//! empty numbered ones a recording leaves between them.

/// The lanes of `track` worth stepping to, in order: named, or holding
/// one of `items`.
#[must_use]
pub fn steps(track: &daw_proto::Track, items: &[daw_proto::Item]) -> Vec<u32> {
    (0..track.lane_count.min(64))
        .filter(|&lane| {
            let named = name(track, lane).is_some();
            named || items.iter().any(|i| i.fixed_lane.unwrap_or(0) == lane)
        })
        .collect()
}

/// Lane `lane`'s name, when it has one other than its number.
#[must_use]
pub fn name(track: &daw_proto::Track, lane: u32) -> Option<&str> {
    let name = track.lane_names.get(usize::try_from(lane).ok()?)?.trim();
    (!name.is_empty() && name.parse::<u32>().is_err()).then_some(name)
}

/// The lane that plays (the first, when several do).
#[must_use]
pub fn playing(track: &daw_proto::Track) -> Option<u32> {
    (0..track.lane_count.min(64)).find(|lane| track.lane_play_mask & (1_u64 << lane) != 0)
}

/// What the lane line shows: the playing lane's name, or its number
/// from 1 as REAPER numbers lanes.
#[must_use]
pub fn label(track: &daw_proto::Track) -> String {
    playing(track).map_or_else(
        || "-".to_owned(),
        |lane| name(track, lane).map_or_else(|| lane.saturating_add(1).to_string(), str::to_owned),
    )
}

/// The play mask after a step forward (`next`) or back from the playing
/// lane, round the ends; `None` with nowhere else to go.
#[must_use]
pub fn stepped(track: &daw_proto::Track, items: &[daw_proto::Item], next: bool) -> Option<u64> {
    let steps = steps(track, items);
    if steps.len() < 2 {
        return None;
    }
    let at = playing(track)
        .and_then(|lane| steps.iter().position(|&s| s == lane))
        .unwrap_or(0);
    let count = steps.len();
    let to = if next {
        at.saturating_add(1).checked_rem(count)?
    } else {
        at.checked_sub(1).unwrap_or_else(|| count.saturating_sub(1))
    };
    Some(1_u64 << steps.get(to)?)
}

/// The other tracks `guid`'s media-edit group moves with it, while
/// grouping is on: those following the group it leads.
#[must_use]
pub fn grouped_tracks(tracks: &[daw_proto::Track], guid: &str) -> Vec<String> {
    if !crate::options::GROUPING.get() {
        return Vec::new();
    }
    let Some(lead) = tracks
        .iter()
        .find(|t| t.guid == guid)
        .map(|t| t.grouping.media_edit_lead)
        .filter(|lead| *lead != 0)
    else {
        return Vec::new();
    };
    tracks
        .iter()
        .filter(|t| t.guid != guid && t.grouping.media_edit_follow & lead != 0)
        .map(|t| t.guid.clone())
        .collect()
}

/// The tracks a row's lane line speaks for: a track with lanes itself; a
/// folder the tracks under it that have lanes (all of a kit's mics, say);
/// none for a track with one lane.
#[must_use]
pub fn shown_on<'a>(
    tracks: &'a [daw_proto::Track],
    track: &'a daw_proto::Track,
) -> Vec<&'a daw_proto::Track> {
    if !track.is_folder {
        return if track.lane_count > 1 {
            vec![track]
        } else {
            Vec::new()
        };
    }
    tracks
        .iter()
        .filter(|t| t.lane_count > 1 && !t.is_folder && is_under(tracks, t, &track.guid))
        .collect()
}

/// Whether `track` is somewhere inside the folder `folder`.
fn is_under(tracks: &[daw_proto::Track], track: &daw_proto::Track, folder: &str) -> bool {
    let mut at = track.parent_guid.as_deref();
    for _ in 0..tracks.len() {
        match at {
            Some(guid) if guid == folder => return true,
            Some(guid) => {
                at = tracks
                    .iter()
                    .find(|t| t.guid == guid)
                    .and_then(|t| t.parent_guid.as_deref());
            }
            None => return false,
        }
    }
    false
}

/// What a lane line over `tracks` names: the lane they all play, or
/// `mixed` when they differ.
#[must_use]
pub fn label_of(tracks: &[&daw_proto::Track]) -> String {
    let mut names = tracks.iter().map(|t| label(t));
    let Some(first) = names.next() else {
        return String::new();
    };
    if names.all(|n| n == first) {
        first
    } else {
        "mixed".to_owned()
    }
}

/// The play mask for `track` that plays its lane called `name`.
#[must_use]
pub fn mask_named(track: &daw_proto::Track, name: &str) -> Option<u64> {
    (0..track.lane_count.min(64))
        .find(|&lane| name_or_number(track, lane) == name)
        .map(|lane| 1_u64 << lane)
}

fn name_or_number(track: &daw_proto::Track, lane: u32) -> String {
    name(track, lane).map_or_else(|| lane.saturating_add(1).to_string(), str::to_owned)
}

/// The tracks that show all their lanes; the rest show only the one that
/// plays.
fn expanded_set() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static EXPANDED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::LazyLock::new(Default::default);
    &EXPANDED
}

/// Whether `guid` shows all its lanes (it shows the one that plays until
/// expanded).
#[must_use]
pub fn expanded(guid: &str) -> bool {
    expanded_set().lock().is_ok_and(|set| set.contains(guid))
}

/// Show all of each of `guids`' lanes, or only the playing one again: all
/// expanded if any was not, else all collapsed. The rows are planned again.
pub fn toggle_expanded(guids: &[String]) {
    if let Ok(mut set) = expanded_set().lock() {
        let open = guids.iter().any(|g| !set.contains(g));
        for g in guids {
            if open {
                set.insert(g.clone());
            } else {
                set.remove(g);
            }
        }
    }
    crate::folds::touch();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> daw_proto::Track {
        daw_proto::Track {
            lane_count: 36,
            lane_names: [
                vec!["COMP".to_owned(), "C1".to_owned()],
                (3..36).map(|n| n.to_string()).collect(),
                vec!["EDIT".to_owned()],
            ]
            .concat(),
            lane_play_mask: 1 << 35,
            ..daw_proto::Track::default()
        }
    }

    /// The empty numbered lanes between the comp and the edit are not
    /// stepped through: from EDIT forward is round to COMP, back is C1.
    /// A folder's lane line speaks for the tracks under it that have lanes,
    /// and names the lane they all play.
    #[test]
    fn a_folder_names_the_lane_its_tracks_play() {
        let folder = daw_proto::Track {
            guid: "kick".to_owned(),
            is_folder: true,
            lane_count: 2,
            lane_names: vec!["COMP".to_owned(), "C1".to_owned()],
            lane_play_mask: 1 << 1,
            ..daw_proto::Track::default()
        };
        let mic = |guid: &str| daw_proto::Track {
            guid: guid.to_owned(),
            parent_guid: Some("kick".to_owned()),
            ..track()
        };
        let tracks = vec![folder, mic("in"), mic("out")];
        let under = shown_on(&tracks, &tracks[0]);
        assert_eq!(under.len(), 2, "the mics, not the folder's own lanes");
        assert_eq!(label_of(&under), "EDIT");
        assert_eq!(mask_named(&tracks[1], "COMP"), Some(1));
    }

    #[test]
    fn steps_skip_the_empty_numbered_lanes() {
        let t = track();
        assert_eq!(steps(&t, &[]), vec![0, 1, 35]);
        assert_eq!(label(&t), "EDIT");
        assert_eq!(stepped(&t, &[], true), Some(1));
        assert_eq!(stepped(&t, &[], false), Some(1 << 1));
    }
}
