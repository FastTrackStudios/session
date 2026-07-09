//! The guide engine: renders click / count-in / guide voices into
//! caller-provided buffers, driven by a caller-provided clock.
//!
//! This is the portable replacement for the legacy plugin's
//! `Plugin::process` — no REAPER, no audio I/O, no threads. The host
//! (daw-standalone's render callback, a test harness, an offline
//! bouncer) calls [`GuideEngine::render`] once per block with a
//! [`BlockClock`] describing the transport at block start.

use crate::audio::{
    ClickPlayer, ClickPlayerState, CountPlayer, CountPlayerState, GuidePlayer, GuidePlayerState,
};
use crate::samples::SampleBank;
use crate::schedule::{CueEvent, CueSchedule, GuideSection, GuideSongTiming, ScheduleOptions};

/// Transport snapshot at the start of a render block.
#[derive(Debug, Clone, Copy)]
pub struct BlockClock {
    /// Whether the transport is rolling. When false the engine only
    /// flushes voice tails (no new triggers).
    pub playing: bool,
    /// Timeline position in seconds at the first frame of the block.
    pub pos_seconds: f64,
    /// Timeline position in quarter notes at the first frame of the block.
    pub pos_beats: f64,
    /// Tempo in quarter notes per minute.
    pub tempo_bpm: f64,
    /// Time signature numerator.
    pub time_sig_num: u32,
    /// Time signature denominator.
    pub time_sig_den: u32,
    /// Render sample rate.
    pub sample_rate: f64,
}

/// Output buses for one render block. All six slices must be the same
/// length; the engine ADDS into them (clear them first if needed).
pub struct GuideBuses<'a> {
    pub click_l: &'a mut [f32],
    pub click_r: &'a mut [f32],
    pub count_l: &'a mut [f32],
    pub count_r: &'a mut [f32],
    pub guide_l: &'a mut [f32],
    pub guide_r: &'a mut [f32],
}

/// Engine configuration (the legacy plugin's parameters, minus the
/// REAPER/GUI plumbing).
#[derive(Debug, Clone)]
pub struct GuideConfig {
    /// Click on every beat (quarter note).
    pub enable_beat: bool,
    /// Click on eighth notes.
    pub enable_eighth: bool,
    /// Click on sixteenth notes.
    pub enable_sixteenth: bool,
    /// Click on beat-unit triplets.
    pub enable_triplet: bool,
    /// Accent click on beat 1 of each measure.
    pub enable_measure_accent: bool,
    /// Count-in voices.
    pub enable_count: bool,
    /// Section guide announcements.
    pub enable_guide: bool,
    /// Linear gain for the click bus.
    pub click_gain: f32,
    /// Linear gain for the count bus.
    pub count_gain: f32,
    /// Linear gain for the guide bus.
    pub guide_gain: f32,
    /// Count-in / announcement scheduling options.
    pub schedule: ScheduleOptions,
}

impl Default for GuideConfig {
    fn default() -> Self {
        Self {
            enable_beat: true,
            enable_eighth: false,
            enable_sixteenth: false,
            enable_triplet: false,
            enable_measure_accent: true,
            enable_count: true,
            enable_guide: true,
            click_gain: 1.0,
            count_gain: 1.0,
            guide_gain: 1.0,
            schedule: ScheduleOptions::default(),
        }
    }
}

/// A trigger resolved to a sample offset within the current block.
#[derive(Debug, Clone)]
enum Trigger {
    Beat,
    Accent,
    Eighth,
    Sixteenth,
    Triplet,
    Count(usize),
    Guide(String),
}

/// The guide engine. See module docs.
pub struct GuideEngine {
    pub config: GuideConfig,
    bank: SampleBank,
    schedule: CueSchedule,

    click_state: ClickPlayerState,
    count_state: CountPlayerState,
    guide_state: GuidePlayerState,
    current_guide_key: Option<String>,

    /// Last-triggered grid indices per subdivision (i64 grid steps).
    /// `i64::MIN` means "re-initialize on next rolling block" (fresh
    /// start or after a stop/seek).
    last_beat_idx: i64,
    last_eighth_idx: i64,
    last_sixteenth_idx: i64,
    last_triplet_idx: i64,

    /// Timeline seconds at the end of the previous rolling block, used to
    /// detect seeks and relocate the cue cursor. `None` forces relocation.
    prev_block_end_seconds: Option<f64>,
    /// Cursor into `schedule.cues`.
    next_cue: usize,

    /// Scratch trigger list, reused across blocks.
    pending: Vec<(usize, Trigger)>,
}

impl GuideEngine {
    pub fn new(config: GuideConfig) -> Self {
        Self {
            config,
            bank: SampleBank::default(),
            schedule: CueSchedule::default(),
            click_state: ClickPlayerState::new(),
            count_state: CountPlayerState::new(),
            guide_state: GuidePlayerState::new(),
            current_guide_key: None,
            last_beat_idx: i64::MIN,
            last_eighth_idx: i64::MIN,
            last_sixteenth_idx: i64::MIN,
            last_triplet_idx: i64::MIN,
            prev_block_end_seconds: None,
            next_cue: 0,
            pending: Vec::with_capacity(32),
        }
    }

    /// Access the sample bank (to load click/count/guide PCM into it).
    pub fn bank_mut(&mut self) -> &mut SampleBank {
        &mut self.bank
    }

    pub fn bank(&self) -> &SampleBank {
        &self.bank
    }

    /// Replace the sample bank wholesale.
    pub fn set_bank(&mut self, bank: SampleBank) {
        self.bank = bank;
    }

    /// Build and install the cue schedule for a song's sections.
    pub fn set_sections(&mut self, sections: &[GuideSection], timing: &GuideSongTiming) {
        self.schedule = CueSchedule::build(sections, timing, &self.config.schedule);
        self.prev_block_end_seconds = None;
        self.next_cue = 0;
    }

    /// Install a prebuilt cue schedule (e.g. one with extra spoken cues).
    pub fn set_schedule(&mut self, schedule: CueSchedule) {
        self.schedule = schedule;
        self.prev_block_end_seconds = None;
        self.next_cue = 0;
    }

    pub fn schedule(&self) -> &CueSchedule {
        &self.schedule
    }

    /// Reset all playback state (voice tails, grid trackers, cue cursor).
    pub fn reset(&mut self) {
        self.click_state.reset();
        self.count_state.reset();
        self.guide_state.reset();
        self.current_guide_key = None;
        self.last_beat_idx = i64::MIN;
        self.last_eighth_idx = i64::MIN;
        self.last_sixteenth_idx = i64::MIN;
        self.last_triplet_idx = i64::MIN;
        self.prev_block_end_seconds = None;
        self.next_cue = 0;
    }

    /// Render one block into separate click / count / guide buses
    /// (adding into the slices).
    pub fn render(&mut self, buses: &mut GuideBuses<'_>, clock: &BlockClock) {
        let n = buses.click_l.len();
        debug_assert!(
            [
                buses.click_r.len(),
                buses.count_l.len(),
                buses.count_r.len(),
                buses.guide_l.len(),
                buses.guide_r.len(),
            ]
            .iter()
            .all(|&len| len == n),
            "all guide buses must be the same length"
        );
        self.prepare_block(n, clock);
        self.mix_block(n, |i, [cl, cr, nl, nr, gl, gr]| {
            buses.click_l[i] += cl;
            buses.click_r[i] += cr;
            buses.count_l[i] += nl;
            buses.count_r[i] += nr;
            buses.guide_l[i] += gl;
            buses.guide_r[i] += gr;
        });
    }

    /// Render one block with all three buses summed to a stereo pair
    /// (adding into the slices).
    pub fn render_stereo(&mut self, left: &mut [f32], right: &mut [f32], clock: &BlockClock) {
        let n = left.len().min(right.len());
        self.prepare_block(n, clock);
        self.mix_block(n, |i, [cl, cr, nl, nr, gl, gr]| {
            left[i] += cl + nl + gl;
            right[i] += cr + nr + gr;
        });
    }

    /// Gather this block's triggers into `self.pending` (sorted by offset).
    fn prepare_block(&mut self, n: usize, clock: &BlockClock) {
        self.pending.clear();
        if n == 0 || clock.sample_rate <= 0.0 {
            return;
        }

        if !clock.playing {
            // Legacy behavior: stopping clears the beat tracker so the next
            // start re-initializes on (or before) the current position.
            self.last_beat_idx = i64::MIN;
            self.last_eighth_idx = i64::MIN;
            self.last_sixteenth_idx = i64::MIN;
            self.last_triplet_idx = i64::MIN;
            self.prev_block_end_seconds = None;
            return;
        }

        let samples_per_quarter = clock.sample_rate * 60.0 / clock.tempo_bpm.max(1.0);
        let block_quarters = n as f64 / samples_per_quarter;
        let beats_start = clock.pos_beats;
        let beats_end = beats_start + block_quarters;

        // ── Click subdivisions (grid-index walk; sample accurate) ────────
        let num = clock.time_sig_num.max(1) as i64;
        let beat_unit_quarters = 4.0 / f64::from(clock.time_sig_den.max(1));
        let triplet_interval = beat_unit_quarters / 3.0;

        let mut schedule_grid =
            |interval: f64,
             last_idx: &mut i64,
             pending: &mut Vec<(usize, Trigger)>,
             make: &dyn Fn(i64) -> Trigger| {
                if interval <= 0.0 {
                    return;
                }
                if *last_idx == i64::MIN {
                    // Fresh start / after seek: include a grid point that
                    // lands exactly on the block start (legacy "just
                    // started on a beat" handling).
                    *last_idx = ((beats_start / interval) - 1e-6).ceil() as i64 - 1;
                }
                loop {
                    let k = *last_idx + 1;
                    let t = k as f64 * interval;
                    if t >= beats_end {
                        break;
                    }
                    let offset = (((t - beats_start) * samples_per_quarter).round() as i64)
                        .clamp(0, n as i64 - 1) as usize;
                    pending.push((offset, make(k)));
                    *last_idx = k;
                }
            };

        if self.config.enable_beat || self.config.enable_measure_accent {
            let enable_beat = self.config.enable_beat;
            let enable_accent = self.config.enable_measure_accent;
            // Legacy parity: "beats" are quarter notes and the measure
            // accent fires when the quarter-note index is a multiple of the
            // time-signature numerator.
            let mut tmp: Vec<(usize, Trigger)> = Vec::new();
            schedule_grid(1.0, &mut self.last_beat_idx, &mut tmp, &|k| {
                if enable_accent && k.rem_euclid(num) == 0 {
                    Trigger::Accent
                } else {
                    Trigger::Beat
                }
            });
            for (offset, trig) in tmp {
                match trig {
                    Trigger::Accent => {
                        // Accent replaces the plain beat click on beat 1
                        // only when the accent sample is present.
                        if self.bank.measure_accent.is_some() {
                            self.pending.push((offset, Trigger::Accent));
                        } else if enable_beat {
                            self.pending.push((offset, Trigger::Beat));
                        }
                    }
                    trig => {
                        if enable_beat {
                            self.pending.push((offset, trig));
                        }
                    }
                }
            }
        }
        if self.config.enable_eighth {
            let mut tmp = std::mem::take(&mut self.pending);
            schedule_grid(0.5, &mut self.last_eighth_idx, &mut tmp, &|_| {
                Trigger::Eighth
            });
            self.pending = tmp;
        }
        if self.config.enable_sixteenth {
            let mut tmp = std::mem::take(&mut self.pending);
            schedule_grid(0.25, &mut self.last_sixteenth_idx, &mut tmp, &|_| {
                Trigger::Sixteenth
            });
            self.pending = tmp;
        }
        if self.config.enable_triplet {
            let mut tmp = std::mem::take(&mut self.pending);
            schedule_grid(triplet_interval, &mut self.last_triplet_idx, &mut tmp, &|_| {
                Trigger::Triplet
            });
            self.pending = tmp;
        }

        // ── Scheduled cues (count-in voices, guide announcements) ────────
        let sec_start = clock.pos_seconds;
        let sec_end = sec_start + n as f64 / clock.sample_rate;
        let continuous = self
            .prev_block_end_seconds
            .is_some_and(|prev| (prev - sec_start).abs() < 1e-4);
        if !continuous {
            // Seek (or first rolling block): relocate the cue cursor.
            self.next_cue = self
                .schedule
                .cues
                .partition_point(|c| c.time_seconds < sec_start - 1e-9);
        }
        while let Some(cue) = self.schedule.cues.get(self.next_cue) {
            if cue.time_seconds >= sec_end {
                break;
            }
            let offset = (((cue.time_seconds - sec_start) * clock.sample_rate).round() as i64)
                .clamp(0, n as i64 - 1) as usize;
            match &cue.event {
                CueEvent::Count { index } => {
                    if self.config.enable_count && *index < 8 {
                        self.pending.push((offset, Trigger::Count(*index)));
                    }
                }
                CueEvent::Guide { keys } => {
                    if self.config.enable_guide {
                        if let Some(key) = keys.iter().find(|k| self.bank.guides.contains_key(*k))
                        {
                            self.pending.push((offset, Trigger::Guide(key.clone())));
                        }
                    }
                }
            }
            self.next_cue += 1;
        }
        self.prev_block_end_seconds = Some(sec_end);

        self.pending.sort_by_key(|(offset, _)| *offset);
    }

    /// Mix `n` frames, applying pending triggers at their offsets.
    fn mix_block(&mut self, n: usize, mut write: impl FnMut(usize, [f32; 6])) {
        let click_gain = self.config.click_gain;
        let count_gain = self.config.count_gain;
        let guide_gain = self.config.guide_gain;

        let pending = std::mem::take(&mut self.pending);
        let mut ti = 0;
        for i in 0..n {
            while ti < pending.len() && pending[ti].0 == i {
                match &pending[ti].1 {
                    Trigger::Beat => {
                        self.click_state.is_playing_beat = true;
                        self.click_state.playback_position_beat = 0;
                    }
                    Trigger::Accent => {
                        self.click_state.is_playing_measure_accent = true;
                        self.click_state.playback_position_measure_accent = 0;
                    }
                    Trigger::Eighth => {
                        self.click_state.is_playing_eighth = true;
                        self.click_state.playback_position_eighth = 0;
                    }
                    Trigger::Sixteenth => {
                        self.click_state.is_playing_sixteenth = true;
                        self.click_state.playback_position_sixteenth = 0;
                    }
                    Trigger::Triplet => {
                        self.click_state.is_playing_triplet = true;
                        self.click_state.playback_position_triplet = 0;
                    }
                    Trigger::Count(index) => {
                        self.count_state.is_playing_count[*index] = true;
                        self.count_state.playback_position_count[*index] = 0;
                        self.count_state.current_count_number = *index as i32 + 1;
                    }
                    Trigger::Guide(key) => {
                        self.current_guide_key = Some(key.clone());
                        self.guide_state.is_playing_guide = true;
                        self.guide_state.playback_position_guide = 0;
                    }
                }
                ti += 1;
            }

            let mut click_left = 0.0f32;
            let mut click_right = 0.0f32;
            let mut count_left = 0.0f32;
            let mut count_right = 0.0f32;
            let mut guide_left = 0.0f32;
            let mut guide_right = 0.0f32;

            ClickPlayer::play_all(
                &mut self.click_state,
                self.bank.beat.as_ref(),
                self.bank.eighth.as_ref(),
                self.bank.sixteenth.as_ref(),
                self.bank.triplet.as_ref(),
                self.bank.measure_accent.as_ref(),
                click_gain,
                &mut click_left,
                &mut click_right,
            );
            CountPlayer::play_all(
                &mut self.count_state,
                &self.bank.counts,
                count_gain,
                &mut count_left,
                &mut count_right,
            );
            GuidePlayer::play(
                &mut self.guide_state,
                &self.current_guide_key,
                &self.bank.guides,
                guide_gain,
                &mut guide_left,
                &mut guide_right,
            );

            write(
                i,
                [
                    click_left,
                    click_right,
                    count_left,
                    count_right,
                    guide_left,
                    guide_right,
                ],
            );
        }
        // Give the scratch buffer back (empty) for reuse.
        self.pending = pending;
        self.pending.clear();
    }
}

impl Default for GuideEngine {
    fn default() -> Self {
        Self::new(GuideConfig::default())
    }
}
