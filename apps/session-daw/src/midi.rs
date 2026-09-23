//! The notes inside an item, for drawing it.
//!
//! An item's body should be what the item CONTAINS. Audio has peaks and
//! MIDI has notes, and until now every item in the arrangement was
//! drawn from the same synthetic envelope — a function of the track's
//! index and the time, which never touched the item, the take, or a
//! single sample. That is why a chord track looked like a shaker.
//!
//! Notes arrive the way peaks were always meant to: after the window is
//! already standing. Reading them is a round trip per item, and a
//! session with four hundred MIDI items would spend the whole open
//! doing it — so the window draws what it has, asks for the rest, and
//! redraws as the answers land.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One note, reduced to what a preview draws.
///
/// Not `MidiNote`: a preview needs where and how high, and carries
/// neither channel nor selection nor mute. Reduced at the edge so the
/// drawing has nothing to decide and the cache stays small — a big
/// session holds a lot of these.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Note {
    /// Where it starts, as a fraction of the item's length.
    ///
    /// A fraction rather than a time, because an item preview is drawn
    /// to the item's width and that is the only thing it is measured
    /// against. It also makes the cache independent of the zoom, so a
    /// scroll never invalidates one.
    pub at: f32,
    /// How long, as the same fraction. Never zero: a note with no
    /// length is a note you cannot see.
    pub len: f32,
    /// MIDI pitch, 0..=127.
    pub pitch: u8,
    /// How hard, 0..=127 — the preview draws it as brightness.
    pub velocity: u8,
}

/// The notes of the items that have been read so far.
///
/// Absent means "not asked yet or still reading", and an empty list
/// means "read, and it has none". The two have to be distinguishable:
/// an item with no notes should stop being asked about, and an item
/// still loading should not be drawn as empty.
/// An audio item's waveform, reduced to what a lane draws: the loudest
/// and the lowest the take reaches across each stretch, every channel
/// folded into one, from the item's own start.
///
/// The take's peaks, not the source's — the engine has already applied
/// the item's slip, play rate and stretch, so point `i` is item time
/// `i * step` and nothing here has to know how it got there.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave {
    /// Seconds each point stands for.
    pub step: f64,
    /// `(max, min)` per point, in −1..1.
    pub points: Vec<(f32, f32)>,
}

impl Wave {
    /// Fold the engine's per-channel `[min, max, …]` blocks into one
    /// envelope. `None` for a take with no audio to show.
    #[must_use]
    pub fn from_peaks(data: &daw_proto::TakePeakData) -> Option<Self> {
        let channels = usize::try_from(data.num_channels.max(1)).ok()?;
        let stride = channels * 2;
        if data.peaks.len() < stride || data.sample_rate <= 0.0 {
            return None;
        }
        let points = data
            .peaks
            .chunks_exact(stride)
            .map(|block| {
                let (mut max, mut min) = (0.0_f64, 0.0_f64);
                for pair in block.chunks_exact(2) {
                    min = min.min(pair[0]);
                    max = max.max(pair[1]);
                }
                #[expect(clippy::cast_possible_truncation, reason = "a level in −1..1")]
                (max as f32, min as f32)
            })
            .collect();
        Some(Self {
            step: f64::from(data.samples_per_peak.max(1)) / data.sample_rate,
            points,
        })
    }
}

/// Samples per point a lane asks the engine for: about forty points a
/// second at 44.1 kHz, finer than the pixels at the zoom a session opens
/// at and coarse enough to fold from the peaks cache's mipmap rather
/// than read the audio.
pub const WAVE_BLOCK: u32 = 1024;

#[derive(Clone, Default)]
pub struct Previews {
    known: Arc<Mutex<HashMap<String, Vec<Note>>>>,
    /// The audio items' waveforms, by item GUID.
    waves: Arc<Mutex<HashMap<String, Arc<Wave>>>>,
}

impl Previews {
    /// The notes for an item, if they have been read.
    #[must_use]
    pub fn get(&self, guid: &str) -> Option<Vec<Note>> {
        self.known.lock().ok()?.get(guid).cloned()
    }

    /// The waveform for an audio item, if its peaks have been read.
    #[must_use]
    pub fn wave(&self, guid: &str) -> Option<Arc<Wave>> {
        self.waves.lock().ok()?.get(guid).cloned()
    }

    /// Read the peaks of every audio item named, now, on this thread —
    /// [`Self::fill_blocking`]'s twin, for the same reason: a window that
    /// opens on blank lanes and fills them in reads as a broken session.
    pub fn fill_waves_blocking(&self, guids: Vec<String>) {
        let Some(runtime) = crate::open::runtime() else {
            return;
        };
        runtime.block_on(self.fill_waves(guids));
    }

    /// Read the peaks, awaited — the web build's way in.
    pub async fn fill_waves(&self, guids: Vec<String>) {
        let Some(daw) = daw::rpc::Daw::try_get() else {
            return;
        };
        let Ok(project) = daw.current_project().await else {
            return;
        };
        for guid in guids {
            let Some(wave) = read_wave(&project, &guid).await else {
                continue;
            };
            if let Ok(mut waves) = self.waves.lock() {
                waves.insert(guid, Arc::new(wave));
            }
        }
    }

    /// Read the notes now, on this thread.
    ///
    /// For a renderer rather than a window: something drawing one frame
    /// and exiting has no later to redraw in, so an empty cache would
    /// mean every MIDI item drawn blank — a worse picture than the one
    /// this replaced.
    pub fn fill_blocking(&self, wanted: Vec<(String, f64)>) {
        let Some(runtime) = crate::open::runtime() else {
            return;
        };
        runtime.block_on(self.fill(wanted));
    }

    /// Read the notes, awaited: [`Self::fill_blocking`] where there is no
    /// thread to block (the web build).
    pub async fn fill(&self, wanted: Vec<(String, f64)>) {
        let Some(daw) = daw::rpc::Daw::try_get() else {
            return;
        };
        let Ok(project) = daw.current_project().await else {
            return;
        };
        for (guid, length) in wanted {
            let notes = read(&project, &guid, length).await;
            if let Ok(mut known) = self.known.lock() {
                known.insert(guid, notes);
            }
        }
    }
}

/// Read one audio item's waveform from its active take's peaks. `None`
/// for an item the engine has no audio for — drawn plain, not faked.
async fn read_wave(project: &daw_control::Project, guid: &str) -> Option<Wave> {
    let item = project.items().by_guid(guid).await.ok()??;
    let data = item.active_take().peaks(WAVE_BLOCK).await.ok()?;
    Wave::from_peaks(&data)
}

/// Read one item's notes, as fractions of its length.
///
/// An empty answer is a real answer — an empty MIDI item is a thing
/// that exists, and drawing it as blank is right.
async fn read(project: &daw_control::Project, guid: &str, length: f64) -> Vec<Note> {
    let Ok(Some(item)) = project.items().by_guid(guid).await else {
        return Vec::new();
    };
    let Ok(notes) = item.active_take().midi().notes().await else {
        return Vec::new();
    };
    // The span the notes actually cover, because a take's PPQ tells us
    // ticks per quarter note and not how many quarter notes the ITEM
    // is. Measuring the notes themselves means the preview fills the
    // block whatever the tempo, which is what it is for: a shape, not a
    // clock.
    let span = notes
        .iter()
        .map(|n| n.start_ppq + n.length_ppq)
        .fold(0.0_f64, f64::max);
    let span = if span > 0.0 { span } else { length.max(1.0) };
    notes
        .iter()
        .map(|note| Note {
            at: (note.start_ppq / span) as f32,
            len: (note.length_ppq / span) as f32,
            pitch: note.pitch,
            velocity: note.velocity,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Note, Previews};

    /// Absent and empty mean different things.
    ///
    /// An item still being read must not draw as an empty one: a chord
    /// track that flashed blank on every open and then filled in would
    /// read as a bug in the session, not as a load.
    #[test]
    fn nothing_read_yet_is_not_the_same_as_nothing_there() {
        let previews = Previews::default();
        assert_eq!(previews.get("unread"), None);
        previews
            .known
            .lock()
            .unwrap()
            .insert("empty".into(), Vec::new());
        assert_eq!(previews.get("empty"), Some(Vec::new()));
    }

    /// The shape a preview is drawn from: fractions, so the cache
    /// survives a zoom.
    #[test]
    fn a_note_is_a_fraction_of_its_item() {
        let note = Note {
            at: 0.25,
            len: 0.5,
            pitch: 60,
            velocity: 100,
        };
        assert!(note.at + note.len <= 1.0, "a note ran past its item");
    }
}

#[cfg(test)]
mod wave_tests {
    use super::Wave;

    /// Channels fold into one envelope — the loudest of either, the lowest
    /// of either — and a point stands for its block's worth of seconds.
    #[test]
    fn channels_fold_and_the_step_is_the_block() {
        let data = daw_proto::TakePeakData {
            sample_rate: 48_000.0,
            num_channels: 2,
            // [ch0 min, ch0 max, ch1 min, ch1 max] per block
            peaks: vec![-0.2, 0.5, -0.7, 0.1, -0.1, 0.0, -0.3, 0.9],
            samples_per_peak: 480,
        };
        let wave = Wave::from_peaks(&data).expect("a wave");
        assert!((wave.step - 0.01).abs() < 1e-12);
        assert_eq!(wave.points, vec![(0.5, -0.7), (0.9, -0.3)]);
    }

    /// A take with no audio has no wave, not a flat one.
    #[test]
    fn no_peaks_is_no_wave() {
        let data = daw_proto::TakePeakData {
            peaks: Vec::new(),
            ..daw_proto::TakePeakData::default()
        };
        assert_eq!(Wave::from_peaks(&data), None);
    }
}
