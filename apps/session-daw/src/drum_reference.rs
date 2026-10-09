//! The drum part as written — the song's drum MIDI — beside the drums as
//! played, to edit the recording against.
//!
//! A session's `DRUM MIDI` is the intended part: every fill is not what
//! was played, but it says what the drummer meant, bar by bar. Read in
//! project time ([`notes`]) it is a reference the recorded hits can be
//! held up to ([`compare`]) — how far each hit is from the note it
//! plays, which notes were not played, which hits were not written.
//!
//! The notes' map is a drum library's, not General MIDI, and differs
//! from library to library — so it is not assumed but learned
//! ([`learn_map`]): a note number whose notes land on a piece's trigger
//! hits is that piece's note. A pitch that lands on nothing is a cymbal,
//! an articulation, a piece with no trigger — said as unmapped.

#![cfg(feature = "native")]

use daw::service::{
    ItemRef, Items as _, Midi as _, MidiTakeLocation, ProjectContext, TakeRef, TempoMap as _,
    TrackRef, Tracks,
};

/// One written note, in project time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefNote {
    pub time: f64,
    pub pitch: u8,
    pub velocity: u8,
}

/// Seconds at `qn` quarter notes after `start` seconds, through
/// `tempos` (`(seconds, bpm)`, by time; the tempo before the first point
/// is `default_bpm`). Each tempo holds until the next point.
#[must_use]
pub fn seconds_after(start: f64, qn: f64, tempos: &[(f64, f64)], default_bpm: f64) -> f64 {
    let bpm_at = |t: f64| {
        tempos
            .iter()
            .take_while(|(at, _)| *at <= t + 1e-9)
            .last()
            .map_or(default_bpm, |(_, bpm)| *bpm)
            .max(1.0)
    };
    let mut time = start;
    let mut left = qn;
    loop {
        let bpm = bpm_at(time);
        let next = tempos
            .iter()
            .map(|(at, _)| *at)
            .find(|at| *at > time + 1e-9);
        let per_qn = 60.0 / bpm;
        match next {
            Some(change) if (change - time) / per_qn < left => {
                left -= (change - time) / per_qn;
                time = change;
            }
            _ => return left.mul_add(per_qn, time),
        }
    }
}

/// Every note on `track` the song plays (its playing lane, unmuted
/// items), in project time, by time; a note doubled by overlapping
/// items counted once.
#[must_use]
pub fn notes(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    track: &str,
) -> Vec<RefNote> {
    let Some(t) = Tracks::get(daw, project.clone(), TrackRef::Guid(track.to_owned())) else {
        return Vec::new();
    };
    let tempos: Vec<(f64, f64)> = daw
        .get_tempo_points(project.clone())
        .into_iter()
        .filter_map(|p| Some((p.position.time?.as_seconds(), p.bpm)))
        .collect();
    let default_bpm = daw.get_default_tempo(project.clone());
    let mut out: Vec<RefNote> = Vec::new();
    for item in daw.get_items(project.clone(), TrackRef::Guid(track.to_owned())) {
        if item.muted || !daw_ui::studio::project::on_playing_lane(&t, &item) {
            continue;
        }
        let start = item.position.as_seconds();
        let end = start + item.length.as_seconds();
        let location = MidiTakeLocation::new(
            project.clone(),
            ItemRef::Guid(item.guid.clone()),
            TakeRef::Active,
        );
        for note in daw.notes(location) {
            if note.muted || note.velocity == 0 {
                continue;
            }
            // The engine's `start_ppq` is already quarter notes from the
            // item's start (its loader divides REAPER's ticks out).
            let time = seconds_after(start, note.start_ppq, &tempos, default_bpm);
            if time < end {
                out.push(RefNote {
                    time,
                    pitch: note.pitch,
                    velocity: note.velocity,
                });
            }
        }
    }
    out.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.pitch.cmp(&b.pitch)));
    out.dedup_by(|a, b| a.pitch == b.pitch && (a.time - b.time).abs() < 0.002);
    out
}

/// The drum MIDI track: one named for drum MIDI, else none.
#[must_use]
pub fn find_track(daw: &daw::standalone::Standalone, project: &ProjectContext) -> Option<String> {
    Tracks::all(daw, project.clone())
        .into_iter()
        .find(|t| {
            let name = t.name.to_lowercase();
            !t.is_folder && name.contains("drum") && name.contains("midi")
        })
        .map(|t| t.guid)
}

/// One note number's place in the kit, as learned.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapped {
    pub pitch: u8,
    /// How many notes it has.
    pub notes: usize,
    /// The piece it plays — `None` when it lands on none.
    pub piece: Option<String>,
    /// The share of its notes on that piece's hits (0..1).
    pub agreement: f64,
}

/// How much of a pitch's notes must land on a piece's hits for it to be
/// that piece's note.
pub const MAP_AGREEMENT: f64 = 0.5;

/// Each note number mapped to the piece whose hits most of its notes
/// land on (within `window` seconds) — `pieces` are each piece's name
/// and its hits' times, by time.
///
/// What counts is how much more often than chance: a piece hit all the
/// time — a dead trigger reading its own noise — has every note near one
/// of its "hits", and would take every pitch. `agreement` is the share
/// over what that piece's hit density would give by chance, scaled back
/// to 0..1.
#[must_use]
pub fn learn_map(notes: &[RefNote], pieces: &[(String, Vec<f64>)], window: f64) -> Vec<Mapped> {
    let span = notes
        .iter()
        .map(|n| n.time)
        .chain(pieces.iter().flat_map(|(_, h)| h.iter().copied()))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), t| {
            (lo.min(t), hi.max(t))
        });
    let length = (span.1 - span.0).max(window);
    let chance = |hits: &[f64]| {
        (f64::from(u32::try_from(hits.len()).unwrap_or(u32::MAX)) * 2.0 * window / length).min(1.0)
    };
    let mut pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
    pitches.sort_unstable();
    pitches.dedup();
    pitches
        .into_iter()
        .map(|pitch| {
            let times: Vec<f64> = notes
                .iter()
                .filter(|n| n.pitch == pitch)
                .map(|n| n.time)
                .collect();
            let best = pieces
                .iter()
                .map(|(name, hits)| {
                    let on = times.iter().filter(|t| near(hits, **t, window)).count();
                    let by_chance = chance(hits);
                    let over = if by_chance >= 1.0 {
                        0.0
                    } else {
                        ((ratio(on, times.len()) - by_chance) / (1.0 - by_chance)).max(0.0)
                    };
                    (name, over)
                })
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let (piece, agreement) = match best {
                Some((name, share)) if share >= MAP_AGREEMENT => (Some(name.clone()), share),
                Some((_, share)) => (None, share),
                None => (None, 0.0),
            };
            Mapped {
                pitch,
                notes: times.len(),
                piece,
                agreement,
            }
        })
        .collect()
}

fn ratio(part: usize, whole: usize) -> f64 {
    let part = u32::try_from(part).unwrap_or(u32::MAX);
    let whole = u32::try_from(whole).unwrap_or(u32::MAX).max(1);
    f64::from(part) / f64::from(whole)
}

/// Whether a time in `sorted` is within `window` of `t`.
fn near(sorted: &[f64], t: f64, window: f64) -> bool {
    let at = sorted.partition_point(|x| *x < t - window);
    sorted.get(at).is_some_and(|x| (x - t).abs() <= window)
}

/// The written notes against the played hits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comparison {
    /// Each note played: the note's time, the hit's, the hit's offset
    /// from it (ms, positive late).
    pub matched: Vec<(f64, f64, f64)>,
    /// Notes with no hit near them: written, not played.
    pub missing: Vec<f64>,
    /// Hits with no note near them: played, not written.
    pub extra: Vec<f64>,
}

/// Pair each note with the nearest hit within `window` seconds, each hit
/// used once, in order; what is left over is missing or extra. Both
/// sorted by time.
#[must_use]
pub fn compare(notes: &[f64], hits: &[f64], window: f64) -> Comparison {
    let mut out = Comparison::default();
    let mut used = vec![false; hits.len()];
    for &note in notes {
        let from = hits.partition_point(|h| *h < note - window);
        let best = hits
            .iter()
            .enumerate()
            .skip(from)
            .take_while(|(_, h)| **h <= note + window)
            .filter(|(i, _)| !used.get(*i).copied().unwrap_or(true))
            .min_by(|a, b| (a.1 - note).abs().total_cmp(&(b.1 - note).abs()))
            .map(|(i, h)| (i, *h));
        match best {
            Some((i, hit)) => {
                if let Some(u) = used.get_mut(i) {
                    *u = true;
                }
                out.matched.push((note, hit, (hit - note) * 1_000.0));
            }
            None => out.missing.push(note),
        }
    }
    out.extra = hits
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(h, _)| *h)
        .collect();
    out
}

/// The kit's pieces with a trigger, and their triggers: a folder in the
/// Drums folder with a trigger in it or in its Sum — `(name, trigger
/// GUID)`.
#[must_use]
pub fn pieces(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
) -> Vec<(String, String)> {
    let tracks = Tracks::all(daw, project.clone());
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let mut out: Vec<(String, String)> = Vec::new();
    for trigger in tracks
        .iter()
        .filter(|t| !t.is_folder && dynamic_template::apply::is_trigger(&t.name))
    {
        let path = crate::gain_stage::path_of(trigger, &by_guid);
        if !path
            .first()
            .is_some_and(|p| p.trim().eq_ignore_ascii_case("drums"))
        {
            continue;
        }
        // The piece: the folder it is in, or above its Sum — a folder of
        // triggers (an unsplit kit's toms) is not a piece.
        let mut parent = trigger
            .parent_guid
            .as_deref()
            .and_then(|p| by_guid.get(p).copied());
        if parent.is_some_and(|p| p.name.trim().eq_ignore_ascii_case("sum")) {
            parent = parent
                .and_then(|p| p.parent_guid.as_deref())
                .and_then(|p| by_guid.get(p).copied());
        }
        if let Some(piece) = parent.filter(|p| !dynamic_template::apply::is_trigger(&p.name)) {
            out.push((piece.name.clone(), trigger.guid.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{RefNote, compare, learn_map, seconds_after};

    /// A quarter at 120 is half a second; across a change to 60, the rest
    /// is a second a quarter.
    #[test]
    fn quarters_walk_the_tempo_map() {
        assert!((seconds_after(1.0, 4.0, &[], 120.0) - 3.0).abs() < 1e-9);
        let tempos = [(0.0, 120.0), (2.0, 60.0)];
        // 4 quarters from 0: 2 s at 120, then 0 left → 2 s; 6: 2 more at 60.
        assert!((seconds_after(0.0, 4.0, &tempos, 120.0) - 2.0).abs() < 1e-9);
        assert!((seconds_after(0.0, 6.0, &tempos, 120.0) - 4.0).abs() < 1e-9);
    }

    /// A note number whose notes land on a piece's hits is that piece's;
    /// one that lands on nothing is unmapped.
    #[test]
    fn the_map_is_learned_off_the_hits() {
        let kick = vec![0.0, 1.0, 2.0, 3.0];
        let snare = vec![0.5, 1.5, 2.5, 3.5];
        let note = |time, pitch| RefNote {
            time,
            pitch,
            velocity: 100,
        };
        let notes = vec![
            note(0.005, 24),
            note(1.0, 24),
            note(2.01, 24),
            note(3.0, 24),
            note(0.5, 26),
            note(1.49, 26),
            note(2.5, 26),
            note(0.25, 55),
            note(0.75, 55),
        ];
        let map = learn_map(
            &notes,
            &[("Kick".into(), kick), ("Snare".into(), snare)],
            0.03,
        );
        let of = |p: u8| {
            map.iter()
                .find(|m| m.pitch == p)
                .and_then(|m| m.piece.clone())
        };
        assert_eq!(of(24).as_deref(), Some("Kick"));
        assert_eq!(of(26).as_deref(), Some("Snare"));
        assert_eq!(of(55), None, "lands on nothing: a cymbal");

        // A dead trigger "hit" every 20 ms is near every note, by chance
        // alone: it takes nothing.
        let noise: Vec<f64> = (0..200).map(|i| f64::from(i) * 0.02).collect();
        let map = learn_map(
            &notes,
            &[
                ("Kick".into(), vec![0.0, 1.0, 2.0, 3.0]),
                ("Snare".into(), vec![0.5, 1.5, 2.5, 3.5]),
                ("Dead".into(), noise),
            ],
            0.03,
        );
        let of = |p: u8| {
            map.iter()
                .find(|m| m.pitch == p)
                .and_then(|m| m.piece.clone())
        };
        assert_eq!(of(24).as_deref(), Some("Kick"));
        assert_eq!(of(55), None);
    }

    /// Each note takes its nearest hit once; a note with none is missing,
    /// a hit with no note extra; offsets are late-positive, in ms.
    #[test]
    fn notes_are_held_up_to_hits() {
        let c = compare(&[1.0, 2.0, 3.0], &[1.01, 1.02, 2.995, 5.0], 0.05);
        assert_eq!(c.matched.len(), 2);
        assert!((c.matched[0].2 - 10.0).abs() < 1e-6);
        assert!((c.matched[1].2 + 5.0).abs() < 1e-6);
        assert_eq!(c.missing, vec![2.0]);
        assert_eq!(c.extra, vec![1.02, 5.0]);
    }
}
