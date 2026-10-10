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
    #[test]
    fn steps_skip_the_empty_numbered_lanes() {
        let t = track();
        assert_eq!(steps(&t, &[]), vec![0, 1, 35]);
        assert_eq!(label(&t), "EDIT");
        assert_eq!(stepped(&t, &[], true), Some(1));
        assert_eq!(stepped(&t, &[], false), Some(1 << 1));
    }
}
