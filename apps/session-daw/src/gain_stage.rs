//! Gain staging: every recorded track brought to one reference level,
//! without touching its audio.
//!
//! A track's level is read from its takes' peaks — its strong hits, the
//! 90th percentile of the 100 ms windows within [`HIT_RANGE_DB`] of its
//! loudest — and the difference to its role's [`target`] goes on every
//! item as item volume. Nothing is rendered or written into the audio: an
//! item's volume set back to 1 is the track as it was recorded.
//!
//! The targets are a sampled kit's: GGD Modern & Massive 2, the loudest
//! velocity layer of each piece's mics, measured. A kit staged to it sits
//! at that library's balance with every fader at unity, so a staged kit
//! is processed the way that kit is — and every song's the same way.
//!
//! A guitar DI is never staged: it feeds amps and reamps at the level it
//! was recorded at, and stays there. A bass DI is.
//!
//! And the kit is laid out: the staged tracks' faders at unity, the toms
//! with hits spread across the stereo field in order — the first 80%
//! left, the last 80% right — and an `L` / `R` pair hard to its side.

use daw::service::{ItemRef, Items, Peaks, ProjectContext, TakeRef, TrackRef, Tracks};

/// What a recorded track is, for its level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    KickIn,
    KickOut,
    KickSub,
    SnareTop,
    SnareBottom,
    Tom,
    HiHat,
    Ride,
    Overheads,
    Room,
    BassDi,
}

/// The level a role's strong hits are brought to, dBFS peak.
///
/// GGD Modern & Massive 2, the loudest velocity layer, close mics and
/// rooms averaged over three drums of each piece and their round robins
/// (kick In 1 0.0, In 2 −6.2, Sub −4.8; snare Top −3.3, Bottom −13.2; toms
/// −7.2 to −7.7; hats −26.0; ride −21.3; the snare in the overheads
/// −10.6, in the rooms −8.3 to −9.2). A bass DI has no kit to follow:
/// −10, a DI's strong notes with room above them.
#[must_use]
pub const fn target(role: Role) -> f64 {
    match role {
        Role::KickIn => 0.0,
        Role::KickOut => -6.2,
        Role::KickSub => -4.8,
        Role::SnareTop => -3.3,
        Role::SnareBottom => -13.2,
        Role::Tom => -7.5,
        Role::HiHat => -26.0,
        Role::Ride => -21.3,
        Role::Overheads => -10.6,
        Role::Room => -8.5,
        Role::BassDi => -10.0,
    }
}

/// How far under a track's loudest a window still counts as a strong hit.
///
/// Under it is bleed and ring, and the soft notes. 12, not more: a tom
/// hit a dozen times a song is mostly the kit in its mic, and at 24 dB
/// that bleed outnumbered the hits (unbreakable's T4 read −35 dBFS).
pub const HIT_RANGE_DB: f64 = 12.0;

/// How far a track is moved at most: past it, the reading is wrong (a
/// track of nothing but bleed), not the track.
const MOST_DB: f64 = 24.0;

/// The role of a track by where the organizer filed it: `path` its
/// folders from the top, then its own name (`Drums`, `Kick`, `In`).
/// `None` for anything not staged — a guitar DI above all, and a trigger.
#[must_use]
pub fn role(path: &[&str]) -> Option<Role> {
    let lower: Vec<String> = path.iter().map(|p| p.trim().to_lowercase()).collect();
    let name = lower.last()?;
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    let under = |what: &str| lower.iter().any(|p| p.contains(what));
    // A trigger is a sampler's source; the references, a rehearsal's
    // `Kick - Song` among them, are for listening against.
    if has("trig")
        || has("trigger")
        || under(&dynamic_template::apply::REFERENCES_FOLDER.to_lowercase())
    {
        return None;
    }
    if has("di") {
        return (under("bass") && !under("guitar")).then_some(Role::BassDi);
    }
    if !under("drum") && !under("kit") {
        return None;
    }
    let tom = words.iter().any(|w| {
        *w == "tom" || *w == "toms" || w.strip_prefix('t').is_some_and(|n| n.parse::<u8>().is_ok())
    });
    Some(if under("kick") || has("kick") {
        if has("sub") {
            Role::KickSub
        } else if has("out") {
            Role::KickOut
        } else {
            Role::KickIn
        }
    } else if under("snare") || has("snare") {
        if has("bottom") || has("bot") {
            Role::SnareBottom
        } else {
            Role::SnareTop
        }
    } else if tom || under("tom") {
        Role::Tom
    } else if has("hat") || has("hats") || has("hh") || name.contains("hi hat") {
        Role::HiHat
    } else if has("ride") {
        Role::Ride
    } else if has("room") || has("rooms") || under("room") {
        Role::Room
    } else if has("oh") || has("ohs") || name.contains("overhead") || under("cymbal") {
        Role::Overheads
    } else {
        return None;
    })
}

/// A track's strong-hit level, dBFS, from its peaks.
///
/// The peaks are per-point absolute maxima, `step` seconds each; the
/// level is the 90th percentile of its 100 ms windows' peaks within
/// [`HIT_RANGE_DB`] of the loudest. `None` for silence.
#[must_use]
pub fn level(points: &[f32], step: f64) -> Option<f64> {
    if step <= 0.0 {
        return None;
    }
    let per = usize::try_from(std::time::Duration::from_millis(100).as_nanos())
        .ok()
        .zip(usize::try_from(std::time::Duration::from_secs_f64(step).as_nanos()).ok())
        .and_then(|(window, point)| window.checked_div(point))
        .unwrap_or(1)
        .max(1);
    let windows: Vec<f32> = points
        .chunks(per)
        .map(|w| w.iter().fold(0.0_f32, |p, x| p.max(x.abs())))
        .collect();
    let loudest = windows.iter().copied().fold(0.0_f32, f32::max);
    if loudest <= 0.0 {
        return None;
    }
    let floor = f64::from(loudest) * 10.0_f64.powf(-HIT_RANGE_DB / 20.0);
    let mut hits: Vec<f64> = windows
        .into_iter()
        .map(f64::from)
        .filter(|w| *w >= floor)
        .collect();
    hits.sort_by(f64::total_cmp);
    // Nearest rank, over the gaps between hits: ten hits read the ninth,
    // not the one loudest.
    let at = hits.len().saturating_sub(1).saturating_mul(9) / 10;
    hits.get(at.min(hits.len().saturating_sub(1)))
        .map(|peak| 20.0 * peak.log10())
}

/// Where `count` toms sit, in order: spread evenly from 80% left to 80%
/// right — one, in the middle.
#[must_use]
pub fn tom_pans(count: usize) -> Vec<f64> {
    const WIDE: f64 = 0.8;
    match count {
        0 => Vec::new(),
        1 => vec![0.0],
        n => {
            let steps = u32::try_from(n.saturating_sub(1)).unwrap_or(u32::MAX);
            (0..steps.saturating_add(1))
                .map(|i| -WIDE + 2.0 * WIDE * f64::from(i) / f64::from(steps))
                .collect()
        }
    }
}

/// A side named in a track's name: `OH L` is −1, `Room R` 1.
fn side(name: &str) -> Option<f64> {
    let last = name.split_whitespace().last()?.to_lowercase();
    match last.as_str() {
        "l" | "left" => Some(-1.0),
        "r" | "right" => Some(1.0),
        _ => None,
    }
}

/// A track's folders from the top, then its name.
fn path_of<'a>(
    track: &'a daw_proto::Track,
    by_guid: &std::collections::HashMap<&str, &'a daw_proto::Track>,
) -> Vec<&'a str> {
    let mut path = vec![track.name.as_str()];
    let mut at = track
        .parent_guid
        .as_deref()
        .and_then(|p| by_guid.get(p).copied());
    while let Some(parent) = at {
        path.push(parent.name.as_str());
        at = parent
            .parent_guid
            .as_deref()
            .and_then(|p| by_guid.get(p).copied());
    }
    path.reverse();
    path
}

/// What staging did to one track.
#[derive(Clone, Debug, PartialEq)]
pub struct Staged {
    pub name: String,
    pub role: Role,
    /// Its strong-hit level before, dBFS.
    pub level: f64,
    /// The item volume put on, dB.
    pub gain_db: f64,
}

/// What [`plan`] decided, and [`stage`] / [`stage_live`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub staged: Vec<Staged>,
    /// Tracks with a role but no level (nothing recorded on them).
    pub silent: Vec<String>,
    /// The toms panned, in order, and where.
    pub toms: Vec<(String, f64)>,
}

/// One track's part of a [`Plan`].
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub track: String,
    /// The item volume every one of its items gets.
    pub volume: f64,
    /// Its pan, when it is set.
    pub pan: Option<f64>,
    pub items: Vec<String>,
}

/// Everything staging changes, decided before any of it is done.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub moves: Vec<Move>,
    pub report: Report,
}

/// Decide the staging of `tracks`: `items` every item of the project,
/// `wave` an item's peaks ([`crate::midi::Wave`], read from its active
/// take), asked only of the items on a playing lane.
#[must_use]
pub fn plan(
    tracks: &[daw_proto::Track],
    items: &[daw_proto::Item],
    mut wave: impl FnMut(&daw_proto::Item) -> Option<crate::midi::Wave>,
) -> Plan {
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let mut out = Plan::default();
    let mut toms: Vec<usize> = Vec::new();
    for track in tracks.iter().filter(|t| !t.is_folder) {
        let Some(role) = role(&path_of(track, &by_guid)) else {
            continue;
        };
        let own: Vec<&daw_proto::Item> = items
            .iter()
            .filter(|i| i.track_guid == track.guid)
            .collect();
        // Read from what plays: a fixed-lane track's other lanes hold
        // other takes (unbreakable's, another song's), at their own
        // levels. The gain goes on every item, so a lane switched to is
        // staged alike.
        let mut points: Vec<f32> = Vec::new();
        let mut step = 0.0;
        for item in own
            .iter()
            .filter(|item| daw_ui::studio::project::on_playing_lane(track, item))
        {
            if let Some(read) = wave(item) {
                step = read.step;
                points.extend(
                    read.points
                        .iter()
                        .map(|(max, min)| max.abs().max(min.abs())),
                );
            }
        }
        let Some(level) = level(&points, step) else {
            out.report.silent.push(track.name.clone());
            continue;
        };
        let gain_db = (target(role) - level).clamp(-MOST_DB, MOST_DB);
        if role == Role::Tom {
            toms.push(out.moves.len());
        }
        out.moves.push(Move {
            track: track.guid.clone(),
            volume: 10.0_f64.powf(gain_db / 20.0),
            pan: side(&track.name),
            items: own.iter().map(|i| i.guid.clone()).collect(),
        });
        out.report.staged.push(Staged {
            name: track.name.clone(),
            role,
            level,
            gain_db,
        });
    }
    for (at, pan) in toms.iter().zip(tom_pans(toms.len())) {
        if let Some(each) = out.moves.get_mut(*at) {
            each.pan = Some(pan);
            let name = out
                .report
                .staged
                .get(*at)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            out.report.toms.push((name, pan));
        }
    }
    out
}

/// Stage `project` on an engine in hand (prepare's): [`plan`], then the
/// item volumes, the faders at unity and the pans.
///
/// # Errors
///
/// A backend call failed.
pub fn stage<D>(daw: &D, project: &ProjectContext) -> eyre::Result<Report>
where
    D: Tracks + Items + Peaks,
{
    let tracks = Tracks::all(daw, project.clone());
    let items = daw.get_all_items(project.clone());
    let planned = plan(&tracks, &items, |item| {
        let data = daw.take_peaks(
            project.clone(),
            ItemRef::Guid(item.guid.clone()),
            TakeRef::Active,
            crate::midi::WAVE_BLOCK,
        );
        crate::midi::Wave::from_peaks(&data)
    });
    for each in &planned.moves {
        for item in &each.items {
            Items::set_volume(
                daw,
                project.clone(),
                ItemRef::Guid(item.clone()),
                each.volume,
            )?;
        }
        let at = TrackRef::Guid(each.track.clone());
        Tracks::set_volume(daw, project.clone(), at.clone(), 1.0)?;
        if let Some(pan) = each.pan {
            Tracks::set_pan(daw, project.clone(), at, pan)?;
        }
    }
    Ok(planned.report)
}

/// Stage the session through the facade — whatever this window drives,
/// the engine or REAPER: [`plan`], then the same changes as [`stage`].
///
/// # Errors
///
/// A facade call failed.
pub async fn stage_live(project: &daw_control::Project) -> eyre::Result<Report> {
    let tracks = project.tracks().all().await?;
    let items = project.items().all().await?;
    let mut waves: std::collections::HashMap<String, crate::midi::Wave> =
        std::collections::HashMap::new();
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    for item in &items {
        // Only what [`plan`] will ask for: a staged track's playing lane.
        let Some(track) = by_guid.get(item.track_guid.as_str()) else {
            continue;
        };
        if role(&path_of(track, &by_guid)).is_none()
            || !daw_ui::studio::project::on_playing_lane(track, item)
        {
            continue;
        }
        let Ok(Some(handle)) = project.items().by_guid(&item.guid).await else {
            continue;
        };
        if let Ok(data) = handle.active_take().peaks(crate::midi::WAVE_BLOCK).await
            && let Some(wave) = crate::midi::Wave::from_peaks(&data)
        {
            waves.insert(item.guid.clone(), wave);
        }
    }
    let planned = plan(&tracks, &items, |item| waves.remove(&item.guid));
    for each in &planned.moves {
        for guid in &each.items {
            if let Some(item) = project.items().by_guid(guid).await? {
                item.set_volume(each.volume).await?;
            }
        }
        if let Some(track) = project.tracks().by_guid(&each.track).await? {
            track.set_volume(1.0).await?;
            if let Some(pan) = each.pan {
                track.set_pan(pan).await?;
            }
        }
    }
    Ok(planned.report)
}

/// Undo staging through the facade: every item of a track staging would
/// stage back at an item volume of 1 — the track as it was recorded.
/// Faders and pans stay where they are. Returns the tracks cleared.
///
/// # Errors
///
/// A facade call failed.
pub async fn clear_live(project: &daw_control::Project) -> eyre::Result<Vec<String>> {
    let tracks = project.tracks().all().await?;
    let items = project.items().all().await?;
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let mut cleared = Vec::new();
    for track in tracks.iter().filter(|t| !t.is_folder) {
        if role(&path_of(track, &by_guid)).is_none() {
            continue;
        }
        for item in items.iter().filter(|i| i.track_guid == track.guid) {
            if (item.volume - 1.0).abs() > 1e-9
                && let Some(handle) = project.items().by_guid(&item.guid).await?
            {
                handle.set_volume(1.0).await?;
            }
        }
        cleared.push(track.name.clone());
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::{Role, level, role, tom_pans};

    /// A track's role is where it was filed: the kit by piece and mic, a
    /// bass DI staged, a guitar DI and a trigger never.
    #[test]
    fn a_track_is_staged_by_where_it_was_filed() {
        assert_eq!(role(&["Drums", "Kick", "In"]), Some(Role::KickIn));
        assert_eq!(role(&["Drums", "Kick", "Out"]), Some(Role::KickOut));
        assert_eq!(role(&["Drums", "Kick", "Sub"]), Some(Role::KickSub));
        assert_eq!(role(&["Drums", "Snare", "Top"]), Some(Role::SnareTop));
        assert_eq!(role(&["Drums", "Snare", "Bottom"]), Some(Role::SnareBottom));
        assert_eq!(role(&["Drums", "Toms", "T3"]), Some(Role::Tom));
        assert_eq!(role(&["Drums", "Cymbals", "OHs"]), Some(Role::Overheads));
        assert_eq!(role(&["Drums", "Cymbals", "Hi Hat"]), Some(Role::HiHat));
        assert_eq!(role(&["Drums", "Cymbals", "Ride"]), Some(Role::Ride));
        assert_eq!(role(&["Drums", "Rooms", "Rooms"]), Some(Role::Room));
        assert_eq!(role(&["Bass", "Bass DI"]), Some(Role::BassDi));
        assert_eq!(
            role(&["Guitars", "Clean", "L", "DI"]),
            None,
            "a guitar DI never"
        );
        assert_eq!(role(&["Drums", "Kick", "Trig"]), None, "a trigger never");
        assert_eq!(role(&["Vocals", "Lead Vocal"]), None);
        assert_eq!(
            role(&["REFERENCES", "REHEARSAL", "Drums", "Kick - Unbreakable"]),
            None,
            "a rehearsal's kick is no mic"
        );
    }

    /// The strong hits, not the loudest one nor the bleed: two hits at
    /// −6 dBFS and one at 0 in a bed of bleed at −40 read −6.
    #[test]
    fn the_level_is_the_strong_hits() {
        let step = 0.02;
        let bleed = 10.0_f32.powi(-2);
        let mut points = vec![bleed; 500];
        let hit = 10.0_f32.powf(-6.0 / 20.0);
        for at in [50, 100, 150, 200, 250, 300, 350, 400, 450] {
            points[at] = hit;
        }
        points[475] = 1.0;
        let read = level(&points, step).expect("a level");
        assert!((read - -6.0).abs() < 0.1, "{read}");
        assert_eq!(level(&[0.0; 10], step), None, "silence");
    }

    /// The plan: a staged track's gain on all its items, read from its
    /// playing lane only; a silent tom left out of the spread; a guitar
    /// DI untouched.
    #[test]
    fn the_plan_stages_what_plays_and_spreads_the_toms_heard() {
        let folder = |guid: &str, name: &str, parent: Option<&str>| daw_proto::Track {
            guid: guid.into(),
            name: name.into(),
            is_folder: true,
            parent_guid: parent.map(Into::into),
            ..daw_proto::Track::default()
        };
        let track = |guid: &str, name: &str, parent: &str| daw_proto::Track {
            guid: guid.into(),
            name: name.into(),
            parent_guid: Some(parent.into()),
            ..daw_proto::Track::default()
        };
        let mut kick_in = track("in", "In", "kick");
        kick_in.lane_count = 2;
        kick_in.lane_play_mask = 0b1;
        let tracks = vec![
            folder("drums", "Drums", None),
            folder("kick", "Kick", Some("drums")),
            kick_in,
            folder("toms", "Toms", Some("drums")),
            track("t1", "T1", "toms"),
            track("t2", "T2", "toms"),
            track("t3", "T3", "toms"),
            folder("gtr", "Guitars", None),
            track("di", "DI", "gtr"),
        ];
        let item = |guid: &str, on: &str, lane: Option<u32>| daw_proto::Item {
            guid: guid.into(),
            track_guid: on.into(),
            fixed_lane: lane,
            ..daw_proto::Item::default()
        };
        let items = vec![
            item("in-play", "in", Some(0)),
            item("in-other", "in", Some(1)),
            item("t1-a", "t1", None),
            item("t3-a", "t3", None),
            item("di-a", "di", None),
        ];
        let at = |db: f32| crate::midi::Wave {
            step: 0.02,
            points: vec![(10.0_f32.powf(db / 20.0), 0.0); 50],
        };
        let mut asked = Vec::new();
        let planned = super::plan(&tracks, &items, |item| {
            asked.push(item.guid.clone());
            match item.guid.as_str() {
                "in-play" => Some(at(-12.0)),
                "in-other" => Some(at(-1.0)),
                "t1-a" | "t3-a" => Some(at(-20.0)),
                _ => None,
            }
        });
        assert!(
            !asked.contains(&"in-other".to_string()),
            "the other lane is not read"
        );
        assert!(
            !asked.contains(&"di-a".to_string()),
            "a guitar DI is not read"
        );
        let kick = &planned.moves[0];
        assert_eq!(
            kick.items,
            ["in-play", "in-other"],
            "but every item gets the gain"
        );
        assert!(
            20.0_f64.mul_add(kick.volume.log10(), -12.0).abs() < 0.1,
            "-12 to 0: +12 dB"
        );
        assert_eq!(planned.report.silent, ["T2"]);
        let toms: Vec<(String, f64)> = planned.report.toms.clone();
        assert_eq!(toms, [("T1".to_string(), -0.8), ("T3".to_string(), 0.8)]);
    }

    #[test]
    fn the_toms_spread_eighty_left_to_eighty_right() {
        let pans = tom_pans(4);
        assert_eq!(pans.len(), 4);
        assert!((pans[0] + 0.8).abs() < 1e-9 && (pans[3] - 0.8).abs() < 1e-9);
        assert!(pans[1] < 0.0 && pans[2] > 0.0);
        assert_eq!(tom_pans(1), [0.0]);
    }
}
