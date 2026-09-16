//! Triggering: a sample per hit, rendered from the hit list.
//!
//! # Why MIDI, and not a rendered audio track
//!
//! REAPER's own answer is to print audio: a sample dropped at each
//! detected transient. That severs the sample from the hit the moment
//! either moves, and re-rendering is the only way back.
//!
//! A MIDI item into the sampler keeps the link. The notes are
//! **derived** from the hit list, so a hit slipped in the editor moves
//! its sample with it and a hit removed takes its sample away — for
//! free, because there is nothing to keep in step. It is also cheap to
//! rewrite, editable by hand when one hit wants a different sample, and
//! the sampler already exists.
//!
//! # Why velocity is measured and never stored
//!
//! A stretch marker carries a position and nothing else, which is the
//! whole reason the hit list can be markers ([`crate::hits`]). Velocity
//! therefore cannot live on the hit — and should not, because a stored
//! velocity goes stale the instant the hit moves. It is measured from
//! the piece's audio **at render time**, so a hit slipped onto a
//! quieter part of the take comes out quieter, which is what anyone
//! would expect and what a stored number would get wrong.
//!
//! # Why per piece
//!
//! A kick sample fires from the kick's hits and a snare's from the
//! snare's. The "other" lane — hats, cymbals, rooms — carries no hit
//! list at all, so nothing can be triggered from it by accident.

use crate::hits::Hit;

/// One sampler note: when, and how hard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    /// Position in the take, seconds — the hit's own.
    pub at: f64,
    /// MIDI velocity, 1–127.
    pub velocity: u8,
}

/// The loudest a measurement can be, and the quietest that still counts
/// as a hit.
///
/// Zero is not a velocity: a note-on at zero velocity is a note-OFF in
/// MIDI, so a hit measured at silence would delete itself rather than
/// play quietly. One is the floor.
const MIN_VELOCITY: u8 = 1;
const MAX_VELOCITY: u8 = 127;

/// Turn a linear peak into a MIDI velocity.
///
/// Linear, not dB-scaled, and deliberately: the sampler's own velocity
/// layers are chosen by ear against the peak of a hit, so a curve here
/// would fight whatever curve the pack author already applied.
#[must_use]
pub fn velocity_of(peak: f64) -> u8 {
    let scaled = peak.clamp(0.0, 1.0) * f64::from(MAX_VELOCITY);
    // Found by comparison rather than converted by `as`. A float-to-int
    // cast is silent about NaN and about anything out of range, and the
    // failure it hides here would be a note that plays at the wrong
    // strength or not at all. Walking down from the top is at most 127
    // comparisons for a value that is asked for once per hit, and it is
    // total by construction: NaN matches nothing and falls to the
    // quietest audible note.
    (MIN_VELOCITY..=MAX_VELOCITY)
        .rev()
        .find(|level| f64::from(*level) <= scaled + 0.5)
        .unwrap_or(MIN_VELOCITY)
}

/// Render a piece's trigger notes from its hit list.
///
/// `peak_at` measures the piece's audio at a position; it is a
/// parameter rather than a read inside, so the render can be tested
/// without audio and so the caller decides which mic is measured — the
/// close mic, not the room.
///
/// r[impl flow.drums.trigger.from-hits]
#[must_use]
pub fn notes(hits: &[Hit], peak_at: &dyn Fn(f64) -> f64) -> Vec<Note> {
    hits.iter()
        .map(|hit| Note {
            at: hit.at,
            velocity: velocity_of(peak_at(hit.at)),
        })
        .collect()
}

/// How a piece's sample sits against its mics.
///
/// The blend is on the piece's Sum, where the close mics already meet,
/// so the sample is mixed against what it is reinforcing rather than
/// against the whole kit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    /// The sample's level against the mics, linear.
    pub level: f64,
    /// Whether the sample is flipped against the close mic.
    ///
    /// A sample recorded from the other side of a head is upside down
    /// relative to the close mic, and summing the two thins exactly the
    /// low end the sample was added for. It is a control rather than a
    /// fixed choice because which way round a pack is recorded is the
    /// pack's business.
    pub inverted: bool,
}

impl Default for Blend {
    fn default() -> Self {
        Self {
            // Present but under the mics: a trigger that arrives at the
            // same level as the close mic is a new drum, not a
            // reinforcement, and that is a decision to make on purpose.
            level: 0.5,
            inverted: false,
        }
    }
}
