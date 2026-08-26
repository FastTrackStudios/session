use daw_proto::{assert_tracks_equal, TrackGroup, TrackStructureBuilder};
use dynamic_template::*;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn swing_jazz_quartet() -> Result<()> {
    // -- Setup & Fixtures
    // MusicDelta Swing Jazz: 4-stem traditional swing jazz quartet from MedleyDB.
    // Double bass, drum set, piano, and clarinet. Classic Benny Goodman-style
    // small combo. Tests minimal jazz instrumentation with clarinet.
    let items = vec![
        "01_DoubleBass.wav",
        "02_Drums.wav",
        "03_Piano.wav",
        "04_Clarinet.wav",
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

    let drums = TrackGroup::single_track("Drums", "02_Drums.wav");
    let keys = TrackGroup::single_track("Keys", "03_Piano.wav");

    // Double bass → Orchestra/Contrabass, Clarinet → Orchestra/Clarinets
    let orchestra = TrackGroup::folder("Orchestra")
        .track("Contrabass")
        .item("01_DoubleBass.wav")
        .track("Clarinets")
        .item("04_Clarinet.wav")
        .end();

    let expected = TrackStructureBuilder::new()
        .group(drums)
        .group(keys)
        .group(orchestra)
        .build();

    assert_tracks_equal(&tracks, &expected)?;

    Ok(())
}
