//! `fts.session` as an extension service: the provider's actions answer
//! for `cx.caller`, and each method takes only its own kind of request.

use session::doc::SessionDoc;
use session::extension::{answer, call_json, response_of};
use session::service::{READ, Request, Response, STRUCTURE, Sessions, WRITE};
use session::{MarkId, Region, Time, Value};

fn sessions() -> Sessions {
    let s = Sessions::default();
    s.open("song-1", SessionDoc::new());
    s
}

fn splice(text: &str) -> String {
    call_json(
        "song-1",
        Request::TextSplice {
            name: "chart".into(),
            at: 0,
            delete: 0,
            insert: text.into(),
        },
    )
    .unwrap()
}

#[test]
fn the_caller_in_the_context_is_the_namespace() {
    let s = sessions();
    answer(&s, WRITE, &splice("Verse: C G\n"), Some("keyflow")).unwrap();
    let read = call_json("song-1", Request::ReadData).unwrap();
    let Response::Data(k) =
        response_of(&answer(&s, READ, &read, Some("keyflow")).unwrap()).unwrap()
    else {
        panic!("not data")
    };
    assert_eq!(k.text("chart"), Some("Verse: C G\n"));
    let Response::Data(i) =
        response_of(&answer(&s, READ, &read, Some("ignition")).unwrap()).unwrap()
    else {
        panic!("not data")
    };
    assert_eq!(i.text("chart"), None);
}

#[test]
fn a_method_takes_only_its_own_kind_of_request() {
    let s = sessions();
    // An edit sent as a read (the read_only action a guest may run) is
    // refused, and nothing is written.
    let e = answer(&s, READ, &splice("vandal "), Some("keyflow")).unwrap_err();
    assert!(e.contains("is a `write`, not a `read`"), "{e}");
    let region = call_json(
        "song-1",
        Request::PutRegion {
            id: MarkId::from("verse"),
            region: Region {
                start: Time::QuarterNotes(0.0),
                end: Time::QuarterNotes(16.0),
                name: "Verse".into(),
                color: None,
                ruler: None,
            },
        },
    )
    .unwrap();
    assert!(answer(&s, WRITE, &region, Some("keyflow")).is_err());
    answer(&s, STRUCTURE, &region, Some("keyflow")).unwrap();
    assert_eq!(s.get("song-1").unwrap().read().regions.len(), 1);
    let chart = s.get("song-1").unwrap().read_ext(&"keyflow".into());
    assert_eq!(chart.text("chart"), None);
}

#[test]
fn only_an_extension_asks_and_only_open_sessions_answer() {
    let s = sessions();
    let read = call_json("song-1", Request::ReadData).unwrap();
    assert!(
        answer(&s, READ, &read, None)
            .unwrap_err()
            .contains("only an extension")
    );
    assert!(answer(&s, READ, &read, Some("")).is_err());
    let other = call_json("song-9", Request::ReadData).unwrap();
    assert!(
        answer(&s, READ, &other, Some("keyflow"))
            .unwrap_err()
            .contains("no session song-9")
    );
    assert!(
        answer(&s, READ, "{not json", Some("keyflow"))
            .unwrap_err()
            .starts_with("arguments")
    );
}

#[test]
fn payloads_cross_json_whole() {
    let s = sessions();
    let set = call_json(
        "song-1",
        Request::Set {
            name: "meta".into(),
            value: Value::map([
                ("key", Value::from("D")),
                ("capo", Value::Int(2)),
                ("swing", Value::Float(0.6)),
                (
                    "tags",
                    Value::List(vec![Value::from("ballad"), Value::Bool(true)]),
                ),
            ]),
        },
    )
    .unwrap();
    answer(&s, WRITE, &set, Some("keyflow")).unwrap();
    let read = call_json("song-1", Request::ReadData).unwrap();
    let Response::Data(k) =
        response_of(&answer(&s, READ, &read, Some("keyflow")).unwrap()).unwrap()
    else {
        panic!("not data")
    };
    let meta = k.register("meta").unwrap();
    assert_eq!(meta.get("capo"), Some(&Value::Int(2)));
    assert_eq!(meta.get("swing"), Some(&Value::Float(0.6)));
}

/// Every response crosses JSON: the timeline's maps are keyed by ids.
#[test]
fn the_timeline_crosses_json_keyed_by_ids() {
    use session::{ExtensionId, Item, ItemId, Lane, LaneId};
    let s = sessions();
    let lane = LaneId::from("cues");
    let put = call_json(
        "song-1",
        Request::PutLane {
            id: lane.clone(),
            lane: Lane {
                owner: ExtensionId::from("ignition"),
                kind: "ignition.cues".into(),
                name: "Main".into(),
                order: 0.0,
                parent: None,
            },
        },
    )
    .unwrap();
    answer(&s, WRITE, &put, Some("ignition")).unwrap();
    let item = call_json(
        "song-1",
        Request::PutItem {
            id: ItemId::from("cues/1"),
            item: Item {
                owner: ExtensionId::from("ignition"),
                lane: lane.clone(),
                kind: "ignition.cue".into(),
                at: Time::QuarterNotes(4.0),
                end: None,
                payload: Value::map([("look", Value::from("Warm"))]),
            },
        },
    )
    .unwrap();
    answer(&s, WRITE, &item, Some("ignition")).unwrap();
    let read = call_json("song-1", Request::ReadTimeline).unwrap();
    let Response::Timeline { lanes, items } =
        response_of(&answer(&s, READ, &read, Some("ignition")).unwrap()).unwrap()
    else {
        panic!("not the timeline")
    };
    assert_eq!(lanes[&lane].name, "Main");
    assert_eq!(items[&ItemId::from("cues/1")].at, Time::QuarterNotes(4.0));
    // An id is the string it wraps on the wire.
    assert!(
        call_json(
            "cues",
            Request::RemoveItem {
                id: ItemId::from("cues/1")
            }
        )
        .unwrap()
        .contains("\"cues/1\"")
    );
}
