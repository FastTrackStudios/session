//! What a runtime extension may do through its host: its own namespace,
//! its own lanes and items, the structure only with a grant.

use session::doc::SessionDoc;
use session::service::{
    Grants, Meter, Request, Response, ServiceError, TempoChange, TempoMapData, handle,
};
use session::{ExtensionId, Item, ItemId, LaneId, MarkId, Region, Time, Value};

use crate::time::lane;

fn ext(id: &str) -> ExtensionId {
    ExtensionId::from(id)
}

fn ask(doc: &SessionDoc, caller: &str, request: Request) -> Result<Response, ServiceError> {
    handle(doc, &ext(caller), Grants::default(), request)
}

#[allow(clippy::needless_pass_by_value)] // Reads as `refused(ask(..))`.
fn refused(r: Result<Response, ServiceError>) -> bool {
    matches!(r, Err(ServiceError::Refused(_)))
}

fn cue(owner: &str, lane: &str, at: f64) -> Item {
    Item {
        owner: ext(owner),
        lane: LaneId::from(lane),
        kind: format!("{owner}.thing"),
        at: Time::Seconds(at),
        end: None,
        payload: Value::map([("secret", Value::from(owner))]),
    }
}

/// A doc with daw's vocal lane and one take on it.
fn studio() -> SessionDoc {
    let doc = SessionDoc::new();
    ask(
        &doc,
        "daw",
        Request::PutLane {
            id: "vocals".into(),
            lane: lane(&ext("daw"), "daw.track"),
        },
    )
    .unwrap();
    ask(
        &doc,
        "daw",
        Request::PutItem {
            id: "take".into(),
            item: cue("daw", "vocals", 0.0),
        },
    )
    .unwrap();
    doc
}

#[test]
fn an_extension_reads_and_writes_only_its_own_namespace() {
    let doc = studio();
    ask(
        &doc,
        "keyflow",
        Request::TextSplice {
            name: "chart".into(),
            at: 0,
            delete: 0,
            insert: "Verse: C G\n".into(),
        },
    )
    .unwrap();
    ask(
        &doc,
        "keyflow",
        Request::TextSplice {
            name: "chart".into(),
            at: 7,
            delete: 1,
            insert: "Am".into(),
        },
    )
    .unwrap();
    ask(
        &doc,
        "ignition",
        Request::MapSet {
            name: "chart".into(),
            key: "universe".into(),
            value: Value::Int(1),
        },
    )
    .unwrap();

    let Response::Data(k) = ask(&doc, "keyflow", Request::ReadData).unwrap() else {
        panic!()
    };
    assert_eq!(k.text("chart"), Some("Verse: Am G\n"));
    assert_eq!(k.map("chart"), None);
    let Response::Data(i) = ask(&doc, "ignition", Request::ReadData).unwrap() else {
        panic!()
    };
    assert_eq!(i.text("chart"), None);
    assert_eq!(i.map("chart").unwrap()["universe"], Value::Int(1));

    // A splice past the end is refused, and nothing is written.
    assert!(refused(ask(
        &doc,
        "keyflow",
        Request::TextSplice {
            name: "chart".into(),
            at: 99,
            delete: 0,
            insert: "x".into()
        }
    )));
}

#[test]
fn an_extension_edits_only_what_it_owns() {
    let doc = studio();
    // Ignition may put its own cue on the DAW's lane…
    ask(
        &doc,
        "ignition",
        Request::PutItem {
            id: "blackout".into(),
            item: cue("ignition", "vocals", 2.0),
        },
    )
    .unwrap();
    // …but not claim to be the DAW, touch the DAW's take, or its lane.
    assert!(refused(ask(
        &doc,
        "ignition",
        Request::PutItem {
            id: "fake".into(),
            item: cue("daw", "vocals", 1.0)
        }
    )));
    assert!(refused(ask(
        &doc,
        "ignition",
        Request::PutItem {
            id: "take".into(),
            item: cue("ignition", "vocals", 9.0)
        }
    )));
    assert!(refused(ask(
        &doc,
        "ignition",
        Request::RemoveItem { id: "take".into() }
    )));
    assert!(refused(ask(
        &doc,
        "ignition",
        Request::RemoveLane {
            id: "vocals".into()
        }
    )));
    // Nor put an item on a lane that does not exist.
    assert!(refused(ask(
        &doc,
        "ignition",
        Request::PutItem {
            id: "lost".into(),
            item: cue("ignition", "nowhere", 1.0)
        }
    )));
    assert_eq!(
        doc.item(&ItemId::from("take")).unwrap().at,
        Time::Seconds(0.0)
    );

    // The DAW removing its lane takes its own take, not Ignition's cue.
    ask(
        &doc,
        "daw",
        Request::RemoveLane {
            id: "vocals".into(),
        },
    )
    .unwrap();
    let items = doc.items();
    assert_eq!(
        items.keys().map(ItemId::as_str).collect::<Vec<_>>(),
        vec!["blackout"]
    );
}

#[test]
fn everyone_sees_the_timeline_and_only_their_own_payloads() {
    let doc = studio();
    ask(
        &doc,
        "ignition",
        Request::PutItem {
            id: "blackout".into(),
            item: cue("ignition", "vocals", 2.0),
        },
    )
    .unwrap();
    let Response::Timeline { lanes, items } = ask(&doc, "ignition", Request::ReadTimeline).unwrap()
    else {
        panic!()
    };
    assert!(lanes.contains_key(&LaneId::from("vocals")));
    let take = &items[&ItemId::from("take")];
    assert_eq!(
        (take.kind.as_str(), take.at),
        ("daw.thing", Time::Seconds(0.0))
    );
    assert_eq!(take.payload, Value::Null);
    assert_eq!(
        items[&ItemId::from("blackout")].payload.get("secret"),
        Some(&Value::from("ignition"))
    );
}

#[test]
fn the_song_structure_needs_a_grant() {
    let doc = studio();
    let section = Request::PutRegion {
        id: MarkId::from("verse"),
        region: Region {
            start: Time::QuarterNotes(0.0),
            end: Time::QuarterNotes(16.0),
            name: "Verse".into(),
            color: None,
            ruler: Some(1),
        },
    };
    assert!(refused(ask(&doc, "keyflow", section.clone())));
    let tempo = Request::SetTempo {
        tempo: TempoMapData {
            default_bpm: 92.0,
            numerator: 4,
            denominator: 4,
            points: vec![TempoChange {
                seconds: 10.0,
                bpm: 92.0,
                meter: Some(Meter {
                    numerator: 6,
                    denominator: 8,
                }),
                shape: None,
                tension: None,
            }],
        },
    };
    assert!(refused(ask(&doc, "keyflow", tempo.clone())));

    let granted = Grants { structure: true };
    handle(&doc, &ext("keyflow"), granted, section).unwrap();
    handle(&doc, &ext("keyflow"), granted, tempo).unwrap();
    let s = doc.read();
    assert_eq!(s.regions[&MarkId::from("verse")].name, "Verse");
    assert_eq!(s.tempo.default_bpm(), 92.0);
    assert_eq!(s.tempo.meter_at(11.0).numerator, 6);

    // A tempo of zero is refused even with the grant.
    let bad = Request::SetTempo {
        tempo: TempoMapData {
            default_bpm: 0.0,
            numerator: 4,
            denominator: 4,
            points: vec![],
        },
    };
    assert!(refused(handle(&doc, &ext("keyflow"), granted, bad)));
}

#[test]
fn everyone_reads_the_structure_with_its_ramps() {
    let doc = studio();
    let granted = Grants { structure: true };
    // A ramp from 60 to 120 BPM over 10 s (shape 0: linear), then held.
    let tempo = TempoMapData {
        default_bpm: 60.0,
        numerator: 4,
        denominator: 4,
        points: vec![
            TempoChange {
                seconds: 0.0,
                bpm: 60.0,
                meter: None,
                shape: Some(0),
                tension: None,
            },
            TempoChange {
                seconds: 10.0,
                bpm: 120.0,
                meter: None,
                shape: None,
                tension: None,
            },
        ],
    };
    handle(
        &doc,
        &ext("keyflow"),
        granted,
        Request::SetTempo {
            tempo: tempo.clone(),
        },
    )
    .unwrap();
    handle(
        &doc,
        &ext("keyflow"),
        granted,
        Request::PutMarker {
            id: MarkId::from("drop"),
            marker: session::Marker {
                at: Time::QuarterNotes(32.0),
                name: "Drop".into(),
                color: None,
                ruler: None,
            },
        },
    )
    .unwrap();
    // Ignition, granted nothing, reads it.
    let Response::Structure {
        tempo: read,
        markers,
        regions,
    } = ask(&doc, "ignition", Request::ReadStructure).unwrap()
    else {
        panic!("not the structure")
    };
    assert_eq!(read, tempo);
    assert_eq!(markers[&MarkId::from("drop")].name, "Drop");
    assert!(regions.is_empty());
    // The ramp is kept: seconds resolve as the session's own map does.
    let map = read.to_tempo_map().unwrap();
    let qn = Time::Seconds(7.25).quarter_notes(&map);
    assert!((qn - Time::Seconds(7.25).quarter_notes(&doc.tempo())).abs() < 1e-12);
    assert!((map.bpm_at(11.0) - 120.0).abs() < 1e-12);
}
