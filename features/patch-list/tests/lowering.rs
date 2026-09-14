//! On load the list lowers to flat `(selector, entry)` pairs in the
//! scene engine's own `Selector` vocabulary (decision #29: "nothing new
//! is invented for matching").
//!
//! r[verify flow.patch-list.plan]

use dynamic_template::scenes::Selector;
use patch_list::{Entry, PatchList};

const ALBUM: &str = include_str!("../fixtures/album/patch-list.styx");

#[test]
fn every_entry_lowers_to_one_selector_in_file_order() {
    let list = PatchList::from_styx(ALBUM).expect("parse");
    let lowered = list.entries();
    // 6 + 4 + 1 + 1 + 1 + 1 + 1 + 14 + 2 + 1 + 1 rig entries in the fixture.
    assert_eq!(lowered.len(), 33);

    let first = &lowered[0];
    assert_eq!(first.performer, "cody");
    assert_eq!(first.kind, "guitar");
    assert_eq!(first.key, "di");
    assert_eq!(first.entry, Entry::Role("DI 3".into()));
    assert_eq!(
        first.selector,
        Selector {
            performer: Some("cody".into()),
            kind: Some("guitar".into()),
            multi_mic: Some("di".into()),
            ..Selector::default()
        }
    );
}

#[test]
fn a_slash_in_the_key_is_a_group_path_over_the_mic() {
    let list = PatchList::from_styx(ALBUM).expect("parse");
    let lowered = list.entries();

    let amp = lowered
        .iter()
        .find(|e| e.performer == "cody" && e.key == "amp-a/57")
        .expect("cody's amp A 57");
    assert_eq!(amp.selector.group, ["amp-a"]);
    assert_eq!(amp.selector.multi_mic.as_deref(), Some("57"));
    assert_eq!(amp.entry, Entry::Role("Mic 5".into()));

    let kick = lowered
        .iter()
        .find(|e| e.performer == "drummer" && e.key == "kick/in")
        .expect("the kick in mic");
    assert_eq!(kick.selector.performer.as_deref(), Some("drummer"));
    assert_eq!(kick.selector.kind.as_deref(), Some("drums"));
    assert_eq!(kick.selector.group, ["kick"]);
    assert_eq!(kick.selector.multi_mic.as_deref(), Some("in"));
}

#[test]
fn a_midi_entry_lowers_like_any_other() {
    let list = PatchList::from_styx(ALBUM).expect("parse");
    let synth = list
        .entries()
        .into_iter()
        .find(|e| e.performer == "john" && e.kind == "keys")
        .expect("john's synth");
    assert_eq!(synth.key, "synth");
    assert_eq!(synth.selector.multi_mic.as_deref(), Some("synth"));
    assert!(matches!(synth.entry, Entry::Midi { .. }));
}

#[test]
fn the_role_of_an_entry_is_only_a_role() {
    assert_eq!(Entry::Role("DI 3".into()).role(), Some("DI 3"));
    assert_eq!(
        Entry::Midi {
            device: "Nord".into(),
            channel: None
        }
        .role(),
        None
    );
}
