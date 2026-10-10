//! Drum samples under the recorded kit: signal's sampled kit, played from
//! the hits the triggers caught.
//!
//! Session reads the kit's hits offline (its triggers, through the drum
//! edit's detector, on the EDIT lane once there is one); signal plays a
//! sampled kit from notes. This joins them, blended under the recording:
//!
//! - each piece's hits written as notes on its **Sample** track (the
//!   kit's General MIDI note for the piece, the velocity from the hit's
//!   level), and an `FTS Kit Mic` there playing that piece's **close**
//!   mics;
//! - an **OH Sample** track beside the overheads and a **Room Sample**
//!   track beside the rooms, every piece's notes on each, playing the
//!   kit's overhead and room mics — sent into the recorded overheads and
//!   rooms, so the sample's ambience is processed and faded with theirs.
//!
//! A piece's voices on all three get the same notes, so each hit is the
//! same sample on every one (signal picks a round robin from the notes).

use daw::service::TempoMap as _;
use daw::service::{FxChainContext, ItemRef, Items as _, ProjectContext, TrackRef, Tracks};
use daw_proto::{Midi as _, MidiNoteCreate, MidiTakeLocation, Routing as _, TakeRef};

/// The kit's note for a piece of the session's kit, by its name: General
/// MIDI's kick and snare, the toms high to low onto the kit's rack then
/// floor toms (`Tom 1` the smallest).
#[must_use]
pub fn note_for(piece: &str) -> Option<u8> {
    let name = piece.trim().to_ascii_lowercase();
    if name.contains("kick") {
        return Some(36);
    }
    if name.contains("snare") {
        return Some(38);
    }
    let tom = name.strip_prefix("tom")?.trim().parse::<u8>().ok()?;
    [45, 47, 41, 43]
        .get(usize::from(tom.checked_sub(1)?))
        .copied()
}

/// A hit's velocity from its level against its piece's loudest (dB, 0
/// the loudest): the loudest at 127, half the velocity every 12 dB down.
#[must_use]
pub fn velocity(relative_db: f64) -> u8 {
    let v = (127.0 * 10_f64.powf(relative_db.min(0.0) / 40.0)).round();
    num_traits::ToPrimitive::to_u8(&v.clamp(1.0, 127.0)).unwrap_or(1)
}

/// How long each written note is, quarter notes: a drum's sample plays
/// out whatever the length.
const NOTE_QN: f64 = 0.25;

/// A project file's MIDI resolution, ticks a quarter note: a note is
/// written on one, so the saved song plays it where the live one did (off
/// a tick, a hit moved by up to half of one, ~0.3 ms at 100 bpm, on save).
const TICKS_PER_QN: f64 = 960.0;

/// What setting the samples up did, or would.
#[derive(Debug, Default)]
pub struct Report {
    /// One line a track.
    pub tracks: Vec<String>,
    pub notes: usize,
}

/// One piece's notes: `(time, note, velocity)`.
type Notes = Vec<(f64, u8, u8)>;

/// Set the samples up on the kit; `dry`, only say.
///
/// # Errors
///
/// No song open, or the engine refusing a write.
pub fn set_up(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    preset: &std::path::Path,
    dry: bool,
) -> Result<Report, String> {
    let settings = crate::drum_settings::current();
    let kit = crate::actions::kit(daw, project, &settings);
    let triggers = crate::drum_reference::pieces(daw, project);
    let tracks = Tracks::all(daw, project.clone());
    let mut report = Report::default();
    let mut every: Notes = Vec::new();
    // The kit's pieces this session has triggers for (a kick, a snare,
    // toms): the overhead and room voices load only theirs — a cymbal no
    // trigger caught would never be played.
    let mut played: Vec<String> = Vec::new();
    for (name, _, hits) in &kit {
        let Some(note) = note_for(name) else {
            report
                .tracks
                .push(format!("{name}: no note in the kit for it"));
            continue;
        };
        let Some(piece) = signal_fx::kit::piece_for_note(preset, note) else {
            report
                .tracks
                .push(format!("{name}: the kit has nothing on note {note}"));
            continue;
        };
        let Some(sample) = triggers
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, trigger)| sample_track(&tracks, trigger))
        else {
            report
                .tracks
                .push(format!("{name}: no Sample track beside its Trig"));
            continue;
        };
        let notes: Notes = hits
            .iter()
            .map(|h| (h.time, note, velocity(h.relative_db)))
            .collect();
        report.notes = report.notes.saturating_add(notes.len());
        report.tracks.push(format!(
            "{}: {} notes on {note} ({piece}), its close mics",
            sample.name,
            notes.len()
        ));
        every.extend(notes.iter().copied());
        if !played.contains(&piece) {
            played.push(piece.clone());
        }
        if !dry {
            write_notes(daw, project, &sample.guid, &notes)?;
            let state =
                serde_json::json!({ "preset": preset, "pieces": [piece], "mics": ["close"] });
            kit_mic(daw, project, &sample.guid, &state)?;
        }
    }
    every.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Each just after the bus it feeds, at its level.
    for (name, beside, mics) in [
        ("OH Sample", &["ohs", "overheads", "oh"][..], "overhead"),
        ("Room Sample", &["rooms", "room"][..], "room"),
    ] {
        let Some(bus) = bus_track(&tracks, beside) else {
            report.tracks.push(format!(
                "{name}: no {} track to send into",
                beside.join(" / ")
            ));
            continue;
        };
        report.tracks.push(format!(
            "{name}: {} notes, the {mics} mics of {}, sent into {}",
            every.len(),
            played.join(", "),
            bus.name
        ));
        if dry {
            continue;
        }
        let guid = if let Some(t) = tracks
            .iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(name))
        {
            t.guid.clone()
        } else {
            let at = bus.index.saturating_add(1);
            let guid =
                Tracks::add(daw, project.clone(), name, Some(at)).map_err(|e| e.to_string())?;
            // Into the bus, not past it as well: its parent hears it
            // through the bus.
            daw.set_parent_send_enabled(project.clone(), TrackRef::Guid(guid.clone()), false)
                .map_err(|e| e.to_string())?;
            daw.add_send(
                project.clone(),
                TrackRef::Guid(guid.clone()),
                TrackRef::Guid(bus.guid.clone()),
            )
            .ok_or("the send would not add")?;
            guid
        };
        write_notes(daw, project, &guid, &every)?;
        let state = serde_json::json!({ "preset": preset, "pieces": played, "mics": [mics] });
        kit_mic(daw, project, &guid, &state)?;
    }
    if !dry {
        crate::studio::request_resync();
        crate::actions::list().changed("drums.samples", "the kit's samples were set up");
    }
    Ok(report)
}

/// The recorded mics' track a sample bus sends into: the first of
/// `names` (in order of preference) that a track of the kit is called —
/// never a reverb's return of the same name (`Kick/Verb/Room`).
fn bus_track<'a>(tracks: &'a [daw_proto::Track], names: &[&str]) -> Option<&'a daw_proto::Track> {
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let in_a_verb = |t: &daw_proto::Track| {
        crate::gain_stage::path_of(t, &by_guid)
            .iter()
            .any(|p| p.trim().eq_ignore_ascii_case("verb"))
    };
    names.iter().find_map(|name| {
        tracks
            .iter()
            .find(|t| !t.is_folder && t.name.trim().eq_ignore_ascii_case(name) && !in_a_verb(t))
    })
}

/// The Sample track beside a piece's trigger: a sibling named `Sample`
/// (`Sample`, `T1 Sample`).
fn sample_track<'a>(tracks: &'a [daw_proto::Track], trigger: &str) -> Option<&'a daw_proto::Track> {
    let parent = tracks
        .iter()
        .find(|t| t.guid == trigger)?
        .parent_guid
        .clone();
    tracks.iter().find(|t| {
        t.parent_guid == parent
            && !t.is_folder
            && t.name.trim().to_ascii_lowercase().ends_with("sample")
    })
}

/// `notes` as the one MIDI item on `track`: what was there before is
/// replaced (a Sample track holds the notes this writes, nothing else).
fn write_notes(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    track: &str,
    notes: &[(f64, u8, u8)],
) -> Result<(), String> {
    for item in daw.get_items(project.clone(), TrackRef::Guid(track.to_owned())) {
        daw.delete_item(project.clone(), ItemRef::Guid(item.guid))
            .map_err(|e| e.to_string())?;
    }
    let (Some(first), Some(last)) = (notes.first(), notes.last()) else {
        return Ok(());
    };
    let start = (first.0 - 1.0).max(0.0);
    let location: MidiTakeLocation = daw
        .create_midi_item(
            project.clone(),
            TrackRef::Guid(track.to_owned()),
            start,
            last.0 + 5.0,
        )
        .ok_or("the MIDI item would not make")?;
    let tempos: Vec<(f64, f64)> = daw
        .get_tempo_points(project.clone())
        .into_iter()
        .filter_map(|p| Some((p.position.time?.as_seconds(), p.bpm)))
        .collect();
    let bpm = daw.get_default_tempo(project.clone());
    let created = placed(notes, |time| {
        crate::drum_reference::qn_between(start, time, &tempos, bpm)
    });
    let _ = daw.add_notes_ppq(
        MidiTakeLocation::new(location.project, location.item, TakeRef::Active),
        created,
    );
    Ok(())
}

/// `notes` as MIDI notes, at `qn(time)` quarter notes into the item: each
/// on a tick ([`TICKS_PER_QN`]) and ending a tick before the next of its
/// pitch — overlapping notes of one pitch are not two notes once written
/// as ons and offs (they read back as one cut short and one of no
/// length), and a drum's sample plays out whatever the note's length.
fn placed(notes: &[(f64, u8, u8)], qn: impl Fn(f64) -> f64) -> Vec<MidiNoteCreate> {
    // Whole ticks, divided once — as a file's reader does — so a note
    // reads back as the very value it was written with.
    let ticks: Vec<(f64, u8, u8)> = notes
        .iter()
        .map(|&(time, pitch, vel)| ((qn(time) * TICKS_PER_QN).round(), pitch, vel))
        .collect();
    let longest = NOTE_QN * TICKS_PER_QN;
    ticks
        .iter()
        .enumerate()
        .map(|(i, &(start, pitch, vel))| {
            let next = ticks
                .iter()
                .skip(i.saturating_add(1))
                .find(|n| n.1 == pitch && n.0 > start)
                .map_or(f64::INFINITY, |n| n.0);
            let length = longest.min(next - start - 1.0).max(1.0);
            MidiNoteCreate::new(pitch, vel, start / TICKS_PER_QN, length / TICKS_PER_QN)
        })
        .collect()
}

/// An `FTS Kit Mic` on `track` (the one there, or a new one) playing what
/// `state` says.
fn kit_mic(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    track: &str,
    state: &serde_json::Value,
) -> Result<(), String> {
    let chain = FxChainContext::Track(track.to_owned());
    let fx = match daw::service::Effects::list(daw, project.clone(), chain.clone())
        .into_iter()
        .find(|f| f.name.eq_ignore_ascii_case(signal_fx::KIT_MIC))
    {
        Some(f) => f.guid,
        None => daw::service::Effects::add(daw, project.clone(), chain, signal_fx::KIT_MIC)
            .ok_or("the kit mic would not add")?,
    };
    daw.apply_plugin_state(&fx, state.to_string().as_bytes())
        .map_err(|e| format!("the kit mic would not load: {e}"))
}

#[cfg(test)]
mod tests {

    /// Notes land on ticks, and one ends a tick before the next of its
    /// pitch — never under it — while another pitch is left alone.
    #[test]
    fn notes_are_on_ticks_and_never_overlap_their_pitch() {
        let notes = [(0.0, 36, 100), (0.1, 36, 90), (0.1, 38, 80), (1.0, 36, 70)];
        let placed = placed(&notes, |t| t + 0.000_31);
        let tick = 1.0 / TICKS_PER_QN;
        for n in &placed {
            let ticks = n.start_ppq / tick;
            assert!(
                (ticks - ticks.round()).abs() < 1e-6,
                "on a tick: {}",
                n.start_ppq
            );
        }
        let first = &placed[0];
        assert!((first.start_ppq + first.length_ppq - (placed[1].start_ppq - tick)).abs() < 1e-9);
        assert!(
            (placed[2].length_ppq - NOTE_QN).abs() < 1e-9,
            "the snare keeps its length"
        );
        assert!(
            (placed[3].length_ppq - NOTE_QN).abs() < 1e-9,
            "room to ring"
        );
    }

    use super::*;

    #[test]
    fn pieces_take_the_kits_notes() {
        assert_eq!(note_for("Kick"), Some(36));
        assert_eq!(note_for("Snare"), Some(38));
        assert_eq!(note_for("Tom 1"), Some(45));
        assert_eq!(note_for("tom 4"), Some(43));
        assert_eq!(note_for("Tom 5"), None);
        assert_eq!(note_for("Hi Hat"), None);
    }

    #[test]
    fn velocity_follows_the_level() {
        assert_eq!(velocity(0.0), 127);
        assert_eq!(velocity(3.0), 127, "never past the loudest");
        assert_eq!(velocity(-12.0), 64);
        assert_eq!(velocity(-24.0), 32);
        assert_eq!(velocity(-200.0), 1);
    }
}
