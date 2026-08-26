use daw_proto::{assert_tracks_equal, TrackGroup, TrackStructureBuilder};
use dynamic_template::*;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn mars_sws() -> Result<()> {
    // -- Setup & Fixtures
    // Mars SWS: 124-track session with duplicate items and extensive BGV arrangements.
    // No drums — only piano (with room mic), synth bass, lead vocals (Kim + Steve),
    // and a large BGV stack (4 parts × SUM + A/B/C/D, many duplicates).
    let items = vec![
        "Bass Synth.L.wav",
        "Bass Synth.R.wav",
        "Bass Synth.wav",
        "Bass Synth.wav",
        "Bass Synth.wav",
        "BGV1_SUM.wav",
        "BGV1_SUM.wav",
        "BGV1A.wav",
        "BGV1A.wav",
        "BGV1A.wav",
        "BGV1A.wav",
        "BGV1B.wav",
        "BGV1B.wav",
        "BGV1B.wav",
        "BGV1B.wav",
        "BGV1C.wav",
        "BGV1C.wav",
        "BGV1C.wav",
        "BGV1C.wav",
        "BGV1D.wav",
        "BGV1D.wav",
        "BGV1D.wav",
        "BGV1D.wav",
        "BGV2_SUM.wav",
        "BGV2_SUM.wav",
        "BGV2A.wav",
        "BGV2A.wav",
        "BGV2A.wav",
        "BGV2A.wav",
        "BGV2B.wav",
        "BGV2B.wav",
        "BGV2B.wav",
        "BGV2B.wav",
        "BGV2C.wav",
        "BGV2C.wav",
        "BGV2C.wav",
        "BGV2C.wav",
        "BGV2D.wav",
        "BGV2D.wav",
        "BGV2D.wav",
        "BGV2D.wav",
        "BGV3_SUM.wav",
        "BGV3_SUM.wav",
        "BGV3A.wav",
        "BGV3A.wav",
        "BGV3A.wav",
        "BGV3A.wav",
        "BGV3B.wav",
        "BGV3B.wav",
        "BGV3B.wav",
        "BGV3B.wav",
        "BGV3C.wav",
        "BGV3C.wav",
        "BGV3C.wav",
        "BGV3C.wav",
        "BGV3D.wav",
        "BGV3D.wav",
        "BGV3D.wav",
        "BGV3D.wav",
        "BGV4_SUM.wav",
        "BGV4_SUM.wav",
        "BGV4A.wav",
        "BGV4A.wav",
        "BGV4A.wav",
        "BGV4A.wav",
        "BGV4B.wav",
        "BGV4B.wav",
        "BGV4B.wav",
        "BGV4B.wav",
        "BGV4C.wav",
        "BGV4C.wav",
        "BGV4C.wav",
        "BGV4C.wav",
        "BGV4D.wav",
        "BGV4D.wav",
        "BGV4D.wav",
        "BGV4D.wav",
        "Crystalizer_Print_1.wav",
        "Crystalizer_Print.wav",
        "Crystalizer_Print.wav",
        "Kim VOX_1.wav",
        "Kim VOX_2.wav",
        "Kim VOX_2.wav",
        "Kim VOX_SUM.wav",
        "Kim VOX_SUM.wav",
        "Kim VOX.wav",
        "Kim VOX.wav",
        "Kim VOX.wav",
        "Mars Kim vx.wav",
        "Mars_PLAP.wav",
        "Piano L.wav",
        "Piano L.wav",
        "Piano L.wav",
        "Piano L.wav",
        "Piano R.wav",
        "Piano R.wav",
        "Piano R.wav",
        "Piano R.wav",
        "Piano Room Mono.wav",
        "Piano Room Mono.wav",
        "Piano Room Mono.wav",
        "Piano Room Mono.wav",
        "Piano_Rough_76bpm.wav",
        "Piano_SUM.wav",
        "Piano_SUM.wav",
        "Reverse Piano.wav",
        "Reverse Piano.wav",
        "Reverse Piano.wav",
        "Reverse Piano.wav",
        "Steve VOX_SUM.wav",
        "Steve VOX_SUM.wav",
        "Steve VOX.wav",
        "Steve VOX.wav",
        "Steve VOX.wav",
        "Steve VOX.wav",
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

    // --- Bass ---
    // Bass Synth L/R are the stereo DI pair; remaining three are duplicate takes.
    let bass = TrackGroup::folder("Bass")
        .track("Synth 1")
        .item("Bass Synth.L.wav")
        .track("Synth 2")
        .item("Bass Synth.R.wav")
        .track("Synth 3")
        .item("Bass Synth.wav")
        .track("Synth 4")
        .item("Bass Synth.wav")
        .track("Synth 5")
        .item("Bass Synth.wav")
        .end();

    // --- Keys ---
    // All piano variants: L/R stereo (×4 each), Piano Room Mono (×4, now goes to Keys
    // since Room exclusions were added to Drums/Rooms for non-drum instruments),
    // Rough scratch, SUM (×2), Reverse Piano (×4).
    let keys = TrackGroup::folder("Keys")
        .track("Piano 1")
        .item("Piano L.wav")
        .track("Piano 2")
        .item("Piano L.wav")
        .track("Piano 3")
        .item("Piano L.wav")
        .track("Piano 4")
        .item("Piano L.wav")
        .track("Piano 5")
        .item("Piano R.wav")
        .track("Piano 6")
        .item("Piano R.wav")
        .track("Piano 7")
        .item("Piano R.wav")
        .track("Piano 8")
        .item("Piano R.wav")
        .track("Piano 9")
        .item("Piano Room Mono.wav")
        .track("Piano 10")
        .item("Piano Room Mono.wav")
        .track("Piano 11")
        .item("Piano Room Mono.wav")
        .track("Piano 12")
        .item("Piano Room Mono.wav")
        .track("Piano 13")
        .item("Piano_Rough_76bpm.wav")
        .track("Piano 14")
        .item("Piano_SUM.wav")
        .track("Piano 15")
        .item("Piano_SUM.wav")
        .track("Piano 16")
        .item("Reverse Piano.wav")
        .track("Piano 17")
        .item("Reverse Piano.wav")
        .track("Piano 18")
        .item("Reverse Piano.wav")
        .track("Piano 19")
        .item("Reverse Piano.wav")
        .end();

    // --- Vocals ---
    // Kim: VOX_SUM (×2) and bare VOX (×3) share the same base name and group into
    // an inner Kim folder. VOX_1 (playlist take 1) lands beside it as a sibling
    // track. VOX_2 (×2) forms a "Kim 2" layer subfolder.
    let kim_inner = TrackGroup::folder("Kim")
        .track("Kim 1")
        .item("Kim VOX_SUM.wav")
        .track("Kim 2")
        .item("Kim VOX_SUM.wav")
        .track("Kim 3")
        .item("Kim VOX.wav")
        .track("Kim 4")
        .item("Kim VOX.wav")
        .track("Kim 5")
        .item("Kim VOX.wav")
        .end();

    let kim_2 = TrackGroup::folder("Kim 2")
        .track("Kim 1")
        .item("Kim VOX_2.wav")
        .track("Kim 2")
        .item("Kim VOX_2.wav")
        .end();

    let kim = TrackGroup::folder("Kim")
        .group(kim_inner)
        .track("Kim 1")
        .item("Kim VOX_1.wav")
        .group(kim_2)
        .end();

    let steve = TrackGroup::folder("Steve")
        .track("Steve 1")
        .item("Steve VOX_SUM.wav")
        .track("Steve 2")
        .item("Steve VOX_SUM.wav")
        .track("Steve 3")
        .item("Steve VOX.wav")
        .track("Steve 4")
        .item("Steve VOX.wav")
        .track("Steve 5")
        .item("Steve VOX.wav")
        .track("Steve 6")
        .item("Steve VOX.wav")
        .end();

    // BGVs: 4 parts × (SUM×2 + A×4 + B×4 + C×4 + D×4) = 72 flat tracks.
    // Note: the first two BGV4A entries render without a sequence number — this is
    // a monarchy counter quirk when the embedded layer digit "4" clashes with the
    // existing sequence state.
    let bgvs = TrackGroup::folder("BGVs")
        // BGV1
        .track("BGVs 1")
        .item("BGV1_SUM.wav")
        .track("BGVs 2")
        .item("BGV1_SUM.wav")
        .track("BGVs 3")
        .item("BGV1A.wav")
        .track("BGVs 4")
        .item("BGV1A.wav")
        .track("BGVs 5")
        .item("BGV1A.wav")
        .track("BGVs 6")
        .item("BGV1A.wav")
        .track("BGVs 7")
        .item("BGV1B.wav")
        .track("BGVs 8")
        .item("BGV1B.wav")
        .track("BGVs 9")
        .item("BGV1B.wav")
        .track("BGVs 10")
        .item("BGV1B.wav")
        .track("BGVs 11")
        .item("BGV1C.wav")
        .track("BGVs 12")
        .item("BGV1C.wav")
        .track("BGVs 13")
        .item("BGV1C.wav")
        .track("BGVs 14")
        .item("BGV1C.wav")
        .track("BGVs 15")
        .item("BGV1D.wav")
        .track("BGVs 16")
        .item("BGV1D.wav")
        .track("BGVs 17")
        .item("BGV1D.wav")
        .track("BGVs 18")
        .item("BGV1D.wav")
        // BGV2
        .track("BGVs 19")
        .item("BGV2_SUM.wav")
        .track("BGVs 20")
        .item("BGV2_SUM.wav")
        .track("BGVs 21")
        .item("BGV2A.wav")
        .track("BGVs 22")
        .item("BGV2A.wav")
        .track("BGVs 23")
        .item("BGV2A.wav")
        .track("BGVs 24")
        .item("BGV2A.wav")
        .track("BGVs 25")
        .item("BGV2B.wav")
        .track("BGVs 26")
        .item("BGV2B.wav")
        .track("BGVs 27")
        .item("BGV2B.wav")
        .track("BGVs 28")
        .item("BGV2B.wav")
        .track("BGVs 29")
        .item("BGV2C.wav")
        .track("BGVs 30")
        .item("BGV2C.wav")
        .track("BGVs 31")
        .item("BGV2C.wav")
        .track("BGVs 32")
        .item("BGV2C.wav")
        .track("BGVs 33")
        .item("BGV2D.wav")
        .track("BGVs 34")
        .item("BGV2D.wav")
        .track("BGVs 35")
        .item("BGV2D.wav")
        .track("BGVs 36")
        .item("BGV2D.wav")
        // BGV3
        .track("BGVs 37")
        .item("BGV3_SUM.wav")
        .track("BGVs 38")
        .item("BGV3_SUM.wav")
        .track("BGVs 39")
        .item("BGV3A.wav")
        .track("BGVs 40")
        .item("BGV3A.wav")
        .track("BGVs 41")
        .item("BGV3A.wav")
        .track("BGVs 42")
        .item("BGV3A.wav")
        .track("BGVs 43")
        .item("BGV3B.wav")
        .track("BGVs 44")
        .item("BGV3B.wav")
        .track("BGVs 45")
        .item("BGV3B.wav")
        .track("BGVs 46")
        .item("BGV3B.wav")
        .track("BGVs 47")
        .item("BGV3C.wav")
        .track("BGVs 48")
        .item("BGV3C.wav")
        .track("BGVs 49")
        .item("BGV3C.wav")
        .track("BGVs 50")
        .item("BGV3C.wav")
        .track("BGVs 51")
        .item("BGV3D.wav")
        .track("BGVs 52")
        .item("BGV3D.wav")
        .track("BGVs 53")
        .item("BGV3D.wav")
        .track("BGVs 54")
        .item("BGV3D.wav")
        // BGV4 (first two BGV4A entries render without a sequence number)
        .track("BGVs 55")
        .item("BGV4_SUM.wav")
        .track("BGVs 56")
        .item("BGV4_SUM.wav")
        .track("BGVs")
        .item("BGV4A.wav")
        .track("BGVs")
        .item("BGV4A.wav")
        .track("BGVs 59")
        .item("BGV4A.wav")
        .track("BGVs 60")
        .item("BGV4A.wav")
        .track("BGVs 61")
        .item("BGV4B.wav")
        .track("BGVs 62")
        .item("BGV4B.wav")
        .track("BGVs 63")
        .item("BGV4B.wav")
        .track("BGVs 64")
        .item("BGV4B.wav")
        .track("BGVs 65")
        .item("BGV4C.wav")
        .track("BGVs 66")
        .item("BGV4C.wav")
        .track("BGVs 67")
        .item("BGV4C.wav")
        .track("BGVs 68")
        .item("BGV4C.wav")
        .track("BGVs 69")
        .item("BGV4D.wav")
        .track("BGVs 70")
        .item("BGV4D.wav")
        .track("BGVs 71")
        .item("BGV4D.wav")
        .track("BGVs 72")
        .item("BGV4D.wav")
        .end();

    let lead = TrackGroup::folder("Lead").group(kim).group(steve).end();

    let vocals = TrackGroup::folder("Vocals").group(lead).group(bgvs).end();

    // --- Reference ---
    let reference = TrackGroup::folder("Reference")
        .track("Crystalizer_ 1")
        .item("Crystalizer_Print_1.wav")
        .track("Crystalizer_ 2")
        .item("Crystalizer_Print.wav")
        .track("Crystalizer_ 3")
        .item("Crystalizer_Print.wav")
        .end();

    // --- Unsorted ---
    // "Mars Kim vx" and "Mars_PLAP" have no recognised instrument keywords.
    let unsorted = TrackGroup::folder("Unsorted")
        .track("Mars Kim Vx")
        .item("Mars Kim vx.wav")
        .track("Mars_PLAP")
        .item("Mars_PLAP.wav")
        .end();

    // ============================================================================
    // Compose final structure
    // ============================================================================

    let expected = TrackStructureBuilder::new()
        .group(bass)
        .group(keys)
        .group(vocals)
        .group(reference)
        .group(unsorted)
        .build();

    assert_tracks_equal(&tracks, &expected)?;

    Ok(())
}
