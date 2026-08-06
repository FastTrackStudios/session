//! REAPER integration tests for the Key track.
//!
//! The unit tests prove labels round-trip as strings. These prove the
//! thing that actually matters and that only REAPER can answer: that a
//! key change written as an item label survives the trip through a real
//! project and reads back as the same key.
//!
//! ```sh
//! just reaper daw-test key_track
//! just reaper daw-test --gui key_track_round_trips   # watch it
//! ```

use daw::test::reaper_test;
use daw_proto::primitives::{Duration, PositionInSeconds};
use session::key::{KEY_TRACK, format_key, parse_key};

use keyflow::key::Key;
use keyflow::primitives::MusicalNote;

fn key_of(root: &str, major: bool) -> Key {
    let note = MusicalNote::from_string(root).expect("a note");
    if major { Key::major(note) } else { Key::minor(note) }
}

/// A key written into an item's label comes back as the same key. This is
/// the whole premise of storing key changes in the project rather than in
/// a side-car: if REAPER mangles the text, the design fails.
#[reaper_test(isolated)]
async fn key_track_round_trips_through_reaper(
    ctx: &daw::test::ReaperTestContext,
) -> eyre::Result<()> {
    let project = ctx.project().clone();
    let track = project.tracks().add(KEY_TRACK, None).await?;

    let key = key_of("Eb", true);
    let item = track
        .items()
        .add(
            PositionInSeconds::from_seconds(0.0),
            Duration::from_seconds(2.0),
        )
        .await?;
    item.set_label(&format_key(&key)).await?;

    let read_back = item.label().await?.expect("the label should persist");
    ctx.log(&format!("label survived as {read_back:?}"));
    assert_eq!(read_back, "Eb major");
    assert_eq!(parse_key(&read_back), Some(key), "same key after the trip");
    Ok(())
}

/// Several changes, each on its own item, stay distinct and keep their
/// positions — the property "the key at bar N" depends on.
#[reaper_test(isolated)]
async fn key_changes_keep_their_positions(
    ctx: &daw::test::ReaperTestContext,
) -> eyre::Result<()> {
    let project = ctx.project().clone();
    let track = project.tracks().add(KEY_TRACK, None).await?;

    let plan = [(0.0, "C", true), (4.0, "A", false), (8.0, "F#", false)];
    for (at, root, major) in plan {
        let item = track
            .items()
            .add(
                PositionInSeconds::from_seconds(at),
                Duration::from_seconds(2.0),
            )
            .await?;
        item.set_label(&format_key(&key_of(root, major))).await?;
    }

    let items = track.items().all().await?;
    assert_eq!(items.len(), 3, "one item per change");


    let mut found: Vec<(f64, String)> = Vec::new();
    for info in items {
        let handle = track
            .items()
            .by_guid(&info.guid)
            .await?
            .expect("the item we just made");
        let label = handle.label().await?.unwrap_or_default();
        found.push((info.position.as_seconds(), label));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));

    ctx.log(&format!("{found:?}"));
    assert_eq!(found[0], (0.0, "C major".to_string()));
    assert_eq!(found[1], (4.0, "A minor".to_string()));
    assert_eq!(found[2], (8.0, "F# minor".to_string()));
    Ok(())
}

/// Accidentals are the part most likely to be mangled by a round trip
/// through a text field, and the part that matters most — Db major and
/// C# major are different keys.
#[reaper_test(isolated)]
async fn accidentals_survive_the_round_trip(
    ctx: &daw::test::ReaperTestContext,
) -> eyre::Result<()> {
    let project = ctx.project().clone();
    let track = project.tracks().add(KEY_TRACK, None).await?;

    for (i, root) in ["C#", "Db", "F#", "Gb", "Bb"].iter().enumerate() {
        let key = key_of(root, true);
        let item = track
            .items()
            .add(
                PositionInSeconds::from_seconds(i as f64 * 2.0),
                Duration::from_seconds(1.0),
            )
            .await?;
        item.set_label(&format_key(&key)).await?;

        let back = item.label().await?.expect("label");
        assert_eq!(
            parse_key(&back),
            Some(key),
            "{root} major came back as {back:?}"
        );
        ctx.log(&format!("{root} major ok"));
    }
    Ok(())
}
