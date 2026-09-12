//! Which tracks a visual preset shows, and how large.
//!
//! A preset is not a list of tracks. It is a set of RULES resolved
//! against the session's own taxonomy — "every instrument bus wide
//! enough to work on, every microphone as a rail beside it" — which is
//! what lets the same preset mean the right thing in a five-track demo
//! and in a sixty-track kit, and survive a track being added.
//!
//! That engine already exists: [`dynamic_template::visibility_rules`]
//! is the one the REAPER-side template uses, and it resolves
//! `(tracks, config, mode) → Vec<TrackPlan>` with no REAPER calls in
//! it. This module is the adapter — it hands that engine the session
//! and turns the plans back into rows the panels can record.
//!
//! # Why sizes come back as classes
//!
//! A rule says [`Size::Working`], not 133 pixels. The same plan is
//! applied to two panels with different axes and to screens from 1080p
//! to a 5120 ultrawide, so a rule that stated pixels would need a
//! template per monitor. Turning a class into a number is the
//! SURFACE's job, and it is done here: [`mixer_width`] and
//! [`row_height`] are the two places it happens.

use daw_proto::Track;
use dynamic_template::visibility_rules::{self as rules, Size};

/// Which surface a plan is being applied to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    /// The arrangement's track panel: rows, so heights.
    Arrange,
    /// The mixer: strips, so widths.
    Mixer,
}

/// The preset a rail button recalls, and the rule set behind it.
///
/// The label is what the rail prints — rails are 38 pixels wide, so it
/// is three or four letters — and the slug is what
/// [`rules::mode_visibility_for`] answers to. They are paired here
/// rather than derived from each other because neither is a good name
/// for the other: "Over" is not a slug and "overview" does not fit.
pub const PRESETS: [(&str, &str); 3] = [("Mix", "mix"), ("Rec", "record"), ("Over", "overview")];

/// The slug for a rail label, if it names a preset.
#[must_use]
pub fn slug(label: &str) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, slug)| *slug)
}

/// Apply a preset to a track list.
///
/// Returns the tracks that survive it, each carrying the size the
/// preset gives it — written into `Track::width` and `Track::height`,
/// which is where both panels already read a size from. Nothing
/// downstream learns that a preset exists.
///
/// `panel` is the surface's own extent in the axis being sized: the
/// mixer's height (a focused strip's width comes off it — see
/// [`crate::settings::Settings::focus_width`]) or the arrangement's.
///
/// An unknown preset returns the tracks unchanged rather than an empty
/// panel. A window showing everything is a window that has not applied
/// a rule; a window showing nothing looks broken.
#[must_use]
pub fn apply(
    tracks: &[(Track, u32)],
    preset: &str,
    surface: Surface,
    settings: crate::settings::Settings,
    panel: f64,
) -> Vec<(Track, u32)> {
    let Some(mode) = rules::mode_visibility_for(preset) else {
        return tracks.to_vec();
    };
    let config = dynamic_template::default_config();
    let inputs: Vec<rules::TrackInput> = tracks
        .iter()
        .map(|(track, _)| rules::TrackInput {
            guid: track.guid.clone(),
            name: track.name.clone(),
            index: track.index,
            is_folder: track.is_folder,
        })
        .collect();
    let plans = rules::resolve(&inputs, &config, &mode);

    tracks
        .iter()
        .zip(&plans)
        .filter(|(_, plan)| match surface {
            Surface::Arrange => plan.arrange_show,
            Surface::Mixer => plan.mixer_show,
        })
        .map(|((track, depth), plan)| {
            let mut track = track.clone();
            match surface {
                Surface::Mixer => {
                    if let Some(size) = plan.mixer_width {
                        track.width = Some(pixels(mixer_width(size, settings, panel)));
                    }
                }
                Surface::Arrange => {
                    if let Some(size) = plan.arrange_height {
                        track.height = Some(pixels(row_height(size)));
                    }
                }
            }
            (track, *depth)
        })
        .collect()
}

/// How wide a size class is in the mixer.
///
/// The four fixed classes are the widths this panel already has names
/// for; only `Focus` depends on the display, and it depends on the
/// panel's HEIGHT rather than its width so that a wider screen holds
/// more focused tracks instead of fatter ones.
#[must_use]
pub fn mixer_width(size: Size, settings: crate::settings::Settings, panel_h: f64) -> f64 {
    match size {
        Size::Minimum => crate::layout::STRIP_NARROW,
        Size::Compact => crate::layout::STRIP_WIDE,
        Size::Normal => crate::tone::WORKING,
        // Wide enough that the rack's curves are readable, which is
        // what "working on this track" means in a mix pass.
        Size::Working => crate::tone::WORKING,
        Size::Focus => settings.focus_width(panel_h),
    }
}

/// And how tall one is in the arrangement.
///
/// No `Focus` case that differs from `Working`: a row's height is what
/// makes its waveform readable, and past the point where the controls
/// are all drawn at their authored size, more height buys a bigger
/// picture of the same thing rather than another control.
#[must_use]
pub fn row_height(size: Size) -> f64 {
    match size {
        // The band tier: a coloured line that says a track is there.
        Size::Minimum => crate::layout::NAME_LEGIBLE,
        Size::Compact => crate::layout::CONTROL_ROW,
        Size::Normal => crate::layout::CONTROL_ROW * 2.0,
        Size::Working | Size::Focus => crate::layout::CONTROL_ROW * 3.0,
    }
}

/// A pixel count, for the `u32` the track model stores.
fn pixels(value: f64) -> u32 {
    crate::num::index(value.max(1.0)).try_into().unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(guid: &str, name: &str, index: u32, is_folder: bool) -> (Track, u32) {
        (
            Track {
                guid: guid.to_owned(),
                name: name.to_owned(),
                index,
                is_folder,
                ..Track::default()
            },
            0,
        )
    }

    fn kit() -> Vec<(Track, u32)> {
        vec![
            track("kick", "Kick", 0, true),
            track("kick-in", "Kick In", 1, false),
            track("kick-out", "Kick Out", 2, false),
            track("snare", "Snare", 3, true),
            track("snare-top", "Snare Top", 4, false),
        ]
    }

    /// Mix gives the buses a working width and keeps the mics as a
    /// rail — the layout the whole preset exists to produce.
    #[test]
    fn mix_opens_the_buses_and_rails_the_mics() {
        let rows = apply(
            &kit(),
            "mix",
            Surface::Mixer,
            crate::settings::Settings::default(),
            1440.0,
        );
        assert_eq!(rows.len(), 5, "mix hides nothing");
        let by = |guid: &str| {
            rows.iter()
                .find(|(t, _)| t.guid == guid)
                .expect("the track")
                .0
                .width
        };
        assert_eq!(by("kick"), Some(133));
        assert_eq!(by("kick-in"), Some(30));
    }

    /// Overview drops the leaves entirely, which is the one preset that
    /// changes how many strips there are rather than how wide they are.
    #[test]
    fn overview_keeps_only_the_skeleton() {
        let rows = apply(
            &kit(),
            "overview",
            Surface::Mixer,
            crate::settings::Settings::default(),
            1440.0,
        );
        assert_eq!(rows.len(), 2, "two buses, no mics");
        assert!(rows.iter().all(|(t, _)| t.is_folder));
    }

    /// Record is the inverse of mix, and the assertion is the
    /// COMPARISON: a mic must be wider in record than in mix, or the
    /// preset has not done anything you would notice.
    #[test]
    fn record_is_wider_for_a_mic_than_mix_is() {
        let settings = crate::settings::Settings::default();
        let width = |preset: &str| {
            apply(&kit(), preset, Surface::Mixer, settings, 1440.0)
                .into_iter()
                .find(|(t, _)| t.guid == "kick-in")
                .expect("the mic")
                .0
                .width
        };
        assert!(width("record") > width("mix"));
    }

    /// A preset nobody defined leaves the session alone. Showing
    /// everything is a view; showing nothing is a bug that looks like a
    /// crash.
    #[test]
    fn an_unknown_preset_changes_nothing() {
        let rows = apply(
            &kit(),
            "nonesuch",
            Surface::Mixer,
            crate::settings::Settings::default(),
            1440.0,
        );
        assert_eq!(rows.len(), kit().len());
        assert!(rows.iter().all(|(t, _)| t.width.is_none()));
    }

    /// The labels the rail prints and the slugs the rules answer to
    /// stay paired — a rail button whose label has no slug is a button
    /// that does nothing.
    #[test]
    fn every_rail_label_names_a_real_rule_set() {
        for (label, _) in PRESETS {
            let slug = slug(label).expect("a slug for every label");
            assert!(
                rules::mode_visibility_for(slug).is_some(),
                "{label} -> {slug} has no rules"
            );
        }
        assert!(slug("nonesuch").is_none());
    }

    /// The arrangement sizes by HEIGHT and the mixer by width, and
    /// applying one surface's plan must not write the other's field.
    #[test]
    fn a_surface_only_sets_its_own_axis() {
        let settings = crate::settings::Settings::default();
        let mixer = apply(&kit(), "mix", Surface::Mixer, settings, 1440.0);
        assert!(mixer.iter().all(|(t, _)| t.height.is_none()));
        let arrange = apply(&kit(), "mix", Surface::Arrange, settings, 1440.0);
        assert!(arrange.iter().all(|(t, _)| t.width.is_none()));
        assert!(arrange.iter().all(|(t, _)| t.height.is_some()));
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
                .map(|(track, _)| index.get(track.guid.as_str()).copied().unwrap_or(usize::MAX))
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
            .map(|g| (all.iter().find(|t| t.guid == g).expect("a track").clone(), 0))
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
