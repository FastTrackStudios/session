//! The patch list round-trips through styx (decision #29: "styx from
//! day one, Facet types, a schema, a round-trip test").
//!
//! r[verify flow.patch-list.plan]
//! r[verify flow.patch-list.project-level]

use patch_list::{Entry, PatchList};

const ALBUM: &str = include_str!("../fixtures/album/patch-list.styx");

#[test]
fn the_fixture_album_parses_with_its_shape_intact() {
    let list = PatchList::from_styx(ALBUM).expect("the fixture album parses");

    // Performers in the order the file lists them — the view and the
    // performer rows sort by it.
    let performers: Vec<&str> = list.performers.keys().map(String::as_str).collect();
    assert_eq!(
        performers,
        [
            "cody", "john", "ron", "belen", "aline", "drummer", "bassist", "engineer", "producer"
        ]
    );

    // performer → source kind → channel or mic → input role.
    let cody = &list.performers["cody"];
    assert_eq!(cody["guitar"]["di"], Entry::Role("DI 3".into()));
    assert_eq!(cody["guitar"]["amp-a/57"], Entry::Role("Mic 5".into()));

    // A MIDI source is an entry giving the device and channel.
    assert_eq!(
        list.performers["john"]["keys"]["synth"],
        Entry::Midi {
            device: "Prophet-6".into(),
            channel: Some(1),
        }
    );

    // The kit: the drummer's rig of kind drums, keyed piece/mic.
    let kit = &list.performers["drummer"]["drums"];
    assert_eq!(kit.len(), 14);
    assert_eq!(kit["snare/top"], Entry::Role("Snare Top".into()));

    // Headphone buses, with who each is for.
    assert_eq!(list.headphones["cody"].output, "HP 1");
    assert_eq!(list.headphones["cody"].audience, ["cody"]);
    assert_eq!(list.headphones["broadcast"].output, "Broadcast");
    assert_eq!(list.headphones.len(), 10);
}

#[test]
fn the_fixture_album_round_trips() {
    let list = PatchList::from_styx(ALBUM).expect("parse");
    let text = list.to_styx().expect("serialize");
    let back = PatchList::from_styx(&text).expect("parse what we wrote");
    assert_eq!(back, list, "written text was:\n{text}");
}

#[test]
fn a_bus_for_several_people_is_one_bus() {
    let text = "headphones {\n    choir {output \"HP 8\", for (soprano-1 soprano-2 alto)}\n}";
    let list = PatchList::from_styx(text).expect("parse");
    assert_eq!(
        list.headphones["choir"].audience,
        ["soprano-1", "soprano-2", "alto"]
    );
    assert!(list.performers.is_empty());
}

#[test]
fn a_midi_entry_without_a_channel_means_every_channel() {
    let text = "performers {\n    ron {keys {nord @midi{device \"Nord Stage 3\"}}}\n}";
    let list = PatchList::from_styx(text).expect("parse");
    assert_eq!(
        list.performers["ron"]["keys"]["nord"],
        Entry::Midi {
            device: "Nord Stage 3".into(),
            channel: None,
        }
    );
}

#[test]
fn a_document_that_is_not_a_patch_list_is_an_error() {
    let err = PatchList::from_styx("performers (a b c)").expect_err("a sequence is not a map");
    assert!(!err.to_string().is_empty());
}
