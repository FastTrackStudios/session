//! `impl AudioAccessors for Standalone` — decode a take's source file.
//!
//! REAPER's audio accessor is a handle you pull rendered samples from,
//! at whatever rate and channel count you ask for. Standalone has no
//! render graph to pull from, but it does have the take's source file,
//! and decoding it gives the same thing: the audio that take plays.
//!
//! That is enough for the callers this exists for — the expression
//! editor's audio session, and anything else that wants to *analyse* a
//! take rather than hear it. What it deliberately does not do is apply
//! item FX or track processing: neither is modelled here, and a caller
//! that got FX from REAPER and dry audio from standalone would be
//! comparing two different signals without being told.
//!
//! Accessors are handles rather than values because that is the shape
//! REAPER's API has and the facade follows it. Here a handle owns the
//! decoded buffer, so `get_samples` is a copy rather than a decode and
//! reading a take in chunks costs one decode rather than one per chunk.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use daw_proto::{
    AudioAccessors, AudioSampleData, GetSamplesRequest, ItemRef, ProjectContext, TakeRef, TrackRef,
};

use crate::sync::Standalone;

#[cfg(any(feature = "audio", feature = "decode"))]
type OpenAccessor = crate::take_reader::TakeReader;

/// Without a decoder there is no audio to read; the table holds nothing.
#[cfg(not(any(feature = "audio", feature = "decode")))]
struct OpenAccessor;

/// Open accessors, keyed by the handle handed out.
///
/// Process-global rather than per-`Standalone`, because `Standalone` is
/// a cloneable handle to shared state and an accessor created through
/// one clone has to be readable through another — the same way a
/// REAPER accessor is valid wherever the API is.
fn table() -> &'static Mutex<HashMap<String, Arc<OpenAccessor>>> {
    static TABLE: OnceLock<Mutex<HashMap<String, Arc<OpenAccessor>>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_id() -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    format!("sa-acc-{}", N.fetch_add(1, Ordering::Relaxed))
}

#[cfg(any(feature = "audio", feature = "decode"))]
fn open(
    daw: &Standalone,
    project: ProjectContext,
    item: ItemRef,
    take: TakeRef,
) -> Option<OpenAccessor> {
    crate::take_reader::TakeReader::open_or_decode(daw, project, item, take)
}

#[cfg(not(any(feature = "audio", feature = "decode")))]
fn open(
    _daw: &Standalone,
    _project: ProjectContext,
    _item: ItemRef,
    _take: TakeRef,
) -> Option<OpenAccessor> {
    // Without the decoder there is no way to read a source file, and
    // returning silence would be worse than declining: a caller would
    // analyse it and report a take with no notes.
    None
}

impl AudioAccessors for Standalone {
    /// Not supported: a track's audio is its items mixed through its
    /// FX, and standalone models neither the mix nor the FX.
    ///
    /// `None` rather than a silent buffer, so a caller finds out here
    /// instead of concluding the track is empty.
    fn create_track_accessor(&self, _project: ProjectContext, _track: TrackRef) -> Option<String> {
        None
    }

    fn create_take_accessor(
        &self,
        project: ProjectContext,
        item: ItemRef,
        take: TakeRef,
    ) -> Option<String> {
        let open = open(self, project, item, take)?;
        let id = next_id();
        table().lock().ok()?.insert(id.clone(), Arc::new(open));
        Some(id)
    }

    /// Always false: standalone's sources are files on disk, and this
    /// exists for REAPER's case of an item being re-rendered underneath
    /// an open accessor.
    fn has_state_changed(&self, _accessor_id: &str) -> bool {
        false
    }

    // r[impl drums.open.accessor-placement]
    #[cfg(any(feature = "audio", feature = "decode"))]
    fn get_samples(&self, request: GetSamplesRequest) -> AudioSampleData {
        let Some(acc) = table()
            .lock()
            .ok()
            .and_then(|t| t.get(&request.accessor_id).cloned())
        else {
            return AudioSampleData::default();
        };
        let src_channels = acc.channels() as u32;
        let src_rate = acc.sample_rate() as f64;

        // A zero in either field means "whatever you have", which is
        // how a caller discovers the format before reading in earnest.
        let out_channels = if request.num_channels == 0 {
            src_channels
        } else {
            request.num_channels
        };
        let rate = if request.sample_rate <= 0.0 {
            src_rate
        } else {
            request.sample_rate
        };
        let want = request.num_samples as usize;
        if want == 0 || out_channels == 0 {
            return AudioSampleData {
                samples: Vec::new(),
                sample_rate: src_rate,
                num_channels: src_channels,
                num_samples: 0,
            };
        }

        // Each output frame is a take time; the reader maps it through
        // placement and markers to a source frame and interpolates. A
        // request at a foreign rate is therefore resampled linearly —
        // good enough for analysis; a caller that wants the exact
        // samples reads at the source rate the probe reported.
        let src_ch = src_channels.max(1) as usize;
        let mut samples = Vec::with_capacity(want * out_channels as usize);
        for i in 0..want {
            let t = request.start_time + i as f64 / rate;
            for ch in 0..out_channels as usize {
                // Fewer source channels than asked for: repeat the last
                // one, so a mono file read as stereo is centred rather
                // than half-silent.
                let sc = ch.min(src_ch - 1);
                samples.push(acc.sample(t, sc) as f64);
            }
        }

        AudioSampleData {
            samples,
            sample_rate: rate,
            num_channels: out_channels,
            num_samples: want as u32,
        }
    }

    #[cfg(not(any(feature = "audio", feature = "decode")))]
    fn get_samples(&self, _request: GetSamplesRequest) -> AudioSampleData {
        AudioSampleData::default()
    }

    fn destroy_accessor(&self, accessor_id: &str) {
        if let Ok(mut t) = table().lock() {
            t.remove(accessor_id);
        }
    }
}
