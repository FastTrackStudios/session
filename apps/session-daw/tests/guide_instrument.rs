//! Every note the guide generator writes is heard through the guide
//! instrument: the click and count always (synthesized if need be), and
//! each section cue from the FTS-GUIDE sample library when it is there.

use daw::plugin::{PluginEvents, PluginInstance, PluginMidiEvent};
use daw_proto::live_midi::{Channel, KeyNumber, MidiEvent, Velocity};
use session_daw::guide_instrument::{samples_dir, GuideInstrument};

const RATE: f64 = 48_000.0;

/// Play one note into a fresh instrument and return the loudest sample
/// over the next two seconds.
fn peak_of(note: u8) -> f32 {
    let mut guide = GuideInstrument::new();
    guide.prepare(RATE, 512).expect("prepare");
    let on = [PluginMidiEvent {
        offset: 0,
        message: MidiEvent::NoteOn {
            channel: Channel::new(0),
            key: KeyNumber::new(note),
            velocity: Velocity::new(100),
        },
    }];
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let silence = vec![0.0f32; 512];
    let mut peak = 0.0f32;
    for block in 0..(2 * 48_000 / 512) {
        let events = if block == 0 {
            PluginEvents { midi: &on, ..PluginEvents::EMPTY }
        } else {
            PluginEvents::EMPTY
        };
        guide
            .process_block(&silence, &silence, &mut l, &mut r, &events)
            .expect("process");
        peak = l.iter().chain(&r).fold(peak, |m, s| m.max(s.abs()));
    }
    peak
}

#[test]
fn the_click_and_count_sound() {
    for (what, note) in [("accent", 60), ("beat", 61), ("count 1", 72), ("count 4", 75)] {
        assert!(peak_of(note) > 0.01, "{what} (note {note}) is silent");
    }
}

#[test]
fn every_section_cue_sounds_when_the_library_is_installed() {
    if !samples_dir().join("Guide").is_dir() {
        eprintln!("(skip) no guide samples at {}", samples_dir().display());
        return;
    }
    let cues = [
        ("Verse", 84), ("Chorus", 85), ("Bridge", 86), ("Intro", 87), ("Outro", 88),
        ("Instrumental", 89), ("Pre-Chorus", 90), ("Post-Chorus", 91), ("Breakdown", 92),
        ("Interlude", 93), ("Tag", 94), ("End", 95), ("Solo", 96), ("Vamp", 97),
        ("Turnaround", 98), ("Refrain", 99),
    ];
    let silent: Vec<&str> = cues
        .iter()
        .filter(|(_, note)| peak_of(*note) <= 0.01)
        .map(|(name, _)| *name)
        .collect();
    assert!(silent.is_empty(), "silent cues: {silent:?}");
}
