//! Two people editing one session at the same time.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

use session_sync::{
    Change, ItemField, ItemState, MarkerState, ORIGIN_LOCAL, SessionDoc, SessionModel, TakeState,
    TempoPoint, TrackField, TrackState, diff,
};

fn track(guid: &str, parent: Option<&str>, name: &str) -> TrackState {
    TrackState {
        guid: guid.into(),
        parent: parent.map(Into::into),
        name: name.into(),
        volume: 1.0,
        parent_send: true,
        visible_in_tcp: true,
        visible_in_mixer: true,
        ..TrackState::default()
    }
}

fn item(track: &str, position: f64) -> ItemState {
    ItemState {
        track: track.into(),
        position,
        length: 4.0,
        volume: 1.0,
        fade_in_shape: "linear".into(),
        fade_out_shape: "linear".into(),
        take: TakeState {
            name: "Kick".into(),
            source: Some("Media/Kick.wav".into()),
            start_offset: 0.0,
            playrate: 1.0,
        },
        ..ItemState::default()
    }
}

fn song() -> SessionModel {
    let mut m = SessionModel {
        tracks: vec![
            track("drums", None, "Drums"),
            track("kick", Some("drums"), "Kick"),
            track("snare", Some("drums"), "Snare"),
            track("bass", None, "Bass"),
        ],
        tempo: vec![TempoPoint {
            at: 0.0,
            bpm: 72.0,
            beats_per_bar: 4,
            beat_unit: 4,
        }],
        chart: "Song\n72bpm 4/4 #E\n\nVS 4\n1 4 6m 5\n".into(),
        ..SessionModel::default()
    };
    m.items.insert("i-kick".into(), item("kick", 0.0));
    m.items.insert("i-bass".into(), item("bass", 2.0));
    m.markers.insert(
        "m1".into(),
        MarkerState {
            at: 8.0,
            name: "Count".into(),
            color: None,
        },
    );
    m
}

/// Two replicas of `song()`, as two collaborators who both opened it.
fn two_peers() -> (SessionDoc, SessionDoc) {
    let a = SessionDoc::new();
    a.write(&song(), ORIGIN_LOCAL).unwrap();
    let b = SessionDoc::new();
    b.import(&a.snapshot().unwrap(), "remote").unwrap();
    (a, b)
}

/// Exchange everything both ways.
fn sync(a: &SessionDoc, b: &SessionDoc) {
    let to_b = a.updates_since(&b.version()).unwrap();
    let to_a = b.updates_since(&a.version()).unwrap();
    b.import(&to_b, "remote").unwrap();
    a.import(&to_a, "remote").unwrap();
}

#[test]
fn a_model_reads_back_as_written() {
    let doc = SessionDoc::new();
    doc.write(&song(), ORIGIN_LOCAL).unwrap();
    assert_eq!(doc.read(), song());
}

#[test]
fn writing_the_same_model_again_records_nothing() {
    let doc = SessionDoc::new();
    doc.write(&song(), ORIGIN_LOCAL).unwrap();
    let before = doc.version();
    doc.write(&song(), ORIGIN_LOCAL).unwrap();
    assert_eq!(doc.version(), before);
}

#[test]
fn one_moves_an_item_while_the_other_turns_it_down() {
    let (a, b) = two_peers();
    let mut ma = a.read();
    ma.items.get_mut("i-kick").unwrap().position = 1.5;
    a.write(&ma, ORIGIN_LOCAL).unwrap();
    let mut mb = b.read();
    mb.items.get_mut("i-kick").unwrap().volume = 0.5;
    b.write(&mb, ORIGIN_LOCAL).unwrap();

    sync(&a, &b);
    let merged = a.read();
    assert_eq!(merged, b.read());
    assert_eq!(merged.items["i-kick"].position, 1.5);
    assert_eq!(merged.items["i-kick"].volume, 0.5);
}

#[test]
fn one_renames_a_track_while_the_other_moves_it_into_a_folder() {
    let (a, b) = two_peers();
    let mut ma = a.read();
    ma.tracks
        .iter_mut()
        .find(|t| t.guid == "bass")
        .unwrap()
        .name = "Bass DI".into();
    a.write(&ma, ORIGIN_LOCAL).unwrap();

    let mut mb = b.read();
    let bass = mb.tracks.pop().unwrap();
    mb.tracks.insert(
        1,
        TrackState {
            parent: Some("drums".into()),
            ..bass
        },
    );
    b.write(&mb, ORIGIN_LOCAL).unwrap();

    sync(&a, &b);
    let merged = a.read();
    assert_eq!(merged, b.read());
    let names: Vec<(&str, Option<&str>)> = merged
        .tracks
        .iter()
        .map(|t| (t.name.as_str(), t.parent.as_deref()))
        .collect();
    assert_eq!(
        names,
        vec![
            ("Drums", None),
            ("Bass DI", Some("drums")),
            ("Kick", Some("drums")),
            ("Snare", Some("drums"))
        ]
    );
}

#[test]
fn both_add_tracks_and_neither_is_lost() {
    let (a, b) = two_peers();
    let mut ma = a.read();
    ma.tracks.push(track("keys", None, "Keys"));
    a.write(&ma, ORIGIN_LOCAL).unwrap();
    let mut mb = b.read();
    mb.tracks.push(track("vox", None, "Vox"));
    b.write(&mb, ORIGIN_LOCAL).unwrap();

    sync(&a, &b);
    let merged = a.read();
    assert_eq!(merged, b.read());
    assert!(merged.track("keys").is_some() && merged.track("vox").is_some());
    assert_eq!(merged.tracks.len(), 6);
}

#[test]
fn concurrent_chart_edits_both_land() {
    let (a, b) = two_peers();
    let mut ma = a.read();
    ma.chart = ma.chart.replace("VS 4\n", "IN 2\n1 5\n\nVS 4\n");
    a.write(&ma, ORIGIN_LOCAL).unwrap();
    let mut mb = b.read();
    mb.chart.push_str("\nCH 4\n4 1 5 6m\n");
    b.write(&mb, ORIGIN_LOCAL).unwrap();

    sync(&a, &b);
    let chart = a.read().chart;
    assert_eq!(chart, b.read().chart);
    assert!(chart.contains("IN 2\n1 5\n"), "{chart}");
    assert!(chart.contains("CH 4\n4 1 5 6m\n"), "{chart}");
}

#[test]
fn a_deleted_track_stays_deleted_and_its_folder_keeps_the_rest() {
    let (a, b) = two_peers();
    let mut ma = a.read();
    ma.tracks.retain(|t| t.guid != "snare");
    a.write(&ma, ORIGIN_LOCAL).unwrap();
    let mut mb = b.read();
    mb.tracks
        .iter_mut()
        .find(|t| t.guid == "kick")
        .unwrap()
        .muted = true;
    b.write(&mb, ORIGIN_LOCAL).unwrap();

    sync(&a, &b);
    let merged = a.read();
    assert_eq!(merged, b.read());
    assert!(merged.track("snare").is_none());
    assert!(merged.track("kick").unwrap().muted);
}

#[test]
fn history_keeps_every_version() {
    let doc = SessionDoc::new();
    doc.write(&song(), ORIGIN_LOCAL).unwrap();
    let first = doc.loro().state_frontiers();
    let mut m = doc.read();
    m.items.get_mut("i-bass").unwrap().position = 10.0;
    doc.write(&m, ORIGIN_LOCAL).unwrap();

    // Open the session as it was before the move.
    let then = SessionDoc::from_loro(doc.loro().fork_at(&first).unwrap());
    assert_eq!(then.read().items["i-bass"].position, 2.0);
    assert_eq!(doc.read().items["i-bass"].position, 10.0);
}

#[test]
fn a_snapshot_reopens_with_its_history() {
    let doc = SessionDoc::new();
    doc.write(&song(), ORIGIN_LOCAL).unwrap();
    let reopened = SessionDoc::new();
    reopened.import(&doc.snapshot().unwrap(), "file").unwrap();
    assert_eq!(reopened.read(), song());
    assert_eq!(reopened.version(), doc.version());
}

#[test]
fn diff_names_what_the_engine_must_do() {
    let old = song();
    let mut new = song();
    new.tracks
        .iter_mut()
        .find(|t| t.guid == "bass")
        .unwrap()
        .volume = 0.7;
    new.tracks.push(track("keys", None, "Keys"));
    new.items.get_mut("i-kick").unwrap().position = 3.0;
    new.items.insert("i-keys".into(), item("keys", 0.0));
    new.tracks.retain(|t| t.guid != "snare");
    new.markers.clear();

    let changes = diff(&old, &new);
    assert!(changes.contains(&Change::TrackAdded {
        track: track("keys", None, "Keys"),
        index: 2
    }));
    assert!(changes.iter().any(|c| matches!(c, Change::TrackChanged { track, fields } if track.guid == "bass" && fields == &[TrackField::Volume])));
    assert!(changes.iter().any(|c| matches!(c, Change::ItemChanged { guid, fields, .. } if guid == "i-kick" && fields == &[ItemField::Position])));
    assert!(changes.contains(&Change::MarkerRemoved { guid: "m1".into() }));
    // Tracks are added before items go on them, and removed last.
    let added = changes
        .iter()
        .position(|c| matches!(c, Change::TrackAdded { .. }))
        .unwrap();
    let item_added = changes
        .iter()
        .position(|c| matches!(c, Change::ItemAdded { .. }))
        .unwrap();
    assert!(added < item_added);
    assert!(matches!(changes.last(), Some(Change::TrackRemoved { guid }) if guid == "snare"));
    assert!(diff(&new, &new).is_empty());
}

/// Tracks and items converge even when two replicas each wrote the same
/// song before syncing. The chart does NOT: two independent inserts of
/// the same text are two texts to a text CRDT. Which is why joining a
/// session always starts from the host's doc (`import`, then read), and
/// never from the joiner's own copy of the file.
#[test]
fn two_who_opened_their_own_copy_converge_on_one_set_of_tracks() {
    let a = SessionDoc::new();
    a.write(&song(), ORIGIN_LOCAL).unwrap();
    let b = SessionDoc::new();
    b.write(&song(), ORIGIN_LOCAL).unwrap();
    sync(&a, &b);
    // The next write on each side removes the duplicate nodes.
    a.write(&a.read(), ORIGIN_LOCAL).unwrap();
    b.write(&b.read(), ORIGIN_LOCAL).unwrap();
    sync(&a, &b);
    let (ma, mb) = (a.read(), b.read());
    assert_eq!(ma.tracks, song().tracks);
    assert_eq!(mb.tracks, song().tracks);
    assert_eq!(ma.items, song().items);
    assert_eq!(ma, mb);
}
