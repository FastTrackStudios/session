//! Derive karaoke artifacts (MIDI with lyric meta events, LRC) from aligned
//! word timings.
//!
//! The aligned [`crate::WordTiming`]s are *when* each word is sung (absolute
//! seconds). A Standard MIDI File at constant tempo maps tick→seconds linearly,
//! so placing a short note + a `Lyric` meta event (MIDI meta 0x05 — the karaoke
//! standard) at each word's tick makes a lyrics track that lines up with the
//! audio second-for-second when the project tempo matches.

use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
    num::u4,
};

use crate::align::WordTiming;
use crate::error::{Result, SyncError};

/// One word placed on the timeline.
#[derive(Debug, Clone)]
pub struct LyricEvent {
    pub word: String,
    pub start: f32,
    pub end: f32,
}

impl LyricEvent {
    pub fn from_words(words: &[WordTiming]) -> Vec<Self> {
        words
            .iter()
            .map(|w| LyricEvent {
                word: w.word.clone(),
                start: w.start,
                end: w.end,
            })
            .collect()
    }
}

const TPQN: u16 = 480; // ticks per quarter note

fn secs_to_ticks(secs: f32, bpm: f32) -> u32 {
    // ticks = seconds * (beats/sec) * ticks/beat
    (secs * (bpm / 60.0) * TPQN as f32).round().max(0.0) as u32
}

/// Render a karaoke Standard MIDI File: a tempo track plus a "Lyrics" track
/// with one note + `Lyric` meta event per word, placed at the aligned times.
/// `note` is the placeholder pitch (e.g. 60 = middle C) until real melody is wired.
pub fn lyrics_midi(events: &[LyricEvent], bpm: f32, note: u8) -> Result<Vec<u8>> {
    // Collect absolute-tick events, then convert to midly's delta-time tracks.
    // (tick, ordering, kind) — ordering keeps Lyric+NoteOn before NoteOff at a tie.
    let mut abs: Vec<(u32, u8, TrackEventKind)> = Vec::new();
    for e in events {
        let on = secs_to_ticks(e.start, bpm);
        let off = secs_to_ticks(e.end.max(e.start + 0.05), bpm).max(on + 1);
        abs.push((
            on,
            0,
            TrackEventKind::Meta(MetaMessage::Lyric(e.word.as_bytes())),
        ));
        abs.push((
            on,
            1,
            TrackEventKind::Midi {
                channel: 0.into(),
                message: MidiMessage::NoteOn {
                    key: note.into(),
                    vel: 80.into(),
                },
            },
        ));
        abs.push((
            off,
            2,
            TrackEventKind::Midi {
                channel: 0.into(),
                message: MidiMessage::NoteOff {
                    key: note.into(),
                    vel: 0.into(),
                },
            },
        ));
    }
    abs.sort_by_key(|(t, ord, _)| (*t, *ord));

    let mut lyric_track = Track::new();
    let mut prev = 0u32;
    for (tick, _, kind) in abs {
        let delta = tick - prev;
        prev = tick;
        lyric_track.push(TrackEvent {
            delta: delta.into(),
            kind,
        });
    }
    lyric_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    // Tempo track.
    let us_per_qn = (60_000_000.0 / bpm) as u32;
    let mut tempo_track = Track::new();
    tempo_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(us_per_qn.into())),
    });
    tempo_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(b"Lyrics")),
    });
    tempo_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(TPQN.into())),
        tracks: vec![tempo_track, lyric_track],
    };
    let mut buf = Vec::new();
    smf.write(&mut buf)
        .map_err(|e| SyncError::Sidecar(format!("midi write: {e}")))?;
    Ok(buf)
}

// ── multi-voice melody MIDI ─────────────────────────────────────────────────

/// One melody note for a voice: resolved MIDI pitch, optional syllable, and
/// absolute start/end seconds.
#[derive(Debug, Clone)]
pub struct VoiceNote {
    pub midi: u8,
    pub lyric: Option<String>,
    pub start: f32,
    pub end: f32,
}

/// A vocal part → its own MIDI track + channel, with a display color.
#[derive(Debug, Clone)]
pub struct VoiceTrack {
    pub name: String,
    pub color: Option<String>,
    pub channel: u8,
    pub notes: Vec<VoiceNote>,
}

/// Render a multi-track Standard MIDI File: a tempo track plus one track per
/// voice (each on its own MIDI channel), with real melody pitches and a `Lyric`
/// meta event on every note that carries a syllable. Track names are the voice
/// names; per-voice color is applied in the Reaper export, not the SMF.
pub fn voices_midi(voices: &[VoiceTrack], bpm: f32) -> Result<Vec<u8>> {
    let us_per_qn = (60_000_000.0 / bpm) as u32;
    let mut tempo_track = Track::new();
    tempo_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(us_per_qn.into())),
    });
    tempo_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    let mut tracks = vec![tempo_track];

    for v in voices {
        let ch: u4 = (v.channel & 0x0F).into();
        let mut abs: Vec<(u32, u8, TrackEventKind)> = Vec::new();
        for n in &v.notes {
            let on = secs_to_ticks(n.start, bpm);
            let off = secs_to_ticks(n.end.max(n.start + 0.05), bpm).max(on + 1);
            if let Some(l) = &n.lyric {
                abs.push((
                    on,
                    0,
                    TrackEventKind::Meta(MetaMessage::Lyric(l.as_bytes())),
                ));
            }
            abs.push((
                on,
                1,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::NoteOn {
                        key: n.midi.into(),
                        vel: 80.into(),
                    },
                },
            ));
            abs.push((
                off,
                2,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::NoteOff {
                        key: n.midi.into(),
                        vel: 0.into(),
                    },
                },
            ));
        }
        abs.sort_by_key(|(t, ord, _)| (*t, *ord));

        let mut track = Track::new();
        track.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(v.name.as_bytes())),
        });
        let mut prev = 0u32;
        for (tick, _, kind) in abs {
            track.push(TrackEvent {
                delta: (tick - prev).into(),
                kind,
            });
            prev = tick;
        }
        track.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        tracks.push(track);
    }

    let smf = Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(TPQN.into())),
        tracks,
    };
    let mut buf = Vec::new();
    smf.write(&mut buf)
        .map_err(|e| SyncError::Sidecar(format!("midi write: {e}")))?;
    Ok(buf)
}

/// Render an enhanced `.lrc` (per-word `<mm:ss.cc>` tags) — a portable karaoke
/// text format, handy for quick verification.
pub fn lyrics_lrc(events: &[LyricEvent]) -> String {
    let mut out = String::new();
    for e in events {
        let m = (e.start / 60.0) as u32;
        let s = e.start - (m as f32) * 60.0;
        out.push_str(&format!("[{:02}:{:05.2}]{}\n", m, s, e.word));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evt(w: &str, s: f32, e: f32) -> LyricEvent {
        LyricEvent {
            word: w.into(),
            start: s,
            end: e,
        }
    }

    #[test]
    fn writes_valid_midi_with_lyrics() {
        let events = vec![evt("hello", 1.0, 1.4), evt("world", 1.5, 2.0)];
        let bytes = lyrics_midi(&events, 120.0, 60).unwrap();
        // MThd magic + parses back.
        assert_eq!(&bytes[0..4], b"MThd");
        let smf = Smf::parse(&bytes).unwrap();
        assert_eq!(smf.tracks.len(), 2);
        let lyrics: Vec<_> = smf.tracks[1]
            .iter()
            .filter_map(|ev| match ev.kind {
                TrackEventKind::Meta(MetaMessage::Lyric(b)) => {
                    Some(std::str::from_utf8(b).unwrap())
                }
                _ => None,
            })
            .collect();
        assert_eq!(lyrics, ["hello", "world"]);
    }

    #[test]
    fn tick_conversion_scales_with_tempo() {
        // 1s at 120bpm = 2 beats = 2*480 ticks
        assert_eq!(secs_to_ticks(1.0, 120.0), 960);
        assert_eq!(secs_to_ticks(0.5, 120.0), 480);
    }

    #[test]
    fn lrc_format() {
        let lrc = lyrics_lrc(&[evt("hi", 65.5, 66.0)]);
        assert_eq!(lrc, "[01:05.50]hi\n");
    }
}
