//! Drum editing's settings: sticky, one set for the session.
//!
//! What a detector and a slip are dialled to stays dialled — set once,
//! every detection, fill search and slip after uses it, and it is still
//! set after a restart (`~/.config/fts/drum-edit.json`). That is how
//! they are tuned: change one, look at what is detected now, change it
//! again, for whatever stretch is being edited. A call may still carry
//! its own for that call alone ([`Patch`], flattened into its
//! arguments), without moving the ones that stay.

use facet::Facet;

/// How the hits are found.
#[derive(Facet, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Detector {
    /// A spike within `under_db` of the trigger's loudest — for a
    /// trigger, whose spikes are all there is to it.
    Level,
    /// Perfect Timing!'s: the attack outrunning the signal by
    /// `sensitivity_db`, over `threshold_db` — where a hit STARTS, however
    /// hard; for mics as well as triggers.
    Transient,
}

/// What to move the hits to.
#[derive(Facet, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Target {
    /// The grid, the drum MIDI as a guide: a hit with a written note near
    /// goes to the line that note is written on — the MIDI is quantized,
    /// so it says WHICH line (straight or a triplet), the grid says when;
    /// a hit with none (a fill played otherwise) to the nearest line when
    /// it is plainly near one (`sure_ms`); else it is left, to be
    /// listened to.
    Guided,
    /// The nearest note of the drum MIDI.
    Reference,
    /// The nearest grid line.
    Grid,
}

/// What a slip does with the fills.
#[derive(Facet, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Fills {
    /// Leave them as played.
    Skip,
    /// Edit them like the rest.
    Include,
    /// Edit only them.
    Only,
}

/// Every setting.
#[derive(Facet, Clone, Debug, PartialEq)]
pub struct Settings {
    pub detector: Detector,
    /// Level: how far under the trigger's loudest a hit can be, dB.
    pub under_db: f64,
    /// Transient: a hit is louder than this, dBFS.
    pub threshold_db: f64,
    /// Transient: how far the attack outruns the signal, dB.
    pub sensitivity_db: f64,
    /// From one hit to the next, ms.
    pub retrig_ms: f64,
    /// Added to every hit found, ms.
    pub offset_ms: f64,
    /// The grid: a whole note's division (32: thirty-seconds).
    pub grid: u32,
    /// One hit per grid line, the loudest near it.
    pub grid_scan: bool,
    /// How near its line a hit must be for the grid scan to keep it, ms
    /// (none: every hit is some line's).
    pub grid_tolerance_ms: Option<f64>,
    pub target: Target,
    /// How far a hit's written note may be, ms.
    pub window_ms: f64,
    /// Guided, no note near: how near a line a hit must be to go to it,
    /// ms.
    pub sure_ms: f64,
    /// A hit this near its target already stays, ms.
    pub tolerance_ms: f64,
    /// How much of each move is made, 0..1.
    pub strength: f64,
    /// The cut before each hit, ms.
    pub lead_ms: f64,
    /// The crossfade across each seam, ms.
    pub xfade_ms: f64,
    pub fills: Fills,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            detector: Detector::Level,
            under_db: 24.0,
            threshold_db: -60.0,
            sensitivity_db: 5.0,
            retrig_ms: 40.0,
            offset_ms: 0.0,
            grid: 32,
            grid_scan: false,
            grid_tolerance_ms: None,
            target: Target::Guided,
            window_ms: 30.0,
            sure_ms: 15.0,
            tolerance_ms: 3.0,
            strength: 1.0,
            lead_ms: 5.0,
            xfade_ms: 5.0,
            fills: Fills::Skip,
        }
    }
}

/// Some settings: each one given replaces the one there.
#[derive(Facet, Clone, Debug, Default, PartialEq)]
pub struct Patch {
    pub detector: Option<Detector>,
    pub under_db: Option<f64>,
    pub threshold_db: Option<f64>,
    pub sensitivity_db: Option<f64>,
    pub retrig_ms: Option<f64>,
    pub offset_ms: Option<f64>,
    pub grid: Option<u32>,
    pub grid_scan: Option<bool>,
    pub grid_tolerance_ms: Option<f64>,
    pub target: Option<Target>,
    pub window_ms: Option<f64>,
    pub sure_ms: Option<f64>,
    pub tolerance_ms: Option<f64>,
    pub strength: Option<f64>,
    pub lead_ms: Option<f64>,
    pub xfade_ms: Option<f64>,
    pub fills: Option<Fills>,
}

impl Settings {
    /// These, with `patch`'s in place of theirs. A transient setting
    /// given without a detector means that detector: tuning its
    /// sensitivity says which one is meant.
    #[must_use]
    pub fn with(&self, patch: &Patch) -> Self {
        let pick = |given: Option<f64>, was: f64| given.unwrap_or(was);
        let transient_tuned = patch.threshold_db.is_some() || patch.sensitivity_db.is_some();
        Self {
            detector: patch.detector.unwrap_or(if transient_tuned {
                Detector::Transient
            } else {
                self.detector
            }),
            under_db: pick(patch.under_db, self.under_db),
            threshold_db: pick(patch.threshold_db, self.threshold_db),
            sensitivity_db: pick(patch.sensitivity_db, self.sensitivity_db),
            retrig_ms: pick(patch.retrig_ms, self.retrig_ms).max(0.0),
            offset_ms: pick(patch.offset_ms, self.offset_ms),
            grid: patch.grid.unwrap_or(self.grid).max(1),
            grid_scan: patch.grid_scan.unwrap_or(self.grid_scan),
            // Zero or less: no tolerance — every hit is some line's.
            grid_tolerance_ms: match patch.grid_tolerance_ms {
                Some(ms) if ms > 0.0 => Some(ms),
                Some(_) => None,
                None => self.grid_tolerance_ms,
            },
            target: patch.target.unwrap_or(self.target),
            window_ms: pick(patch.window_ms, self.window_ms).max(0.0),
            sure_ms: pick(patch.sure_ms, self.sure_ms).max(0.0),
            tolerance_ms: pick(patch.tolerance_ms, self.tolerance_ms).max(0.0),
            strength: pick(patch.strength, self.strength).clamp(0.0, 1.0),
            lead_ms: pick(patch.lead_ms, self.lead_ms).max(0.0),
            xfade_ms: pick(patch.xfade_ms, self.xfade_ms).max(0.0),
            fills: patch.fills.unwrap_or(self.fills),
        }
    }

    /// The transient detector's settings.
    #[must_use]
    pub fn detect(&self) -> crate::drum_edit::Detect {
        crate::drum_edit::Detect {
            threshold_db: self.threshold_db,
            sensitivity_db: self.sensitivity_db,
            retrig: self.retrig_ms / 1_000.0,
        }
    }
}

/// The settings as they stand.
fn store() -> &'static std::sync::Mutex<Settings> {
    static STORE: std::sync::OnceLock<std::sync::Mutex<Settings>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| std::sync::Mutex::new(load().unwrap_or_default()))
}

/// The settings as they stand.
#[must_use]
pub fn current() -> Settings {
    store()
        .lock()
        .map_or_else(|e| e.into_inner().clone(), |s| s.clone())
}

/// Change some, keep them (and save them), and say what they are now.
pub fn set(patch: &Patch) -> Settings {
    let now = {
        let mut held = store()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *held = held.with(patch);
        held.clone()
    };
    save(&now);
    now
}

/// Back to the defaults (and saved).
pub fn reset() -> Settings {
    let now = Settings::default();
    *store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = now.clone();
    save(&now);
    now
}

/// Where they are kept.
fn path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("fts").join("drum-edit.json"))
}

fn load() -> Option<Settings> {
    let text = std::fs::read(path()?).ok()?;
    // Saved as a patch: a setting added since is its default.
    let patch: Patch = facet_json::from_slice(&text).ok()?;
    Some(Settings::default().with(&patch))
}

fn save(settings: &Settings) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent()
        && std::fs::create_dir_all(dir).is_err()
    {
        return;
    }
    match facet_json::to_vec(settings) {
        Ok(bytes) => {
            if std::fs::write(&path, bytes).is_err() {
                tracing::warn!("drum edit: the settings could not be saved");
            }
        }
        Err(error) => tracing::warn!(error = %error, "drum edit: the settings would not serialize"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_changes_what_it_names_and_nothing_else() {
        let was = Settings::default();
        let now = was.with(&Patch {
            lead_ms: Some(8.0),
            grid_scan: Some(true),
            ..Patch::default()
        });
        assert!((now.lead_ms - 8.0).abs() < f64::EPSILON);
        assert!(now.grid_scan);
        assert_eq!(
            Settings {
                lead_ms: was.lead_ms,
                grid_scan: was.grid_scan,
                ..now
            },
            was
        );
    }

    /// Tuning the sensitivity means the detector that has one.
    #[test]
    fn tuning_the_transient_detector_chooses_it() {
        let now = Settings::default().with(&Patch {
            sensitivity_db: Some(9.0),
            ..Patch::default()
        });
        assert_eq!(now.detector, Detector::Transient);
        let back = now.with(&Patch {
            detector: Some(Detector::Level),
            ..Patch::default()
        });
        assert_eq!(back.detector, Detector::Level);
        assert!(
            (back.sensitivity_db - 9.0).abs() < f64::EPSILON,
            "and its tuning is kept for later"
        );
    }

    /// A call's arguments, its settings written in among them.
    #[derive(Facet)]
    struct Call {
        #[facet(flatten, default)]
        with: Patch,
        from: Option<f64>,
    }

    /// A call that gives no settings still parses — and one that does
    /// gives them.
    #[test]
    fn a_call_carries_its_settings_or_none() {
        let none: Call = facet_json::from_slice(b"{}").expect("no settings");
        assert_eq!(none.with, Patch::default());
        let some: Call =
            facet_json::from_slice(br#"{"lead_ms":3.0,"from":1.5}"#).expect("some settings");
        assert_eq!(some.with.lead_ms, Some(3.0));
        assert_eq!(some.from, Some(1.5));
    }

    /// What is saved reads back the same, and a file missing settings
    /// added since reads with their defaults.
    #[test]
    fn saved_settings_read_back() {
        let tuned = Settings::default().with(&Patch {
            detector: Some(Detector::Transient),
            grid_tolerance_ms: Some(25.0),
            fills: Some(Fills::Only),
            ..Patch::default()
        });
        let bytes = facet_json::to_vec(&tuned).expect("serializes");
        let patch: Patch = facet_json::from_slice(&bytes).expect("reads as a patch");
        assert_eq!(Settings::default().with(&patch), tuned);
        let old: Patch = facet_json::from_slice(br#"{"lead_ms":7.0}"#).expect("an older file");
        assert_eq!(
            Settings::default().with(&old).xfade_ms.to_bits(),
            5.0_f64.to_bits()
        );
    }
}
