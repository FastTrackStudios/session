use daw_proto::{assert_tracks_equal, TrackGroup, TrackStructureBuilder};
use dynamic_template::*;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn entwistle_dont_you_ever() -> Result<()> {
    // -- Setup & Fixtures
    // Matthew Entwistle - Don't You Ever: 7-stem orchestral pop session from MedleyDB.
    // Double bass, piano, brass section, drums, two violin sections, and male vocalist.
    // Tests orchestral pop with brass and string sections.
    let items = vec![
        "01_DoubleBass.wav",
        "02_Piano.wav",
        "03_BrassSection.wav",
        "04_Drums.wav",
        "05_Violins1.wav",
        "06_Violins2.wav",
        "07_LeadVox.wav",
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

    let drums = TrackGroup::single_track("Drums", "04_Drums.wav");
    let keys = TrackGroup::single_track("Keys", "02_Piano.wav");
    let vocals = TrackGroup::single_track("Vocals", "07_LeadVox.wav");

    let violins = TrackGroup::folder("Violins")
        .track("Violins 1")
        .item("05_Violins1.wav")
        .track("Violins 2")
        .item("06_Violins2.wav")
        .end();

    let strings = TrackGroup::folder("Strings")
        .group(violins)
        .track("Contrabass")
        .item("01_DoubleBass.wav")
        .end();

    let orchestra = TrackGroup::folder("Orchestra")
        .group(strings)
        .track("Brass")
        .item("03_BrassSection.wav")
        .end();

    let expected = TrackStructureBuilder::new()
        .group(drums)
        .group(keys)
        .group(vocals)
        .group(orchestra)
        .build();

    assert_tracks_equal(&tracks, &expected)?;

    Ok(())
}
