//! The time model: bars and beats, meters, and the two time bases.

use session::time::TimeSignature;
use session::{ExtensionId, Item, ItemId, Lane, LaneId, Session, TempoMap, Time, Value};

use crate::close;

#[test]
fn bars_and_beats_in_four_four() {
    let tempo = TempoMap::default(); // 120 BPM, 4/4
    // Two seconds at 120 BPM is four quarter notes: bar 2, beat 1.
    let at = Time::Seconds(2.0);
    close(at.quarter_notes(&tempo), 4.0, 1e-9);
    let bb = at.bar_beat(&tempo);
    assert_eq!((bb.bar, bb.beat), (2, 1));
    close(bb.fraction, 0.0, 1e-9);
    // Halfway through beat 3 of bar 1.
    let bb = Time::QuarterNotes(2.5).bar_beat(&tempo);
    assert_eq!((bb.bar, bb.beat), (1, 3));
    close(bb.fraction, 0.5, 1e-9);
}

#[test]
fn a_six_eight_bar_counts_eighths() {
    let tempo = TempoMap::new(120.0, TimeSignature::new(6, 8), Vec::new());
    // A 6/8 bar is three quarter notes; its beats are eighths.
    let bb = tempo.bar_beat(3.0);
    assert_eq!((bb.bar, bb.beat), (2, 1));
    let bb = tempo.bar_beat(0.5);
    assert_eq!((bb.bar, bb.beat), (1, 2));
}

#[test]
fn a_meter_change_moves_the_bar_lines_after_it() {
    // 4/4 at 120 for two bars (4 s), then 3/4.
    let tempo = TempoMap::new(
        120.0,
        TimeSignature::COMMON_TIME,
        vec![TempoMap::point(4.0, 120.0, Some(TimeSignature::WALTZ))],
    );
    // Bar 3 starts at 8 qn; bar 4 three quarter notes later.
    let bar4 = tempo.at_bar(4, 1, 0.0);
    close(bar4.quarter_notes(&tempo), 11.0, 1e-9);
    close(bar4.seconds(&tempo), 5.5, 1e-9);
    let back = bar4.bar_beat(&tempo);
    assert_eq!((back.bar, back.beat), (4, 1));
    close(tempo.meter_at(5.0).quarter_notes_per_measure(), 3.0, 1e-12);
}

#[test]
fn a_ramp_converts_both_ways() {
    // 60 → 120 BPM linearly over 10 s (REAPER shape 0), then held.
    let mut start = TempoMap::point(0.0, 60.0, None);
    start.shape = Some(0);
    let tempo = TempoMap::new(
        60.0,
        TimeSignature::COMMON_TIME,
        vec![start, TempoMap::point(10.0, 120.0, None)],
    );
    for seconds in [0.5, 3.0, 7.25, 10.0, 12.0] {
        let qn = Time::Seconds(seconds).quarter_notes(&tempo);
        close(Time::QuarterNotes(qn).seconds(&tempo), seconds, 1e-6);
    }
    close(tempo.bpm_at(11.0), 120.0, 1e-12);
}

/// A chord on bar 3 moves when the song gets slower; a cue at 4 s does
/// not. That is the whole reason a [`Time`] carries its base.
#[test]
fn musical_time_moves_with_the_tempo_and_absolute_time_stays() {
    let mut session = Session::new("Song");
    let chords = LaneId::from("chords");
    let cues = LaneId::from("cues");
    let keyflow = ExtensionId::from("keyflow");
    let ignition = ExtensionId::from("ignition");
    session
        .lanes
        .insert(chords.clone(), lane(&keyflow, "keyflow.chords"));
    session
        .lanes
        .insert(cues.clone(), lane(&ignition, "ignition.cues"));
    session.items.insert(
        ItemId::from("chord"),
        item(&keyflow, &chords, "keyflow.chord", Time::QuarterNotes(8.0)),
    );
    session.items.insert(
        ItemId::from("cue"),
        item(&ignition, &cues, "ignition.cue", Time::Seconds(4.0)),
    );
    let at = |s: &Session, id: &str| s.items[&ItemId::from(id)].at.seconds(&s.tempo);
    close(at(&session, "chord"), 4.0, 1e-9);
    close(at(&session, "cue"), 4.0, 1e-9);

    session.tempo = TempoMap::new(60.0, TimeSignature::COMMON_TIME, Vec::new());
    close(at(&session, "chord"), 8.0, 1e-9);
    close(at(&session, "cue"), 4.0, 1e-9);
    // What a player schedules from is resolved under the tempo map now.
    let chords: Vec<f64> = session
        .timed("keyflow.chord")
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    assert_eq!(chords, vec![8.0]);
}

pub fn lane(owner: &ExtensionId, kind: &str) -> Lane {
    Lane {
        owner: owner.clone(),
        kind: kind.to_owned(),
        name: kind.to_owned(),
        order: 0.0,
        parent: None,
    }
}

pub fn item(owner: &ExtensionId, lane: &LaneId, kind: &str, at: Time) -> Item {
    Item {
        owner: owner.clone(),
        lane: lane.clone(),
        kind: kind.to_owned(),
        at,
        end: None,
        payload: Value::Null,
    }
}
