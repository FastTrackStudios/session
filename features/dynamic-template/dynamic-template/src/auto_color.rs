//! Auto-color: classify track names by instrument group and map to colors.
//!
//! This module provides the core classification-to-color pipeline:
//! track names → monarchy_sort() → Structure → color_for_path() → color map.
//!
//! It also provides DAW-agnostic functions to apply and clear colors on tracks
//! via the `Tracks` trait. These work with any backend (daw-standalone for
//! testing, daw-reaper for production).

use crate::colors;
use color_palette::Color;
use daw_proto::ProjectContext;
use daw_proto::track::{Track, TrackRef, Tracks};
use monarchy::Metadata;
use std::collections::HashMap;

/// Classify track names via monarchy sort and return a color mapping.
///
/// Each track name is classified into an instrument group hierarchy
/// (e.g. "kick_in.wav" → Drums/Kick/In), then the group path is looked up
/// in the color palette to produce a `Color`.
///
/// Returns a map from original track name → `Color`. Tracks that don't match
/// any known group (or whose group has no color) are omitted from the map.
pub fn classify_and_color(track_names: Vec<String>) -> HashMap<String, Color> {
    let config = crate::default_config();
    let structure = match monarchy::monarchy_sort(track_names, &config) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    let mut color_map = HashMap::new();
    collect_colors_from_structure(&structure, &[], &mut color_map);
    color_map
}

/// Walk a Structure tree, mapping item original names → their group's color.
fn collect_colors_from_structure<M: Metadata>(
    structure: &monarchy::Structure<M>,
    parent_path: &[&str],
    color_map: &mut HashMap<String, Color>,
) {
    // Build path for this node
    let mut current_path: Vec<&str> = parent_path.to_vec();
    if !structure.name.is_empty() && structure.name != "root" {
        current_path.push(&structure.name);
    }

    // Look up color for this group path
    let color = colors::color_for_path(&current_path);

    // Assign color to items at this level
    if let Some(c) = color {
        for item in &structure.items {
            color_map.insert(item.original.clone(), c);
        }
    }

    // Recurse into children — children may have more specific colors
    for child in &structure.children {
        collect_colors_from_structure(child, &current_path, color_map);
    }
}

// ============================================================================
// Track Color Application (via Tracks)
// ============================================================================

/// Classify all tracks in a project and apply colors.
///
/// Collects track names, runs them through `classify_and_color()` to get a
/// name→Color mapping, then applies each color via `Tracks::set_color()`.
///
/// Returns the number of tracks that were colored.
pub fn apply_colors(service: &impl Tracks, project: ProjectContext) -> u32 {
    let tracks = service.all(project.clone());
    if tracks.is_empty() {
        return 0;
    }

    let names: Vec<String> = tracks.iter().map(|t| t.name.clone()).collect();
    let color_map = classify_and_color(names);

    apply_color_map(service, project, &tracks, &color_map)
}

/// Apply a pre-computed color map to tracks.
///
/// For each track, looks up its name in the color map and sets the color
/// via `Tracks::set_color()`.
///
/// Returns the number of tracks that were colored.
pub fn apply_color_map(
    service: &impl Tracks,
    project: ProjectContext,
    tracks: &[Track],
    color_map: &HashMap<String, Color>,
) -> u32 {
    let mut colored = 0u32;

    for track in tracks {
        if let Some(color) = color_map.get(&track.name) {
            let rgb = color.to_hex();
            if service
                .set_color(project.clone(), TrackRef::Guid(track.guid.clone()), rgb)
                .is_ok()
            {
                colored += 1;
            }
        }
    }

    colored
}

/// Clear colors from all tracks in a project (reset to default).
///
/// Sets color to 0 for every track, which the DAW interprets as "default/no custom color".
///
/// Returns the number of tracks cleared.
pub fn clear_colors(service: &impl Tracks, project: ProjectContext) -> u32 {
    let tracks = service.all(project.clone());
    let mut cleared = 0u32;

    for track in &tracks {
        if service
            .set_color(project.clone(), TrackRef::Guid(track.guid.clone()), 0)
            .is_ok()
        {
            cleared += 1;
        }
    }

    cleared
}
