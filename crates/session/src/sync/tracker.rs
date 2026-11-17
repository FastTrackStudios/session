//! Following a playhead without an engine: a chart, a lyric display, a
//! lighting rig on another machine.
//!
//! The leader publishes its [`Transport`] when it changes (and a keepalive
//! between, [`crate::sync::PublishGate`]), stamped in the shared clock. A
//! follower knows its offset to that clock ([`crate::sync::ClockEstimator`])
//! and asks "where is the song now?" every frame. Because every message
//! says *when* it was true, network delay does not move the answer; only
//! the clock estimate's error does.
//!
//! [`Tracker`] adds what a display needs on top of [`Transport::at`]: a
//! move of the leader's own (play, stop, a seek, a rate change) shows at
//! once, and a correction (the offset estimate moving by a fraction of a
//! millisecond) is slewed onto rather than jumped, so a playhead drawn at
//! 240 fps never twitches.

use crate::transport::Transport;

/// A follower's view of the leader's playhead.
#[derive(Clone, Debug)]
pub struct Tracker {
    /// The leader's transport, stamped in the shared clock.
    leader: Option<Transport>,
    /// What was last shown: (local µs, seconds).
    shown: Option<(f64, f64)>,
    /// The last observation was a move of the leader's own (play, stop, a
    /// seek, a rate change), not a correction: show it at once.
    moved: bool,
    /// A difference above this is shown at once, seconds.
    jump_seconds: f64,
    /// How long a smaller difference takes to close, seconds.
    slew_seconds: f64,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new(0.050, 0.25)
    }
}

impl Tracker {
    /// Jumps for a difference over `jump_seconds`, slews over
    /// `slew_seconds` below it.
    #[must_use]
    pub const fn new(jump_seconds: f64, slew_seconds: f64) -> Self {
        Self {
            leader: None,
            shown: None,
            moved: false,
            jump_seconds,
            slew_seconds,
        }
    }

    /// The leader's latest transport, stamped in the shared clock. An
    /// older one than what is held (by its stamp) is ignored: messages may
    /// arrive out of order.
    pub fn observe(&mut self, leader: Transport) {
        if let Some(held) = self.leader {
            if held.host_micros > leader.host_micros {
                return;
            }
            // Did the leader itself move, or is this the same motion told
            // again (a keepalive)?
            let expected = held.at(leader.host_micros);
            self.moved |= held.is_playing != leader.is_playing
                || (held.playrate - leader.playrate).abs() > 1e-9
                || (expected - leader.playhead_seconds).abs() > self.jump_seconds;
        } else {
            self.moved = true;
        }
        self.leader = Some(leader);
    }

    /// The leader's transport as last heard; `None` before the first.
    #[must_use]
    pub const fn leader(&self) -> Option<Transport> {
        self.leader
    }

    /// Where the leader's playhead is exactly, at `local_micros` in this
    /// machine's clock, `offset_micros` being the shared clock minus this
    /// one. `None` before the first message.
    #[must_use]
    pub fn exact(&self, local_micros: f64, offset_micros: f64) -> Option<f64> {
        self.leader.map(|t| t.at(local_micros + offset_micros))
    }

    /// Where to show the playhead at `local_micros`: the exact place,
    /// reached smoothly. Call once per frame.
    pub fn position(&mut self, local_micros: f64, offset_micros: f64) -> Option<f64> {
        let leader = self.leader?;
        let target = leader.at(local_micros + offset_micros);
        let moved = core::mem::take(&mut self.moved);
        let shown = match self.shown {
            Some((at, was)) if leader.is_playing && !moved && local_micros > at => {
                let dt = (local_micros - at) * 1e-6;
                let coasting = dt.mul_add(leader.playrate, was);
                let gap = target - coasting;
                if gap.abs() > self.jump_seconds {
                    target
                } else {
                    let share = (dt / self.slew_seconds).clamp(0.0, 1.0);
                    gap.mul_add(share, coasting)
                }
            }
            _ => target,
        };
        self.shown = Some((local_micros, shown));
        Some(shown)
    }
}
