//! Phrase analysis / micro-dynamics / fade shapes — port of `css_engine.lua`
//! §6b (`smooth01`, `metricWeight`, `buildPhrasing`, `archAt`, `swellAt`,
//! `microFor`, `fadeOutProgress`, `fadeShape`, `vibBloomAt`).

use super::{FadeMode, WNote};
use crate::config::{Config, PhraseConfig};

pub const EPS: f64 = 1e-6;

/// Smooth 0..1 ramp (raised cosine).
pub fn smooth01(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    0.5 - 0.5 * (std::f64::consts::PI * u).cos()
}

/// Metric weight of a note (0 = weak/offbeat, 1 = downbeat) from bar position.
pub fn metric_weight(n: &WNote) -> f64 {
    let beats = if n.beats == 0 { 4 } else { n.beats };
    let bt = if n.beat_type == 0 { 4 } else { n.beat_type };
    let beat_len = 4.0 / bt as f64;
    let idx = n.beat_qn / beat_len;
    let nearest = (idx + 0.5).floor();
    if (idx - nearest).abs() > 0.05 {
        return 0.2; // offbeat
    }
    let b = (nearest as i64).rem_euclid(beats as i64) as u32;
    if b == 0 {
        return 1.0;
    }
    if beats % 2 == 0 && b == beats / 2 {
        return 0.7; // mid-bar (beat 3 of 4/4)
    }
    if beats == 6 && b == 3 {
        return 0.7; // 6/8 secondary
    }
    if beats == 9 && (b == 3 || b == 6) {
        return 0.65;
    }
    if beats == 12 && b.is_multiple_of(3) {
        return 0.65;
    }
    0.45
}

/// A phrase segment on one channel.
#[derive(Debug, Clone, Copy)]
pub struct Phrase {
    pub t0: f64,
    pub t1: f64,
    pub peak_t: f64,
}

/// Segment a channel's notes (indices sorted by onset) into phrases and
/// annotate each note with its metric/contour/leap weights and phrase id.
/// Appends phrases to `phrases` and returns nothing (annotations live on notes).
pub fn build_phrasing(
    notes: &mut [WNote],
    list: &[usize],
    p: &PhraseConfig,
    phrases: &mut Vec<Phrase>,
) {
    // Split into phrase index-groups.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, &ni) in list.iter().enumerate() {
        let boundary = if i == 0 {
            true
        } else {
            let prev = &notes[list[i - 1]];
            let n = &notes[ni];
            let gap = n.onset - (prev.onset + prev.dur);
            gap > p.phrase_gap_qn
                || prev.dur >= p.long_note_qn
                || prev.art.has("fermata")
                // slurs (when present) define phrases: a breath at each slur
                // end, and a new slur starts a new phrase.
                || (prev.slurred && !prev.slur_continues)
                || (n.slur_start && !prev.slur_continues)
        };
        if boundary {
            groups.push(Vec::new());
        }
        groups.last_mut().unwrap().push(ni);
    }

    for group in groups {
        let first = &notes[group[0]];
        let last = &notes[*group.last().unwrap()];
        let t0 = first.onset;
        let t1 = last.onset + last.dur;
        let (mut sum, mut lo, mut hi) = (0.0, f64::INFINITY, f64::NEG_INFINITY);
        let mut hi_pitch = f64::NEG_INFINITY;
        let mut peak_t = t0;
        for &ni in &group {
            let pitch = notes[ni].pitch as f64;
            sum += pitch;
            lo = lo.min(pitch);
            hi = hi.max(pitch);
            if pitch > hi_pitch {
                hi_pitch = pitch;
                peak_t = notes[ni].onset;
            }
        }
        let mean = sum / group.len() as f64;
        let half_range = ((hi - lo) / 2.0).max(1.0);
        let def_peak = t0 + p.arch_peak_frac * (t1 - t0);
        let phrase = Phrase {
            t0,
            t1,
            peak_t: p.peak_blend * peak_t + (1.0 - p.peak_blend) * def_peak,
        };
        let phrase_id = phrases.len();
        phrases.push(phrase);

        for (j, &ni) in group.iter().enumerate() {
            let c = ((notes[ni].pitch as f64 - mean) / half_range).clamp(-1.0, 1.0);
            let mut leap = 0.0;
            if j > 0 {
                let d = notes[ni].pitch - notes[group[j - 1]].pitch;
                let ad = d.abs() as f64;
                if ad > p.leap_threshold {
                    leap = ((ad - p.leap_threshold) / 8.0).min(1.0);
                    if d < 0 {
                        leap *= 0.5; // descending leaps gentler
                    }
                }
            }
            let n = &mut notes[ni];
            n.contour = c;
            n.leap = leap;
            n.metric = metric_weight_of(n);
            n.phrase = Some(phrase_id);
            n.phrase_end = j == group.len() - 1;
        }
    }
}

fn metric_weight_of(n: &WNote) -> f64 {
    metric_weight(n)
}

/// Phrase arch contribution (CC) at time t.
pub fn arch_at(ph: &Phrase, t: f64, p: &PhraseConfig) -> f64 {
    if ph.t1 - ph.t0 < EPS {
        return 0.0;
    }
    if t <= ph.peak_t {
        let s = smooth01((t - ph.t0) / (ph.peak_t - ph.t0).max(EPS));
        return -p.arch_start_dip + (p.arch_gain + p.arch_start_dip) * s;
    }
    let s = smooth01((t - ph.peak_t) / (ph.t1 - ph.peak_t).max(EPS));
    p.arch_gain - (p.arch_gain + p.arch_end_dip) * s
}

/// Intra-note bloom (messa di voce) on long notes; phrase-final notes decay sooner.
pub fn swell_at(n: &WNote, t: f64, p: &PhraseConfig) -> f64 {
    if n.dur < p.swell_min_qn {
        return 0.0;
    }
    let u = ((t - n.onset) / n.dur).clamp(0.0, 1.0);
    let peak = if n.phrase_end { 0.35 } else { 0.55 };
    let shape = if u <= peak {
        smooth01(u / peak)
    } else {
        1.0 - smooth01((u - peak) / (1.0 - peak))
    };
    p.swell_gain * shape * if n.phrase_end { 0.8 } else { 1.0 }
}

/// Total micro-dynamic offset (CC) for a note at time t.
pub fn micro_for(n: &WNote, phrases: &[Phrase], t: f64, p: &PhraseConfig) -> f64 {
    let mut m = 0.0;
    if p.do_arch {
        if let Some(ph) = n.phrase.map(|i| &phrases[i]) {
            m += arch_at(ph, t, p);
        }
    }
    if p.do_metric {
        m += p.metric_gain * (n.metric - 0.5) * 2.0;
    }
    if p.do_contour {
        m += p.contour_gain * n.contour;
    }
    if p.do_leap {
        m += p.leap_gain * n.leap;
    }
    if p.do_swell {
        m += swell_at(n, t, p);
    }
    m *= p.intensity;
    m.clamp(-p.micro_max, p.micro_max)
}

/// Fade-out progress 0..1 across a normalized span. Default is a symmetric
/// smoothstep; `fade_out_bias > 1` leans the drop LATE.
pub fn fade_out_progress(un: f64, cfg: &Config) -> f64 {
    let un = un.clamp(0.0, 1.0);
    if cfg.fade_out_bias <= 1.0 {
        smooth01(un)
    } else {
        un.powf(cfg.fade_out_bias)
    }
}

/// Fade shape at fraction u (0..1) through a note. Returns (cc1 value,
/// volume fraction 0..1).
pub fn fade_shape(mode: FadeMode, u: f64, peak: f64, cfg: &Config) -> (f64, f64) {
    let u = u.clamp(0.0, 1.0);
    let floor = cfg.fade_niente_floor;
    match mode {
        FadeMode::In => {
            let s = smooth01(u / cfg.fade_in_reach_frac);
            (floor + (peak - floor) * s, s)
        }
        FadeMode::Out => {
            let s = fade_out_progress(u / cfg.fade_reach_frac, cfg);
            (peak + (floor - peak) * s, 1.0 - s)
        }
        FadeMode::Swell => {
            let (in_f, out_start) = (0.30, 0.55);
            if u < in_f {
                let s = smooth01(u / in_f);
                (floor + (peak - floor) * s, s)
            } else if u > out_start {
                let s = fade_out_progress(
                    (u - out_start) / (cfg.fade_reach_frac - out_start).max(EPS),
                    cfg,
                );
                (peak + (floor - peak) * s, 1.0 - s)
            } else {
                (peak, 1.0)
            }
        }
    }
}

/// Vibrato bloom (CC delta) at the attack of a sustained note (strings xfade only).
pub fn vib_bloom_at(n: &WNote, t: f64, p: &PhraseConfig) -> f64 {
    let bt = n.dur.min(p.vib_bloom_time_qn);
    if bt <= EPS {
        return 0.0;
    }
    let u = (t - n.onset) / bt;
    if !(0.0..1.0).contains(&u) {
        return 0.0;
    }
    -p.vib_bloom_depth * (1.0 - smooth01(u)) * p.intensity
}

/// Interpolated CC1 dynamics value at a QN position — port of `dynInterp`.
pub fn dyn_interp(anchors: &[(f64, f64)], qn: f64) -> Option<f64> {
    if anchors.is_empty() {
        return None;
    }
    if qn <= anchors[0].0 {
        return Some(anchors[0].1);
    }
    let last = anchors[anchors.len() - 1];
    if qn >= last.0 {
        return Some(last.1);
    }
    for w in anchors.windows(2) {
        let (a, b) = (w[0], w[1]);
        if qn >= a.0 && qn <= b.0 {
            if b.0 - a.0 < EPS {
                return Some(b.1);
            }
            let t = (qn - a.0) / (b.0 - a.0);
            return Some(a.1 + (b.1 - a.1) * t);
        }
    }
    Some(last.1)
}
