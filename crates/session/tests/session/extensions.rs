//! Extension data: two extensions keep what they need without seeing, or
//! clobbering, each other's.

use session::{Entry, ExtensionId, ItemId, LaneId, Session, Time, Value};

use crate::time::{item, lane};

#[test]
fn two_extensions_keep_the_same_names_apart() {
    let mut session = Session::new("Song");
    let keyflow = ExtensionId::from("keyflow");
    let ignition = ExtensionId::from("ignition");

    // Both use an entry called "main"; each sees only its own.
    session
        .ext_mut(&keyflow)
        .set_text("main", "Verse: C G Am F");
    session.ext_mut(&ignition).edit_map("main", |m| {
        m.insert("universe".into(), Value::Int(1));
    });

    assert_eq!(
        session.ext(&keyflow).unwrap().text("main"),
        Some("Verse: C G Am F")
    );
    assert_eq!(session.ext(&keyflow).unwrap().map("main"), None);
    assert_eq!(
        session.ext(&ignition).unwrap().map("main").unwrap()["universe"],
        Value::Int(1)
    );
    assert_eq!(session.ext(&ignition).unwrap().text("main"), None);
    assert!(session.ext(&ExtensionId::from("lyrics")).is_none());
}

#[test]
fn removing_an_extension_takes_only_what_it_brought() {
    let mut session = Session::new("Song");
    let daw = ExtensionId::from("daw");
    let ignition = ExtensionId::from("ignition");
    let vocals = LaneId::from("vocals");
    let cues = LaneId::from("cues");
    session
        .lanes
        .insert(vocals.clone(), lane(&daw, "daw.track"));
    session
        .lanes
        .insert(cues.clone(), lane(&ignition, "ignition.cues"));
    session.items.insert(
        ItemId::from("take"),
        item(&daw, &vocals, "daw.item", Time::Seconds(0.0)),
    );
    session.items.insert(
        ItemId::from("cue"),
        item(&ignition, &cues, "ignition.cue", Time::Seconds(1.0)),
    );
    // Ignition's cue on the vocal track's lane: it goes with the lane.
    session.items.insert(
        ItemId::from("blackout"),
        item(&ignition, &vocals, "ignition.cue", Time::Seconds(2.0)),
    );
    session.ext_mut(&daw).set("sample_rate", Value::Int(48_000));
    session.ext_mut(&ignition).set("rig", Value::from("Club"));

    session.remove_extension(&daw);

    assert!(session.ext(&daw).is_none());
    assert_eq!(session.lanes.keys().collect::<Vec<_>>(), vec![&cues]);
    assert_eq!(
        session.items.keys().collect::<Vec<_>>(),
        vec![&ItemId::from("cue")]
    );
    assert_eq!(
        session.ext(&ignition).unwrap().get("rig"),
        Some(&Entry::Register(Value::from("Club")))
    );
}

#[test]
fn an_extension_finds_its_lanes_in_order() {
    let mut session = Session::new("Song");
    let daw = ExtensionId::from("daw");
    for (id, order) in [("bass", 2.0), ("drums", 1.0), ("keys", 1.5)] {
        let mut l = lane(&daw, "daw.track");
        l.order = order;
        session.lanes.insert(LaneId::from(id), l);
    }
    session.lanes.insert(
        LaneId::from("cues"),
        lane(&ExtensionId::from("ignition"), "ignition.cues"),
    );
    let ids: Vec<_> = session
        .lanes_of(&daw)
        .into_iter()
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(ids, vec!["drums", "keys", "bass"]);
}
