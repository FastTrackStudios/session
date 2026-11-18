//! Transport as data: where a playhead was, how fast it moves, and when
//! that was true.
//!
//! A playhead is never streamed. It is published when it changes (play,
//! stop, a seek, a rate change), stamped with the instant it was true, and
//! anyone extrapolates it: a chart, a lyric display or a lighting rig
//! follows the song with no engine, from a [`Transport`] and the
//! [`crate::TempoMap`].
//!
//! This is the one transport value of the stack. `daw-transport-sync`'s
//! `Position` (the leader's playhead in `sync`) is this type
//! ([`crate::sync::Position`]), and Architect's presence `PlayState` is the
//! same four numbers in milliseconds (the `architect` feature converts).
//!
//! What is *not* here: the play state of one person's own engine in
//! "independent" mode is presence, not document; a loop range and the
//! metronome are a player's settings (the DAW's extension data). The
//! transport is not in the document at all: it moves every buffer and has
//! nothing to merge.

use crate::timeline::{BarBeat, TempoMap};

/// A playhead at a known instant, in a known clock.
#[derive(Clone, Copy, Debug, PartialEq, facet::Facet)]
pub struct Transport {
    /// The instant, in the clock this transport is stamped in,
    /// microseconds.
    pub host_micros: f64,
    /// The playhead at that instant, seconds from the session's start.
    pub playhead_seconds: f64,
    /// Playhead seconds per clock second (1.0 = nominal).
    pub playrate: f64,
    pub is_playing: bool,
}

impl Transport {
    /// Stopped at `seconds`, as of `host_micros`.
    #[must_use]
    pub const fn stopped(seconds: f64, host_micros: f64) -> Self {
        Self {
            host_micros,
            playhead_seconds: seconds,
            playrate: 1.0,
            is_playing: false,
        }
    }

    /// Playing from `seconds` at `host_micros`, at `playrate`.
    #[must_use]
    pub const fn playing(seconds: f64, host_micros: f64, playrate: f64) -> Self {
        Self {
            host_micros,
            playhead_seconds: seconds,
            playrate,
            is_playing: true,
        }
    }

    /// Where the playhead is at `now_micros` (same clock): moved on by the
    /// time since, at its rate, if it is playing. Pure arithmetic: safe on
    /// an audio thread.
    #[must_use]
    pub fn at(&self, now_micros: f64) -> f64 {
        if self.is_playing {
            ((now_micros - self.host_micros) * 1e-6).mul_add(self.playrate, self.playhead_seconds)
        } else {
            self.playhead_seconds
        }
    }

    /// This transport stamped in another clock, `offset_micros` ahead of
    /// this one (`other = this + offset`).
    #[must_use]
    pub fn shifted(self, offset_micros: f64) -> Self {
        Self {
            host_micros: self.host_micros + offset_micros,
            ..self
        }
    }

    /// Where the playhead is at `now_micros`, in quarter notes.
    #[must_use]
    pub fn quarter_notes_at(&self, now_micros: f64, tempo: &TempoMap) -> f64 {
        tempo.engine().seconds_to_quarter_notes(self.at(now_micros))
    }

    /// Where the playhead is at `now_micros`, in bars and beats.
    #[must_use]
    pub fn bar_beat_at(&self, now_micros: f64, tempo: &TempoMap) -> BarBeat {
        tempo.bar_beat(self.quarter_notes_at(now_micros, tempo))
    }
}

impl Default for Transport {
    fn default() -> Self {
        Self::stopped(0.0, 0.0)
    }
}
