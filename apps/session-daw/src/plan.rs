//! Which tracks a scene shows, and how large — as this window sees it.
//!
//! The engine is [`dynamic_template::scenes`]: a table of `Scene` values
//! resolved by one function into a row list. This module is the adapter.
//! It hands that engine the window's track list plus the taxonomy the
//! template wrote into the project, and turns the rows that come back
//! into the `(Track, depth)` pairs the panels record.
//!
//! # Why sizes come back as classes
//!
//! A rule says [`Size::Working`], not 133 pixels. The same scene is
//! applied to two panels and to screens from 1080p to a 5120 ultrawide,
//! so a rule that stated pixels would need a template per monitor.
//! Turning a class into a number is the SURFACE's job, and the table it
//! reads is `scenes::TABLES` — [`mixer_width`] and [`row_height`] are
//! this window's two doors onto it.

use std::collections::HashMap;

use daw_proto::Track;
use dynamic_template::golden_session::{TrackExt, read_kinds};
use dynamic_template::scenes::{self, Fold, Scene, Size};

pub use dynamic_template::scenes::Surface;

/// The taxonomy the template wrote into the project, by GUID.
///
/// A scene's selectors match what a track IS — `Process`, `Verb`, a
/// `Kick` piece — and what it is comes from the ext-state the template
/// writes when it creates the track, not from its name. This is that,
/// read once from the project file the window opened.
///
/// Empty is a valid answer: a session the template never touched has no
/// taxonomy, every selector that asks for a kind matches nothing, and a
/// scene falls back to its default rather than to a wrong guess.
#[derive(Clone, Debug, Default)]
pub struct Kinds {
    by_guid: HashMap<String, TrackExt>,
}

impl Kinds {
    /// Read a project file's ext-state.
    #[must_use]
    pub fn read(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        Self {
            by_guid: read_kinds(&text)
                .into_iter()
                .map(|ext| (ext.guid.clone(), ext))
                .collect(),
        }
    }

    /// How many tracks it knows about.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    /// Whether it knows about none, which is the untouched-session case.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }
}

/// Apply a scene to a track list.
///
/// Returns the rows that survive it, each carrying the size the scene
/// gives it — written into `Track::width` or `Track::height`, which is
/// where both panels already read a size from. Nothing downstream learns
/// that a scene exists.
///
/// `panel` is the surface's own extent in the axis the focus width comes
/// off: the mixer's height, or the arrangement's.
#[must_use]
pub fn apply_scene(
    tracks: &[(Track, u32)],
    kinds: &Kinds,
    scene: &Scene,
    surface: Surface,
    mode: Option<&str>,
    panel: f64,
) -> Vec<(Track, u32)> {
    let facts = scenes::from_tracks(tracks, &kinds.by_guid);
    let rows = scenes::resolve(scene, &facts, surface, mode);
    let by_guid: HashMap<&str, &(Track, u32)> = tracks
        .iter()
        .map(|row| (row.0.guid.as_str(), row))
        .collect();
    rows.iter()
        .filter_map(|row| {
            let guid = row.guid()?;
            let (track, _) = by_guid.get(guid)?;
            let mut track = (*track).clone();
            match surface {
                Surface::Mixer => {
                    track.width = Some(pixels(mixer_width(row.size, panel)));
                    // A focused strip is the selected one: that is what
                    // the mixer opens to the focus width and reads the
                    // rack of.
                    track.selected = row.size == Size::Focus;
                }
                Surface::Arrange => track.height = Some(pixels(row_height(row.size))),
            }
            Some((track, row.depth))
        })
        .collect()
}

/// The folders a scene folds, as GUIDs — what the window's own folder
/// state is set to when the scene is recalled, so the strips' fold icons
/// agree with the scene and a click on one carries on from where the
/// scene left it.
#[must_use]
pub fn collapsed_by(
    tracks: &[(Track, u32)],
    kinds: &Kinds,
    scene: &Scene,
    mode: Option<&str>,
) -> Vec<String> {
    let facts = scenes::from_tracks(tracks, &kinds.by_guid);
    scenes::resolve(scene, &facts, Surface::Mixer, mode)
        .iter()
        .filter(|row| row.fold == Fold::Collapsed)
        .filter_map(|row| row.guid())
        // Only the folders the window can actually fold.
        .filter(|guid| {
            tracks
                .iter()
                .any(|(track, _)| track.guid == *guid && track.is_folder)
        })
        .map(str::to_owned)
        .collect()
}

/// Which instrument a track belongs to, for follow-mode.
///
/// The top of its taxonomy path — `Drums/Drum Kit/Kick` is the drums —
/// normalised to the word a scene's `instrument` uses. `None` for a
/// track the template never placed, which is what makes follow-mode
/// fall back to the shown scene's instrument rather than to a guess.
#[must_use]
pub fn instrument_of(tracks: &[(Track, u32)], kinds: &Kinds, guid: &str) -> Option<String> {
    let facts = scenes::from_tracks(tracks, &kinds.by_guid);
    let top = facts
        .iter()
        .find(|fact| fact.guid == guid)?
        .path
        .first()?
        .name
        .to_ascii_lowercase();
    // The template's group names are plural collections; a scene names
    // the instrument. Only the two that differ need saying.
    Some(match top.as_str() {
        "guitars" => "guitar".to_owned(),
        "vocals" => "vocal".to_owned(),
        _ => top,
    })
}

/// How wide a size class is in the mixer, for a panel `height` tall.
#[must_use]
pub fn mixer_width(size: Size, height: f64) -> f64 {
    scenes::TABLES.width(size, height)
}

/// And how tall one is in the arrangement.
#[must_use]
pub fn row_height(size: Size) -> f64 {
    scenes::TABLES.height(size)
}

/// A pixel count, for the `u32` the track model stores.
fn pixels(value: f64) -> u32 {
    crate::num::index(value.max(1.0))
        .try_into()
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(guid: &str, name: &str, index: u32, is_folder: bool, depth: u32) -> (Track, u32) {
        (
            Track {
                guid: guid.to_owned(),
                name: name.to_owned(),
                index,
                is_folder,
                ..Track::default()
            },
            depth,
        )
    }

    /// The golden session's own tracks, with the taxonomy the template
    /// wrote — the same input the fixtures are resolved from, reached
    /// the way the window reaches it.
    fn golden() -> (Vec<(Track, u32)>, Kinds) {
        let dir = dynamic_template::golden_session::fixtures_dir();
        let kinds = Kinds::read(&dir.join("template.rpp"));
        let tracks = dynamic_template::golden_session::rpp::flatten(
            &dynamic_template::golden_session::maximal(),
        )
        .iter()
        .enumerate()
        .map(|(i, flat)| {
            track(
                &flat.guid,
                &flat.name,
                u32::try_from(i).unwrap_or(u32::MAX),
                flat.is_folder,
                flat.depth,
            )
        })
        .collect();
        (tracks, kinds)
    }

    /// The window reads the taxonomy out of the project file it opened,
    /// and it finds one — the whole scene table depends on it.
    #[test]
    fn the_window_reads_the_templates_taxonomy() {
        let (_, kinds) = golden();
        assert!(kinds.len() > 100, "{} tracks carry a kind", kinds.len());
        assert!(!kinds.is_empty());
    }

    /// A session the template never touched has no taxonomy, and a
    /// scene over it falls back to its default rather than to a guess.
    #[test]
    fn a_session_with_no_taxonomy_still_resolves() {
        let tracks = vec![
            track("a", "Kick", 0, true, 0),
            track("b", "Kick In", 1, false, 1),
        ];
        let scene = scenes::scene("drum-mixing").expect("the scene");
        let rows = apply_scene(
            &tracks,
            &Kinds::default(),
            scene,
            Surface::Mixer,
            None,
            1440.0,
        );
        assert_eq!(
            rows.len(),
            2,
            "nothing is hidden by a rule that matched nothing"
        );
        assert_eq!(rows[0].0.width, Some(86), "the folder takes the default");
        assert_eq!(rows[1].0.width, Some(30), "the leaf takes the leaf rule");
    }

    /// Mix opens the pieces and rails the mics — the layout the scene
    /// exists to produce, through the window's own adapter.
    #[test]
    fn drum_mixing_opens_the_pieces_and_rails_the_mics() {
        let (tracks, kinds) = golden();
        let scene = scenes::scene("drum-mixing").expect("the scene");
        let rows = apply_scene(&tracks, &kinds, scene, Surface::Mixer, None, 1440.0);
        let by = |name: &str| {
            rows.iter()
                .find(|(t, _)| t.name == name)
                .unwrap_or_else(|| panic!("no {name}"))
                .0
                .width
        };
        assert_eq!(by("Kick"), Some(133), "the piece is the instrument");
        assert_eq!(by("In"), Some(30), "and its mics are a rail");
    }

    /// The arrangement sizes by HEIGHT and the mixer by width, and
    /// applying one surface's scene must not write the other's field.
    #[test]
    fn a_surface_only_sets_its_own_axis() {
        let (tracks, kinds) = golden();
        let scene = scenes::scene("drum-mixing").expect("the scene");
        let mixer = apply_scene(&tracks, &kinds, scene, Surface::Mixer, None, 1440.0);
        assert!(mixer.iter().all(|(t, _)| t.height.is_none()));
        let arrange = apply_scene(&tracks, &kinds, scene, Surface::Arrange, None, 1440.0);
        assert!(arrange.iter().all(|(t, _)| t.width.is_none()));
        assert!(arrange.iter().all(|(t, _)| t.height.is_some()));
    }

    /// The overview folds the pieces shut, and the window's folder state
    /// is told which folders those were so its fold icons agree.
    #[test]
    fn the_overview_hands_the_window_its_folds() {
        let (tracks, kinds) = golden();
        let scene = scenes::scene("drum-overview").expect("the scene");
        let folded = collapsed_by(&tracks, &kinds, scene, None);
        let names: Vec<&str> = folded
            .iter()
            .filter_map(|guid| {
                tracks
                    .iter()
                    .find(|(t, _)| t.guid == *guid)
                    .map(|(t, _)| t.name.as_str())
            })
            .collect();
        assert_eq!(names, ["Kick", "Snare", "Toms", "Cymbals", "Rooms"]);
    }

    /// The pixel tables are the scenes module's, not this window's: a
    /// class means the same number wherever it is applied.
    #[test]
    fn the_window_reads_the_scene_modules_pixel_tables() {
        let same = |a: f64, b: f64| (a - b).abs() < f64::EPSILON;
        assert!(same(
            mixer_width(Size::Minimum, 1440.0),
            crate::layout::STRIP_NARROW
        ));
        assert!(same(
            mixer_width(Size::Compact, 1440.0),
            crate::layout::STRIP_WIDE
        ));
        assert!(same(
            mixer_width(Size::Working, 1440.0),
            crate::tone::WORKING
        ));
        assert!(same(mixer_width(Size::Focus, 1440.0), 618.0));
        assert!(same(row_height(Size::Minimum), crate::layout::NAME_LEGIBLE));
        assert!(same(row_height(Size::Compact), crate::layout::CONTROL_ROW));
    }
}

/// Which of the window's live tracks each panel row is showing.
///
/// A preset can hide a track, so the mixer's third strip is not
/// necessarily the session's third track — and the live values (mute,
/// fader, meter) are kept in the SESSION's order, because that is the
/// order the engine's events and its meter frames are indexed by.
///
/// This is the one place those two orders meet. Without it the overlay
/// reads a neighbour's fader whenever a preset hides anything, which is
/// a bug that looks like the mixer being subtly, consistently wrong
/// rather than like anything crashing.
#[derive(Clone, Debug, Default)]
pub struct Rows {
    /// `rows[panel] = index into the window's tracks`.
    rows: Vec<usize>,
}

impl Rows {
    /// Build the map by matching GUIDs.
    ///
    /// GUIDs rather than positions, because that is the only identity a
    /// track has that survives being filtered, reordered or renamed —
    /// and a map built from positions would be the very assumption this
    /// type exists to remove.
    #[must_use]
    pub fn of(panel: &[(Track, u32)], tracks: &[Track]) -> Self {
        let mut index: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::with_capacity(tracks.len());
        for (i, track) in tracks.iter().enumerate() {
            index.entry(track.guid.as_str()).or_insert(i);
        }
        Self {
            rows: panel
                .iter()
                .map(|(track, _)| {
                    index
                        .get(track.guid.as_str())
                        .copied()
                        .unwrap_or(usize::MAX)
                })
                .collect(),
        }
    }

    /// The live track a panel row is showing.
    ///
    /// `None` for a row whose track has gone — which happens for one
    /// frame between a project reload and the re-record that follows
    /// it, and is a row to skip rather than a reason to panic.
    #[must_use]
    pub fn live<'a>(&self, tracks: &'a [Track], row: usize) -> Option<&'a Track> {
        tracks.get(*self.rows.get(row)?)
    }

    /// Whether any row's track is selected — what decides whether the
    /// unselected ones are dimmed at all.
    #[must_use]
    pub fn any_selected(&self, tracks: &[Track]) -> bool {
        self.rows
            .iter()
            .filter_map(|i| tracks.get(*i))
            .any(|t| t.selected)
    }

    /// The same, as an index — for the paths that need to write.
    #[must_use]
    pub fn index(&self, row: usize) -> Option<usize> {
        self.rows.get(row).copied().filter(|i| *i != usize::MAX)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;

    fn tracks() -> Vec<Track> {
        ["a", "b", "c", "d"]
            .into_iter()
            .map(|guid| Track {
                guid: guid.to_owned(),
                ..Track::default()
            })
            .collect()
    }

    /// The case the type exists for: a preset hid a track, and the
    /// panel's second row is the session's third.
    #[test]
    fn a_filtered_panel_still_finds_its_own_tracks() {
        let all = tracks();
        let panel: Vec<(Track, u32)> = ["a", "c"]
            .into_iter()
            .map(|g| {
                (
                    all.iter().find(|t| t.guid == g).expect("a track").clone(),
                    0,
                )
            })
            .collect();
        let map = Rows::of(&panel, &all);
        assert_eq!(map.index(0), Some(0));
        assert_eq!(map.index(1), Some(2), "row 1 is the session's track 2");
        assert_eq!(map.live(&all, 1).map(|t| t.guid.as_str()), Some("c"));
    }

    /// A row whose track has gone is skipped, not a panic — there is
    /// one frame between a reload and the re-record where that is true.
    #[test]
    fn a_row_with_no_track_is_none() {
        let all = tracks();
        let panel = vec![(
            Track {
                guid: "gone".to_owned(),
                ..Track::default()
            },
            0,
        )];
        let map = Rows::of(&panel, &all);
        assert_eq!(map.index(0), None);
        assert!(map.live(&all, 0).is_none());
        assert!(map.live(&all, 99).is_none());
    }

    /// An unfiltered panel is the identity, which is what makes this
    /// free in the ordinary case.
    #[test]
    fn an_unfiltered_panel_maps_to_itself() {
        let all = tracks();
        let panel: Vec<(Track, u32)> = all.iter().cloned().map(|t| (t, 0)).collect();
        let map = Rows::of(&panel, &all);
        assert_eq!(map.len(), all.len());
        for i in 0..all.len() {
            assert_eq!(map.index(i), Some(i));
        }
    }
}
