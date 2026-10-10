//! A trigger and a calibrated mic made one list of hits.
//!
//! Each has what the other lacks. A trigger hears only its drum, so it
//! is the best word on *whether* the drum was hit — but its spike is a
//! contact mic's, not the sound that is cut, and crosses its threshold
//! part way up. The close mic is the sound itself, so it is the word on
//! *when* the attack starts and *how hard* — but it hears the kit.
//!
//! So: a stroke both report is certain, and placed on the mic's attack.
//! One only the trigger reports is still placed on the mic (the trigger
//! says where to look) but flagged, being a choke or a neighbour's
//! crosstalk as often as a soft hit. One only the mic reports is a ghost
//! the trigger was too quiet for, or bleed — kept, flagged, and left
//! for someone to look at.

use crate::attack::{attack_peak, attack_start};
use crate::calibrate::TOLERANCE;

/// One stroke on a piece.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Where the attack starts on the close mic, in samples.
    pub at: usize,
    /// The attack's peak on the close mic, linear.
    pub peak: f32,
    /// How sure: 0 to 1.
    pub confidence: f64,
    /// The trigger reported it.
    pub from_trigger: bool,
    /// The close mic reported it.
    pub from_mic: bool,
    /// Much quieter than the piece usually is ([`GHOST_DB`] under its
    /// median).
    pub ghost: bool,
    /// The other stroke of a flam: its index in the same list.
    pub flam_with: Option<usize>,
}

/// A stroke both report.
pub const BOTH: f64 = 0.95;
/// A stroke only the trigger reports.
pub const TRIGGER_ONLY: f64 = 0.6;
/// A stroke only the calibrated mic reports.
pub const MIC_ONLY: f64 = 0.4;
/// How far under the piece's median peak a ghost note is, dB.
pub const GHOST_DB: f64 = 15.0;
/// Two strokes nearer than this are a flam, seconds.
pub const FLAM: f64 = 0.035;
/// Two attacks this near are one stroke found twice, seconds.
const SAME: f64 = 0.002;

/// One hit at `at`, measured on `mic`.
fn hit(mic: &[f32], rate: f64, at: usize, confidence: f64, trigger: bool, own: bool) -> Hit {
    let at = attack_start(mic, rate, at);
    Hit {
        at,
        peak: attack_peak(mic, rate, at),
        confidence,
        from_trigger: trigger,
        from_mic: own,
        ghost: false,
        flam_with: None,
    }
}

/// The piece's hits from its trigger's (`trigger`) and its calibrated
/// mic's (`mic_hits`), both sorted sample positions on the same clock,
/// placed and measured on `mic`.
#[must_use]
pub fn fuse(mic: &[f32], rate: f64, trigger: &[usize], mic_hits: &[usize]) -> Vec<Hit> {
    let tolerance = crate::samples(TOLERANCE, rate);
    let mut out = Vec::with_capacity(trigger.len().max(mic_hits.len()));
    let (mut i, mut j) = (0_usize, 0_usize);
    loop {
        match (mic_hits.get(i), trigger.get(j)) {
            (Some(&m), Some(&t)) if m.abs_diff(t) <= tolerance => {
                out.push(hit(mic, rate, m, BOTH, true, true));
                i = i.saturating_add(1);
                j = j.saturating_add(1);
            }
            (Some(&m), Some(&t)) if m < t => {
                out.push(hit(mic, rate, m, MIC_ONLY, false, true));
                i = i.saturating_add(1);
            }
            (_, Some(&t)) => {
                out.push(hit(mic, rate, t, TRIGGER_ONLY, true, false));
                j = j.saturating_add(1);
            }
            (Some(&m), None) => {
                out.push(hit(mic, rate, m, MIC_ONLY, false, true));
                i = i.saturating_add(1);
            }
            (None, None) => break,
        }
    }
    settle(out, rate)
}

/// Sort, merge strokes found twice, mark the ghosts and the flams.
#[must_use]
pub fn settle(mut hits: Vec<Hit>, rate: f64) -> Vec<Hit> {
    hits.sort_by_key(|h| h.at);
    let same = crate::samples(SAME, rate);
    let mut merged: Vec<Hit> = Vec::with_capacity(hits.len());
    for h in hits {
        match merged.last_mut() {
            Some(last) if h.at.abs_diff(last.at) <= same => {
                last.from_trigger |= h.from_trigger;
                last.from_mic |= h.from_mic;
                last.confidence = if last.from_trigger && last.from_mic {
                    BOTH
                } else {
                    last.confidence.max(h.confidence)
                };
                last.peak = last.peak.max(h.peak);
            }
            _ => merged.push(h),
        }
    }
    let mut peaks: Vec<f32> = merged.iter().map(|h| h.peak).collect();
    peaks.sort_by(f32::total_cmp);
    if let Some(median) = peaks.get(peaks.len() / 2) {
        let floor = f64::from(*median) * crate::gain(-GHOST_DB);
        for h in &mut merged {
            h.ghost = f64::from(h.peak) < floor;
        }
    }
    pair_flams(&mut merged, rate);
    merged
}

/// Mark each two neighbouring strokes nearer than [`FLAM`] as a flam's
/// two halves.
pub fn pair_flams(hits: &mut [Hit], rate: f64) {
    let window = crate::samples(FLAM, rate);
    let pairs: Vec<(usize, usize)> = hits
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| match w {
            [a, b] if b.at.saturating_sub(a.at) < window => Some((i, i.saturating_add(1))),
            _ => None,
        })
        .collect();
    for (a, b) in pairs {
        if let Some(h) = hits.get_mut(a) {
            h.flam_with = Some(b);
        }
        if let Some(h) = hits.get_mut(b) {
            h.flam_with = Some(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(sig: &mut [f32], at: usize, peak: f32) {
        for (i, s) in sig.iter_mut().skip(at).take(400).enumerate() {
            *s = peak * (-num_traits::ToPrimitive::to_f32(&i).unwrap() / 80.0).exp();
        }
    }

    #[test]
    fn both_trigger_only_and_mic_only_are_kept_and_told_apart() {
        let rate = 48_000.0;
        let mut mic = vec![0.0_f32; 48_000];
        for at in [4_800, 14_400, 24_000] {
            click(&mut mic, at, 0.5);
        }
        let hits = fuse(&mic, rate, &[4_790, 14_390, 40_000], &[4_810, 24_010]);
        let kinds: Vec<(bool, bool)> = hits.iter().map(|h| (h.from_trigger, h.from_mic)).collect();
        assert_eq!(
            kinds,
            vec![(true, true), (true, false), (false, true), (true, false)]
        );
        assert!(hits[0].at.abs_diff(4_800) < 30, "{:?}", hits[0]);
        assert!((hits[0].confidence - BOTH).abs() < 1e-9);
    }

    #[test]
    fn a_flam_is_paired() {
        let rate = 48_000.0;
        let mut mic = vec![0.0_f32; 48_000];
        click(&mut mic, 4_800, 0.1);
        click(&mut mic, 5_500, 0.5);
        let hits = fuse(&mic, rate, &[4_800, 5_500], &[4_800, 5_500]);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].flam_with, Some(1));
        assert_eq!(hits[1].flam_with, Some(0));
    }
}
