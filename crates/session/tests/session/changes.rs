//! What changed, by kind: the stream an engine follows in process, and
//! the version-to-version question a view or a remote engine asks
//! through the service.

use std::sync::{Arc, Mutex};

use loro::ExportMode;
use session::changes::{Change, ChangeSet};
use session::doc::SessionDoc;
use session::service::{Grants, Request, Response, handle};
use session::{ExtensionId, Item, ItemId, LaneId, TempoMap, Time};

use crate::time::lane;

fn item(owner: &str, lane: &str, at: f64) -> Item {
    Item {
        owner: ExtensionId::from(owner),
        lane: LaneId::from(lane),
        kind: format!("{owner}.clip"),
        at: Time::Seconds(at),
        end: None,
        payload: session::Value::Null,
    }
}

/// What a subscription heard, kept for the test to read.
fn heard(doc: &SessionDoc) -> (Arc<Mutex<Vec<ChangeSet>>>, loro::Subscription) {
    let got: Arc<Mutex<Vec<ChangeSet>>> = Arc::default();
    let into = got.clone();
    let sub = doc.subscribe_changes(move |set| into.lock().unwrap().push(set));
    (got, sub)
}

fn take(got: &Arc<Mutex<Vec<ChangeSet>>>) -> Vec<ChangeSet> {
    std::mem::take(&mut *got.lock().unwrap())
}

#[test]
fn an_engine_hears_what_each_commit_changed_by_kind() {
    let doc = SessionDoc::new();
    let (got, _sub) = heard(&doc);

    doc.set_tempo(&TempoMap::new(
        128.0,
        session::time::TimeSignature::new(4, 4),
        Vec::new(),
    ))
    .unwrap();
    doc.doc().commit();
    assert_eq!(
        take(&got),
        [ChangeSet {
            changes: vec![Change::Tempo],
            remote: false
        }]
    );

    doc.put_lane(
        &LaneId::from("drums"),
        &lane(&ExtensionId::from("daw"), "drums"),
    )
    .unwrap();
    doc.put_item(&ItemId::from("kick-1"), &item("daw", "drums", 0.0))
        .unwrap();
    doc.doc().commit();
    let sets = take(&got);
    let all: Vec<Change> = sets.iter().flat_map(|s| s.changes.clone()).collect();
    assert!(
        all.contains(&Change::Lane(LaneId::from("drums"))),
        "{all:?}"
    );
    assert!(
        all.contains(&Change::Item(ItemId::from("kick-1"))),
        "{all:?}"
    );

    // An edit inside an item already there names that item.
    doc.put_item(&ItemId::from("kick-1"), &item("daw", "drums", 2.0))
        .unwrap();
    doc.doc().commit();
    let all: Vec<Change> = take(&got).into_iter().flat_map(|s| s.changes).collect();
    assert_eq!(all, [Change::Item(ItemId::from("kick-1"))]);

    // An extension's own data names the extension.
    let keyflow = ExtensionId::from("keyflow");
    doc.ext(&keyflow)
        .unwrap()
        .set_text("chart", "Verse: C G Am F\n")
        .unwrap();
    doc.doc().commit();
    let all: Vec<Change> = take(&got).into_iter().flat_map(|s| s.changes).collect();
    assert!(all.contains(&Change::Extension(keyflow)), "{all:?}");
}

#[test]
fn another_replicas_edit_arrives_as_a_remote_change() {
    let a = SessionDoc::new();
    let b_doc = loro::LoroDoc::new();
    b_doc
        .import(&a.doc().export(ExportMode::Snapshot).unwrap())
        .unwrap();
    let b = SessionDoc::from_doc(b_doc);
    let (got, _sub) = heard(&b);

    a.put_item(&ItemId::from("cue-7"), &item("ignition", "cues", 12.0))
        .unwrap();
    a.doc().commit();
    b.doc()
        .import(
            &a.doc()
                .export(ExportMode::updates(&b.doc().oplog_vv()))
                .unwrap(),
        )
        .unwrap();
    let sets = take(&got);
    assert!(sets.iter().all(|s| s.remote), "{sets:?}");
    let all: Vec<Change> = sets.into_iter().flat_map(|s| s.changes).collect();
    assert!(
        all.contains(&Change::Item(ItemId::from("cue-7"))),
        "{all:?}"
    );
}

fn changes(doc: &SessionDoc, caller: &str, since: Option<String>) -> (String, bool, Vec<Change>) {
    match handle(
        doc,
        &ExtensionId::from(caller),
        Grants::default(),
        Request::ReadChanges { since },
    )
    .unwrap()
    {
        Response::Changes {
            version,
            whole,
            changes,
        } => (version, whole, changes),
        other => panic!("not changes: {other:?}"),
    }
}

#[test]
fn a_view_asks_what_changed_since_its_version_and_hears_only_what_is_its() {
    let doc = SessionDoc::new();
    // First ask: no version yet, so read everything.
    let (v0, whole, _) = changes(&doc, "daw", None);
    assert!(whole);

    doc.put_lane(
        &LaneId::from("gtr"),
        &lane(&ExtensionId::from("daw"), "gtr"),
    )
    .unwrap();
    doc.put_item(&ItemId::from("riff"), &item("daw", "gtr", 4.0))
        .unwrap();
    doc.ext(&ExtensionId::from("daw"))
        .unwrap()
        .set_text("notes", "louder")
        .unwrap();
    doc.ext(&ExtensionId::from("keyflow"))
        .unwrap()
        .set_text("chart", "Verse: C\n")
        .unwrap();
    doc.doc().commit();

    let (v1, whole, seen) = changes(&doc, "daw", Some(v0.clone()));
    assert!(!whole);
    assert!(
        seen.contains(&Change::Lane(LaneId::from("gtr"))),
        "{seen:?}"
    );
    assert!(
        seen.contains(&Change::Item(ItemId::from("riff"))),
        "{seen:?}"
    );
    assert!(
        seen.contains(&Change::Extension(ExtensionId::from("daw"))),
        "{seen:?}"
    );
    // Keyflow's chart is not the DAW's to hear about.
    assert!(
        !seen.contains(&Change::Extension(ExtensionId::from("keyflow"))),
        "{seen:?}"
    );
    // Keyflow hears of its own chart from the same version.
    let (_, _, keyflow_saw) = changes(&doc, "keyflow", Some(v0));
    assert!(
        keyflow_saw.contains(&Change::Extension(ExtensionId::from("keyflow"))),
        "{keyflow_saw:?}"
    );

    // Nothing since the version handed out: nothing changed.
    let (v2, whole, seen) = changes(&doc, "daw", Some(v1.clone()));
    assert!(!whole && seen.is_empty(), "{seen:?}");
    assert_eq!(v1, v2);

    // A version that is not this session's: read everything again.
    let (_, whole, _) = changes(&doc, "daw", Some("zz".to_owned()));
    assert!(whole);
}
