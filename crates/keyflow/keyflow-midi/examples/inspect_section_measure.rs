use std::path::PathBuf;

use keyflow_midi::import::MidiFile;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().map(PathBuf::from).expect("midi path");
    let bytes = std::fs::read(&path).unwrap();
    let midi = MidiFile::parse(&bytes).unwrap();
    let ppq = midi.ppq() as i64;
    let ticks_per_measure = ppq * 4;

    let Some(section_name) = args.next() else {
        for section in midi.section_markers_absolute() {
            println!(
                "{} start_measure={} length={:?}",
                section.name,
                section
                    .explicit_start_measure
                    .unwrap_or(section.position.measure),
                section.explicit_length
            );
        }
        return;
    };
    let local_measure: i32 = args
        .next()
        .expect("local measure (1-based)")
        .parse()
        .expect("measure number");

    let section = midi
        .section_markers_absolute()
        .into_iter()
        .find(|s| s.name == section_name)
        .unwrap_or_else(|| panic!("section not found: {section_name}"));

    let start_measure = section
        .explicit_start_measure
        .unwrap_or(section.position.measure);
    let target_measure = start_measure + local_measure - 1;
    let start_tick = i64::from(section.tick) + (i64::from(local_measure - 1) * ticks_per_measure);
    let end_tick = start_tick + ticks_per_measure;

    println!(
        "section={} start_measure={} local_measure={} target_measure={} start_tick={} end_tick={}",
        section_name, start_measure, local_measure, target_measure, start_tick, end_tick
    );

    for note in midi.all_notes() {
        let note_start = note.start_tick as i64;
        let note_end = note_start + note.duration_ticks as i64;
        let overlap_start = note_start.max(start_tick);
        let overlap_end = note_end.min(end_tick);
        if overlap_end > overlap_start {
            println!(
                "ch={} pitch={} start={} end={} dur={} clipped_start={} clipped_end={}",
                note.channel,
                note.pitch,
                note_start,
                note_end,
                note.duration_ticks,
                overlap_start,
                overlap_end
            );
        }
    }
}
