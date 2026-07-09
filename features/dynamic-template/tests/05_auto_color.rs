//! Tests for auto-color: classification and track color application.
//!
//! Uses daw-standalone to verify that `apply_colors` and `clear_colors`
//! correctly set and reset track colors via the `TrackService` trait.

use daw_proto::{ProjectContext, TrackService};
use daw_standalone::StandaloneTrack;
use dynamic_template::auto_color;
use dynamic_template::colors;

fn current() -> ProjectContext {
    ProjectContext::Current
}

/// Marc Martel "Don't Stop Me Now" track list — real-world multitrack input
fn marc_martel_track_names() -> Vec<&'static str> {
    vec![
        "Kick In",
        "Kick Out",
        "Kick Sample",
        "Snare Top",
        "Snare Bottom",
        "Snare Sample",
        "Snare Sample Two",
        "Tom1",
        "Tom2",
        "HighHat",
        "OH",
        "Rooms",
        "Percussion",
        "Bass DI",
        "Piano",
        "Lead Guitar Amplitube Left",
        "Lead Guitar Amplitube Right",
        "Lead Guitar Clean DI Left",
        "Lead Guitar Clean DI Right",
        "Vocal",
        "H3000.One",
        "H3000.Two",
        "H3000.Three",
        "Vocal.Eko.Plate",
        "Vocal.Magic",
        "BGV1",
        "BGV2",
        "BGV3",
        "BGV4",
    ]
}

/// Create a StandaloneTrack with the Marc Martel track names.
async fn setup_marc_martel() -> StandaloneTrack {
    let svc = StandaloneTrack::new();
    for name in marc_martel_track_names() {
        svc.add_track(name).await;
    }
    svc
}

// =============================================================================
// Classification tests
// =============================================================================

#[test]
fn classify_and_color_returns_color_map() {
    let names: Vec<String> = marc_martel_track_names()
        .into_iter()
        .map(String::from)
        .collect();
    let color_map = auto_color::classify_and_color(names);

    // Should classify most tracks — at least drums, bass, guitars, vocals
    assert!(
        color_map.len() >= 20,
        "Expected at least 20 classified tracks, got {}",
        color_map.len()
    );

    // Key tracks should be classified (have a color)
    for name in ["Kick In", "Snare Top", "Bass DI", "Piano", "Vocal", "BGV1"] {
        assert!(
            color_map.contains_key(name),
            "Track '{}' should be classified with a color",
            name
        );
    }

    // Verify specific group colors where they're unambiguous
    assert_eq!(
        color_map.get("Bass DI").map(|c| c.to_hex()),
        Some(colors::groups::BASS.to_hex()),
        "Bass DI should get bass group color"
    );
    assert_eq!(
        color_map.get("Piano").map(|c| c.to_hex()),
        Some(colors::keys::PIANO.to_hex()),
        "Piano should get piano color"
    );
}

#[test]
fn classify_empty_list_returns_empty() {
    let color_map = auto_color::classify_and_color(vec![]);
    assert!(color_map.is_empty());
}

// =============================================================================
// Apply colors tests
// =============================================================================

#[tokio::test]
async fn apply_colors_sets_track_colors() {
    let svc = setup_marc_martel().await;

    // All tracks should start with no color
    let tracks = svc.get_tracks(current()).await;
    for track in &tracks {
        // Skip the default tracks (Track 1, Track 2, Vocals, Drums)
        if track.name.starts_with("Track ") {
            continue;
        }
        assert_eq!(
            track.color, None,
            "Track '{}' should have no color initially",
            track.name
        );
    }

    // Apply auto-color
    let colored = auto_color::apply_colors(&svc, current()).await;
    assert!(colored > 0, "Should have colored at least some tracks");

    // Verify colors were actually set
    let tracks = svc.get_tracks(current()).await;
    let mut colored_count = 0;
    for track in &tracks {
        if track.color.is_some() {
            colored_count += 1;
        }
    }
    assert!(
        colored_count >= 20,
        "Expected at least 20 colored tracks, got {}",
        colored_count
    );
}

#[tokio::test]
async fn drums_get_red_family_colors() {
    let svc = setup_marc_martel().await;

    auto_color::apply_colors(&svc, current()).await;

    let tracks = svc.get_tracks(current()).await;

    // Check drum tracks have colors in the red family
    // Red family: hue around 0°/360°, amber/orange around 30-45°
    let drum_names = [
        "Kick In",
        "Kick Out",
        "Kick Sample",
        "Snare Top",
        "Snare Bottom",
    ];
    for name in drum_names {
        let track = tracks.iter().find(|t| t.name == name);
        assert!(track.is_some(), "Should find track '{}'", name);
        let track = track.unwrap();
        assert!(
            track.color.is_some(),
            "Drum track '{}' should have a color",
            name
        );
    }
}

#[tokio::test]
async fn guitars_get_sky_family_colors() {
    let svc = setup_marc_martel().await;

    auto_color::apply_colors(&svc, current()).await;

    let tracks = svc.get_tracks(current()).await;
    let guitar_names = [
        "Lead Guitar Amplitube Left",
        "Lead Guitar Amplitube Right",
        "Lead Guitar Clean DI Left",
        "Lead Guitar Clean DI Right",
    ];

    let sky_color = colors::groups::GUITARS.to_hex();
    for name in guitar_names {
        let track = tracks.iter().find(|t| t.name == name).unwrap();
        assert!(
            track.color.is_some(),
            "Guitar track '{}' should have a color",
            name
        );
        // Guitar tracks should get sky/guitars color family
        let color = track.color.unwrap();
        assert_eq!(
            color, sky_color,
            "Guitar track '{}' should get guitars color (sky-600 = 0x{:06X}), got 0x{:06X}",
            name, sky_color, color
        );
    }
}

// =============================================================================
// Clear colors tests
// =============================================================================

#[tokio::test]
async fn clear_colors_resets_all() {
    let svc = setup_marc_martel().await;

    // Apply colors first
    auto_color::apply_colors(&svc, current()).await;

    // Verify some tracks have colors
    let tracks = svc.get_tracks(current()).await;
    let has_colors = tracks.iter().any(|t| t.color.is_some());
    assert!(has_colors, "Should have colored tracks before clearing");

    // Clear all colors
    let cleared = auto_color::clear_colors(&svc, current()).await;
    assert!(cleared > 0, "Should have cleared some tracks");

    // Verify all colors are gone
    let tracks = svc.get_tracks(current()).await;
    for track in &tracks {
        assert_eq!(
            track.color, None,
            "Track '{}' should have no color after clearing",
            track.name
        );
    }
}

// =============================================================================
// apply_color_map tests
// =============================================================================

#[tokio::test]
async fn apply_color_map_uses_provided_colors() {
    let svc = setup_marc_martel().await;

    // Create a custom color map — just color "Vocal" and "Bass DI"
    let mut custom_map = std::collections::HashMap::new();
    custom_map.insert("Vocal".to_string(), color_palette::Color::hex(0xFF0000));
    custom_map.insert("Bass DI".to_string(), color_palette::Color::hex(0x00FF00));

    let tracks = svc.get_tracks(current()).await;
    let colored = auto_color::apply_color_map(&svc, current(), &tracks, &custom_map).await;
    assert_eq!(colored, 2, "Should color exactly 2 tracks");

    // Verify only those tracks got colored
    let tracks = svc.get_tracks(current()).await;
    let vocal = tracks.iter().find(|t| t.name == "Vocal").unwrap();
    assert_eq!(vocal.color, Some(0xFF0000));

    let bass = tracks.iter().find(|t| t.name == "Bass DI").unwrap();
    assert_eq!(bass.color, Some(0x00FF00));

    // Other tracks should remain uncolored
    let kick = tracks.iter().find(|t| t.name == "Kick In").unwrap();
    assert_eq!(
        kick.color, None,
        "Kick In should not be colored by custom map"
    );
}
