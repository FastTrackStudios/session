//! Rule-based, per-surface track visibility.
//!
//! The old snapshot-based visibility manager stored captured per-GUID states,
//! which break the moment the template changes size. This module is **rule
//! based** instead: rules match tracks by their parsed taxonomy (band /
//! instrument) and structural role (folder *bus* vs audio *leaf*), so the same
//! rule resolves against any session — one Kick or five, 24 tracks or 200.
//!
//! Each rule carries independent intents for the two surfaces REAPER exposes:
//! the **arrange** view (TCP) and the **mixer** (MCP). That's what lets a mode
//! say "mixer shows the instrument buses collapsed, arrange shows just the audio
//! track."
//!
//! [`resolve`] is pure — it maps `(tracks, config, mode) → Vec<TrackPlan>` with
//! no REAPER calls, so it's unit-testable. The caller (`daw_module`) applies the
//! plans via the daw-reaper primitives.

use std::collections::HashMap;

use crate::track_schema::classify_track_with_config;
use crate::DynamicTemplateConfig;

/// Minimal structural view of a track the engine needs (taken from
/// `daw_reaper::Reaper.all`).
#[derive(Debug, Clone)]
pub struct TrackInput {
    pub guid: String,
    pub name: String,
    /// 0-based position in the track list — used to pick the "topmost" variant.
    pub index: u32,
    /// Whether this track is a folder parent (an instrument *bus*).
    pub is_folder: bool,
}

/// Structural role a selector can match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// A folder-parent track (instrument bus).
    Bus,
    /// A non-folder track (audio leaf).
    Leaf,
    /// Either.
    #[default]
    Any,
}

/// How many of the matched tracks a rule applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rank {
    /// Every track that matches the selector.
    #[default]
    All,
    /// Only the topmost (lowest-index) track per instrument group — e.g. keep
    /// one mic of a multi-mic instrument visible.
    TopmostPerInstrument,
}

/// Folder-collapse state for a surface (maps to REAPER's compact values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldState {
    Open,
    Small,
    Collapsed,
}

impl FoldState {
    fn to_compact(self) -> i32 {
        match self {
            FoldState::Open => 0,
            FoldState::Small => 1,
            FoldState::Collapsed => 2,
        }
    }
}

/// Predicate matching tracks by taxonomy + role. Absent fields don't constrain.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    /// Top-level visibility group, e.g. "Drums" (case-insensitive).
    pub band: Option<String>,
    /// Instrument (group-path leaf), e.g. "Kick" (case-insensitive).
    pub instrument: Option<String>,
    /// Structural role; defaults to `Any`.
    pub role: Role,
    /// Which matched tracks to keep; defaults to `All`.
    pub rank: Rank,
}

/// What a rule does to one surface.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceEffect {
    pub show: bool,
    /// Folder-collapse to apply (only meaningful for folder/bus tracks).
    pub fold: Option<FoldState>,
}

impl SurfaceEffect {
    pub fn show() -> Self {
        Self {
            show: true,
            fold: None,
        }
    }
    pub fn hide() -> Self {
        Self {
            show: false,
            fold: None,
        }
    }
    pub fn show_collapsed() -> Self {
        Self {
            show: true,
            fold: Some(FoldState::Collapsed),
        }
    }
}

/// A single visibility rule: a selector plus optional per-surface effects.
/// A surface left `None` falls through to earlier rules / the mode default.
#[derive(Debug, Clone, Default)]
pub struct VisibilityRule {
    pub selector: Selector,
    pub arrange: Option<SurfaceEffect>,
    pub mixer: Option<SurfaceEffect>,
}

/// A mode's complete visibility intent: per-surface defaults for unmatched
/// tracks, then ordered rules (last matching rule wins, per surface).
#[derive(Debug, Clone)]
pub struct ModeVisibility {
    pub default_arrange_show: bool,
    pub default_mixer_show: bool,
    pub rules: Vec<VisibilityRule>,
}

/// Resolved per-track action for the caller to apply.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackPlan {
    pub guid: String,
    pub arrange_show: bool,
    pub mixer_show: bool,
    /// Arrange folder-compact (`I_FOLDERCOMPACT`), only set for folder tracks.
    pub arrange_fold: Option<i32>,
    /// Mixer folder-compact (`BUSCOMP` field 2), only set for folder tracks.
    pub mixer_fold: Option<i32>,
}

/// Per-track parsed taxonomy, computed once.
struct Parsed {
    band: Option<String>,
    instrument: Option<String>,
    /// Grouping key for `TopmostPerInstrument` — the full matched group path, so
    /// all variants of one instrument share it but distinct instruments don't.
    /// Falls back to the track name when nothing classifies.
    group_key: String,
}

fn parse_track(t: &TrackInput, config: &DynamicTemplateConfig) -> Parsed {
    let cls = classify_track_with_config(&t.name, config);
    let band = cls.visibility_groups.first().cloned();
    let instrument = cls.matched_groups.last().cloned();
    let group_key = if cls.matched_groups.is_empty() {
        t.name.clone()
    } else {
        cls.matched_groups.join("/")
    };
    Parsed {
        band,
        instrument,
        group_key,
    }
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Does `track`/`parsed` satisfy everything in `sel` except its rank?
fn matches_basic(sel: &Selector, track: &TrackInput, parsed: &Parsed) -> bool {
    let role_ok = match sel.role {
        Role::Any => true,
        Role::Bus => track.is_folder,
        Role::Leaf => !track.is_folder,
    };
    if !role_ok {
        return false;
    }
    if let Some(band) = &sel.band {
        if parsed.band.as_deref().map(|x| eq_ci(x, band)) != Some(true) {
            return false;
        }
    }
    if let Some(inst) = &sel.instrument {
        if parsed.instrument.as_deref().map(|x| eq_ci(x, inst)) != Some(true) {
            return false;
        }
    }
    true
}

/// From `base` indices, keep only the lowest-index track per instrument group.
fn topmost_per_instrument(base: &[usize], tracks: &[TrackInput], parsed: &[Parsed]) -> Vec<usize> {
    let mut best: HashMap<&str, usize> = HashMap::new();
    for &i in base {
        let key = parsed[i].group_key.as_str();
        match best.get(key) {
            Some(&j) if tracks[j].index <= tracks[i].index => {}
            _ => {
                best.insert(key, i);
            }
        }
    }
    best.into_values().collect()
}

/// Resolve a mode's rules against the live track list into per-track plans.
///
/// Pure: no REAPER calls. Rules are evaluated in order; for each surface the
/// last rule that both matches a track and specifies that surface wins, falling
/// back to the mode default.
pub fn resolve(
    tracks: &[TrackInput],
    config: &DynamicTemplateConfig,
    mode: &ModeVisibility,
) -> Vec<TrackPlan> {
    let parsed: Vec<Parsed> = tracks.iter().map(|t| parse_track(t, config)).collect();

    let n = tracks.len();
    let mut arrange_show = vec![mode.default_arrange_show; n];
    let mut arrange_fold = vec![None; n];
    let mut mixer_show = vec![mode.default_mixer_show; n];
    let mut mixer_fold = vec![None; n];

    for rule in &mode.rules {
        let base: Vec<usize> = (0..n)
            .filter(|&i| matches_basic(&rule.selector, &tracks[i], &parsed[i]))
            .collect();
        let applies = match rule.selector.rank {
            Rank::All => base,
            Rank::TopmostPerInstrument => topmost_per_instrument(&base, tracks, &parsed),
        };
        for i in applies {
            if let Some(e) = rule.arrange {
                arrange_show[i] = e.show;
                arrange_fold[i] = e.fold;
            }
            if let Some(e) = rule.mixer {
                mixer_show[i] = e.show;
                mixer_fold[i] = e.fold;
            }
        }
    }

    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| TrackPlan {
            guid: t.guid.clone(),
            arrange_show: arrange_show[i],
            mixer_show: mixer_show[i],
            // Folder-compact only applies to folder tracks.
            arrange_fold: t
                .is_folder
                .then(|| arrange_fold[i].map(FoldState::to_compact))
                .flatten(),
            mixer_fold: t
                .is_folder
                .then(|| mixer_fold[i].map(FoldState::to_compact))
                .flatten(),
        })
        .collect()
}

/// Built-in mode rule sets. Returns `None` for modes we don't touch (their
/// tracks are left as-is). This is the temporary in-Rust source of rules; it
/// will be superseded by per-mode styx config, but the engine above is already
/// config-shaped so that swap is a deserialize.
pub fn mode_visibility_for(slug: &str) -> Option<ModeVisibility> {
    match slug {
        "edit" => Some(edit_mode()),
        _ => None,
    }
}

/// Edit mode: mixer shows every instrument bus collapsed (mics tuck behind,
/// expandable); arrange shows one audio track per instrument (the topmost mic),
/// hiding the buses and the other mics.
fn edit_mode() -> ModeVisibility {
    ModeVisibility {
        default_arrange_show: false,
        default_mixer_show: false,
        rules: vec![
            // Mixer: every instrument bus visible + collapsed.
            VisibilityRule {
                selector: Selector {
                    role: Role::Bus,
                    ..Default::default()
                },
                arrange: None,
                mixer: Some(SurfaceEffect::show_collapsed()),
            },
            // Arrange: the topmost audio leaf per instrument.
            VisibilityRule {
                selector: Selector {
                    role: Role::Leaf,
                    rank: Rank::TopmostPerInstrument,
                    ..Default::default()
                },
                arrange: Some(SurfaceEffect::show()),
                mixer: None,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(guid: &str, name: &str, index: u32, is_folder: bool) -> TrackInput {
        TrackInput {
            guid: guid.into(),
            name: name.into(),
            index,
            is_folder,
        }
    }

    #[test]
    fn edit_mode_collapses_kick_mics_to_one_in_arrange() {
        let config = crate::default_config();
        // A Kick bus (folder) over three mic leaves, plus a Snare leaf.
        let tracks = vec![
            t("kick-bus", "Kick", 0, true),
            t("kick-in", "Kick In", 1, false),
            t("kick-out", "Kick Out", 2, false),
            t("kick-trig", "Kick Trig", 3, false),
            t("snare", "Snare", 4, false),
        ];
        let plans = resolve(&tracks, &config, &edit_mode());
        let by = |g: &str| plans.iter().find(|p| p.guid == g).unwrap().clone();

        // Mixer: bus visible + collapsed; leaves hidden.
        let bus = by("kick-bus");
        assert!(bus.mixer_show, "bus should show in mixer");
        assert_eq!(bus.mixer_fold, Some(2), "bus should be collapsed in mixer");
        assert!(!bus.arrange_show, "bus should be hidden in arrange");

        // Arrange: only the topmost Kick mic (Kick In, index 1) shows.
        assert!(
            by("kick-in").arrange_show,
            "topmost kick mic shows in arrange"
        );
        assert!(
            !by("kick-out").arrange_show,
            "other kick mics hidden in arrange"
        );
        assert!(
            !by("kick-trig").arrange_show,
            "other kick mics hidden in arrange"
        );

        // Snare is its own instrument → its single leaf shows.
        assert!(by("snare").arrange_show, "snare leaf shows in arrange");
    }
}
