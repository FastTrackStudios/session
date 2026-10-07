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

/// One child item under a folded item (a shut folder's row): where it
/// sits, and what it holds — see [`fold_waves`] and [`fold_notes`].
pub struct Child<'a> {
    /// Its position and length, in seconds.
    pub at: f64,
    pub length: f64,
    /// Its waveform, if it is audio and its peaks are read.
    pub wave: Option<&'a Wave>,
    /// Its notes, if it is MIDI and they are read.
    pub notes: Option<&'a [Note]>,
}

/// The most points a folded waveform holds, however long its span.
const FOLDED_POINTS: usize = 1 << 16;

/// A shut folder's item, `start..end` seconds, as the envelope of the
/// audio under it: per point, the loudest of every child sounding there.
///
/// At the finest of the children's resolutions, so folding loses nothing
/// a child showed open. `None` where no child under it has a waveform.
#[must_use]
pub fn fold_waves(start: f64, end: f64, children: &[Child<'_>]) -> Option<Wave> {
    let waves: Vec<(f64, &Wave)> = children
        .iter()
        .filter_map(|c| {
            c.wave
                .filter(|w| w.step > 0.0 && !w.points.is_empty())
                .map(|w| (c.at, w))
        })
        .collect();
    let finest = waves
        .iter()
        .map(|(_, w)| w.step)
        .fold(f64::INFINITY, f64::min);
    let span = end - start;
    if !finest.is_finite() || span <= 0.0 {
        return None;
    }
    let count = crate::num::index((span / finest).ceil()).clamp(1, FOLDED_POINTS);
    let step = span / crate::num::coord(count);
    let points = (0..count)
        .map(|k| {
            let from = crate::num::coord(k).mul_add(step, start);
            let to = from + step;
            waves
                .iter()
                .fold((0.0_f32, 0.0_f32), |(max, min), (at, wave)| {
                    if to <= *at {
                        return (max, min);
                    }
                    // The child's points across `from..to` (`index` floors a
                    // point before the child's start to its first).
                    let first = crate::num::index((from - at) / wave.step);
                    let last =
                        crate::num::index(((to - at) / wave.step).ceil()).min(wave.points.len());
                    wave.points
                        .get(first..last.max(first))
                        .unwrap_or_default()
                        .iter()
                        .fold((max, min), |(max, min), (hi, lo)| {
                            (max.max(*hi), min.min(*lo))
                        })
                })
        })
        .collect();
    Some(Wave { step, points })
}

/// A shut folder's item, `start..end` seconds, as the notes under it: each
/// child's notes placed where they sound and cut to the span, as fractions
/// of it. `None` where no child under it has notes there.
#[must_use]
pub fn fold_notes(start: f64, end: f64, children: &[Child<'_>]) -> Option<Vec<Note>> {
    let span = end - start;
    if span <= 0.0 {
        return None;
    }
    let mut out = Vec::new();
    for child in children {
        for note in child.notes.unwrap_or_default() {
            let from = f64::from(note.at).mul_add(child.length, child.at);
            let to = f64::from(note.len).mul_add(child.length, from);
            if to <= start || from >= end {
                continue;
            }
            let (from, to) = (from.max(start), to.min(end));
            out.push(Note {
                at: crate::num::narrow((from - start) / span),
                // Never zero: a note with no length cannot be seen.
                len: crate::num::narrow((to - from) / span).max(f32::MIN_POSITIVE),
                pitch: note.pitch,
                velocity: note.velocity,
            });
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Samples per point a lane asks the engine for: about forty points a
/// second at 44.1 kHz, finer than the pixels at the zoom a session opens
/// at and coarse enough to fold from the peaks cache's mipmap rather
/// than read the audio.
pub const WAVE_BLOCK: u32 = 1024;

/// How many items' peaks are read at once.
const WAVE_READS: usize = 16;

#[derive(Clone, Default)]
pub struct Previews {
    known: Arc<Mutex<HashMap<String, Vec<Note>>>>,
    /// The audio items' waveforms, by item GUID.
    waves: Arc<Mutex<HashMap<String, Arc<Wave>>>>,
    /// Bumped whenever notes or waveforms arrive — so a picture recorded
    /// before they did (a browser's waveforms come after the song opens)
    /// knows to record itself again.
    generation: Arc<std::sync::atomic::AtomicU64>,
}

impl Previews {
    /// The notes for an item, if they have been read.
    #[must_use]
    pub fn get(&self, guid: &str) -> Option<Vec<Note>> {
        self.known.lock().ok()?.get(guid).cloned()
    }

    /// How many times notes or waveforms have arrived: a holder of a
    /// picture drawn from them records it again when this has moved.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    fn arrived(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
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
        self.fill_waves_in(&project, guids).await;
    }

    /// [`Self::fill_waves`] for `project`, whichever is current.
    pub async fn fill_waves_in(&self, project: &daw_control::Project, guids: Vec<String>) {
        use futures_util::StreamExt as _;
        // Several at once: each is a decode or a peaks-file read the engine
        // answers on its own, and a song of nine hundred items read one at
        // a time held its opening for half a second.
        let mut reads = futures_util::stream::iter(guids)
            .map(|guid| async move { (read_wave(project, &guid).await, guid) })
            .buffer_unordered(WAVE_READS);
        while let Some((wave, guid)) = reads.next().await {
            let Some(wave) = wave else {
                continue;
            };
            if let Ok(mut waves) = self.waves.lock() {
                waves.insert(guid, Arc::new(wave));
            }
        }
        self.arrived();
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
        self.fill_in(&project, wanted).await;
    }

    /// [`Self::fill`] from a given project rather than the current one —
    /// a song a page opens in the background.
    pub async fn fill_in(&self, project: &daw_control::Project, wanted: Vec<(String, f64)>) {
        for (guid, length) in wanted {
            let notes = read(project, &guid, length).await;
            if let Ok(mut known) = self.known.lock() {
                known.insert(guid, notes);
            }
        }
        self.arrived();
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

#[cfg(test)]
mod fold_tests {
    use super::{Child, Note, Wave, fold_notes, fold_waves};

    fn flat(level: f32, points: usize) -> Wave {
        Wave {
            step: 0.1,
            points: vec![(level, -level); points],
        }
    }

    fn child<'a>(
        at: f64,
        length: f64,
        wave: Option<&'a Wave>,
        notes: Option<&'a [Note]>,
    ) -> Child<'a> {
        Child {
            at,
            length,
            wave,
            notes,
        }
    }

    /// A shut folder's item is the loudest of the children sounding at
    /// each moment, placed where each child sits — silent where none is.
    #[test]
    fn a_folded_wave_is_the_loudest_child_where_it_sounds() {
        let quiet = flat(0.2, 10); // 0..1 s
        let loud = flat(0.8, 10); // 2..3 s
        let both = flat(0.5, 30); // 0..3 s
        let wave = fold_waves(
            0.0,
            3.0,
            &[
                child(0.0, 1.0, Some(&quiet), None),
                child(2.0, 1.0, Some(&loud), None),
                child(0.0, 3.0, Some(&both), None),
            ],
        )
        .expect("a wave");
        let at = |secs: f64| wave.points[crate::num::index(secs / wave.step)];
        let near = |got: f32, want: f32| (got - want).abs() < 1e-6;
        assert!(
            near(at(0.45).0, 0.5),
            "the louder of the two over the first second"
        );
        assert!(near(at(1.5).0, 0.5), "only the long child in the middle");
        let (max, min) = at(2.5);
        assert!(
            near(max, 0.8) && near(min, -0.8),
            "the loud one where it sounds"
        );
        assert!(fold_waves(0.0, 1.0, &[child(0.0, 1.0, None, None)]).is_none());
    }

    /// Notes land where they sound in time, as fractions of the folded
    /// span; a note outside it is left out, one across its edge is cut.
    #[test]
    fn folded_notes_land_where_they_sound() {
        let notes = [
            Note {
                at: 0.0,
                len: 0.25,
                pitch: 60,
                velocity: 100,
            },
            Note {
                at: 0.5,
                len: 0.25,
                pitch: 64,
                velocity: 100,
            },
        ];
        // The child is 4 s long at 2 s: notes at 2..3 s and 4..5 s.
        let folded = fold_notes(2.0, 6.0, &[child(2.0, 4.0, None, Some(&notes))]).expect("notes");
        assert_eq!(folded.len(), 2);
        assert!(
            (folded[1].at - 0.5).abs() < 1e-6,
            "4 s is half way through 2..6 s"
        );
        let cut = fold_notes(2.5, 4.0, &[child(2.0, 4.0, None, Some(&notes))]).expect("notes");
        assert_eq!(cut.len(), 1, "the note at 4 s starts at the span's end");
        assert!(
            (cut[0].len - (0.5 / 1.5)).abs() < 1e-6,
            "cut to its half inside"
        );
    }
}
