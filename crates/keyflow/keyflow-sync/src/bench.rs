//! Separation-quality metrics — the comparison harness.
//!
//! The benchmark article you're working from uses `mir_eval.bss_eval_sources`,
//! whose SDR involves a 512-tap least-squares projection of the estimate onto
//! the reference subspace. That's accurate but heavy and famously fiddly to
//! reproduce bit-for-bit. We implement the two metrics that are both robust and
//! cheap in pure Rust:
//!
//! * **SI-SDR** (scale-invariant SDR) — the modern de-facto standard for
//!   source separation. Invariant to a global gain mismatch between estimate
//!   and reference, so it doesn't punish a tool for outputting a quieter stem.
//! * **SNR-style SDR** — `10·log10(‖ref‖² / ‖ref−est‖²)`. Simple, gain
//!   sensitive; reported alongside SI-SDR for continuity with naive scripts.
//!
//! Neither equals `bss_eval` exactly — don't compare these numbers against
//! published `bss_eval` figures. They ARE directly comparable *between tools*
//! on the same reference, which is the whole point of a bake-off.

use crate::error::{Result, SyncError};

/// Scores for one estimated stem against a ground-truth reference. Higher dB is
/// better. Rough reading (matches the article's bands): `>8` professional,
/// `5..8` good, `2..5` audible artifacts, `<2` poor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StemScores {
    pub si_sdr: f32,
    pub sdr: f32,
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(&x, &y)| x as f64 * y as f64).sum()
}

fn energy(a: &[f32]) -> f64 {
    a.iter().map(|&x| x as f64 * x as f64).sum()
}

/// Scale-invariant SDR in dB. Projects the reference onto the estimate's
/// optimal-gain line, then measures target vs residual energy.
pub fn si_sdr(reference: &[f32], estimate: &[f32]) -> Result<f32> {
    let n = reference.len().min(estimate.len());
    if n == 0 {
        return Err(SyncError::Shape("empty signal in si_sdr".into()));
    }
    let reference = &reference[..n];
    let estimate = &estimate[..n];

    let ref_energy = energy(reference);
    if ref_energy <= f64::EPSILON {
        return Err(SyncError::Shape("silent reference in si_sdr".into()));
    }
    // alpha scales the reference to best match the estimate (gain invariance).
    let alpha = dot(estimate, reference) / ref_energy;
    let mut target_energy = 0.0f64;
    let mut noise_energy = 0.0f64;
    for i in 0..n {
        let target = alpha * reference[i] as f64;
        let noise = estimate[i] as f64 - target;
        target_energy += target * target;
        noise_energy += noise * noise;
    }
    if noise_energy <= f64::EPSILON {
        return Ok(f32::INFINITY);
    }
    Ok((10.0 * (target_energy / noise_energy).log10()) as f32)
}

/// SNR-style SDR in dB (gain sensitive).
pub fn sdr(reference: &[f32], estimate: &[f32]) -> Result<f32> {
    let n = reference.len().min(estimate.len());
    if n == 0 {
        return Err(SyncError::Shape("empty signal in sdr".into()));
    }
    let ref_energy = energy(&reference[..n]);
    if ref_energy <= f64::EPSILON {
        return Err(SyncError::Shape("silent reference in sdr".into()));
    }
    let mut residual = 0.0f64;
    for i in 0..n {
        let d = reference[i] as f64 - estimate[i] as f64;
        residual += d * d;
    }
    if residual <= f64::EPSILON {
        return Ok(f32::INFINITY);
    }
    Ok((10.0 * (ref_energy / residual).log10()) as f32)
}

/// Compute both metrics for one stem.
pub fn score_stem(reference: &[f32], estimate: &[f32]) -> Result<StemScores> {
    Ok(StemScores {
        si_sdr: si_sdr(reference, estimate)?,
        sdr: sdr(reference, estimate)?,
    })
}

/// A single tool's result for one stem, ready to drop into a comparison table.
#[derive(Debug, Clone)]
pub struct BenchRow {
    pub tool: String,
    pub stem: String,
    pub scores: StemScores,
    pub elapsed_secs: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_estimate_scores_infinite() {
        let r = vec![0.1, -0.2, 0.3, -0.4];
        assert!(si_sdr(&r, &r).unwrap().is_infinite());
        assert!(sdr(&r, &r).unwrap().is_infinite());
    }

    #[test]
    fn si_sdr_is_gain_invariant_but_sdr_is_not() {
        let r = vec![0.1, -0.2, 0.3, -0.4, 0.15];
        let louder: Vec<f32> = r.iter().map(|x| x * 2.0).collect();
        // A perfectly scaled copy is "perfect" to SI-SDR...
        assert!(si_sdr(&r, &louder).unwrap() > 100.0);
        // ...but the gain-sensitive SDR penalizes it.
        assert!(sdr(&r, &louder).unwrap() < 100.0);
    }

    #[test]
    fn worse_estimate_scores_lower() {
        let r = vec![0.5, -0.5, 0.5, -0.5, 0.4, -0.4];
        let good: Vec<f32> = r.iter().map(|x| x + 0.01).collect();
        let bad: Vec<f32> = r.iter().map(|x| x + 0.3).collect();
        assert!(si_sdr(&r, &good).unwrap() > si_sdr(&r, &bad).unwrap());
    }
}
