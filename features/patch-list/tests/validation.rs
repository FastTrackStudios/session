//! One role, one entry: two entries naming the same role is a
//! validation error unless the profile marks the role shared. A role
//! the profile lacks is unresolved, never an error (decision #29).
//!
//! r[verify flow.patch-list.plan]
//! r[verify flow.patch-list.studio-profiles]

use patch_list::{Error, PatchList, Resolved, StudioProfile, validate};

const ALBUM: &str = include_str!("../fixtures/album/patch-list.styx");
const ROOM: &str = include_str!("../fixtures/studios/golden-room.styx");

#[test]
fn the_fixture_album_is_valid_in_the_fixture_room() {
    let list = PatchList::from_styx(ALBUM).expect("parse");
    let room = StudioProfile::from_styx(ROOM).expect("parse");
    let plan = validate(&list, &room).expect("two talkback mics on a shared role are fine");
    assert_eq!(plan.entries.len(), 33);
    assert_eq!(plan.buses.len(), 10);
}

#[test]
fn a_role_the_room_lacks_is_unresolved_in_the_plan_not_an_error() {
    let list = PatchList::from_styx(ALBUM).expect("parse");
    let room = StudioProfile::from_styx(ROOM).expect("parse");
    let plan = validate(&list, &room).expect("valid");

    let bass_amp = plan
        .entries
        .iter()
        .find(|e| e.lowered.performer == "bassist" && e.lowered.key == "amp")
        .expect("the bassist's amp mic");
    assert_eq!(bass_amp.resolved, Resolved::Unresolved);
    let bass_di = plan
        .entries
        .iter()
        .find(|e| e.lowered.performer == "bassist" && e.lowered.key == "di")
        .expect("the bassist's DI");
    assert_eq!(bass_di.resolved, Resolved::Audio { channel: 0 });

    let bass_bus = plan
        .buses
        .iter()
        .find(|b| b.name == "bassist")
        .expect("the bassist's bus");
    assert_eq!(bass_bus.resolved, Resolved::Unresolved);
    assert_eq!(plan.unresolved(), 2);
}

#[test]
fn a_midi_entry_resolves_by_itself() {
    let list = PatchList::from_styx(ALBUM).expect("parse");
    // A room with no Prophet in it: the entry carries its own device.
    let room = StudioProfile::from_styx("shared (\"Talkback\")").expect("parse");
    let plan = validate(&list, &room).expect("valid");
    let synth = plan
        .entries
        .iter()
        .find(|e| e.lowered.performer == "john" && e.lowered.key == "synth")
        .expect("john's synth");
    assert_eq!(
        synth.resolved,
        Resolved::Midi {
            device: "Prophet-6".into(),
            channel: Some(1)
        }
    );
}

#[test]
fn two_entries_on_one_role_is_an_error() {
    let list = PatchList::from_styx(
        "performers {\n    cody {guitar {di \"DI 3\"}}\n    john {guitar {di \"DI 3\"}}\n}",
    )
    .expect("parse");
    let err = validate(&list, &StudioProfile::default()).expect_err("double-booked DI 3");
    match &err {
        Error::DuplicateRole { role, entries } => {
            assert_eq!(role, "DI 3");
            assert_eq!(entries, &["cody/guitar/di", "john/guitar/di"]);
        }
    }
    assert!(err.to_string().contains("DI 3"), "{err}");
}

#[test]
fn a_shared_role_may_carry_two_entries() {
    let list = PatchList::from_styx(
        "performers {\n    cody {guitar {di \"DI 3\"}}\n    john {guitar {di \"DI 3\"}}\n}",
    )
    .expect("parse");
    let room = StudioProfile::from_styx("shared (\"DI 3\")").expect("parse");
    validate(&list, &room).expect("shared, so allowed");
}

#[test]
fn two_buses_on_one_output_is_an_error_too() {
    let list = PatchList::from_styx(
        "headphones {\n    cody {output \"HP 1\", for (cody)}\n    john {output \"HP 1\", for (john)}\n}",
    )
    .expect("parse");
    let err = validate(&list, &StudioProfile::default()).expect_err("double-booked HP 1");
    assert!(matches!(err, Error::DuplicateRole { ref role, .. } if role == "HP 1"));
    // The negative control: a bus for two people on one output is one
    // entry, not two.
    let list = PatchList::from_styx("headphones {\n    band {output \"HP 1\", for (cody john)}\n}")
        .expect("parse");
    validate(&list, &StudioProfile::default()).expect("one bus, one output");
}
