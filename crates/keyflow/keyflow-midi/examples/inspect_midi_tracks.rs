use std::collections::BTreeMap;
use std::path::PathBuf;

use keyflow_midi::import::MidiFile;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .expect("usage: inspect_midi_tracks <file.mid> [start_tick] [end_tick]");
    let start_tick = args
        .next()
        .map(|arg| arg.to_string_lossy().parse::<u32>().unwrap())
        .unwrap_or(0);
    let end_tick = args
        .next()
        .map(|arg| arg.to_string_lossy().parse::<u32>().unwrap())
        .unwrap_or(u32::MAX);

    let bytes = std::fs::read(&path).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", path.display());
    });
    let midi = MidiFile::parse(&bytes).unwrap_or_else(|err| {
        panic!("failed to parse {}: {err}", path.display());
    });

    println!(
        "file={} ppq={} duration_ticks={}",
        path.display(),
        midi.ppq(),
        midi.duration_ticks()
    );

    for track in midi.tracks() {
        let mut by_channel: BTreeMap<u8, ChannelSummary> = BTreeMap::new();
        for note in &track.notes {
            by_channel
                .entry(note.channel)
                .or_default()
                .observe(note.start_tick, note.duration_ticks);
        }

        println!(
            "track={} name={:?} track_channel={:?} notes={} channels={:?}",
            track.index,
            track.name,
            track.channel,
            track.notes.len(),
            by_channel
        );

        let matching_notes = track
            .notes
            .iter()
            .filter(|note| {
                note.start_tick < end_tick
                    && note.start_tick.saturating_add(note.duration_ticks) > start_tick
            })
            .collect::<Vec<_>>();
        let notes_to_print = if start_tick == 0 && end_tick == u32::MAX {
            matching_notes.into_iter().take(24).collect::<Vec<_>>()
        } else {
            matching_notes
        };

        for note in notes_to_print {
            println!(
                "  ch={} pitch={} velocity={} start={} duration={} end={}",
                note.channel,
                note.pitch,
                note.velocity,
                note.start_tick,
                note.duration_ticks,
                note.start_tick.saturating_add(note.duration_ticks)
            );
        }
    }
}

#[derive(Debug, Default)]
struct ChannelSummary {
    count: usize,
    first_tick: Option<u32>,
    last_tick: u32,
    max_duration: u32,
}

impl ChannelSummary {
    fn observe(&mut self, start_tick: u32, duration_ticks: u32) {
        self.count += 1;
        self.first_tick = Some(
            self.first_tick
                .map_or(start_tick, |tick| tick.min(start_tick)),
        );
        self.last_tick = self
            .last_tick
            .max(start_tick.saturating_add(duration_ticks));
        self.max_duration = self.max_duration.max(duration_ticks);
    }
}
