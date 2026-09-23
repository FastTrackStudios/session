//! Shared transport: one play button for everyone.
//!
//! Two modes, chosen per session:
//!
//! - **Independent** — the doc is shared, the transport is not. Everyone
//!   runs their own engine and plays what they need to hear; the others'
//!   play cursors are ghosts (see `presence::PlayState`).
//! - **Shared** — one transport. Whoever presses play, everyone plays,
//!   from the same place, at the same moment.
//!
//! The shared transport is ONE presence entry, [`KEY`], that anyone may
//! write: last writer wins, which is what a play button pressed by two
//! people at once should do. It says where the song was at a moment of
//! the *shared clock*, so a peer that hears about it late still starts in
//! the right place — [`ClockSync`] maps this machine's clock onto it.
//! Each peer's [`Follower`] turns the entry into what its own engine must
//! do, and keeps it there: a peer that drifts is nudged back.

use loro::LoroValue;

/// The presence key the shared transport lives under.
pub const KEY: &str = "session/transport";

/// How far a following engine may drift before it is corrected, seconds.
pub const DRIFT_TOLERANCE: f64 = 0.030;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportMode {
    #[default]
    Independent,
    Shared,
}

/// The session's transport, as last set by anyone.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedTransport {
    pub mode: TransportMode,
    pub playing: bool,
    /// Seconds into `song` at `at_ms`.
    pub position: f64,
    /// Shared-clock milliseconds.
    pub at_ms: f64,
    /// Which song of the setlist (its id); `None` for a lone session.
    pub song: Option<String>,
    /// Bumped by every write, so a follower acts on each press once.
    pub seq: u64,
    /// Who pressed it — shown beside the transport ("Alice started").
    pub by: String,
}

impl SharedTransport {
    /// Where the song is at `now_ms` (shared clock).
    #[must_use]
    pub fn position_at(&self, now_ms: f64) -> f64 {
        if self.playing {
            ((now_ms - self.at_ms) / 1000.0) + self.position
        } else {
            self.position
        }
    }

    #[must_use]
    pub fn encode(&self) -> LoroValue {
        let m: std::collections::HashMap<String, LoroValue> = [
            (
                "mode",
                LoroValue::from(if self.mode == TransportMode::Shared {
                    "shared"
                } else {
                    "independent"
                }),
            ),
            ("playing", LoroValue::Bool(self.playing)),
            ("position", LoroValue::Double(self.position)),
            ("at", LoroValue::Double(self.at_ms)),
            (
                "song",
                self.song
                    .as_deref()
                    .map_or(LoroValue::Null, LoroValue::from),
            ),
            (
                "seq",
                LoroValue::I64(i64::try_from(self.seq).unwrap_or(i64::MAX)),
            ),
            ("by", LoroValue::from(self.by.as_str())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        LoroValue::from(m)
    }

    #[must_use]
    pub fn decode(v: &LoroValue) -> Option<Self> {
        let LoroValue::Map(m) = v else { return None };
        let f64_of = |k: &str| match m.get(k)? {
            LoroValue::Double(d) => Some(*d),
            LoroValue::I64(i) => i32::try_from(*i).ok().map(f64::from),
            _ => None,
        };
        let str_of = |k: &str| match m.get(k)? {
            LoroValue::String(s) => Some(s.to_string()),
            _ => None,
        };
        Some(Self {
            mode: match str_of("mode")?.as_str() {
                "shared" => TransportMode::Shared,
                _ => TransportMode::Independent,
            },
            playing: matches!(m.get("playing")?, LoroValue::Bool(true)),
            position: f64_of("position")?,
            at_ms: f64_of("at")?,
            song: str_of("song"),
            seq: match m.get("seq")? {
                LoroValue::I64(i) => u64::try_from(*i).ok()?,
                _ => return None,
            },
            by: str_of("by").unwrap_or_default(),
        })
    }
}

/// What this peer's engine is doing right now.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalTransport {
    pub playing: bool,
    pub position: f64,
    pub song: Option<String>,
}

/// What the follower asks of the local engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Open this song first (then the next step applies to it).
    SwitchSong(String),
    /// Start playing from here.
    Play { from: f64 },
    /// Stop, and rest here.
    Stop { at: f64 },
    /// Keep playing, from here — a correction.
    Seek { to: f64 },
}

/// Keeps one engine on the shared transport.
#[derive(Debug, Clone, Default)]
pub struct Follower {
    seen: Option<u64>,
}

impl Follower {
    /// What to do, given the shared transport and this engine's state at
    /// `now_ms` (shared clock). Call on every change of either, and
    /// periodically while playing to correct drift.
    pub fn step(
        &mut self,
        shared: &SharedTransport,
        local: &LocalTransport,
        now_ms: f64,
    ) -> Vec<Command> {
        if shared.mode != TransportMode::Shared {
            return Vec::new();
        }
        let fresh = self.seen != Some(shared.seq);
        self.seen = Some(shared.seq);
        let mut out = Vec::new();
        if let Some(song) = &shared.song
            && local.song.as_ref() != Some(song)
        {
            out.push(Command::SwitchSong(song.clone()));
        }
        let target = shared.position_at(now_ms);
        match (shared.playing, local.playing) {
            (true, false) => out.push(Command::Play { from: target }),
            (false, true) => out.push(Command::Stop { at: target }),
            (true, true) => {
                if fresh || (local.position - target).abs() > DRIFT_TOLERANCE {
                    out.push(Command::Seek { to: target });
                }
            }
            (false, false) => {
                if fresh && (local.position - target).abs() > f64::EPSILON {
                    out.push(Command::Stop { at: target });
                }
            }
        }
        out
    }
}

/// This machine's clock mapped onto the shared one (the host's).
///
/// NTP's trick: send at `t0` (local), the host answers with its time `ts`,
/// the answer arrives at `t1` (local). If the trip was symmetric, the host
/// read its clock at local `(t0 + t1) / 2`. The sample with the shortest
/// round trip has the least room for asymmetry, so it is the one kept.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClockSync {
    best: Option<(f64, f64)>, // (round trip, offset)
}

impl ClockSync {
    /// Record one exchange; all times in milliseconds.
    pub fn sample(&mut self, t0: f64, host_ms: f64, t1: f64) {
        let rtt = t1 - t0;
        if rtt < 0.0 {
            return;
        }
        let offset = host_ms - t0.midpoint(t1);
        if self.best.is_none_or(|(best, _)| rtt < best) {
            self.best = Some((rtt, offset));
        }
    }

    /// Shared-clock time for a local time.
    #[must_use]
    pub fn shared(&self, local_ms: f64) -> f64 {
        local_ms + self.best.map_or(0.0, |(_, offset)| offset)
    }

    /// The round trip of the sample in use, if any — how much to trust it.
    #[must_use]
    pub fn round_trip(&self) -> Option<f64> {
        self.best.map(|(rtt, _)| rtt)
    }
}
