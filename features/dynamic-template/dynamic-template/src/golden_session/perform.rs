//! Performed media: the song sung and played, for the tracks whose audio
//! has to MEAN something.
//!
//! Most of the fixture's audio is a second of tone (`media`): enough to
//! open, to draw, to compare. A take you line a double up against needs
//! more — a line that moves, a double that is not quite on it — so the
//! lead vocals, the BGVs, the choir and the guitars play the song
//! (`golden_session::song`): the singers its melody, at their part's
//! interval, a double a little late and a little sharp, drifting; the
//! guitars its chords, strummed. One file per performance, shared by
//! the tracks that captured it (a guitar's mics, a singer's languages),
//! the length of the song, and each item reads its own stretch of it.

use num_traits::ToPrimitive;

use super::shape::{Section, Shape};
use super::song::{chord_at, melody_notes};

/// The performed media's sample rate: a voice's harmonics and a guitar's
/// strings with room to spare, at a quarter of the size.
pub const RATE: u32 = 11_025;

/// What a track performs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Perf {
    /// The melody sung `shift` semitones off; a double sits late, sharp
    /// and drifting against its lead.
    Sung { shift: f64, double: bool },
    /// The chords strummed, `shift` semitones up; `late` the second side
    /// of a doubled part.
    Strum { shift: f64, late: bool },
}

/// The performance a track's audio is, and the file it shares with the
/// tracks that captured the same one: `None` for a track whose audio is
/// only a tone.
#[must_use]
pub fn performance(path: &str) -> Option<(String, Perf)> {
    let parts: Vec<&str> = path.split('/').collect();
    let key = |n: usize| {
        parts
            .iter()
            .take(n)
            .map(|p| p.to_ascii_lowercase().replace(' ', "-"))
            .collect::<Vec<_>>()
            .join("-")
    };
    match parts.as_slice() {
        ["Vocals", _singer, take @ ("Main" | "DBL"), ..] => Some((
            format!("perf-{}", key(3)),
            Perf::Sung {
                shift: 0.0,
                double: *take == "DBL",
            },
        )),
        ["BGVs", part, ..] => {
            let shift = match *part {
                "Octave Up" => 12.0,
                "Octave Down" => -12.0,
                "Higher Harmony" => 4.0,
                "Lower Harmony" => -3.0,
                _ => 7.0,
            };
            Some((
                format!("perf-{}", key(2)),
                Perf::Sung {
                    shift,
                    double: false,
                },
            ))
        }
        ["Choir", part, ..] => {
            let shift = match *part {
                "Alto" => -5.0,
                "Tenor" => -12.0,
                "Bass" => -19.0,
                _ => 0.0,
            };
            Some((
                format!("perf-{}", key(2)),
                Perf::Sung {
                    shift,
                    double: false,
                },
            ))
        }
        ["Electric" | "Acoustic", ..] => {
            // Up to the side (`L`/`R`) a doubled part was played on; the
            // mics under it captured the same take.
            let side = parts.iter().position(|p| *p == "L" || *p == "R");
            let depth = side.map_or_else(|| parts.len().min(3), |s| s.saturating_add(1));
            let shift = if parts.contains(&"Octave") { 12.0 } else { 0.0 };
            let late = parts.contains(&"R");
            Some((format!("perf-{}", key(depth)), Perf::Strum { shift, late }))
        }
        _ => None,
    }
}

/// A pitch, in semitones from A4, as a frequency.
fn hz(semitones_from_a4: f64) -> f64 {
    440.0 * (semitones_from_a4 / 12.0).exp2()
}

/// The song's beats as seconds.
fn seconds(beats: u32, bpm: f64) -> f64 {
    f64::from(beats) * 60.0 / bpm.max(1.0)
}

/// How many samples a song of `bars` lasts.
fn length(bars: u32, bpm: f64) -> usize {
    (seconds(bars.saturating_mul(4), bpm) * f64::from(RATE))
        .ceil()
        .to_usize()
        .unwrap_or(0)
}

/// `perf` over the song `shape` is: mono samples at [`RATE`].
#[must_use]
pub fn render(perf: Perf, shape: &Shape) -> Vec<i16> {
    let mut buf = vec![0.0_f64; length(shape.bars, shape.bpm)];
    match perf {
        Perf::Sung { shift, double } => sing(&mut buf, shape, shift, double),
        Perf::Strum { shift, late } => strum(
            &mut buf,
            &shape.sections,
            shape.bars,
            shape.bpm,
            shift,
            late,
        ),
    }
    let peak = buf.iter().fold(0.0_f64, |m, s| m.max(s.abs())).max(1e-9);
    buf.iter()
        .map(|s| (s / peak * 26_000.0).round().to_i16().unwrap_or(0))
        .collect()
}

/// The melody, sung: a voice of four harmonics with a vibrato that comes
/// in after the onset, shaped by a short attack and release; a double
/// 30 ms late, wandering ±12 ms, 8 cents sharp.
fn sing(buf: &mut [f64], shape: &Shape, shift: f64, double: bool) {
    let rate = f64::from(RATE);
    let (late, sharp) = if double { (0.030, 0.08) } else { (0.0, 0.0) };
    for (start, len, pitch) in melody_notes(&shape.sections, shape.bars, 60) {
        let t0 = seconds(start, shape.bpm) + late;
        let t1 = seconds(start.saturating_add(len), shape.bpm) + late - 0.04;
        let wander = if double {
            0.012 * (t0 * 0.7).sin()
        } else {
            0.0
        };
        let (t0, t1) = (t0 + wander, t1 + wander);
        let note = f64::from(pitch) - 69.0 + shift + sharp;
        let from = (t0 * rate).max(0.0).to_usize().unwrap_or(0);
        let to = (t1 * rate).max(0.0).to_usize().unwrap_or(0).min(buf.len());
        let Some(span) = buf.get_mut(from..to) else {
            continue;
        };
        let mut phase = 0.0_f64;
        for (i, out) in span.iter_mut().enumerate() {
            let t = i.to_f64().unwrap_or(0.0) / rate;
            let held = (t - 0.15).max(0.0);
            let vibrato = 0.2 * (held * 5.5 * std::f64::consts::TAU).sin() * (held * 4.0).min(1.0);
            phase += hz(note + vibrato) / rate * std::f64::consts::TAU;
            let remaining = (t1 - t0) - t;
            let envelope = (t / 0.04).min(1.0) * (remaining / 0.06).clamp(0.0, 1.0);
            let voice = harmonics(phase, &[1.0, 0.5, 0.3, 0.15]);
            *out += 0.6 * envelope * voice;
        }
    }
}

/// The chords, strummed: on every beat the bar's chord plucked low to
/// high 12 ms apart (high to low on the off-beats), each string a decaying
/// tone; `late` plays it 18 ms behind.
fn strum(buf: &mut [f64], sections: &[Section], bars: u32, bpm: f64, shift: f64, late: bool) {
    let rate = f64::from(RATE);
    let behind = if late { 0.018 } else { 0.0 };
    let beat = seconds(1, bpm);
    for b in 0..bars.saturating_mul(4) {
        let mut strings: Vec<f64> = {
            let chord = chord_at(sections, b.checked_div(4).unwrap_or(0));
            let mut tones = chord_tones(chord);
            tones.push(tones.first().copied().unwrap_or(52.0) + 12.0);
            tones.iter().map(|p| p - 69.0 + shift).collect()
        };
        let up = b.is_multiple_of(2);
        if !up {
            strings.reverse();
        }
        let gain = if up { 1.0 } else { 0.6 };
        for (k, note) in strings.into_iter().enumerate() {
            let onset = 0.012_f64.mul_add(k.to_f64().unwrap_or(0.0), seconds(b, bpm) + behind);
            let from = (onset * rate).to_usize().unwrap_or(0);
            let to = ((onset + beat * 1.5) * rate)
                .to_usize()
                .unwrap_or(0)
                .min(buf.len());
            let Some(span) = buf.get_mut(from..to) else {
                continue;
            };
            let f = hz(note);
            for (i, out) in span.iter_mut().enumerate() {
                let t = i.to_f64().unwrap_or(0.0) / rate;
                let phase = t * f * std::f64::consts::TAU;
                let pluck = harmonics(phase, &[1.0, 0.45, 0.2]);
                *out += 0.25 * gain * (-t * 5.0).exp() * pluck;
            }
        }
    }
}

/// A tone at `phase` with its harmonics: `amps[n]` the (n+1)th's level.
fn harmonics(phase: f64, amps: &[f64]) -> f64 {
    amps.iter().zip(1_u32..).fold(0.0, |sum, (amp, n)| {
        amp.mul_add((f64::from(n) * phase).sin(), sum)
    })
}

/// A chord's three tones, low (around E3), as MIDI pitches.
fn chord_tones(chord: super::song::Degree) -> Vec<f64> {
    chord.pitches(52, 3).into_iter().map(f64::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A singer's languages and a guitar's mics share their take's file;
    /// a take and its double do not.
    #[test]
    fn tracks_that_captured_one_performance_share_it() {
        let file = |p: &str| performance(p).map(|(k, _)| k);
        assert_eq!(file("Vocals/Ron/Main/EN"), file("Vocals/Ron/Main/ES"));
        assert_ne!(file("Vocals/Ron/Main/EN"), file("Vocals/Ron/DBL/EN"));
        assert_eq!(
            file("Electric/Rhythm/Main/L/Amp 1 Royer"),
            file("Electric/Rhythm/Main/L/DI")
        );
        assert_ne!(
            file("Electric/Rhythm/Main/L/DI"),
            file("Electric/Rhythm/Main/R/DI")
        );
        assert_eq!(file("Drum Kit/Kick/Sum/In"), None);
        assert!(matches!(
            performance("Vocals/Ron/DBL/EN"),
            Some((_, Perf::Sung { double: true, .. }))
        ));
    }

    /// A take is the song's length, sings where the melody is, and its
    /// double is not the same samples.
    #[test]
    fn a_take_is_the_song_and_its_double_differs() {
        let shape = super::super::shape::maximal();
        let main = render(
            Perf::Sung {
                shift: 0.0,
                double: false,
            },
            &shape,
        );
        let dbl = render(
            Perf::Sung {
                shift: 0.0,
                double: true,
            },
            &shape,
        );
        assert_eq!(main.len(), length(shape.bars, shape.bpm));
        assert!(main.iter().any(|s| s.unsigned_abs() > 10_000), "it sings");
        assert_ne!(main, dbl);
    }
}
