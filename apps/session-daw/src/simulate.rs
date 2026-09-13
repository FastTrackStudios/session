//! Audio that isn't there, so the panels that show audio can be seen.
//!
//! Three things in the rack only move when signal moves: the mixer's
//! meters, the EQ's spectrum and the compressor's waveform. A template
//! session has no media, a session being laid out has nothing playing,
//! and a machine with no audio device has nothing at all — so the
//! panels that exist to show a signal are the panels you cannot look
//! at while you build them.
//!
//! This is a signal. Not a recording and not noise: a deterministic
//! function of `(track, seconds)` that behaves enough like a drum kit
//! to exercise every path a real one would. Deterministic because a
//! picture you cannot reproduce is a picture you cannot compare — the
//! same second gives the same frame on every machine, which is what
//! lets a screenshot be a test.
//!
//! # It is not a fake meter
//!
//! It feeds the SAME inputs the engine feeds: a `TrackLevels` for the
//! meters and a dB-per-bin spectrum for the EQ. Nothing downstream
//! knows the difference, which is the only way a simulation is worth
//! having — one that took a shortcut past the real path would prove
//! the shortcut works.

use std::f64::consts::TAU;

/// How many bins the spectrum carries.
///
/// Log-spaced across the audible range by the painter, so this is a
/// resolution rather than a bandwidth: 96 puts about a bin every eighth
/// of an octave, which is finer than a strip's EQ panel can draw and
/// about right for one at a focus width.
pub const BINS: usize = 96;

/// One track's simulated signal at one instant.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The peak, linear 0..1 — what a meter and the compressor's
    /// waveform read.
    pub peak: f32,
    /// The spectrum, in dB, one value per bin.
    pub spectrum: Vec<f32>,
}

/// The kind of source a track stands in for.
///
/// Drawn from its INDEX rather than its name, so the simulation is
/// stable across a rename and works on a session whose tracks are
/// called things this file has never heard of. The point is variety
/// that looks like a kit, not a classifier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Voice {
    /// Low, slow, loud — a kick or a floor tom.
    Low,
    /// Mid-forward with a fast decay — a snare.
    Mid,
    /// Bright and busy — cymbals and overheads.
    High,
    /// Broad and steady — a room, a bus, a pad.
    Broad,
}

impl Voice {
    const fn of(track: usize) -> Self {
        match track % 4 {
            0 => Self::Low,
            1 => Self::Mid,
            2 => Self::High,
            _ => Self::Broad,
        }
    }

    /// Where its energy sits, in Hz.
    const fn centre(self) -> f64 {
        match self {
            Self::Low => 80.0,
            Self::Mid => 400.0,
            Self::High => 6_000.0,
            Self::Broad => 800.0,
        }
    }

    /// How wide that peak is, in octaves.
    const fn width(self) -> f64 {
        match self {
            Self::Low => 1.2,
            Self::Mid => 1.6,
            Self::High => 2.2,
            Self::Broad => 3.5,
        }
    }

    /// How many times a second it is struck.
    const fn rate(self) -> f64 {
        match self {
            Self::Low => 2.0,
            Self::Mid => 1.0,
            Self::High => 4.0,
            Self::Broad => 0.5,
        }
    }

    /// And how fast it decays, as a share of its own period.
    const fn decay(self) -> f64 {
        match self {
            Self::Low => 0.35,
            Self::Mid => 0.25,
            Self::High => 0.12,
            Self::Broad => 0.9,
        }
    }
}

/// The signal on `track` at `seconds`.
///
/// Pure: the same arguments give the same frame forever, on every
/// machine. That is what lets a screenshot of a moving panel be
/// compared against another one.
#[must_use]
pub fn frame(track: usize, seconds: f64) -> Frame {
    let voice = Voice::of(track);
    let envelope = envelope(voice, track, seconds);
    Frame {
        peak: crate::mcp::f64_to_f32(envelope),
        spectrum: spectrum(voice, envelope, seconds),
    }
}

/// A struck envelope: a sharp attack every period, decaying after it.
///
/// Offset per track by an irrational-ish step so no two tracks land on
/// the same beat — a mixer where every meter jumps together is one
/// gradient, and the thing being looked at is how tracks differ.
fn envelope(voice: Voice, track: usize, seconds: f64) -> f64 {
    let period = 1.0 / voice.rate();
    let offset = crate::num::coord(track) * 0.6180339887 * period;
    let since = (seconds + offset).rem_euclid(period) / period;
    // A quiet floor under it, so a meter never reads absolute silence
    // between hits — a room mic does not, and a display that drops to
    // nothing between beats reads as a dropout.
    //
    // The numbers are in LINEAR peak because that is what a meter
    // takes, and the displays log-scale them: a floor of 0.004 is −48
    // dBFS and a hit of 0.7 is −3, which is the range a recorded track
    // actually covers. A floor of 0.04 put every trace above half the
    // display and turned the compressor's waveform into a wall.
    let hit = (-since / voice.decay()).exp();
    let floor = 0.004;
    // A slow swell over the whole simulation, so a long look is not a
    // loop: the kit gets louder and quieter the way a performance does.
    let swell = 0.75 + 0.25 * (TAU * seconds / 19.0).sin();
    (floor + hit * 0.7 * swell).clamp(0.0, 1.0)
}

/// The spectrum at this instant, in dB per bin.
///
/// A tilted floor with the voice's own resonance on it, and a wobble
/// so the picture is alive between hits. Tilted because real programme
/// material is: energy falls with frequency, and a flat spectrum reads
/// as noise rather than as music.
fn spectrum(voice: Voice, envelope: f64, seconds: f64) -> Vec<f32> {
    let (low, high) = (20.0_f64, 20_000.0_f64);
    let (log_low, log_high) = (low.log10(), high.log10());
    let centre = voice.centre().log10();
    let width = voice.width() * 0.301;
    (0..BINS)
        .map(|i| {
            let t = crate::num::coord(i) / crate::num::coord(BINS.saturating_sub(1).max(1));
            let log_f = t.mul_add(log_high - log_low, log_low);
            // The tilt: programme material loses energy with
            // frequency, and a flat spectrum reads as noise rather
            // than as music. Gentle, because a steeper one buries
            // every voice's own resonance under the bottom octave and
            // every track looks like a kick.
            let tilt = (log_low - log_f) * 5.0;
            // The voice's resonance, as a bell in log-frequency.
            let away = (log_f - centre) / width;
            let bump = 18.0 * (-away * away).exp();
            // And a shimmer that moves, different per bin, so the
            // spectrum is never two frames the same.
            let shimmer = 3.5 * (TAU * (seconds * 1.7 + t * 9.0)).sin();
            // Sat on the display's own window rather than on absolute
            // dBFS. A strip's EQ panel shows ±18 dB; a spectrum drawn
            // in real dBFS puts everything but the loudest peak on the
            // floor, which reads as one spike on a flat line rather
            // than as the shape of a sound.
            let loud = 24.0 * envelope.max(1e-3).log10() / 3.0;
            let level = tilt + bump + shimmer + loud;
            crate::mcp::f64_to_f32(level.clamp(-40.0, 16.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{BINS, frame};

    /// Deterministic, which is what makes a screenshot of a moving
    /// panel worth comparing to another one.
    #[test]
    fn the_same_moment_is_the_same_signal() {
        let a = frame(3, 1.25);
        let b = frame(3, 1.25);
        assert!((a.peak - b.peak).abs() < f32::EPSILON);
        assert_eq!(a.spectrum, b.spectrum);
    }

    /// It moves — a still simulation would exercise none of the paths
    /// it exists to exercise.
    #[test]
    fn it_moves() {
        let peaks: Vec<f32> = (0..60).map(|i| frame(0, f64::from(i) / 30.0).peak).collect();
        let spread = peaks.iter().copied().fold(f32::MIN, f32::max)
            - peaks.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread > 0.4, "the meter barely moved: {spread}");

        let one = frame(0, 0.0).spectrum;
        let two = frame(0, 0.3).spectrum;
        assert_ne!(one, two, "the spectrum stood still");
    }

    /// No two tracks are in step, so a mixer of them is a mixer rather
    /// than one gradient.
    #[test]
    fn tracks_differ_from_each_other() {
        let at = |track| frame(track, 0.4).peak;
        let peaks: Vec<f32> = (0..8).map(at).collect();
        let same = peaks
            .windows(2)
            .filter(|w| (w[0] - w[1]).abs() < 1e-4)
            .count();
        assert!(same < 2, "too many tracks share a level: {peaks:?}");
    }

    /// Every value stays in the range its consumer accepts, so what is
    /// measured is drawing and never clamping.
    #[test]
    fn nothing_leaves_its_range() {
        for step in 0..120 {
            let f = frame(step % 7, crate::num::coord(step) / 20.0);
            assert!((0.0..=1.0).contains(&f.peak), "{}", f.peak);
            assert_eq!(f.spectrum.len(), BINS);
            for db in &f.spectrum {
                assert!((-40.0..=16.0).contains(db), "{db}");
            }
        }
    }

    /// A voice's energy lands where its name says. A kick whose
    /// spectrum peaked at 6 kHz would look like a cymbal, and the
    /// point of the simulation is that a mixer of it looks like a kit.
    #[test]
    fn each_voice_peaks_in_its_own_register() {
        let brightest = |track: usize| {
            let s = frame(track, 0.0).spectrum;
            s.iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map_or(0, |(i, _)| i)
        };
        // Track 0 is the low voice and track 2 the high one.
        assert!(
            brightest(0) < brightest(2),
            "the low voice was brighter than the high one"
        );
    }
}
