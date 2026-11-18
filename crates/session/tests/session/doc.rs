//! The session as an Architect (Loro) document: a round trip, two people
//! editing two extensions' data at once, and undo.

use std::collections::BTreeMap;

use loro::{ExportMode, LoroDoc, UndoManager};
use session::doc::{self, SessionDoc};
use session::time::TimeSignature;
use session::{
    ExtensionId, Item, ItemId, LaneId, MarkId, Marker, Region, Session, TempoMap, Time, Transport,
    Value,
};

use crate::time::lane;

fn song() -> Session {
    let mut s = Session::new("Belief");
    let mut ramp = TempoMap::point(8.0, 92.0, Some(TimeSignature::new(6, 8)));
    ramp.shape = Some(0);
    s.tempo = TempoMap::new(
        92.0,
        TimeSignature::COMMON_TIME,
        vec![ramp, TempoMap::point(20.0, 100.0, None)],
    );
    s.rulers.insert(0, "Sections".into());
    s.markers.insert(
        MarkId::from("m1"),
        Marker {
            at: Time::QuarterNotes(16.0),
            name: "Key change".into(),
            color: Some(0x00ff_8800),
            ruler: None,
        },
    );
    s.regions.insert(
        MarkId::from("r1"),
        Region {
            start: Time::QuarterNotes(0.0),
            end: Time::QuarterNotes(32.0),
            name: "Verse 1".into(),
            color: None,
            ruler: Some(0),
        },
    );
    let ignition = ExtensionId::from("ignition");
    let cues = LaneId::from("cues");
    s.lanes
        .insert(cues.clone(), lane(&ignition, "ignition.cues"));
    s.items.insert(
        ItemId::from("cue-1"),
        Item {
            owner: ignition.clone(),
            lane: cues.clone(),
            kind: "ignition.cue".into(),
            at: Time::Seconds(4.0),
            end: Some(Time::Seconds(6.5)),
            payload: Value::map([
                ("fade", Value::Float(1.5)),
                ("look", Value::from("Warm wash")),
            ]),
        },
    );
    s.items.insert(
        ItemId::from("cue-2"),
        Item {
            owner: ignition.clone(),
            lane: cues,
            kind: "ignition.cue".into(),
            at: Time::QuarterNotes(24.0),
            end: None,
            payload: Value::Int(7),
        },
    );
    let keyflow = ExtensionId::from("keyflow");
    s.ext_mut(&keyflow).version = 3;
    s.ext_mut(&keyflow).set_text("chart", "Verse: C G Am F\n");
    s.ext_mut(&keyflow).set("key", Value::from("C"));
    // Ids and names with the characters a Loro root name may not hold.
    s.ext_mut(&ExtensionId::from("studio/arrange"))
        .set_text("view:a/b\0%", "zoom 2");
    s.ext_mut(&ignition).edit_map("rig", |m| {
        m.insert("universe".into(), Value::Int(1));
        m.insert("patch".into(), Value::Bytes(vec![1, 2, 3]));
    });
    s
}

#[test]
fn a_session_round_trips_through_its_document() {
    let session = song();
    let d = SessionDoc::new();
    d.write(&session).unwrap();
    assert_eq!(d.kind().as_deref(), Some(doc::KIND));
    assert_eq!(d.read(), session);

    // Through bytes, as a file or a sync would carry it.
    let bytes = d.doc().export(ExportMode::Snapshot).unwrap();
    let other = LoroDoc::new();
    other.import(&bytes).unwrap();
    assert_eq!(SessionDoc::from_doc(other).read(), session);
}

#[test]
fn writing_the_same_session_again_records_nothing() {
    let session = song();
    let d = SessionDoc::new();
    d.write(&session).unwrap();
    let before = d.doc().oplog_vv();
    d.write(&session).unwrap();
    assert_eq!(d.doc().oplog_vv(), before);
}

#[test]
fn removing_from_the_session_removes_from_the_document() {
    let mut session = song();
    let d = SessionDoc::new();
    d.write(&session).unwrap();
    session.remove_extension(&ExtensionId::from("ignition"));
    session.markers.clear();
    session
        .ext_mut(&ExtensionId::from("keyflow"))
        .entries
        .remove("key");
    d.write(&session).unwrap();
    assert_eq!(d.read(), session);
}

/// Two peers, each with its own replica.
fn pair() -> (SessionDoc, SessionDoc) {
    let a = SessionDoc::new();
    a.doc().set_peer_id(1).unwrap();
    a.write(&song()).unwrap();
    let b_doc = LoroDoc::new();
    b_doc.set_peer_id(2).unwrap();
    b_doc
        .import(&a.doc().export(ExportMode::Snapshot).unwrap())
        .unwrap();
    (a, SessionDoc::from_doc(b_doc))
}

fn exchange(a: &SessionDoc, b: &SessionDoc) {
    let to_b = a
        .doc()
        .export(ExportMode::updates(&b.doc().oplog_vv()))
        .unwrap();
    let to_a = b
        .doc()
        .export(ExportMode::updates(&a.doc().oplog_vv()))
        .unwrap();
    b.doc().import(&to_b).unwrap();
    a.doc().import(&to_a).unwrap();
}

#[test]
fn two_people_editing_two_extensions_converge_and_keep_both() {
    let (a, b) = pair();
    let keyflow = ExtensionId::from("keyflow");
    let ignition = ExtensionId::from("ignition");

    // A types into Keyflow's chart; B, at the same time, types at the
    // other end of it, and adds a key to Ignition's rig map.
    let chart = a.ext(&keyflow).unwrap().text("chart").unwrap();
    chart.insert(0, "Intro: C\n").unwrap();
    a.doc().commit();
    let chart = b.ext(&keyflow).unwrap().text("chart").unwrap();
    chart
        .insert(chart.len_unicode(), "Chorus: F G C\n")
        .unwrap();
    b.ext(&ignition)
        .unwrap()
        .map("rig")
        .unwrap()
        .insert("dimmer_curve", "square")
        .unwrap();
    b.doc().commit();

    exchange(&a, &b);
    let (sa, sb) = (a.read(), b.read());
    assert_eq!(sa, sb);
    assert_eq!(
        sa.ext(&keyflow).unwrap().text("chart"),
        Some("Intro: C\nVerse: C G Am F\nChorus: F G C\n")
    );
    let rig = sa.ext(&ignition).unwrap().map("rig").unwrap();
    assert_eq!(rig["dimmer_curve"], Value::from("square"));
    assert_eq!(rig["universe"], Value::Int(1));
    // Keyflow's namespace holds only Keyflow's entries.
    let names: Vec<&String> = sa.ext(&keyflow).unwrap().entries.keys().collect();
    assert_eq!(names, vec!["chart", "key"]);
}

/// The first cue of a new list, made on two machines at once: both land,
/// because a map entry is a named (root) container, not a child each peer
/// creates.
#[test]
fn a_map_entry_created_on_two_machines_at_once_keeps_both() {
    let (a, b) = pair();
    let ignition = ExtensionId::from("ignition");
    a.ext(&ignition)
        .unwrap()
        .map("presets")
        .unwrap()
        .insert("blue", 1)
        .unwrap();
    a.doc().commit();
    b.ext(&ignition)
        .unwrap()
        .map("presets")
        .unwrap()
        .insert("red", 2)
        .unwrap();
    b.doc().commit();
    exchange(&a, &b);
    let presets: BTreeMap<String, Value> = a
        .read()
        .ext(&ignition)
        .unwrap()
        .map("presets")
        .unwrap()
        .clone();
    assert_eq!(presets.keys().collect::<Vec<_>>(), vec!["blue", "red"]);
    assert_eq!(a.read(), b.read());
}

#[test]
fn concurrent_payload_edits_to_one_item_merge_per_key() {
    let (a, b) = pair();
    let mut sa = a.read();
    if let Some(Value::Map(m)) = sa
        .items
        .get_mut(&ItemId::from("cue-1"))
        .map(|i| &mut i.payload)
    {
        m.insert("fade".into(), Value::Float(3.0));
    }
    a.write(&sa).unwrap();
    let mut sb = b.read();
    if let Some(Value::Map(m)) = sb
        .items
        .get_mut(&ItemId::from("cue-1"))
        .map(|i| &mut i.payload)
    {
        m.insert("look".into(), Value::from("Cold strobe"));
    }
    b.write(&sb).unwrap();
    exchange(&a, &b);
    let payload = &a.read().items[&ItemId::from("cue-1")].payload;
    assert_eq!(payload.get("fade"), Some(&Value::Float(3.0)));
    assert_eq!(payload.get("look"), Some(&Value::from("Cold strobe")));
}

/// Undo is the document's: an extension's step undoes like any other, and
/// only this peer's (what `crdt::History` builds on).
#[test]
fn undo_takes_back_my_extension_edit_and_not_yours() {
    let (a, b) = pair();
    let keyflow = ExtensionId::from("keyflow");
    let mut undo = UndoManager::new(a.doc());
    undo.set_merge_interval(0);

    a.ext(&keyflow)
        .unwrap()
        .set("key", &Value::from("D"))
        .unwrap();
    a.doc().commit();
    b.ext(&keyflow)
        .unwrap()
        .set_text("chart", "Verse: D A Bm G\n")
        .unwrap();
    b.doc().commit();
    exchange(&a, &b);

    assert!(undo.undo().unwrap());
    let s = a.read();
    let k = s.ext(&keyflow).unwrap();
    assert_eq!(k.register("key"), Some(&Value::from("C")));
    assert_eq!(k.text("chart"), Some("Verse: D A Bm G\n"));
}

#[test]
fn a_transport_rides_presence_as_a_play_state() {
    let t = Transport::playing(12.5, 3_000_000.0, 1.0);
    let back = doc::transport_of(&doc::play_state(&t)).unwrap();
    assert_eq!(back, t);
}

/// The node hosting the session's transport travels with the document
/// (session#192): what the leader writes, a follower's replica reads, and
/// clearing it is seen too.
#[test]
fn the_transport_host_travels_with_the_document() {
    let (a, b) = pair();
    assert_eq!(b.transport_host(), None);
    a.set_transport_host(Some("4f1e…endpoint")).unwrap();
    exchange(&a, &b);
    assert_eq!(b.transport_host().as_deref(), Some("4f1e…endpoint"));
    a.set_transport_host(None).unwrap();
    exchange(&a, &b);
    assert_eq!(b.transport_host(), None);
    // Clearing what is not there writes nothing.
    let before = a.doc().oplog_vv();
    a.set_transport_host(None).unwrap();
    assert_eq!(a.doc().oplog_vv(), before);
}
