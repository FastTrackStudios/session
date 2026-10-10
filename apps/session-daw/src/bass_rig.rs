//! signal's bass rig on the session's bass tracks, a patch to each.
//!
//! A bass recorded as parallel tracks — a clean DI, a harmonics track, a
//! crunch track — wants a rig voiced for each, not one sound on all. The
//! rig is signal's ([`signal_fx::bass`]: drive capture, amp capture, cab,
//! trims); the patch a track gets is the one whose name its own name
//! holds. The patches are this window's, not signal's library: that is the
//! live rig's, and its order is what a footswitch steps through.

use daw::service::{FxChainContext, ProjectContext, Tracks};

/// One patch: what the rig plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    /// Its name, and what a track's name holds to take it.
    pub name: &'static str,
    pub drive: Option<&'static str>,
    pub amp: Option<&'static str>,
    pub cab: Option<&'static str>,
    pub input_db: f64,
    pub output_db: f64,
}

/// Where the captures are.
const NAM: &str = "/run/media/AudioHaven/Signal/NAM_models";

/// The DP3X bass preamp, dimed: grit that keeps the low end.
const DP3X: &str = "Jason Z Tech21 dUg DP3X bass preamp pedal all dimed no shift.nam";
/// An HM2 voiced bass-heavy: a drive in front of it.
const HM2: &str = "Jason Z Boss HM2 v1 kinda bass heavy with medium distortion pure everything turned all the way up tone.nam";

/// The patches, by what a track is called.
pub const PATCHES: [Patch; 3] = [
    // Barely breaking up: the preamp fed low, its level made up after.
    Patch {
        name: "Clean",
        drive: None,
        amp: Some(DP3X),
        cab: None,
        input_db: -15.0,
        output_db: 9.0,
    },
    // The preamp as captured: its harmonics are the point.
    Patch {
        name: "Harmonics",
        drive: None,
        amp: Some(DP3X),
        cab: None,
        input_db: 0.0,
        output_db: 0.0,
    },
    // A drive in front of it.
    Patch {
        name: "Crunch",
        drive: Some(HM2),
        amp: Some(DP3X),
        cab: None,
        input_db: -6.0,
        output_db: -3.0,
    },
];

/// The patch a track called `name` takes, if one fits.
#[must_use]
pub fn patch_for(name: &str) -> Option<&'static Patch> {
    let name = name.to_ascii_lowercase();
    PATCHES
        .iter()
        .find(|p| name.contains(&p.name.to_ascii_lowercase()))
}

impl Patch {
    /// The rig's state for this patch.
    #[must_use]
    pub fn state(&self) -> serde_json::Value {
        let file = |f: Option<&str>| f.map(|f| format!("{NAM}/{f}"));
        serde_json::json!({
            "amp": file(self.amp),
            "drive": file(self.drive),
            "cab": file(self.cab),
            "input_db": self.input_db,
            "output_db": self.output_db,
        })
    }

    /// Whether every capture it names is on disk.
    #[must_use]
    pub fn present(&self) -> bool {
        [self.drive, self.amp, self.cab]
            .into_iter()
            .flatten()
            .all(|f| std::path::Path::new(NAM).join(f).exists())
    }
}

/// Put the rig on each bass track that a patch fits — first in its chain,
/// before its tone — or, `dry`, say which. One line a track.
///
/// # Errors
///
/// The engine refusing a write.
pub fn set_up(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    dry: bool,
) -> Result<Vec<String>, String> {
    let tracks = Tracks::all(daw, project.clone());
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let mut said = Vec::new();
    for track in tracks.iter().filter(|t| !t.is_folder) {
        let folders = crate::gain_stage::path_of(track, &by_guid);
        if !folders
            .first()
            .is_some_and(|p| p.trim().eq_ignore_ascii_case("bass"))
        {
            continue;
        }
        let Some(patch) = patch_for(&track.name) else {
            said.push(format!("{}: no patch fits it", folders.join("/")));
            continue;
        };
        if !patch.present() {
            said.push(format!(
                "{}: the {} patch's captures are not on disk",
                folders.join("/"),
                patch.name
            ));
            continue;
        }
        said.push(format!("{}: the {} patch", folders.join("/"), patch.name));
        if dry {
            continue;
        }
        let chain = FxChainContext::Track(track.guid.clone());
        let fx = match daw::service::Effects::list(daw, project.clone(), chain.clone())
            .into_iter()
            .find(|f| f.name.eq_ignore_ascii_case(signal_fx::BASS_RIG))
        {
            Some(f) => f.guid,
            None => daw::service::Effects::add_at(
                daw,
                project.clone(),
                daw_proto::fx::AddFxAtRequest {
                    context: chain,
                    name: signal_fx::BASS_RIG.to_owned(),
                    index: 0,
                },
            )
            .ok_or("the bass rig would not add")?,
        };
        daw.apply_plugin_state(&fx, patch.state().to_string().as_bytes())
            .map_err(|e| format!("the bass rig would not load: {e}"))?;
    }
    if !dry {
        crate::studio::request_resync();
        crate::actions::list().changed("bass.rig", "the bass rig was set up");
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_takes_the_patch_its_name_holds() {
        assert_eq!(patch_for("Clean").map(|p| p.name), Some("Clean"));
        assert_eq!(
            patch_for("Bass Harmonics").map(|p| p.name),
            Some("Harmonics")
        );
        assert_eq!(patch_for("crunch 2").map(|p| p.name), Some("Crunch"));
        assert!(patch_for("Sub").is_none());
    }

    #[test]
    fn a_patch_is_the_rigs_state() {
        let crunch = patch_for("Crunch").expect("crunch");
        let state = crunch.state();
        assert!(state["drive"].as_str().is_some_and(|d| d.ends_with(HM2)));
        assert!(state["amp"].as_str().is_some_and(|a| a.ends_with(DP3X)));
        assert!(state["cab"].is_null());
    }
}
