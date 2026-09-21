//! The guide instrument: what makes the Click, Count and Guide MIDI tracks
//! audible on `daw-standalone`.
//!
//! Session writes the click, the count-in and the spoken section cues as
//! MIDI notes (`session_guide::midi`: 60–65 click, 72–79 count, 84–102
//! cues). This is the instrument on those tracks: `session_guide`'s
//! `GuideEngine` in MIDI mode — the same DSP the FTS Guide CLAP/VST3
//! plugin wraps — running in-process as a native `PluginInstance`, so the
//! renderer feeds it each track's notes like any hosted synth.
//!
//! A spoken cue and the count note under it are both in the MIDI — on
//! the Guide and Count tracks — and the cue takes the count's place in the
//! audio: "Verse, 2, 3, 4". The generator tags each count note a cue lands
//! on (`COUNT_UNDER_CUE_VELOCITY`); the instrument on the Guide track
//! (`fts.guide:guide`) reports every block that it is playing unmuted, and
//! the Count track's instrument skips a tagged note while it is. Mute the
//! Guide track and the "1" is back. Mute is a slow fact, so reading the
//! last block's report is enough — which makes it hold in any track order.
//!
//! Samples come from the FTS-GUIDE library at
//! `~/.config/fts/guide-samples/{Click,Counts,Guide}`; anything missing is
//! synthesized (clicks and count beeps), except spoken cues, which need
//! the real samples.

use std::path::PathBuf;
use std::sync::Arc;

use daw::plugin::{
    FxFactory, PluginDescriptor, PluginError, PluginEvents, PluginFormat, PluginInstance,
    PluginParamInfo,
};
use daw_proto::live_midi::MidiEventExt;
use session_guide::{BlockClock, ClickSound, GuideConfig, GuideEngine, TriggerSource};

/// The FX ident the guide generator puts on its tracks.
pub const IDENT: &str = "fts.guide";

/// Where the FTS-GUIDE sample library lives.
#[must_use]
pub fn samples_dir() -> PathBuf {
    std::env::var_os("FTS_GUIDE_SAMPLES").map_or_else(
        || {
            std::env::var_os("HOME")
                .map_or_else(|| PathBuf::from("."), PathBuf::from)
                .join(".config/fts/guide-samples")
        },
        PathBuf::from,
    )
}

/// Whether the Guide track's instrument is playing unmuted — shared by the
/// guide instruments of one engine.
#[derive(Default)]
pub struct CueGate {
    /// The last render cycle the Guide track's instrument ran unmuted in.
    guide_alive: std::sync::atomic::AtomicU64,
}

impl CueGate {
    fn guide_playing_in(&self, cycle: u64) {
        // +1 so zero means "never".
        self.guide_alive
            .fetch_max(cycle + 1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether the Guide track played this block or the one before —
    /// whichever order the two tracks render in.
    fn guide_is_playing(&self, cycle: u64) -> bool {
        let seen = self.guide_alive.load(std::sync::atomic::Ordering::Relaxed);
        seen != 0 && seen + 1 >= cycle
    }
}

/// Which guide track an instrument plays for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Click,
    Count,
    Guide,
    /// Named plainly (`fts.guide`): plays whatever it is given.
    Any,
}

impl Role {
    fn of(fx_name: &str) -> Option<Self> {
        match fx_name.strip_prefix(IDENT)? {
            "" => Some(Self::Any),
            ":click" => Some(Self::Click),
            ":count" => Some(Self::Count),
            ":guide" => Some(Self::Guide),
            _ => None,
        }
    }
}

/// One guide engine, fed from its track's MIDI.
pub struct GuideInstrument {
    engine: Option<GuideEngine>,
    sample_rate: f64,
    gate: Arc<CueGate>,
    role: Role,
}

impl GuideInstrument {
    #[must_use]
    pub fn new() -> Self {
        Self::for_role(Role::Any, Arc::new(CueGate::default()))
    }

    /// An instrument for one guide track, sharing `gate` with the others.
    #[must_use]
    pub fn for_role(role: Role, gate: Arc<CueGate>) -> Self {
        Self {
            engine: None,
            sample_rate: 48_000.0,
            gate,
            role,
        }
    }
}

impl Default for GuideInstrument {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginInstance for GuideInstrument {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: IDENT.to_owned(),
            name: "FTS Guide".to_owned(),
            vendor: "FastTrackStudio".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            format: PluginFormat::Synthetic,
        }
    }

    fn params(&mut self) -> Vec<PluginParamInfo> {
        Vec::new()
    }
    fn param_value(&mut self, _id: u32) -> Option<f64> {
        None
    }
    fn value_to_text(&mut self, _id: u32, _value: f64) -> Option<String> {
        None
    }
    fn text_to_value(&mut self, _id: u32, _text: &str) -> Option<f64> {
        None
    }
    fn latency(&mut self) -> u32 {
        0
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: u32) -> Result<(), PluginError> {
        // MIDI mode: play only the notes handed in, never a grid of its
        // own — the track's notes ARE the click.
        let config = GuideConfig {
            source: TriggerSource::Midi,
            enable_count: true,
            enable_guide: true,
            ..GuideConfig::default()
        };
        let mut engine = GuideEngine::new(config);
        let dir = samples_dir();
        // The bank loads at an integer device rate.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a sample rate: a small positive integer carried as f64"
        )]
        let rate = sample_rate.round() as u32;
        let bank = engine.bank_mut();
        bank.load_click(&dir.join("Click"), ClickSound::Cowbell, rate);
        // On-beats high, off-beat eighths low, the bar's one the same as
        // any other beat — see `SampleBank::beats_high_offbeats_low`.
        bank.beats_high_offbeats_low();
        bank.load_counts(&dir.join("Counts"), "English Female", rate);
        bank.load_guide_dir(&dir.join("Guide"), rate);
        bank.synthesize_defaults(rate);
        self.engine = Some(engine);
        self.sample_rate = sample_rate;
        Ok(())
    }

    fn is_prepared(&self) -> bool {
        self.engine.is_some()
    }

    fn process_block(
        &mut self,
        _in_l: &[f32],
        _in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        events: &PluginEvents<'_>,
    ) -> Result<(), PluginError> {
        out_l.fill(0.0);
        out_r.fill(0.0);
        let Some(engine) = self.engine.as_mut() else {
            return Ok(());
        };
        let cycle = daw::plugin::render_cycle();
        // The Guide track's instrument says it is being heard.
        if self.role == Role::Guide
            && !daw::plugin::track_muted()
            && let Some(cycle) = cycle
        {
            self.gate.guide_playing_in(cycle);
        }
        let guide_playing = cycle.is_some_and(|c| self.gate.guide_is_playing(c));
        for event in events.midi {
            // Note-on with velocity: status 0x9n, velocity > 0.
            let Some((status, key, velocity)) = event.message.to_raw_bytes() else {
                continue;
            };
            if status & 0xF0 != 0x90 || velocity == 0 {
                continue;
            }
            let Some(trigger) = session_guide::midi::trigger_for_midi_note(key) else {
                continue;
            };
            let under_cue = velocity == session_guide::midi::COUNT_UNDER_CUE_VELOCITY;
            if matches!(trigger, session_guide::GuideTrigger::Count(_)) && under_cue && guide_playing {
                continue; // the cue says it instead
            }
            engine.trigger(event.offset as usize, trigger);
        }
        // MIDI mode ignores the transport; the rate is all it reads.
        let clock = BlockClock {
            playing: true,
            pos_seconds: 0.0,
            pos_beats: 0.0,
            tempo_bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            sample_rate: self.sample_rate,
        };
        engine.render_stereo(out_l, out_r, &clock);
        Ok(())
    }

    fn deactivate(&mut self) {
        self.engine = None;
    }
}

/// Makes guide instruments for `Effects::add(.., "fts.guide")`, all sharing
/// one [`CueGate`].
#[derive(Default)]
pub struct GuideFxFactory {
    gate: Arc<CueGate>,
}

impl FxFactory for GuideFxFactory {
    fn installed(&self) -> Vec<daw_proto::fx::InstalledFx> {
        Vec::new()
    }

    fn create(&self, name_or_ident: &str, sample_rate: f64) -> Option<Box<dyn PluginInstance>> {
        let role = Role::of(name_or_ident)?;
        // Prepared here, on the thread adding the FX — loading ~20 MB of
        // samples. The renderer only prepares a plugin that is not
        // prepared yet, and it does that on the AUDIO thread: left to it,
        // the first block after pressing play took 40-60 ms, a dropout.
        let mut guide = GuideInstrument::for_role(role, self.gate.clone());
        guide.prepare(sample_rate, 512).ok()?;
        Some(Box::new(guide))
    }
}

/// Install the guide instrument's factory on `daw`.
pub fn install(daw: &daw::standalone::Standalone) {
    daw.set_fx_factory(Arc::new(GuideFxFactory::default()));
}
