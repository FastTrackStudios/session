//! Engine tunables — port of `css_engine.lua defaultConfig` (values verbatim).

use crate::profile::ProfileKind;
use crate::score::TempoPoint;

/// CC58 keyswitch band centres (from the CSS manual). Magic numbers — keep exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keyswitches {
    pub sustain_ll: u8,
    pub sustain_exp: u8,
    pub spiccato: u8,
    pub staccatissimo: u8,
    pub staccato: u8,
    pub sfz: u8,
    pub pizzicato: u8,
    pub bartok: u8,
    pub col_legno: u8,
    pub trills: u8,
    pub harmonics: u8,
    pub tremolo: u8,
    pub meas_trem: u8,
    pub marcato: u8,
    pub marcato_ov: u8,
    pub legato_on: u8,
    pub legato_off: u8,
    pub con_sord_on: u8,
    pub con_sord_off: u8,
}

impl Default for Keyswitches {
    fn default() -> Self {
        Self {
            sustain_ll: 2,
            sustain_exp: 8,
            spiccato: 13,
            staccatissimo: 18,
            staccato: 23,
            sfz: 28,
            pizzicato: 33,
            bartok: 38,
            col_legno: 43,
            trills: 48,
            harmonics: 53,
            tremolo: 58,
            meas_trem: 63,
            marcato: 68,
            marcato_ov: 73,
            legato_on: 78,
            legato_off: 83,
            con_sord_on: 88,
            con_sord_off: 93,
        }
    }
}

/// CSS legato engine mode — must match the patch GUI (sets both timing
/// compensation and the sustain keyswitch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LegatoMode {
    #[default]
    Expressive,
    LowLatency,
}

/// Legato-speed velocity curve anchor: inter-onset interval (sec) → velocity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelCurveAnchor {
    pub io: f64,
    pub vel: f64,
}

/// Phrase-shaping / micro-dynamics tunables (`cfg.phrase`).
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseConfig {
    /// Master scale: 0.6 subtle, 1.0 natural, 1.6 romantic.
    pub intensity: f64,
    /// Cap on how far micro-dynamics deviate from the marked level (CC units).
    pub micro_max: f64,
    pub do_arch: bool,
    pub do_metric: bool,
    pub do_contour: bool,
    pub do_leap: bool,
    pub do_swell: bool,
    pub do_vib_bloom: bool,
    /// Segmentation: rest gap that splits a phrase (QN).
    pub phrase_gap_qn: f64,
    /// A note this long (QN) ends a phrase segment.
    pub long_note_qn: f64,
    // Phrase arch
    pub arch_gain: f64,
    pub arch_start_dip: f64,
    pub arch_end_dip: f64,
    pub arch_peak_frac: f64,
    pub peak_blend: f64,
    // Per-note
    pub metric_gain: f64,
    pub contour_gain: f64,
    pub leap_gain: f64,
    pub leap_threshold: f64,
    // Intra-note swell (messa di voce)
    pub swell_gain: f64,
    pub swell_min_qn: f64,
    // Vibrato bloom
    pub vib_bloom_depth: f64,
    pub vib_bloom_time_qn: f64,
    /// Default 0 so harmony voices with identical rhythm stay timing-locked.
    pub legato_vel_mod: f64,
}

impl Default for PhraseConfig {
    fn default() -> Self {
        Self {
            intensity: 1.0,
            micro_max: 14.0,
            do_arch: true,
            do_metric: true,
            do_contour: true,
            do_leap: true,
            do_swell: true,
            do_vib_bloom: true,
            phrase_gap_qn: 0.4,
            long_note_qn: 8.0,
            arch_gain: 8.0,
            arch_start_dip: 4.0,
            arch_end_dip: 9.0,
            arch_peak_frac: 0.62,
            peak_blend: 0.5,
            metric_gain: 3.0,
            contour_gain: 4.0,
            leap_gain: 3.5,
            leap_threshold: 4.0,
            swell_gain: 5.0,
            swell_min_qn: 2.0,
            vib_bloom_depth: 16.0,
            vib_bloom_time_qn: 1.0,
            legato_vel_mod: 0.0,
        }
    }
}

/// The engine configuration — port of `defaultConfig` with the same defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    // Profile / CC assignment
    pub profile: ProfileKind,
    pub cc_keyswitch: u8,
    pub cc_dynamics: u8,
    pub cc_vibrato: u8,
    pub ks: Keyswitches,

    // Lead-in / tail / spacing
    pub lead_in_qn: f64,
    pub ks_stagger_qn: f64,
    pub item_tail_qn: f64,

    // Timing compensation
    pub timing_comp: bool,
    pub track_delay_ms: f64,
    pub legato_mode: LegatoMode,
    pub short_note_delay_ms: f64,
    pub attack_delay_ms: f64,
    pub max_lead_qn: f64,

    // Authenticity options
    pub re_bow: bool,
    pub cc_sustain_pedal: u8,
    pub con_sord: bool,
    pub portamento: bool,
    pub cc_portamento: u8,
    pub port_vel: f64,
    pub port_vol: f64,
    pub expand_gliss: bool,
    pub gliss_min_span_qn: f64,
    pub grace: bool,
    pub grace_lead_qn: f64,

    // Slur / auto-phrasing
    pub use_slurs: bool,
    pub auto_slur: bool,
    pub auto_slur_target_qn: f64,
    pub auto_slur_max_qn: f64,
    pub auto_slur_long_note_qn: f64,

    // Note-connection thresholds (QN)
    pub legato_overlap_qn: f64,
    pub break_gap_qn: f64,
    pub rest_threshold_qn: f64,
    pub staccato_len_frac: f64,
    pub spiccato_len_frac: f64,

    // Dynamics → CC1
    pub dyn_scale: f64,
    pub dyn_offset: f64,
    pub dyn_min: f64,
    pub dyn_default: f64,
    pub cc_grid_qn: f64,
    pub cc_deadband: f64,

    // Fade shaping
    pub fade_to_niente: bool,
    pub fade_in: bool,
    pub fade_in_max_dyn: f64,
    pub fade_use_volume: bool,
    pub reset_volume: bool,
    pub fade_tail_hold: bool,
    pub fade_tail_bars: f64,
    pub fade_recover_lead_qn: f64,
    pub cc_fade_volume: u8,
    pub fade_min_rest_qn: f64,
    pub fade_min_note_dur_qn: f64,
    pub fade_niente_floor: f64,
    pub fade_reach_frac: f64,
    pub fade_out_bias: f64,
    pub fade_in_min_dur_qn: f64,
    pub fade_in_reach_frac: f64,

    // Vibrato (CC2)
    pub vib_follow: f64,
    pub vib_base: f64,
    pub vib_tenuto_drop: f64,

    // Phrasing / micro-dynamics
    pub phrasing: bool,
    pub phrase: PhraseConfig,

    // Legato velocity curve (anchors ascending by interval)
    pub legato_vel_curve: Vec<VelCurveAnchor>,
    pub vel_first: f64,
    pub vel_accent_min: f64,

    // Fast-run → marcato
    pub marcato_fast: bool,
    pub marcato_max_sec: f64,
    pub marcato_min_run: usize,
    pub marcato_vel_min: f64,
    pub marcato_vel_range: f64,

    // Solo / section routing
    pub solo_routing: bool,
    pub solo_channel_base: u8,
    pub solo_max_voices: u8,

    // Global
    pub max_channels: u8,

    /// Score-wide tempo map override (union across parts) — set by the caller
    /// so every part shares one map; falls back to the part's own tempos.
    pub tempo_map: Option<Vec<TempoPoint>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profile: ProfileKind::Strings,
            cc_keyswitch: 58,
            cc_dynamics: 1,
            cc_vibrato: 2,
            ks: Keyswitches::default(),

            lead_in_qn: 0.5,
            ks_stagger_qn: 1.0 / 128.0,
            item_tail_qn: 4.0,

            timing_comp: true,
            track_delay_ms: 60.0,
            legato_mode: LegatoMode::Expressive,
            short_note_delay_ms: 60.0,
            attack_delay_ms: 0.0,
            max_lead_qn: 0.6,

            re_bow: true,
            cc_sustain_pedal: 64,
            con_sord: true,
            portamento: true,
            cc_portamento: 5,
            port_vel: 18.0,
            port_vol: 100.0,
            expand_gliss: true,
            gliss_min_span_qn: 0.25,
            grace: true,
            grace_lead_qn: 0.2,

            use_slurs: true,
            auto_slur: true,
            auto_slur_target_qn: 4.0,
            auto_slur_max_qn: 8.0,
            auto_slur_long_note_qn: 2.0,

            legato_overlap_qn: 1.0 / 32.0,
            break_gap_qn: 1.0 / 64.0,
            rest_threshold_qn: 1.0 / 16.0,
            staccato_len_frac: 0.45,
            spiccato_len_frac: 0.30,

            dyn_scale: 1.0,
            dyn_offset: 0.0,
            dyn_min: 5.0,
            dyn_default: 60.0,
            cc_grid_qn: 0.25,
            cc_deadband: 1.0,

            fade_to_niente: true,
            fade_in: true,
            fade_in_max_dyn: 35.0,
            fade_use_volume: false,
            reset_volume: true,
            fade_tail_hold: true,
            fade_tail_bars: 1.0,
            fade_recover_lead_qn: 0.25,
            cc_fade_volume: 11,
            fade_min_rest_qn: 2.0,
            fade_min_note_dur_qn: 1.0,
            fade_niente_floor: 0.0,
            fade_reach_frac: 1.0,
            fade_out_bias: 1.0,
            fade_in_min_dur_qn: 2.0,
            fade_in_reach_frac: 0.85,

            vib_follow: 0.65,
            vib_base: 8.0,
            vib_tenuto_drop: 25.0,

            phrasing: true,
            phrase: PhraseConfig::default(),

            legato_vel_curve: vec![
                VelCurveAnchor {
                    io: 0.10,
                    vel: 122.0,
                },
                VelCurveAnchor {
                    io: 0.25,
                    vel: 104.0,
                },
                VelCurveAnchor {
                    io: 0.45,
                    vel: 82.0,
                },
                VelCurveAnchor {
                    io: 0.80,
                    vel: 50.0,
                },
                VelCurveAnchor {
                    io: 1.40,
                    vel: 22.0,
                },
                VelCurveAnchor { io: 2.50, vel: 8.0 },
            ],
            vel_first: 64.0,
            vel_accent_min: 101.0,

            marcato_fast: true,
            marcato_max_sec: 0.25,
            marcato_min_run: 4,
            marcato_vel_min: 70.0,
            marcato_vel_range: 50.0,

            solo_routing: true,
            solo_channel_base: 5,
            solo_max_voices: 2,

            max_channels: 16,

            tempo_map: None,
        }
    }
}

impl Config {
    /// Map a real-time inter-onset interval (seconds) to a legato-speed
    /// velocity by interpolating the configured curve — port of
    /// `legatoVelFromInterval`. Smaller interval (faster playing) → higher
    /// velocity → faster CSS legato.
    pub fn legato_vel_from_interval(&self, io_sec: f64) -> f64 {
        let a = &self.legato_vel_curve;
        if a.is_empty() {
            return 64.0;
        }
        if io_sec <= a[0].io {
            return a[0].vel;
        }
        if io_sec >= a[a.len() - 1].io {
            return a[a.len() - 1].vel;
        }
        for w in a.windows(2) {
            let (lo, hi) = (w[0], w[1]);
            if io_sec >= lo.io && io_sec <= hi.io {
                let t = (io_sec - lo.io) / (hi.io - lo.io);
                return lo.vel + (hi.vel - lo.vel) * t;
            }
        }
        a[a.len() - 1].vel
    }
}
