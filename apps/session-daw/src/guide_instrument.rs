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

/// One guide engine, fed from its track's MIDI.
pub struct GuideInstrument {
    engine: Option<GuideEngine>,
    sample_rate: f64,
}

impl GuideInstrument {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            engine: None,
            sample_rate: 48_000.0,
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
        for event in events.midi {
            // Note-on with velocity: status 0x9n, velocity > 0.
            let Some((status, key, velocity)) = event.message.to_raw_bytes() else {
                continue;
            };
            if status & 0xF0 != 0x90 || velocity == 0 {
                continue;
            }
            if let Some(trigger) = session_guide::midi::trigger_for_midi_note(key) {
                engine.trigger(event.offset as usize, trigger);
            }
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

/// Makes guide instruments for `Effects::add(.., "fts.guide")`.
pub struct GuideFxFactory;

impl FxFactory for GuideFxFactory {
    fn installed(&self) -> Vec<daw_proto::fx::InstalledFx> {
        Vec::new()
    }

    fn create(&self, name_or_ident: &str, _sample_rate: f64) -> Option<Box<dyn PluginInstance>> {
        (name_or_ident == IDENT).then(|| Box::new(GuideInstrument::new()) as Box<dyn PluginInstance>)
    }
}

/// Install the guide instrument's factory on `daw`.
pub fn install(daw: &daw::standalone::Standalone) {
    daw.set_fx_factory(Arc::new(GuideFxFactory));
}
