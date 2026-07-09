use std::path::PathBuf;

use keyflow_midi::import::MidiFile;

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run -p keyflow-midi --example list_markers -- <midi-path>");

    let bytes = std::fs::read(&path).expect("failed to read MIDI file");
    let midi = MidiFile::parse(&bytes).expect("failed to parse MIDI file");

    for (idx, marker) in midi.markers().iter().enumerate() {
        println!(
            "{:03}\ttick={}\ttype={:?}\t{}",
            idx + 1,
            marker.tick,
            marker.marker_type,
            marker.text
        );
    }
    println!("TOTAL={}", midi.markers().len());
}
