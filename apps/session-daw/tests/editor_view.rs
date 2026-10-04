//! The Editor view's item, on the golden session as the app opens it:
//! the selected track's first MIDI item (else the first any track has),
//! and its notes read back from the engine — what the view shows.
//!
//! One test, one process: the daw facade is bound to the first engine
//! opened.

use std::path::{Path, PathBuf};

use session_daw::studio::StudioSession;

fn template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../features/dynamic-template/fixtures/golden/template.rpp")
}

#[test]
fn the_editor_opens_on_a_midi_item_and_reads_its_notes() {
    let path = template();
    session_daw::open::open_silent(&path).expect("the golden session opens");
    let session = StudioSession::read_current(Some(&path), None).expect("and reads back");
    let project = &session.project.0;

    let target = session_daw::editor_view::target(project).expect("a MIDI item to edit");
    assert!(project.midi.contains(&target.item), "the target is MIDI");
    assert!(target.length > 0.0);

    let snapshot = session_daw::expression::load_take(&target.item, target.bpm, target.length)
        .expect("its notes read back");
    assert!(!snapshot.notes.is_empty());

    // The editor holds them.
    let mut view = session_daw::expression::Expression::from_take(
        &snapshot,
        target.item.clone(),
        target.drums,
        (0.0, 0.0),
        (1200.0, 700.0),
    );
    assert!(!view.stacked(), "a take opens on the roll");

    // An edit, saved, is in the take: the first note's velocity to 13.
    let at_13 =
        |notes: &[daw::service::midi::MidiNote]| notes.iter().filter(|n| n.velocity == 13).count();
    let had = at_13(&snapshot.notes);
    assert!(!view.unsaved(), "nothing changed yet");
    let before = snapshot.notes.len();
    let note = view.editor.doc.notes.first_mut().expect("a note");
    note.velocity = 13.0 / 127.0;
    assert!(view.unsaved());
    let content = view.take_content();
    assert!(!view.unsaved(), "saved");
    assert!(session_daw::expression::save_take(&target.item, content));
    let after = session_daw::expression::load_take(&target.item, target.bpm, target.length)
        .expect("read back");
    assert_eq!(after.notes.len(), before, "a save keeps every note");
    assert_eq!(at_13(&after.notes), had + 1, "the edit is in the take");

    // Select the piano: the editor's item is the piano's part, the song's
    // chords — three notes at a time over a bass root.
    let mut song = session_daw::editor_view::Song::of(project);
    for t in &mut song.tracks {
        t.selected = t.name == "Piano";
    }
    let piano = song.selected().expect("the piano has a MIDI part");
    assert!(!piano.drums);
    let part = session_daw::expression::load_take(&piano.item, piano.bpm, piano.length)
        .expect("the piano's notes");
    let first_beat: Vec<_> = part.notes.iter().filter(|n| n.start_ppq < 1.0).collect();
    assert!(
        first_beat.len() >= 4,
        "a root and a triad on the one: {first_beat:?}"
    );
}
