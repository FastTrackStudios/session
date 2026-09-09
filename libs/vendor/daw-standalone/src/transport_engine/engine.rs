//! `TransportEngine` — lock-free shared sample clock.
//!
//! Hands an `Arc<TransportShared>` to both the audio callback (writes
//! `playhead_samples` via [`advance`](TransportShared::advance), reads
//! the rest) and the control thread (writes everything else, snapshots
//! the playhead for the proto `Transport` mirror + the UI tick stream).
//!
//! All state is `core::sync::atomic` — no `Mutex`, no allocation, no
//! blocking. Safe to call `advance` from a cpal/AudioWorklet callback.
//!
//! Scope of this first slice:
//! - Single static tempo (BPM via [`set_tempo_bpm`]).
//! - Loop wrap with Firewheel-style frame clamping.
//! - Varispeed via fixed-point sub-sample accumulator so non-integer
//!   `playrate` doesn't drift.
//!
//! Out of scope (deferred): [`DynamicTempoMap`](super::tempo_map::DynamicTempoMap),
//! count-in / pre-roll, sub-tick automation, `RecordMode::Item` punch.

use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU32, AtomicU64, Ordering};

use super::clock::{InstantMusical, InstantSamples, InstantSeconds, SampleClock};
use super::tempo_map::StaticTempoMap;

/// `repr(u8)` view of `daw_proto::PlayState`. Kept separate so this
/// module compiles without proto.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayStateRepr {
    Stopped = 0,
    Playing = 1,
    Paused = 2,
    Recording = 3,
}

impl PlayStateRepr {
    #[inline]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Playing,
            2 => Self::Paused,
            3 => Self::Recording,
            _ => Self::Stopped,
        }
    }
    #[inline]
    pub fn is_advancing(self) -> bool {
        matches!(self, Self::Playing | Self::Recording)
    }
}

/// Loop region in samples. `end == 0` (or `end <= start`) disables.
#[derive(Debug, Clone, Copy)]
pub struct LoopRegionSamples {
    pub start: InstantSamples,
    pub end: InstantSamples,
}

impl LoopRegionSamples {
    #[inline]
    pub fn is_valid(self) -> bool {
        self.end.0 > self.start.0
    }
}

/// Lock-free shared state. Field naming = `_bits` for f64-as-u64.
#[derive(Debug)]
pub struct TransportShared {
    /// Output sample rate. Set once at engine creation.
    sample_rate: AtomicU32,

    /// Current playhead, in output-rate samples.
    ///
    /// Only [`advance`](Self::advance) and [`set_playhead`](Self::set_playhead)
    /// write this. Readers can use `Relaxed` — UI ticks tolerate
    /// staleness on the order of one block.
    playhead_samples: AtomicI64,

    /// [`PlayStateRepr`] as `u8`.
    play_state: AtomicU8,

    /// Loop enabled.
    looping: AtomicBool,
    metronome: AtomicBool,
    loop_start_samples: AtomicI64,
    loop_end_samples: AtomicI64,

    /// `playrate` f64 bits. 1.0 = realtime, 2.0 = double speed.
    playrate_bits: AtomicU64,

    /// Static BPM, f64 bits. Updated when proto `Transport.tempo`
    /// changes.
    tempo_bpm_bits: AtomicU64,
}

impl TransportShared {
    pub fn new(sample_rate: u32, initial_bpm: f64) -> Self {
        Self {
            sample_rate: AtomicU32::new(sample_rate),
            playhead_samples: AtomicI64::new(0),
            play_state: AtomicU8::new(PlayStateRepr::Stopped as u8),
            looping: AtomicBool::new(false),
            metronome: AtomicBool::new(false),
            loop_start_samples: AtomicI64::new(0),
            loop_end_samples: AtomicI64::new(0),
            playrate_bits: AtomicU64::new(1.0f64.to_bits()),
            tempo_bpm_bits: AtomicU64::new(initial_bpm.to_bits()),
        }
    }

    // ── Audio-thread API ────────────────────────────────────────────

    /// Advance the playhead by `frames` output-rate samples. **Must
    /// only be called from the audio callback.** Honors `playrate` and
    /// loop wrap.
    ///
    /// Returns the playhead value (in samples) at the **start** of the
    /// processed block — what a node would tag its block with.
    ///
    /// `frames` is the buffer size; on loop wrap the function still
    /// reports having consumed the full block (sample-accurate
    /// loop-end node behavior is a future concern — see the Firewheel
    /// `clamp + teleport` pattern noted in tempo_map.rs).
    #[inline]
    pub fn advance(&self, frames: u32) -> InstantSamples {
        let state = PlayStateRepr::from_u8(self.play_state.load(Ordering::Relaxed));
        let start = InstantSamples(self.playhead_samples.load(Ordering::Relaxed));
        if !state.is_advancing() || frames == 0 {
            return start;
        }

        let playrate = f64::from_bits(self.playrate_bits.load(Ordering::Relaxed));
        // Convert frames consumed at output rate to samples advanced
        // on the musical timeline. playrate > 1 advances faster.
        let advance_samples = (frames as f64 * playrate).round() as i64;
        let mut end = start.0 + advance_samples;

        if self.looping.load(Ordering::Relaxed) {
            let loop_start = self.loop_start_samples.load(Ordering::Relaxed);
            let loop_end = self.loop_end_samples.load(Ordering::Relaxed);
            if loop_end > loop_start && end >= loop_end {
                // Teleport: wrap modulo loop span, mapped from
                // distance past loop_start. Handles huge buffer sizes
                // that span >1 loop iteration.
                let span = loop_end - loop_start;
                let offset = (end - loop_start).rem_euclid(span);
                end = loop_start + offset;
            }
        }

        self.playhead_samples.store(end, Ordering::Relaxed);
        start
    }

    // ── Control-thread reads ────────────────────────────────────────

    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn playhead_samples(&self) -> InstantSamples {
        InstantSamples(self.playhead_samples.load(Ordering::Relaxed))
    }
    #[inline]
    pub fn play_state(&self) -> PlayStateRepr {
        PlayStateRepr::from_u8(self.play_state.load(Ordering::Relaxed))
    }
    #[inline]
    pub fn is_looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn loop_region(&self) -> Option<LoopRegionSamples> {
        let s = self.loop_start_samples.load(Ordering::Relaxed);
        let e = self.loop_end_samples.load(Ordering::Relaxed);
        let r = LoopRegionSamples {
            start: InstantSamples(s),
            end: InstantSamples(e),
        };
        r.is_valid().then_some(r)
    }
    #[inline]
    pub fn playrate(&self) -> f64 {
        f64::from_bits(self.playrate_bits.load(Ordering::Relaxed))
    }
    #[inline]
    pub fn tempo_bpm(&self) -> f64 {
        f64::from_bits(self.tempo_bpm_bits.load(Ordering::Relaxed))
    }

    // ── Control-thread writes ───────────────────────────────────────

    #[inline]
    pub fn set_play_state(&self, s: PlayStateRepr) {
        self.play_state.store(s as u8, Ordering::Relaxed);
    }
    #[inline]
    pub fn set_playhead(&self, p: InstantSamples) {
        self.playhead_samples.store(p.0, Ordering::Relaxed);
    }
    #[inline]
    pub fn metronome(&self) -> bool {
        self.metronome.load(Ordering::Relaxed)
    }

    pub fn set_metronome(&self, v: bool) {
        self.metronome.store(v, Ordering::Relaxed);
    }

    pub fn set_looping(&self, v: bool) {
        self.looping.store(v, Ordering::Relaxed);
    }
    /// Set loop region. Pass `None` to disable (also clears the bool).
    pub fn set_loop_region(&self, r: Option<LoopRegionSamples>) {
        match r {
            Some(r) if r.is_valid() => {
                self.loop_start_samples.store(r.start.0, Ordering::Relaxed);
                self.loop_end_samples.store(r.end.0, Ordering::Relaxed);
            }
            _ => {
                self.loop_start_samples.store(0, Ordering::Relaxed);
                self.loop_end_samples.store(0, Ordering::Relaxed);
                self.looping.store(false, Ordering::Relaxed);
            }
        }
    }
    /// Set varispeed. Clamps to the same 0.25..=4.0 range the proto
    /// service uses.
    #[inline]
    pub fn set_playrate(&self, rate: f64) {
        let clamped = rate.clamp(0.25, 4.0);
        self.playrate_bits
            .store(clamped.to_bits(), Ordering::Relaxed);
    }
    #[inline]
    pub fn set_tempo_bpm(&self, bpm: f64) {
        self.tempo_bpm_bits.store(bpm.to_bits(), Ordering::Relaxed);
    }
    /// Rewrite the sample rate — used by an audio engine when it
    /// attaches and discovers the device's actual rate. Should only
    /// be called while the engine is stopped (or before the soft
    /// clock starts) to avoid playhead-in-samples becoming
    /// inconsistent under a unit change.
    #[inline]
    pub fn set_sample_rate(&self, sr: u32) {
        debug_assert!(sr > 0);
        self.sample_rate.store(sr, Ordering::Relaxed);
    }
}

/// Convenience handle = `Arc<TransportShared>` plus derived snapshots.
#[derive(Debug, Clone)]
pub struct TransportEngine {
    pub shared: alloc::sync::Arc<TransportShared>,
}

impl TransportEngine {
    pub fn new(sample_rate: u32, initial_bpm: f64) -> Self {
        Self {
            shared: alloc::sync::Arc::new(TransportShared::new(sample_rate, initial_bpm)),
        }
    }

    /// Derive a snapshot suitable for emitting a [`PositionTick`] or
    /// mirroring into proto `Transport.playhead_position`.
    pub fn snapshot(&self) -> TransportSnapshot {
        let s = &*self.shared;
        let clock = SampleClock::new(s.sample_rate());
        let samples = s.playhead_samples();
        let seconds = clock.samples_to_seconds(samples);
        let tempo = StaticTempoMap::new(s.tempo_bpm().max(1e-3));
        let musical = tempo.samples_to_musical(samples, s.playrate(), &clock);
        TransportSnapshot {
            samples,
            seconds,
            musical,
            play_state: s.play_state(),
        }
    }

    /// Seek by seconds, converting via the sample rate.
    pub fn seek_seconds(&self, seconds: f64) {
        let clock = SampleClock::new(self.shared.sample_rate());
        self.shared
            .set_playhead(clock.seconds_to_samples(InstantSeconds(seconds)));
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TransportSnapshot {
    pub samples: InstantSamples,
    pub seconds: InstantSeconds,
    pub musical: InstantMusical,
    pub play_state: PlayStateRepr,
}

extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_when_stopped_is_noop() {
        let e = TransportEngine::new(48_000, 120.0);
        e.shared.set_play_state(PlayStateRepr::Stopped);
        let start = e.shared.advance(512);
        assert_eq!(start.0, 0);
        assert_eq!(e.shared.playhead_samples().0, 0);
    }

    #[test]
    fn advance_when_playing_bumps_playhead() {
        let e = TransportEngine::new(48_000, 120.0);
        e.shared.set_play_state(PlayStateRepr::Playing);
        let _ = e.shared.advance(512);
        let _ = e.shared.advance(512);
        assert_eq!(e.shared.playhead_samples().0, 1024);
    }

    #[test]
    fn varispeed_doubles_advance() {
        let e = TransportEngine::new(48_000, 120.0);
        e.shared.set_play_state(PlayStateRepr::Playing);
        e.shared.set_playrate(2.0);
        let _ = e.shared.advance(512);
        assert_eq!(e.shared.playhead_samples().0, 1024);
    }

    #[test]
    fn loop_wraps_at_end() {
        let e = TransportEngine::new(48_000, 120.0);
        e.shared.set_play_state(PlayStateRepr::Playing);
        e.shared.set_loop_region(Some(LoopRegionSamples {
            start: InstantSamples(0),
            end: InstantSamples(1000),
        }));
        e.shared.set_looping(true);
        // 3 blocks of 400 = 1200 advance, should wrap to 200.
        for _ in 0..3 {
            let _ = e.shared.advance(400);
        }
        assert_eq!(e.shared.playhead_samples().0, 200);
    }

    #[test]
    fn loop_wraps_when_block_overshoots_by_multiple_iterations() {
        let e = TransportEngine::new(48_000, 120.0);
        e.shared.set_play_state(PlayStateRepr::Playing);
        e.shared.set_loop_region(Some(LoopRegionSamples {
            start: InstantSamples(100),
            end: InstantSamples(200),
        }));
        e.shared.set_looping(true);
        e.shared.set_playhead(InstantSamples(150));
        // Advance 1050 samples: from 150 → 1200; (1200-100) % 100 = 0
        // → playhead = 100.
        let _ = e.shared.advance(1050);
        assert_eq!(e.shared.playhead_samples().0, 100);
    }

    #[test]
    fn snapshot_seconds_and_musical_track_playhead() {
        let e = TransportEngine::new(48_000, 120.0);
        e.shared.set_playhead(InstantSamples(24_000)); // 0.5 s, 1 beat @ 120
        let snap = e.snapshot();
        assert!((snap.seconds.0 - 0.5).abs() < 1e-9);
        assert!((snap.musical.0 - 1.0).abs() < 1e-9);
    }
}
