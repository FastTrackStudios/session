use std::path::PathBuf;

use keyflow_midi::import::MidiFile;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().map(PathBuf::from).unwrap();
    let range = args
        .next()
        .and_then(|s| {
            let (start, end) = s.split_once("..=")?;
            Some((start.parse::<i64>().ok()?, end.parse::<i64>().ok()?))
        })
        .unwrap_or((107, 111));
    let bytes = std::fs::read(&path).unwrap();
    let midi = MidiFile::parse(&bytes).unwrap();
    let ppq = midi.ppq() as i64;
    let ticks_per_measure = ppq * 4;
    for note in midi.all_notes() {
        let start = note.start_tick as i64;
        let end = start + note.duration_ticks as i64;
        let raw_measure = (start / ticks_per_measure) + 1;
        if (range.0..=range.1).contains(&raw_measure) {
            println!(
                "raw_measure={} start={} end={} dur={} ch={} pitch={}",
                raw_measure, start, end, note.duration_ticks, note.channel, note.pitch
            );
        }
    }
}
