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

    /// Whether any row's track is selected — what decides whether the
    /// unselected ones are dimmed at all.
    #[must_use]
    pub fn any_selected(&self, tracks: &[Track]) -> bool {
        self.rows.iter().filter_map(|i| tracks.get(*i)).any(|t| t.selected)
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


// ── Scenes ────────────────────────────────────────────────────────────

/// A scene: a named answer to "which tracks, how large" for one moment
/// of a session — tracking the drums, mixing their buses, editing the
/// vocal's returns.
///
/// A visual track manager, as a set of rules rather than a list of
/// GUIDs, so the same scene means the right thing in any project that
/// uses the template's names. Recalled from the keyboard while mixing,
/// and rendered by the bench to a PNG so a scene can be looked at
/// without opening anything.
#[derive(Clone, Copy)]
pub struct Scene {
    pub name: &'static str,
    pub slug: &'static str,
    /// The size a track opens at, given its name, whether it is a
    /// folder, and the folders above it (nearest last). A scene sizes
    /// and never hides: hiding is the preset's job, and a scene over a
    /// preset that hid a track would be arguing with it.
    pub size: fn(&str, bool, &[String]) -> Size,
    /// What the scene does to a folder and what it holds: shows it,
    /// collapses it (the folder stays, its rows go), or hides it and
    /// everything in it.
    pub fold: fn(&str, bool, &[String]) -> Fold,
}

/// What a scene does to a folder — see [`Scene::fold`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Show,
    /// The folder stays as one strip; the rows inside it go.
    Collapse,
    /// The folder and everything in it go.
    Hide,
}

/// Every scene's default: nothing folded.
fn show_all(_name: &str, _is_folder: bool, _ancestors: &[String]) -> Fold {
    Fold::Show
}

/// The bus tree hidden: a scene about the instruments, not the mix.
fn hide_buses(name: &str, is_folder: bool, _ancestors: &[String]) -> Fold {
    if is_folder && is(name, &["MIX BUS"]) {
        Fold::Hide
    } else {
        Fold::Show
    }
}

/// Every scene, in the order the number keys recall them.
pub const SCENES: [Scene; 9] = [
    Scene {
        name: "Drum Tracking",
        slug: "drum-tracking",
        size: drum_tracking,
        fold: hide_buses,
    },
    Scene {
        name: "Drum Mixing",
        slug: "drum-mixing",
        size: drum_mixing,
        fold: hide_buses,
    },
    Scene {
        name: "Drum Overview",
        slug: "drum-overview",
        size: drum_overview,
        fold: drum_overview_fold,
    },
    Scene {
        name: "Drum Advanced",
        slug: "drum-advanced",
        size: drum_advanced,
        fold: hide_buses,
    },
    Scene {
        name: "Drum FX",
        slug: "drum-fx",
        size: drum_fx,
        fold: hide_buses,
    },
    Scene {
        name: "Buses",
        slug: "buses",
        size: buses,
        fold: buses_fold,
    },
    Scene {
        name: "Guitar FX",
        slug: "guitar-fx",
        size: guitar_fx,
        fold: guitar_fx_fold,
    },
    Scene {
        name: "Lead Vocal",
        slug: "lead-vocal",
        size: lead_vocal,
        fold: show_all,
    },
    Scene {
        name: "Lead Vocal FX Edit",
        slug: "lead-vocal-fx",
        size: lead_vocal_fx,
        fold: show_all,
    },
];

/// The folders a scene collapses, as GUIDs — what the window's own
/// folder state is set to when the scene is recalled, so the strips'
/// fold icons agree with the scene and a click on one carries on
/// from where the scene left it.
#[must_use]
pub fn collapsed_by(scene: &Scene, tracks: &[(Track, u32)]) -> Vec<String> {
    let mut folders: Vec<(u32, String)> = Vec::new();
    let mut out = Vec::new();
    for (track, depth) in tracks {
        folders.retain(|(at, _)| *at < *depth);
        let ancestors: Vec<String> = folders.iter().map(|(_, n)| n.clone()).collect();
        if track.is_folder {
            folders.push((*depth, track.name.clone()));
            if (scene.fold)(&track.name, true, &ancestors) == Fold::Collapse {
                out.push(track.guid.clone());
            }
        }
    }
    out
}

/// The scene for a slug.
#[must_use]
pub fn scene(slug: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|s| s.slug == slug)
}

fn is(name: &str, any: &[&str]) -> bool {
    any.iter().any(|n| name.eq_ignore_ascii_case(n))
}

fn under(ancestors: &[String], any: &[&str]) -> bool {
    ancestors.iter().any(|a| is(a, any))
}

/// The kit's pieces: the folders a drum mix is made on.
const PIECES: [&str; 5] = ["Kick", "Snare", "Toms", "Cymbals", "Rooms"];

/// Tracking: the core microphones you are getting a sound on, and
/// nothing else that needs reading.
fn drum_tracking(name: &str, is_folder: bool, ancestors: &[String]) -> Size {
    if parallel(name, ancestors) {
        Size::Minimum
    } else if is_folder {
        Size::Compact
    } else if is(name, &["In", "Out", "Top", "Bottom"]) && under(ancestors, &["Kick", "Snare"]) {
        Size::Working
    } else if under(ancestors, &["Drum Kit"]) {
        Size::Minimum
    } else {
        Size::Compact
    }
}

/// The Process folder and everything in it — what the kit is sent
/// to, which the tracking and advanced scenes only need present.
fn parallel(name: &str, ancestors: &[String]) -> bool {
    is(name, &["Process"]) || under(ancestors, &["Process"])
}

/// Mixing: the buses are the instrument; every mic is a rail.
fn drum_mixing(name: &str, is_folder: bool, _ancestors: &[String]) -> Size {
    if is_folder && is(name, &PIECES) {
        Size::Working
    } else if is_folder {
        Size::Compact
    } else {
        Size::Minimum
    }
}

/// Overview: the kit as its pieces — Kick, Snare, Toms, Cymbals,
/// Rooms — each collapsed to one strip at working width, and the
/// Process folder hidden. What the kit sounds like, five faders.
fn drum_overview(name: &str, is_folder: bool, _ancestors: &[String]) -> Size {
    if is_folder && is(name, &PIECES) {
        Size::Working
    } else {
        Size::Compact
    }
}

/// The overview's folds: the pieces shut, the Process folder and the
/// bus tree gone.
fn drum_overview_fold(name: &str, is_folder: bool, ancestors: &[String]) -> Fold {
    if is_folder && is(name, &["Process", "MIX BUS"]) {
        Fold::Hide
    } else if is_folder && is(name, &PIECES) && !under(ancestors, &["Process"]) {
        Fold::Collapse
    } else {
        Fold::Show
    }
}

/// Advanced: the tracks under the pieces that are not the core mics
/// — each Sub, Fund and Trig, and every verb the kit carries — open,
/// the mics and the buses present.
fn drum_advanced(name: &str, is_folder: bool, ancestors: &[String]) -> Size {
    let lower = name.to_lowercase();
    if parallel(name, ancestors) {
        Size::Minimum
    } else if is_folder {
        Size::Compact
    } else if lower == "fund" || lower == "sub" || lower.ends_with("trig") || lower == "verb" || under(ancestors, &["Verb"]) {
        Size::Working
    } else if under(ancestors, &["Drum Kit"]) {
        Size::Minimum
    } else {
        Size::Compact
    }
}

/// The kit's effects: what it is sent to. The room sim in focus, the
/// verb banks — the parallel folder's and the snare's — at working
/// width, the parallel compressors as tight rails (once a compressor
/// is dialled in it is a volume-balance game, and a rail is a fader),
/// and the kit itself present as rails.
fn drum_fx(name: &str, is_folder: bool, ancestors: &[String]) -> Size {
    let parallel = under(ancestors, &["Process"]);
    let compression = under(ancestors, &["Compress"]);
    let snare_verb = under(ancestors, &["Snare"]) && under(ancestors, &["Verb"]);
    if is_folder {
        if is(name, &["Process", "FX", "Verb"]) && (parallel || is(name, &["Process"])) {
            Size::Compact
        } else {
            Size::Minimum
        }
    } else if is(name, &["Room Sim"]) && parallel {
        Size::Focus
    } else if compression {
        Size::Minimum
    } else if parallel || snare_verb {
        Size::Working
    } else if under(ancestors, &["Drum Kit"]) {
        Size::Minimum
    } else {
        Size::Compact
    }
}

/// The instrument bus: the guitars and keys at working width with
/// the Inst FX returns open beside them, the first plate in focus, and
/// the drums, their process and the vocals out of the way.
fn guitar_fx(name: &str, is_folder: bool, ancestors: &[String]) -> Size {
    let fx = under(ancestors, &["Inst FX"]);
    if is_folder {
        Size::Compact
    } else if fx && is(name, &["Fat Plate"]) {
        Size::Focus
    } else if fx || under(ancestors, &["Guitars", "Keys"]) {
        Size::Working
    } else {
        Size::Minimum
    }
}

/// The instrument scene's folds: the kit, its process and the vocals
/// hidden — they are not what this scene is about.
fn guitar_fx_fold(name: &str, is_folder: bool, _ancestors: &[String]) -> Fold {
    if is_folder && is(name, &["Drum Kit", "Process", "Vocals", "MIX BUS"]) {
        Fold::Hide
    } else {
        Fold::Show
    }
}

/// The mix: the bus tree and nothing else — every bus at working
/// width, the stem buses compact, the instruments gone.
fn buses(_name: &str, is_folder: bool, _ancestors: &[String]) -> Size {
    if is_folder {
        Size::Compact
    } else {
        Size::Working
    }
}

/// The bus scene's folds: every top-level folder but the mix bus is
/// hidden.
fn buses_fold(name: &str, is_folder: bool, ancestors: &[String]) -> Fold {
    if is_folder && ancestors.is_empty() && !is(name, &["MIX BUS"]) {
        Fold::Hide
    } else {
        Fold::Show
    }
}

/// The lead vocal open, every return present as a short rail.
fn lead_vocal(name: &str, is_folder: bool, ancestors: &[String]) -> Size {
    let fx = under(ancestors, &["Vox FX"]);
    if is_folder {
        Size::Compact
    } else if fx {
        Size::Minimum
    } else if name.to_lowercase().contains("lead") {
        // The subject of the scene: its whole chain, top to bottom.
        Size::Focus
    } else if under(ancestors, &["Vox Lead"]) {
        Size::Working
    } else {
        Size::Compact
    }
}

/// Editing the returns: one delay and one verb in focus, every other
/// return at the working width with its chain drawn, every folder a
/// rail. The returns are all live — a slap is a slap whether or not it
/// is the one being edited — so the scene shows them all working and
/// opens the two under the hands.
fn lead_vocal_fx(name: &str, is_folder: bool, ancestors: &[String]) -> Size {
    let fx = under(ancestors, &["Vox FX"]);
    if is_folder {
        Size::Minimum
    } else if fx && ((is(name, &["Short"]) && under(ancestors, &["Delay"])) || (is(name, &["Long"]) && under(ancestors, &["Verb"]))) {
        Size::Focus
    } else if fx {
        Size::Working
    } else if name.to_lowercase().contains("lead") {
        // The lead beside its returns, so the chain being fed and the
        // instances feeding it are all open at once.
        Size::Focus
    } else {
        Size::Working
    }
}

/// Apply a scene to a track list — the same contract as [`apply`].
#[must_use]
pub fn apply_scene(
    tracks: &[(Track, u32)],
    scene: &Scene,
    settings: crate::settings::Settings,
    panel: f64,
) -> Vec<(Track, u32)> {
    let mut folders: Vec<(u32, String)> = Vec::new();
    let mut out = Vec::with_capacity(tracks.len());
    // A folded folder: rows deeper than it are dropped until the walk
    // comes back up to its level, and a hidden one drops itself too.
    let mut folded: Option<u32> = None;
    for (track, depth) in tracks {
        folders.retain(|(at, _)| *at < *depth);
        let ancestors: Vec<String> = folders.iter().map(|(_, n)| n.clone()).collect();
        if track.is_folder {
            folders.push((*depth, track.name.clone()));
        }
        if let Some(at) = folded {
            if *depth > at {
                continue;
            }
            folded = None;
        }
        if track.is_folder {
            match (scene.fold)(&track.name, true, &ancestors) {
                Fold::Show => {}
                Fold::Collapse => folded = Some(*depth),
                Fold::Hide => {
                    folded = Some(*depth);
                    continue;
                }
            }
        }
        // One half of a stereo pair is a rail whatever the scene says:
        // the pair's folder carries the processing and the width.
        let size = if crate::tone::is_pair_half(&track.name) && !track.is_folder {
            Size::Minimum
        } else {
            (scene.size)(&track.name, track.is_folder, &ancestors)
        };
        let mut track = track.clone();
        track.width = Some(pixels(mixer_width(size, settings, panel)));
        // A focused strip is the selected one: that is what the mixer
        // opens to the focus width and reads the rack of.
        track.selected = size == Size::Focus;
        out.push((track, *depth));
    }
    out
}

#[cfg(test)]
mod scene_tests {
    use super::{Size, scene};

    #[test]
    fn the_fx_edit_scene_focuses_one_delay_and_one_verb() {
        let s = scene("lead-vocal-fx").expect("the scene");
        let delay: Vec<String> = ["Vox Lead", "Vox FX", "Delay"].iter().map(|s| (*s).to_owned()).collect();
        let verb: Vec<String> = ["Vox Lead", "Vox FX", "Verb"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!((s.size)("Short", false, &delay), Size::Focus);
        assert_eq!((s.size)("Long", false, &verb), Size::Focus);
        assert_eq!((s.size)("Short", false, &verb), Size::Working);
        assert_eq!((s.size)("Delay", true, &delay[..2]), Size::Minimum);
        assert_eq!((s.size)("Lead Vox", false, &delay[..1]), Size::Focus);
        assert_eq!((s.size)("Vox Dbl", false, &delay[..1]), Size::Working);
    }

    #[test]
    fn the_drum_scenes_disagree_about_the_mics() {
        let kick: Vec<String> = ["Drum Kit", "Kick", "Sum"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!((scene("drum-tracking").unwrap().size)("In", false, &kick), Size::Working);
        assert_eq!((scene("drum-mixing").unwrap().size)("In", false, &kick), Size::Minimum);
        assert_eq!((scene("drum-mixing").unwrap().size)("Kick", true, &kick[..1]), Size::Working);
        let snare_verb: Vec<String> = ["Drum Kit", "Snare", "Verb"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!((scene("drum-advanced").unwrap().size)("Nonlin", false, &snare_verb), Size::Working);
        assert_eq!((scene("drum-advanced").unwrap().size)("Sub", false, &kick[..2]), Size::Working);
        assert_eq!((scene("drum-advanced").unwrap().size)("In", false, &kick), Size::Minimum);
    }

    #[test]
    fn the_fx_scene_opens_what_the_kit_is_sent_to() {
        let s = scene("drum-fx").expect("the scene");
        let parallel: Vec<String> = ["Process", "FX"].iter().map(|s| (*s).to_owned()).collect();
        let comp: Vec<String> = ["Process", "Compress"].iter().map(|s| (*s).to_owned()).collect();
        let snare_verb: Vec<String> = ["Drum Kit", "Snare", "Verb"].iter().map(|s| (*s).to_owned()).collect();
        let kick: Vec<String> = ["Drum Kit", "Kick", "Sum"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!((s.size)("Room Sim", false, &parallel), Size::Focus);
        assert_eq!((s.size)("Smash", false, &comp), Size::Minimum);
        assert_eq!((s.size)("Nonlin", false, &snare_verb), Size::Working);
        assert_eq!((s.size)("In", false, &kick), Size::Minimum);
        assert_eq!((s.size)("Kick", true, &kick[..1]), Size::Minimum);
    }

    /// The overview folds the pieces shut and hides the Process folder:
    /// applied, the rows are the kit and its five pieces.
    #[test]
    fn the_overview_is_five_pieces() {
        use daw_proto::Track;
        let folder = |name: &str, depth: u32| {
            let mut t = Track {
                guid: name.to_lowercase(),
                name: name.to_owned(),
                ..Track::default()
            };
            t.is_folder = true;
            (t, depth)
        };
        let leaf = |name: &str, depth: u32| {
            (
                Track {
                    guid: format!("{}-{depth}", name.to_lowercase()),
                    name: name.to_owned(),
                    ..Track::default()
                },
                depth,
            )
        };
        let rows = vec![
            folder("Drum Kit", 0),
            folder("Kick", 1),
            leaf("In", 2),
            leaf("Out", 2),
            folder("Snare", 1),
            leaf("Top", 2),
            folder("Process", 0),
            folder("Compress", 1),
            leaf("Dry", 2),
            folder("Bass", 0),
            leaf("DI", 1),
        ];
        let s = scene("drum-overview").expect("the scene");
        let out = super::apply_scene(&rows, s, crate::settings::Settings::default(), 1000.0);
        let names: Vec<&str> = out.iter().map(|(t, _)| t.name.as_str()).collect();
        assert_eq!(names, ["Drum Kit", "Kick", "Snare", "Bass", "DI"]);
        assert_eq!(super::collapsed_by(s, &rows), ["kick", "snare"]);
    }
}
