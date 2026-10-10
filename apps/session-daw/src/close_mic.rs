//! A drum's hits from its close mic, for a piece whose trigger is dead.
//!
//! A trigger hears only its drum; a close mic hears its drum and the kit
//! around it — the snare and the kick bleed into a floor tom's mic at
//! well over its own quiet hits. So a moment counts as the piece's hit
//! when its mic is loud *for itself* and louder, so measured, than every
//! other piece's close mic at that moment: each mic's level is taken
//! against its own typical hit, and a snare's bleed — a full hit on the
//! snare's mic, a quiet one on the tom's — is the snare's.
//!
//! For the samples under the kit (`crate::drum_samples`), where a dead
//! trigger left a piece with none; not for editing, which stays on the
//! triggers.

use daw::service::ProjectContext;

use crate::drum_edit::KitHit;
use crate::fundamental::{DETECT_RATE, DetectAudio};

/// Under its own typical hit by more than this, a moment is not a hit of
/// the piece (a ghost of it, or its ring).
const QUIETEST_DB: f64 = -24.0;

/// How much louder, each against its own typical hit, the piece's mic must
/// be than any other piece's at the moment.
const OVER_OTHERS_DB: f64 = 3.0;

/// How many of a mic's loudest moments its typical hit is the median of.
const TYPICAL_OF: usize = 20;

/// How long after a moment its peak is read, seconds (as the detector's).
const MEASURE: f64 = 0.025;

/// The hits of the piece whose trigger is `trigger`, from its close mic,
/// each against the loudest kept; empty without a close mic, or with no
/// moment its own.
#[must_use]
pub fn hits(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    trigger: &str,
    settings: &crate::drum_settings::Settings,
) -> Vec<KitHit> {
    let tracks = daw::service::Tracks::all(daw, project.clone());
    let Some(own) = mic_of(daw, project, &tracks, trigger) else {
        return Vec::new();
    };
    let others: Vec<String> = crate::drum_reference::pieces(daw, project)
        .into_iter()
        .filter(|(_, t)| t != trigger)
        .filter_map(|(_, t)| mic_of(daw, project, &tracks, &t))
        .collect();
    let own_audio = crate::fundamental::detect_audio(daw, project, &own);
    let detect = settings.detect();
    let moments: Vec<crate::drum_edit::Transient> = own_audio
        .iter()
        .flat_map(|(start, samples)| {
            crate::drum_edit::transients(samples, DETECT_RATE, *start, &detect)
        })
        .collect();
    let Some(own_level) = typical(&moments) else {
        return Vec::new();
    };
    let other_levels: Vec<(DetectAudio, f64)> = others
        .iter()
        .filter_map(|mic| {
            let audio = crate::fundamental::detect_audio(daw, project, mic);
            let found: Vec<crate::drum_edit::Transient> = audio
                .iter()
                .flat_map(|(start, samples)| {
                    crate::drum_edit::transients(samples, DETECT_RATE, *start, &detect)
                })
                .collect();
            typical(&found).map(|level| (audio, level))
        })
        .collect();
    let kept: Vec<(f64, f64)> = moments
        .iter()
        .filter_map(|m| {
            let mine = m.peak_db - own_level;
            let loudest_other = other_levels
                .iter()
                .map(|(audio, level)| peak_db(audio, m.time) - level)
                .fold(f64::NEG_INFINITY, f64::max);
            (mine >= QUIETEST_DB && mine > loudest_other + OVER_OTHERS_DB).then_some((m.time, mine))
        })
        .collect();
    let loudest = kept.iter().map(|k| k.1).fold(f64::NEG_INFINITY, f64::max);
    let offset = settings.offset_ms / 1_000.0;
    kept.into_iter()
        .map(|(time, level)| KitHit {
            time: time + offset,
            relative_db: level - loudest,
        })
        .collect()
}

/// The close mic beside a piece's trigger: a sibling that is neither a
/// trigger, its sample nor its one-note track, and has audio.
fn mic_of(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    tracks: &[daw_proto::Track],
    trigger: &str,
) -> Option<String> {
    use daw::service::Items as _;
    let parent = tracks
        .iter()
        .find(|t| t.guid == trigger)?
        .parent_guid
        .clone();
    tracks
        .iter()
        .find(|t| {
            let name = t.name.trim().to_ascii_lowercase();
            t.parent_guid == parent
                && t.guid != trigger
                && !t.is_folder
                && !dynamic_template::apply::is_trigger(&t.name)
                && !name.ends_with("sample")
                && !dynamic_template::apply::is_one_note(&t.name)
                && !daw
                    .get_items(
                        project.clone(),
                        daw::service::TrackRef::Guid(t.guid.clone()),
                    )
                    .is_empty()
        })
        .map(|t| t.guid.clone())
}

/// A mic's typical hit, dBFS: the median of its loudest moments.
fn typical(moments: &[crate::drum_edit::Transient]) -> Option<f64> {
    let mut peaks: Vec<f64> = moments.iter().map(|m| m.peak_db).collect();
    peaks.sort_by(|a, b| b.total_cmp(a));
    peaks.truncate(TYPICAL_OF);
    peaks.get(peaks.len() / 2).copied()
}

/// The peak of `audio` in the [`MEASURE`] after `at`, dBFS.
fn peak_db(audio: &DetectAudio, at: f64) -> f64 {
    use num_traits::ToPrimitive as _;
    let peak = audio
        .iter()
        .filter_map(|(start, samples)| {
            let from = ((at - start) * DETECT_RATE).floor();
            let to = MEASURE.mul_add(DETECT_RATE, from);
            let (from, to) = (from.max(0.0).to_usize()?, to.max(0.0).to_usize()?);
            samples
                .get(from..to.min(samples.len()))?
                .iter()
                .map(|s| s.abs())
                .reduce(f32::max)
        })
        .fold(0.0_f32, f32::max);
    20.0 * f64::from(peak.max(1e-9)).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mics_typical_hit_is_the_middle_of_its_loudest() {
        let moments: Vec<crate::drum_edit::Transient> = (0..40)
            .map(|i| crate::drum_edit::Transient {
                time: f64::from(i),
                peak_db: -f64::from(i),
            })
            .collect();
        // The 20 loudest are 0..=-19 dB; their middle is -10.
        assert_eq!(typical(&moments), Some(-10.0));
        assert_eq!(typical(&[]), None);
    }

    #[test]
    fn a_peak_is_read_where_the_moment_is() {
        let mut samples = vec![0.0_f32; 8_000];
        samples[4_000] = 0.5;
        let audio: DetectAudio = std::sync::Arc::new(vec![(10.0, samples)]);
        let half = 20.0 * 0.5_f64.log10();
        assert!((peak_db(&audio, 10.5) - half).abs() < 1e-6);
        assert!(peak_db(&audio, 10.2) < -100.0, "nothing there");
        assert!(peak_db(&audio, 30.0) < -100.0, "past the item");
    }
}
