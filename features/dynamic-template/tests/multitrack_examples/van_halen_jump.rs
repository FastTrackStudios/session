use daw_proto::{assert_tracks_equal, TrackGroup, TrackStructureBuilder};
use dynamic_template::*;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn van_halen_jump() -> Result<()> {
    // -- Setup & Fixtures
    // Van Halen - Jump: 10-stem synth-rock session. Features the iconic OB-Xa synth
    // alongside Eddie's guitar work, full drum kit with separated components,
    // bass, lead vocals and backing vocals. Tests synth-heavy rock arrangement.
    let items = vec![
        "01_Kick.wav",
        "02_Snare.wav",
        "03_HiHat.wav",
        "04_Overheads.wav",
        "05_Bass.wav",
        "06_Guitar.wav",
        "07_Synth1.wav",
        "08_Synth2.wav",
        "09_LeadVox.wav",
        "10_BackingVox.wav",
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

    let cymbals = TrackGroup::folder("Cymbals")
        .track("OH")
        .item("04_Overheads.wav")
        .track("Hi Hat")
        .item("03_HiHat.wav")
        .end();

    let drums = TrackGroup::folder("Drums")
        .track("Kick")
        .item("01_Kick.wav")
        .track("Snare")
        .item("02_Snare.wav")
        .group(cymbals)
        .end();

    let bass = TrackGroup::single_track("Bass", "05_Bass.wav");
    let guitars = TrackGroup::single_track("Guitars", "06_Guitar.wav");

    let synths = TrackGroup::folder("Synths")
        .track("Synth1")
        .item("07_Synth1.wav")
        .track("Synth2")
        .item("08_Synth2.wav")
        .end();

    let vocals = TrackGroup::folder("Vocals")
        .track("Lead")
        .item("09_LeadVox.wav")
        .track("BGVs")
        .item("10_BackingVox.wav")
        .end();

    let expected = TrackStructureBuilder::new()
        .group(drums)
        .group(bass)
        .group(guitars)
        .group(synths)
        .group(vocals)
        .build();

    assert_tracks_equal(&tracks, &expected)?;

    Ok(())
}
