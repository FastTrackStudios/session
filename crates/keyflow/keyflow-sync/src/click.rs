//! Click-track tempo + beat-grid recovery.
//!
//! A worship MultiTracks "Click" stem is a steady metronome: sharp, evenly
//! spaced transients. We recover the grid by energy peak-picking (each click is
//! a clean onset), taking the median inter-onset interval as the beat period,
//! and assuming a fixed meter for the bar grid. Pure Rust, no model — good
//! enough to anchor spoken guide cues (see [`crate::guide_chart`]) to measures.

use crate::audio::AudioBuffer;

/// The recovered beat/bar grid of a click track.
#[derive(Debug, Clone)]
pub struct ClickGrid {
    /// Detected tempo in beats (quarter notes) per minute.
    pub bpm: f32,
    /// Timeline position (seconds) of the first detected click.
    pub first_beat_sec: f32,
    /// Assumed beats per bar (meter numerator) — the click alone doesn't carry
    /// meter, so this is supplied by the caller (default 4).
    pub beats_per_bar: u32,
    /// Every detected beat onset, in seconds.
    pub beats: Vec<f32>,
    /// Downbeat (bar-start) times, seconds — every `beats_per_bar`-th beat.
    pub downbeats: Vec<f32>,
}

impl ClickGrid {
    /// Seconds per bar at the detected tempo.
    pub fn bar_seconds(&self) -> f32 {
        self.beats_per_bar as f32 * 60.0 / self.bpm.max(1.0)
    }

    /// Snap an absolute time to the nearest bar index (0-based from the first
    /// downbeat). Bars before the first downbeat clamp to 0.
    pub fn bar_at(&self, time_sec: f32) -> u32 {
        let rel = (time_sec - self.first_beat_sec) / self.bar_seconds();
        rel.round().max(0.0) as u32
    }
}

/// Energy peak-pick the click onsets in `samples`.
///
/// Frames the signal into ~10 ms hops, takes the per-hop peak amplitude as the
/// envelope, then keeps local maxima above an adaptive threshold with a
/// refractory gap (so a single click isn't counted twice and tempos above the
/// gap's implied ceiling are rejected).
fn detect_onsets(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() || sample_rate == 0 {
        return Vec::new();
    }
    let hop = ((sample_rate as f32 * 0.010) as usize).max(1); // ~10 ms
    let env: Vec<f32> = samples
        .chunks(hop)
        .map(|c| c.iter().fold(0.0f32, |m, x| m.max(x.abs())))
        .collect();
    let peak = env.iter().cloned().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return Vec::new();
    }
    // Threshold at a fraction of the loudest click; clicks are impulsive so a
    // generous fraction separates them cleanly from bleed/noise.
    let thresh = peak * 0.30;
    // Refractory: ignore onsets within 150 ms of the last (ceiling ~400 bpm).
    let refractory = ((sample_rate as f32 * 0.150) / hop as f32).ceil() as usize;

    let mut onsets = Vec::new();
    let mut last: Option<usize> = None;
    for i in 1..env.len().saturating_sub(1) {
        let e = env[i];
        if e < thresh {
            continue;
        }
        // Local maximum within the hop neighborhood.
        if e < env[i - 1] || e < env[i + 1] {
            continue;
        }
        if let Some(l) = last {
            if i - l < refractory {
                // Keep the louder of the two competing peaks.
                if e > env[l] {
                    onsets.pop();
                    onsets.push(i as f32 * hop as f32 / sample_rate as f32);
                    last = Some(i);
                }
                continue;
            }
        }
        onsets.push(i as f32 * hop as f32 / sample_rate as f32);
        last = Some(i);
    }
    onsets
}

fn median(mut v: Vec<f32>) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

/// Recover the beat/bar grid from a click-track buffer. `beats_per_bar` is the
/// assumed meter numerator (4 for 4/4). Returns `bpm = 0` if no clicks found.
pub fn detect_click_grid(audio: &AudioBuffer, beats_per_bar: u32) -> ClickGrid {
    let bpb = beats_per_bar.max(1);
    let beats = detect_onsets(&audio.samples, audio.sample_rate);
    let iois: Vec<f32> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    let raw_bpm = median(iois).map(|m| 60.0 / m.max(1e-3)).unwrap_or(0.0);
    // Octave-correct: a click that plays eighth/sixteenth subdivisions reads as
    // 2×/4× the quarter-note tempo. Fold into a musical quarter-note range
    // [70,160) so a 127-bpm song clicking eighths (~254) resolves to ~127.
    let mut bpm = raw_bpm;
    while bpm >= 160.0 {
        bpm /= 2.0;
    }
    while bpm > 0.0 && bpm < 70.0 {
        bpm *= 2.0;
    }
    let first_beat_sec = beats.first().copied().unwrap_or(0.0);
    let downbeats: Vec<f32> = beats.iter().step_by(bpb as usize).copied().collect();
    ClickGrid {
        bpm,
        first_beat_sec,
        beats_per_bar: bpb,
        beats,
        downbeats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a click track: an impulse every beat at `bpm`, and verify the
    /// detector recovers the tempo and beat count.
    fn synth_clicks(bpm: f32, beats: usize, sample_rate: u32) -> AudioBuffer {
        let period = (sample_rate as f32 * 60.0 / bpm) as usize;
        let mut s = vec![0.0f32; period * beats + sample_rate as usize];
        for b in 0..beats {
            let at = b * period;
            // A short decaying tick so the envelope has a clear single peak.
            for k in 0..(sample_rate as usize / 100) {
                if at + k < s.len() {
                    let env = 1.0 - (k as f32 / (sample_rate as f32 / 100.0));
                    s[at + k] = env; // impulse-ish
                }
            }
        }
        AudioBuffer::new(s, sample_rate)
    }

    #[test]
    fn recovers_120_bpm() {
        let audio = synth_clicks(120.0, 16, 44_100);
        let grid = detect_click_grid(&audio, 4);
        assert!(
            (grid.bpm - 120.0).abs() < 2.0,
            "expected ~120 bpm, got {}",
            grid.bpm
        );
        assert!(grid.beats.len() >= 15, "found {} beats", grid.beats.len());
        assert_eq!(grid.beats_per_bar, 4);
        // 16 beats / 4 = 4 downbeats.
        assert!((3..=5).contains(&grid.downbeats.len()));
    }

    #[test]
    fn recovers_127_bpm() {
        let audio = synth_clicks(127.0, 24, 44_100);
        let grid = detect_click_grid(&audio, 4);
        assert!(
            (grid.bpm - 127.0).abs() < 2.5,
            "expected ~127 bpm, got {}",
            grid.bpm
        );
    }

    #[test]
    fn octave_corrects_eighth_note_clicks() {
        // A click that ticks eighths at 127 bpm fires at ~254 "beats"/min; the
        // detector should fold it back to the quarter-note tempo ~127.
        let audio = synth_clicks(254.0, 48, 44_100);
        let grid = detect_click_grid(&audio, 4);
        assert!(
            (grid.bpm - 127.0).abs() < 3.0,
            "expected ~127 after octave correction, got {}",
            grid.bpm
        );
    }

    #[test]
    fn bar_snapping() {
        let audio = synth_clicks(120.0, 16, 44_100);
        let grid = detect_click_grid(&audio, 4);
        // 120 bpm 4/4 → bar = 2 s. A cue at ~4 s is bar 2.
        assert_eq!(grid.bar_at(4.0), 2);
        assert_eq!(grid.bar_at(0.1), 0);
    }
}
