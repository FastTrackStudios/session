//! signal's rigs on real assets: a bass capture and a multi-mic kit.
//!
//! Skipped (with a note) when the asset drive is not mounted.
#[cfg(test)]
mod assets {
    use std::path::{Path, PathBuf};

    use daw_standalone::plugin::{PluginEvents, PluginInstance, PluginMidiEvent};
    use signal_fx::{bass::BassRig, kit::KitMic};

    const RATE: f64 = 48_000.0;
    const BLOCK: usize = 512;

    fn bass_model() -> PathBuf {
        PathBuf::from(
            "/run/media/AudioHaven/Signal/NAM_models/Jason Z Tech21 dUg DP3X bass preamp pedal all dimed no shift.nam",
        )
    }

    fn kit_preset() -> PathBuf {
        PathBuf::from(
            "/run/media/AudioHaven/Signal/Libraries/Drum Kits/GGD Modern and Massive 2/Presets/Heavy Thwacky.signalpreset",
        )
    }

    fn present(path: &Path) -> bool {
        let here = path.exists();
        if !here {
            eprintln!("skip: {} is not mounted", path.display());
        }
        here
    }

    fn rms(samples: &[f32]) -> f32 {
        let n = u16::try_from(samples.len().min(60_000)).map_or(1.0, f32::from);
        (samples.iter().map(|s| s * s).sum::<f32>() / n.max(1.0)).sqrt()
    }

    /// `blocks` blocks of `plugin`'s output, with `midi` in the first.
    fn run(
        plugin: &mut dyn PluginInstance,
        input: &[f32],
        blocks: usize,
        midi: &[PluginMidiEvent],
    ) -> Vec<f32> {
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0.0_f32; BLOCK], vec![0.0_f32; BLOCK]);
        for b in 0..blocks {
            let at = b.saturating_mul(BLOCK);
            let slice: Vec<f32> = (0..BLOCK)
                .map(|i| input.get(at.saturating_add(i)).copied().unwrap_or(0.0))
                .collect();
            let events = PluginEvents {
                params: &[],
                midi: if b == 0 { midi } else { &[] },
                note_expressions: &[],
            };
            plugin
                .process_block(&slice, &slice, &mut l, &mut r, &events)
                .expect("process");
            out.extend_from_slice(&l);
        }
        out
    }

    /// The DP3X capture colours a bass note: sound comes out, finite, and
    /// not the note itself.
    #[test]
    fn the_bass_rig_plays_its_capture() {
        if !present(&bass_model()) {
            return;
        }
        let mut rig = BassRig::new(RATE);
        let state = serde_json::json!({ "amp": bass_model() }).to_string();
        rig.load_state(state.as_bytes()).expect("state");
        rig.prepare(RATE, 512).expect("prepare");
        let note: Vec<f32> = (0..BLOCK * 40)
            .map(|i| {
                let t = f32::from(u16::try_from(i % 60_000).unwrap_or(0)) / 48_000.0;
                0.3 * (t * 2.0 * std::f32::consts::PI * 55.0).sin()
            })
            .collect();
        let out = run(&mut rig, &note, 40, &[]);
        assert!(out.iter().all(|s| s.is_finite()));
        let (i, o) = (rms(&note[BLOCK * 20..]), rms(&out[BLOCK * 20..]));
        assert!(o > 1e-3, "sound out: {o}");
        let diff: f32 = rms(&note[BLOCK * 20..]
            .iter()
            .zip(&out[BLOCK * 20..])
            .map(|(a, b)| a - b)
            .collect::<Vec<_>>());
        assert!(
            diff > 0.05 * i.max(o),
            "coloured, not a wire: {diff} vs {i}/{o}"
        );
    }

    const fn note(key: u8, velocity: u8) -> PluginMidiEvent {
        PluginMidiEvent {
            offset: 10,
            message: daw_proto::MidiEvent::NoteOn {
                channel: daw_proto::Channel::new(9),
                key: daw_proto::KeyNumber::new(key),
                velocity: daw_proto::Velocity::new(velocity),
            },
        }
    }

    fn kit(pieces: Option<&[&str]>, mics: &[&str]) -> KitMic {
        let mut voices = KitMic::new(RATE);
        let state = serde_json::json!({
            "preset": kit_preset(),
            "pieces": pieces,
            "mics": mics,
        })
        .to_string();
        voices.load_state(state.as_bytes()).expect("state");
        voices.prepare(RATE, 512).expect("prepare");
        voices
    }

    /// A kick hit sounds on its close mics and on the overheads; a snare note
    /// does not sound on voices given only the kick.
    #[test]
    fn a_kick_note_plays_the_kick_where_it_is_asked() {
        if !present(&kit_preset()) {
            return;
        }
        let mut close = kit(Some(&["kick"]), &["close"]);
        let hit = run(&mut close, &[], 40, &[note(36, 110)]);
        assert!(hit.iter().all(|s| s.is_finite()));
        assert!(rms(&hit) > 1e-3, "the kick's close mics: {}", rms(&hit));

        let mut oh = kit(None, &["overhead"]);
        let over = run(&mut oh, &[], 40, &[note(36, 110)]);
        assert!(
            rms(&over) > 1e-4,
            "the kick in the overheads: {}",
            rms(&over)
        );

        let mut only_kick = kit(Some(&["kick"]), &["close"]);
        let snare = run(&mut only_kick, &[], 40, &[note(38, 110)]);
        assert!(
            rms(&snare) < 1e-6,
            "a snare note on kick voices: {}",
            rms(&snare)
        );
    }

    /// How alike two takes of the same length are: 1 the same, 0 unrelated.
    fn likeness(x: &[f32], y: &[f32]) -> f32 {
        let dot: f32 = x.iter().zip(y).map(|(p, q)| p * q).sum();
        let norms = x.iter().map(|p| p * p).sum::<f32>().sqrt()
            * y.iter().map(|q| q * q).sum::<f32>().sqrt();
        dot / norms.max(1e-12)
    }

    /// Two voices fed the same notes pick the same sample each hit — the
    /// round robin follows the notes — so a piece's mics on different tracks
    /// stay one hit. (The same sample, not the same samples: a hit's tail
    /// under the next is rendered voice by voice.) A voice one hit ahead in
    /// its round robin plays another sample: the control.
    #[test]
    fn the_same_notes_pick_the_same_samples() {
        if !present(&kit_preset()) {
            return;
        }
        let hits = [note(36, 100)];
        let mut first = kit(Some(&["kick"]), &["In 1"]);
        let mut second = kit(Some(&["kick"]), &["In 1"]);
        let mut ahead = kit(Some(&["kick"]), &["In 1"]);
        let _ = run(&mut ahead, &[], 20, &hits);
        for _ in 0..3 {
            let one = run(&mut first, &[], 20, &hits);
            let two = run(&mut second, &[], 20, &hits);
            let other = run(&mut ahead, &[], 20, &hits);
            let (same, control) = (likeness(&one, &two), likeness(&one, &other));
            assert!(same > 0.9999, "the same sample: {same}");
            assert!(control < 0.999, "the control is another: {control}");
        }
    }
}
