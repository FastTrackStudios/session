//! Visibility classification tests
//!
//! Verifies that every track in a multitrack session is correctly classified
//! into its top-level visibility group (Drums, Bass, Guitars, etc.) when
//! walking the organized TrackHierarchy using folder-depth propagation.
//!
//! This mirrors the algorithm in `visibility.rs::rebuild_cache()` which
//! propagates group membership from parent folders to all descendants.

use daw_proto::{FolderDepthChange, TrackHierarchy};
use dynamic_template::*;
use std::collections::HashMap;

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

/// Classification results: both item-level and track-level group mappings.
struct Classification {
    /// Original input name → top-level group (e.g., "Kick In" → "Drums")
    items: HashMap<String, String>,
    /// Track display name → top-level group (e.g., "Left" → "Guitars")
    /// Note: track names can repeat across groups (e.g., "Left" in Guitars and "Left" in
    /// another group), so this stores ALL (name, group) pairs as a Vec for lookup.
    tracks: Vec<(String, String)>,
}

/// Walk a TrackHierarchy and classify each track into its top-level visibility group.
///
/// This replicates the folder-depth propagation algorithm from visibility.rs:
/// - Folder tracks that start at depth 0 define a top-level group
/// - All descendants inherit that group via a stack
/// - Tracks that close folder levels pop from the stack
fn classify_tracks(hierarchy: &TrackHierarchy) -> Classification {
    let mut item_to_group: HashMap<String, String> = HashMap::new();
    let mut track_entries: Vec<(String, String)> = Vec::new();

    // Stack of group names — group_stack[0] is always the top-level group
    let mut group_stack: Vec<String> = Vec::new();

    for track in &hierarchy.tracks {
        // The top-level group is the first entry on the stack (if any),
        // or this track's own name if it's a top-level folder
        let current_group = if group_stack.is_empty() {
            // This track IS a top-level entry
            track.name.clone()
        } else {
            group_stack[0].clone()
        };

        // Record this track's group assignment
        track_entries.push((track.name.clone(), current_group.clone()));

        // Map all items on this track to the group
        for item in &track.items {
            item_to_group.insert(item.clone(), current_group.clone());
        }

        // Update the folder stack
        if track.folder_depth_change == FolderDepthChange::FolderStart {
            group_stack.push(track.name.clone());
        }
        if let FolderDepthChange::ClosesLevels(n) = track.folder_depth_change {
            let levels = n.unsigned_abs() as usize;
            for _ in 0..levels {
                group_stack.pop();
            }
        }
    }

    Classification {
        items: item_to_group,
        tracks: track_entries,
    }
}

// =============================================================================
// Marc Martel — Don't Stop Me Now
// =============================================================================
// Groups: Drums (12 items), Percussion (1), Bass (1), Guitars (4), Keys (1),
//         Vocals (7), SFX (3)

#[test]
fn marc_martel_visibility_groups() -> Result<()> {
    let items = vec![
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
    ];

    let config = default_config();
    let tracks = items.clone().organize_into_tracks(&config, None)?;
    let classification = classify_tracks(&tracks);
    let groups = &classification.items;

    // --- Drums: all kick, snare, tom, cymbal, and room tracks ---
    assert_eq!(groups.get("Kick In").unwrap(), "Drums", "Kick In → Drums");
    assert_eq!(groups.get("Kick Out").unwrap(), "Drums", "Kick Out → Drums");
    assert_eq!(
        groups.get("Kick Sample").unwrap(),
        "Drums",
        "Kick Sample → Drums"
    );
    assert_eq!(
        groups.get("Snare Top").unwrap(),
        "Drums",
        "Snare Top → Drums"
    );
    assert_eq!(
        groups.get("Snare Bottom").unwrap(),
        "Drums",
        "Snare Bottom → Drums"
    );
    assert_eq!(
        groups.get("Snare Sample").unwrap(),
        "Drums",
        "Snare Sample → Drums"
    );
    assert_eq!(
        groups.get("Snare Sample Two").unwrap(),
        "Drums",
        "Snare Sample Two → Drums"
    );
    assert_eq!(groups.get("Tom1").unwrap(), "Drums", "Tom1 → Drums");
    assert_eq!(groups.get("Tom2").unwrap(), "Drums", "Tom2 → Drums");
    assert_eq!(groups.get("HighHat").unwrap(), "Drums", "HighHat → Drums");
    assert_eq!(groups.get("OH").unwrap(), "Drums", "OH → Drums");
    assert_eq!(groups.get("Rooms").unwrap(), "Drums", "Rooms → Drums");

    // --- Percussion ---
    assert_eq!(
        groups.get("Percussion").unwrap(),
        "Percussion",
        "Percussion → Percussion"
    );

    // --- Bass ---
    assert_eq!(groups.get("Bass DI").unwrap(), "Bass", "Bass DI → Bass");

    // --- Guitars: all lead guitar tracks (clean and amplitube) ---
    assert_eq!(
        groups.get("Lead Guitar Amplitube Left").unwrap(),
        "Guitars",
        "Lead Guitar Amplitube Left → Guitars"
    );
    assert_eq!(
        groups.get("Lead Guitar Amplitube Right").unwrap(),
        "Guitars",
        "Lead Guitar Amplitube Right → Guitars"
    );
    assert_eq!(
        groups.get("Lead Guitar Clean DI Left").unwrap(),
        "Guitars",
        "Lead Guitar Clean DI Left → Guitars"
    );
    assert_eq!(
        groups.get("Lead Guitar Clean DI Right").unwrap(),
        "Guitars",
        "Lead Guitar Clean DI Right → Guitars"
    );

    // --- Keys ---
    assert_eq!(groups.get("Piano").unwrap(), "Keys", "Piano → Keys");

    // --- Vocals: lead vocal, effects, and BGVs ---
    assert_eq!(groups.get("Vocal").unwrap(), "Vocals", "Vocal → Vocals");
    assert_eq!(
        groups.get("Vocal.Eko.Plate").unwrap(),
        "Vocals",
        "Vocal.Eko.Plate → Vocals"
    );
    assert_eq!(
        groups.get("Vocal.Magic").unwrap(),
        "Vocals",
        "Vocal.Magic → Vocals"
    );
    assert_eq!(groups.get("BGV1").unwrap(), "Vocals", "BGV1 → Vocals");
    assert_eq!(groups.get("BGV2").unwrap(), "Vocals", "BGV2 → Vocals");
    assert_eq!(groups.get("BGV3").unwrap(), "Vocals", "BGV3 → Vocals");
    assert_eq!(groups.get("BGV4").unwrap(), "Vocals", "BGV4 → Vocals");

    // --- SFX: H3000 harmonizer effects ---
    assert_eq!(groups.get("H3000.One").unwrap(), "SFX", "H3000.One → SFX");
    assert_eq!(groups.get("H3000.Two").unwrap(), "SFX", "H3000.Two → SFX");
    assert_eq!(
        groups.get("H3000.Three").unwrap(),
        "SFX",
        "H3000.Three → SFX"
    );

    // Verify all 29 items are classified
    assert_eq!(
        groups.len(),
        items.len(),
        "Every input item should be classified"
    );

    Ok(())
}

/// Verify that ALL tracks in the hierarchy — including intermediate folder tracks
/// and leaf tracks with generic names like "Left", "Right", "SUM", "Top" — inherit
/// the correct top-level group through folder-depth propagation.
///
/// This is the critical test for the visibility manager: in REAPER, these tracks
/// exist as real tracks with names that don't match any monarchy pattern. They
/// must inherit their group from the parent folder chain.
#[test]
fn marc_martel_all_tracks_inherit_group() -> Result<()> {
    let items = vec![
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
    ];

    let config = default_config();
    let tracks = items.organize_into_tracks(&config, None)?;
    let classification = classify_tracks(&tracks);

    // Every track in the hierarchy (folders, subfolders, and leaves) must belong
    // to a top-level group. This verifies folder-depth propagation works for
    // tracks whose display names don't match any monarchy pattern.
    let known_groups = [
        "Drums",
        "Percussion",
        "Bass",
        "Guitars",
        "Keys",
        "Synths",
        "Horns",
        "Harmonica",
        "Vocals",
        "Choir",
        "Orchestra",
        "SFX",
        "Guide",
        "Reference",
    ];

    for (track_name, group) in &classification.tracks {
        assert!(
            known_groups.contains(&group.as_str()),
            "Track '{}' classified as '{}' which is not a known visibility group",
            track_name,
            group
        );
    }

    // Verify specific intermediate/leaf track names that DON'T match monarchy patterns
    // but MUST inherit from their parent folder:

    // Drums substructure: folder and leaf tracks
    let drums_tracks = [
        "Drums", "Kick", "SUM", "In", "Out", "Snare", "Top", "Bottom", "Trig", "Toms", "T1", "T2",
        "Cymbals", "OH", "Hi Hat", "Rooms",
    ];
    for name in &drums_tracks {
        let found = classification
            .tracks
            .iter()
            .find(|(n, g)| n == name && g == "Drums");
        assert!(
            found.is_some(),
            "Track '{}' should be classified as Drums (via folder inheritance)",
            name
        );
    }

    // Guitars substructure
    let guitar_tracks = ["Guitars", "Clean", "Lead", "Left", "Right"];
    for name in &guitar_tracks {
        let found = classification
            .tracks
            .iter()
            .any(|(n, g)| n == name && g == "Guitars");
        assert!(
            found,
            "Track '{}' should be classified as Guitars (via folder inheritance)",
            name
        );
    }

    // Vocals substructure
    let vocal_tracks = ["Vocals", "Lead", "BGVs"];
    for name in &vocal_tracks {
        let found = classification
            .tracks
            .iter()
            .any(|(n, g)| n == name && g == "Vocals");
        assert!(
            found,
            "Track '{}' should be classified as Vocals (via folder inheritance)",
            name
        );
    }

    // SFX tracks
    let sfx_tracks = ["SFX", "H3000.One", "H3000.Two", "H3000.Three"];
    for name in &sfx_tracks {
        let found = classification
            .tracks
            .iter()
            .any(|(n, g)| n == name && g == "SFX");
        assert!(
            found,
            "Track '{}' should be classified as SFX (via folder inheritance)",
            name
        );
    }

    Ok(())
}

/// Verify group counts: the number of items per group should match expectations.
#[test]
fn marc_martel_group_counts() -> Result<()> {
    let items = vec![
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
    ];

    let config = default_config();
    let tracks = items.clone().organize_into_tracks(&config, None)?;
    let classification = classify_tracks(&tracks);
    let groups = &classification.items;

    // Count items per group
    let mut counts: HashMap<String, usize> = HashMap::new();
    for group in groups.values() {
        *counts.entry(group.clone()).or_default() += 1;
    }

    assert_eq!(counts.get("Drums"), Some(&12), "Drums should have 12 items");
    assert_eq!(
        counts.get("Percussion"),
        Some(&1),
        "Percussion should have 1 item"
    );
    assert_eq!(counts.get("Bass"), Some(&1), "Bass should have 1 item");
    assert_eq!(
        counts.get("Guitars"),
        Some(&4),
        "Guitars should have 4 items"
    );
    assert_eq!(counts.get("Keys"), Some(&1), "Keys should have 1 item");
    assert_eq!(counts.get("Vocals"), Some(&7), "Vocals should have 7 items");
    assert_eq!(counts.get("SFX"), Some(&3), "SFX should have 3 items");

    // Total should equal input count
    let total: usize = counts.values().sum();
    assert_eq!(total, 29, "Total classified items should be 29");

    Ok(())
}

/// Verify that no items are left unclassified (every item maps to a known group).
#[test]
fn marc_martel_no_unclassified() -> Result<()> {
    let items = vec![
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
    ];

    let known_groups = [
        "Drums",
        "Percussion",
        "Bass",
        "Guitars",
        "Keys",
        "Synths",
        "Horns",
        "Harmonica",
        "Vocals",
        "Choir",
        "Orchestra",
        "SFX",
        "Guide",
        "Reference",
    ];

    let config = default_config();
    let tracks = items.clone().organize_into_tracks(&config, None)?;
    let classification = classify_tracks(&tracks);
    let groups = &classification.items;

    for item in &items {
        let group = groups
            .get(*item)
            .unwrap_or_else(|| panic!("Item '{}' should be classified", item));
        assert!(
            known_groups.contains(&group.as_str()),
            "Item '{}' classified as '{}' which is not a known visibility group",
            item,
            group
        );
    }

    Ok(())
}
