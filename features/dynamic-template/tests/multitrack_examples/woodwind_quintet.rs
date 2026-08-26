use daw_proto::{assert_tracks_equal, TrackGroup, TrackStructureBuilder};
use dynamic_template::*;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn woodwind_quintet() -> Result<()> {
    // -- Setup & Fixtures
    // Fennel Cartwright - Flower Drum Song: 5-stem woodwind quintet from MedleyDB.
    // French horn, oboe, clarinet, bassoon, and flute. Pure chamber music ensemble
    // with no rhythm section or vocals. Tests woodwind + horn orchestral classification.
    let items = vec![
        "01_FrenchHorn.wav",
        "02_Oboe.wav",
        "03_Clarinet.wav",
        "04_Bassoon.wav",
        "05_Flute.wav",
    ];
    let config = default_config();

    // -- Exec
    let tracks = items.organize_into_tracks(&config, None)?;

    // -- Check
    println!("\nTrack list:");
    daw_proto::display_tracklist(&tracks);

    // ============================================================================
    // Expected structure
    // ============================================================================

    // French horn dual-classified: top-level Horns + Orchestra/Horns
    let horns = TrackGroup::single_track("Horns", "01_FrenchHorn.wav");

    let woodwinds = TrackGroup::folder("Woodwinds")
        .track("Flutes")
        .item("05_Flute.wav")
        .track("Oboes")
        .item("02_Oboe.wav")
        .track("Clarinets")
        .item("03_Clarinet.wav")
        .track("Bassoons")
        .item("04_Bassoon.wav")
        .end();

    let orchestra = TrackGroup::folder("Orchestra")
        .track("Horns")
        .item("01_FrenchHorn.wav")
        .group(woodwinds)
        .end();

    let expected = TrackStructureBuilder::new()
        .group(horns)
        .group(orchestra)
        .build();

    assert_tracks_equal(&tracks, &expected)?;

    Ok(())
}
